//! snap_targets : DatasetSource × SnapScope → streamed declared SnapTarget sets.
//! Reads manifest and selected snap artifacts only; never loads the drainage graph.

use std::collections::BTreeSet;
use std::sync::Arc;

use geo::{Geometry, Intersects, Rect};
use hfx::SnapTarget;
use tracing::instrument;

use crate::error::SessionError;
use crate::reader::manifest::SnapDecl;
use crate::reader::snap_store::{SnapOpenMode, SnapStore};
use crate::source::DatasetSource;

/// A finite EPSG:4326 rectangle, with west < east and south < north.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapExtent(Rect<f64>);

impl SnapExtent {
    /// Parse west, south, east, north coordinates without losing f64 precision.
    ///
    /// # Errors
    /// Returns an error for non-finite, reversed, empty or out-of-range bounds.
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Result<Self, SnapTargetsError> {
        if ![west, south, east, north].iter().all(|v| v.is_finite())
            || west < -180.0
            || east > 180.0
            || south < -90.0
            || north > 90.0
            || west >= east
            || south >= north
        {
            return Err(SnapTargetsError::InvalidExtent {
                west,
                south,
                east,
                north,
            });
        }
        Ok(Self(Rect::new((west, south), (east, north))))
    }

    pub fn bounds(self) -> [f64; 4] {
        [
            self.0.min().x,
            self.0.min().y,
            self.0.max().x,
            self.0.max().y,
        ]
    }
}

/// Supplied EPSG:4326 float32 bounds; equal axes are valid for points and lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapBounds([f32; 4]);

impl SnapBounds {
    /// Parse supplied bounds without padding, rounding or substituting geometry bounds.
    ///
    /// # Errors
    /// Reports non-finite, reversed or out-of-range bounds.
    pub fn new(xmin: f32, ymin: f32, xmax: f32, ymax: f32) -> Result<Self, SnapTargetsError> {
        if ![xmin, ymin, xmax, ymax].iter().all(|v| v.is_finite())
            || xmin < -180.0
            || xmax > 180.0
            || ymin < -90.0
            || ymax > 90.0
            || xmin > xmax
            || ymin > ymax
        {
            return Err(SnapTargetsError::InvalidBounds {
                xmin,
                ymin,
                xmax,
                ymax,
            });
        }
        Ok(Self([xmin, ymin, xmax, ymax]))
    }

    pub fn bounds(self) -> [f32; 4] {
        self.0
    }
}

/// An extracted target with its exact optional supplied bounds.
/// `target` retains the source attributes and WKB; its strict HFX bbox is unused.
#[derive(Debug, Clone)]
pub struct SnapTargetRow {
    pub target: SnapTarget,
    pub bbox: Option<SnapBounds>,
}

/// Explicit extraction scope; there is no implicit whole-dataset selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapScope {
    All,
    Bbox(SnapExtent),
}

impl SnapScope {
    pub(crate) fn intersects(self, geometry: &Geometry<f64>) -> bool {
        match self {
            Self::All => true,
            Self::Bbox(extent) => geometry.intersects(&extent.0),
        }
    }

    pub(crate) fn may_intersect(self, bbox: &hfx::BoundingBox) -> bool {
        match self {
            Self::All => true,
            Self::Bbox(extent) => {
                let [west, south, east, north] = extent.bounds();
                // Cover f32 rounding at the boundary. Geometry filtering remains f64.
                f64::from(bbox.min_x().get().next_down()) <= east
                    && f64::from(bbox.max_x().get().next_up()) >= west
                    && f64::from(bbox.min_y().get().next_down()) <= north
                    && f64::from(bbox.max_y().get().next_up()) >= south
            }
        }
    }
}

/// Declarations for selected sets and the unchanged manifest, including attribution.
#[derive(Debug, Clone)]
pub struct SnapTargetsMetadata {
    pub declarations: Vec<SnapDecl>,
    pub manifest: serde_json::Value,
}

/// A reusable selection with bounded-batch reads on each visit.
#[derive(Debug)]
pub struct SnapTargets {
    metadata: SnapTargetsMetadata,
    stores: Vec<SnapStore>,
    scope: SnapScope,
}

/// Failures while opening an explicit snap selection.
#[derive(Debug, thiserror::Error)]
pub enum SnapTargetsError {
    /// Supplied bounds are non-finite, reversed or outside EPSG:4326.
    #[error("invalid supplied snap bbox [{xmin}, {ymin}, {xmax}, {ymax}]")]
    InvalidBounds {
        xmin: f32,
        ymin: f32,
        xmax: f32,
        ymax: f32,
    },

    /// The extent is non-finite, empty, reversed or outside EPSG:4326.
    #[error(
        "invalid EPSG:4326 extent [{west}, {south}, {east}, {north}]; require -180 <= west < east <= 180 and -90 <= south < north <= 90"
    )]
    InvalidExtent {
        west: f64,
        south: f64,
        east: f64,
        north: f64,
    },
    /// No snap auxiliary declaration exists in the dataset.
    #[error("dataset has no snap declarations")]
    NoSnapDeclarations {},
    /// A requested named set does not exist.
    #[error("unknown snap set {name:?}; available sets: {available:?}")]
    UnknownSet {
        name: String,
        available: Vec<String>,
    },
    /// A selected declaration uses an unsupported snap schema.
    #[error("unsupported snap declaration {schema:?} (set {name:?})")]
    UnsupportedDeclaration {
        schema: String,
        name: Option<String>,
    },
    /// More than one declaration has the same set identity.
    #[error("duplicate snap set name {name:?}")]
    DuplicateSet { name: String },
    /// Manifest or snap artifact reading or validation fails.
    #[error(transparent)]
    Read {
        #[from]
        source: SessionError,
    },
}

impl SnapTargets {
    /// Open the manifest and selected snap Parquet footers, without reading unit data.
    ///
    /// # Errors
    /// Reports missing, unknown, duplicate or unsupported declarations and input failures.
    #[instrument(skip_all, fields(snap_set))]
    pub fn open(
        source: DatasetSource,
        scope: SnapScope,
        snap_set: Option<&str>,
    ) -> Result<Self, SnapTargetsError> {
        let (parsed, manifest) = source.read_manifest()?;
        for decl in &parsed.aux.unreadable {
            if decl.schema.starts_with("hfx.aux.snap.") {
                let name = decl
                    .metadata
                    .get("name")
                    .and_then(serde_json::Value::as_str);
                if snap_set.is_none() || name == snap_set {
                    return Err(SnapTargetsError::UnsupportedDeclaration {
                        schema: decl.schema.clone(),
                        name: name.map(str::to_owned),
                    });
                }
            }
        }
        let mut names = BTreeSet::new();
        for decl in &parsed.aux.snaps {
            if !names.insert(decl.name.clone()) {
                return Err(SnapTargetsError::DuplicateSet {
                    name: decl.name.clone(),
                });
            }
        }
        if let Some(name) = snap_set {
            if !names.contains(name) {
                return Err(SnapTargetsError::UnknownSet {
                    name: name.to_owned(),
                    available: names.into_iter().collect(),
                });
            }
        } else if names.is_empty() {
            return Err(SnapTargetsError::NoSnapDeclarations {});
        }
        let declarations: Vec<_> = parsed
            .aux
            .snaps
            .into_iter()
            .filter(|decl| snap_set.is_none_or(|name| name == decl.name))
            .collect();
        let mut stores = Vec::with_capacity(declarations.len());
        for decl in &declarations {
            if crate::source::path_escapes_root(&decl.snap) {
                return Err(SessionError::AuxiliaryPathEscape {
                    schema: "hfx.aux.snap.v2".to_owned(),
                    artifact: "snap".to_owned(),
                    path: decl.snap.clone(),
                }
                .into());
            }
            let store = match &source {
                DatasetSource::Local(root) => SnapStore::open_lazy(&root.join(&decl.snap)),
                DatasetSource::Remote {
                    store, root, url, ..
                } => {
                    let path = decl
                        .snap
                        .split('/')
                        .fold(root.clone(), |path, segment| path.join(segment));
                    SnapStore::open_remote_with_caches(
                        Arc::clone(store),
                        path,
                        format!(
                            "{}/{path}",
                            url.as_str().trim_end_matches('/'),
                            path = decl.snap
                        ),
                        parsed.manifest.fabric_name().to_owned(),
                        parsed.manifest.adapter_version().to_owned(),
                        parsed.manifest.format_version().to_string(),
                        None,
                        None,
                        None,
                        SnapOpenMode::LazyMetadata,
                    )
                }
            }
            .map_err(|source| SessionError::SnapArtifactRead {
                name: decl.name.clone(),
                path: decl.snap.clone(),
                source: Box::new(source),
            })?;
            stores.push(store);
        }
        Ok(Self {
            metadata: SnapTargetsMetadata {
                declarations,
                manifest,
            },
            stores,
            scope,
        })
    }

    pub fn metadata(&self) -> &SnapTargetsMetadata {
        &self.metadata
    }

    /// Visit each selected feature in declaration and file order, retaining complete WKB.
    /// At most one Parquet row group and one decoded batch are retained by the reader.
    ///
    /// # Errors
    /// Propagates read/schema/row errors and the first callback failure. Never skips failures.
    #[instrument(skip_all)]
    pub fn visit<E: From<SessionError>>(
        &self,
        mut callback: impl FnMut(&SnapDecl, &SnapTargetRow) -> Result<(), E>,
    ) -> Result<(), E> {
        for (decl, store) in self.metadata.declarations.iter().zip(&self.stores) {
            let _span =
                tracing::info_span!("snap_set_read", snap_set = %decl.name, path = %decl.snap)
                    .entered();
            store.visit(self.scope, &decl.name, |target| callback(decl, target))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SnapExtent, SnapScope, SnapTargets, SnapTargetsError};
    use crate::error::SessionError;
    use crate::runtime::RT;
    use crate::source::DatasetSource;
    use crate::testutil::{DatasetBuilder, TestSnapDeclaration, TestSnapGeometry, TestSnapTarget};
    use hfx::SnapTarget;
    use object_store::memory::InMemory;
    use object_store::path::Path as ObjectPath;
    use object_store::{ObjectStoreExt, PutPayload};
    use serde_json::json;
    use std::sync::Arc;
    use url::Url;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let result = DatasetBuilder::new(2)
            .with_custom_snap_declarations(vec![
                TestSnapDeclaration {
                    name: "segment-stems".to_owned(),
                    path: "segments.parquet".to_owned(),
                    references_levels: vec![0, 1],
                    targets: vec![TestSnapTarget {
                        id: 1,
                        catchment_id: 1,
                        weight: 3.5,
                        is_mainstem: true,
                        geometry: TestSnapGeometry::Point(1.0, 1.0),
                    }],
                },
                TestSnapDeclaration {
                    name: "reach-stems".to_owned(),
                    path: "reaches.parquet".to_owned(),
                    references_levels: vec![1],
                    targets: vec![TestSnapTarget {
                        id: 2,
                        catchment_id: 2,
                        weight: 1.25,
                        is_mainstem: false,
                        geometry: TestSnapGeometry::LineString(0.0, 0.0, 4.0, 4.0),
                    }],
                },
            ])
            .build();
        std::fs::remove_file(result.1.join("catchments.parquet")).unwrap();
        std::fs::remove_file(result.1.join("graph.parquet")).unwrap();
        let path = result.1.join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::create_dir(result.1.join("snap")).unwrap();
        for (index, name) in ["segments.parquet", "reaches.parquet"].iter().enumerate() {
            std::fs::rename(result.1.join(name), result.1.join("snap").join(name)).unwrap();
            value["auxiliary"][index]["artifacts"]["snap"] = json!(format!("snap/{name}"));
        }
        value["attribution"] = json!({"license":"CC-BY-4.0","authors":["source producer"]});
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        result
    }

    fn rows(selection: &SnapTargets) -> Vec<(String, SnapTarget)> {
        let mut result = Vec::new();
        selection
            .visit::<SessionError>(|decl, row| {
                result.push((decl.name.clone(), row.target.clone()));
                Ok(())
            })
            .unwrap();
        result
    }

    #[test]
    fn selection_reads_named_sets_and_attribution_without_unit_artifacts() {
        let (_dir, root) = fixture();
        let selection =
            SnapTargets::open(DatasetSource::Local(root.clone()), SnapScope::All, None).unwrap();
        let actual = rows(&selection);
        assert_eq!(
            actual
                .iter()
                .map(|(name, row)| (
                    name.as_str(),
                    row.id().get(),
                    row.unit_id().get(),
                    row.weight().get()
                ))
                .collect::<Vec<_>>(),
            [("segment-stems", 1, 1, 3.5), ("reach-stems", 2, 2, 1.25)]
        );
        assert_eq!(
            selection.metadata().declarations[0].references_levels,
            [0, 1]
        );
        assert_eq!(
            selection.metadata().manifest["attribution"]["license"],
            "CC-BY-4.0"
        );
        let selected = SnapTargets::open(
            DatasetSource::Local(root.clone()),
            SnapScope::All,
            Some("reach-stems"),
        )
        .unwrap();
        assert_eq!(rows(&selected).len(), 1);
        let empty = SnapTargets::open(
            DatasetSource::Local(root),
            SnapScope::Bbox(SnapExtent::new(60.0, 60.0, 61.0, 61.0).unwrap()),
            None,
        )
        .unwrap();
        assert!(rows(&empty).is_empty());
    }

    #[test]
    fn remote_selection_needs_only_manifest_and_nested_snap_objects() {
        let (_dir, root) = fixture();
        let store = Arc::new(InMemory::new());
        RT.block_on(async {
            for path in [
                "manifest.json",
                "snap/segments.parquet",
                "snap/reaches.parquet",
            ] {
                store
                    .put(
                        &ObjectPath::from(format!("hfx/{path}")),
                        PutPayload::from(std::fs::read(root.join(path)).unwrap()),
                    )
                    .await
                    .unwrap();
            }
        });
        let remote = DatasetSource::Remote {
            store,
            http_stats: None,
            root: ObjectPath::from("hfx"),
            url: Url::parse("s3://bucket/hfx").unwrap(),
        };
        let selection = SnapTargets::open(remote, SnapScope::All, None).unwrap();
        assert_eq!(rows(&selection).len(), 2);
    }

    #[test]
    fn selection_reports_absent_unknown_unsupported_and_broken_sets() {
        let (_dir, root) = fixture();
        let source = DatasetSource::Local(root.clone());
        assert!(matches!(
            SnapTargets::open(source.clone(), SnapScope::All, Some("missing")),
            Err(SnapTargetsError::UnknownSet { .. })
        ));
        std::fs::remove_file(root.join("snap/reaches.parquet")).unwrap();
        assert!(SnapTargets::open(source.clone(), SnapScope::All, None).is_err());
        // Unselected broken artifacts do not prevent selecting a valid set.
        assert_eq!(
            rows(
                &SnapTargets::open(source.clone(), SnapScope::All, Some("segment-stems")).unwrap()
            )
            .len(),
            1
        );
        let path = root.join("manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["auxiliary"][1]["schema"] = json!("hfx.aux.snap.v99");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            SnapTargets::open(source.clone(), SnapScope::All, None),
            Err(SnapTargetsError::UnsupportedDeclaration { .. })
        ));
        assert!(matches!(
            SnapTargets::open(source.clone(), SnapScope::All, Some("reach-stems")),
            Err(SnapTargetsError::UnsupportedDeclaration { .. })
        ));
        value["auxiliary"] = json!([]);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            SnapTargets::open(source, SnapScope::All, None),
            Err(SnapTargetsError::NoSnapDeclarations { .. })
        ));
    }

    #[test]
    fn extent_validation_and_f64_precision() {
        for bounds in [
            [f64::NAN, 0.0, 1.0, 1.0],
            [-181.0, 0.0, 1.0, 1.0],
            [0.0, -91.0, 1.0, 1.0],
            [0.0, 0.0, 181.0, 1.0],
            [0.0, 0.0, 1.0, 91.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ] {
            assert!(SnapExtent::new(bounds[0], bounds[1], bounds[2], bounds[3]).is_err());
        }
        let bounds = [8.300000001, 47.200000001, 8.300000002, 47.200000002];
        assert_eq!(
            SnapExtent::new(bounds[0], bounds[1], bounds[2], bounds[3])
                .unwrap()
                .bounds(),
            bounds
        );
    }
}
