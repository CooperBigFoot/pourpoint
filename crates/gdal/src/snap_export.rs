//! snap_export : SnapTargetsMetadata × Stream<SnapTargetRow> → GeoPackage layers.
//! The caller creates the dataset and publishes the file only after `finish` succeeds.

use std::collections::BTreeMap;

use gdal::errors::GdalError;
use gdal::spatial_ref::SpatialRef;
use gdal::vector::sql::Dialect;
use gdal::vector::{
    Feature, Geometry, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType,
};
use gdal::{Dataset, Metadata};
use pourpoint_core::error::SessionError;
use pourpoint_core::reader::manifest::SnapDecl;
use pourpoint_core::snap_targets::{SnapTargetRow, SnapTargetsMetadata};
use tracing::instrument;

/// Failures while streaming snap rows into a GeoPackage.
#[derive(Debug, thiserror::Error)]
pub enum SnapExportError {
    /// GDAL could not create, write, flush, or close the output.
    #[error("GeoPackage export failed: {source}")]
    Gdal {
        #[source]
        source: GdalError,
    },
    /// Reading or validating a selected source row failed.
    #[error("snap source failed: {source}")]
    Source {
        #[source]
        source: Box<SessionError>,
    },
    /// A row was sent for a set not declared when the writer was created.
    #[error("snap set {name:?} is not declared in this export")]
    UnknownSet { name: String },
    /// A row carries a geometry other than an HFX Point or LineString.
    #[error("snap {id} in set {name:?} has unsupported geometry {geometry}")]
    UnsupportedGeometry {
        name: String,
        id: i64,
        geometry: String,
    },
    /// The caller supplied a dataset other than an empty GeoPackage.
    #[error("snap export requires an empty GeoPackage; driver={driver}, layers={layers}")]
    InvalidDataset { driver: String, layers: usize },
}

impl From<GdalError> for SnapExportError {
    fn from(source: GdalError) -> Self {
        Self::Gdal { source }
    }
}

impl From<SessionError> for SnapExportError {
    fn from(source: SessionError) -> Self {
        Self::Source {
            source: Box::new(source),
        }
    }
}

/// A bounded-memory writer with two deterministic layers per declared set.
///
/// Layers are `<name>_points` and `<name>_lines`, except reserved set names
/// `sqlite` and `gpkg`, which receive a collision-free `snap_` prefix.
///
/// Empty Point and LineString layers remain present for empty selections. Each
/// layer stores declaration metadata, and dataset metadata `HFX_MANIFEST` retains
/// the complete supplied manifest, including any attribution and license fields.
pub struct SnapGeoPackage {
    dataset: Dataset,
    layers: BTreeMap<String, [usize; 2]>,
}

impl SnapGeoPackage {
    /// Initialize an already-created, empty GDAL GeoPackage.
    ///
    /// # Errors
    /// Returns an error if the dataset is not an empty GeoPackage or GDAL cannot
    /// create a layer, its fields, spatial reference, or metadata.
    #[instrument(skip_all)]
    pub fn new(
        mut dataset: Dataset,
        metadata: &SnapTargetsMetadata,
    ) -> Result<Self, SnapExportError> {
        if dataset.driver().short_name() != "GPKG" || dataset.layer_count() != 0 {
            return Err(SnapExportError::InvalidDataset {
                driver: dataset.driver().short_name(),
                layers: dataset.layer_count(),
            });
        }
        dataset.set_metadata_item("HFX_MANIFEST", &metadata.manifest.to_string(), "")?;
        let srs = SpatialRef::from_epsg(4326)?;
        let mut layers = BTreeMap::new();
        let mut declarations: Vec<_> = metadata.declarations.iter().collect();
        declarations.sort_by(|left, right| left.name.cmp(&right.name));
        for declaration in declarations {
            let first = dataset.layer_count();
            for (suffix, ty) in [
                ("points", OGRwkbGeometryType::wkbPoint),
                ("lines", OGRwkbGeometryType::wkbLineString),
            ] {
                // SQLite and GeoPackage reserve these table prefixes. The extra
                // underscore cannot collide with a valid kebab-case set name.
                let prefix = match declaration.name.as_str() {
                    "sqlite" | "gpkg" => "snap_",
                    _ => "",
                };
                let layer_name = format!("{prefix}{}_{suffix}", declaration.name);
                let mut layer = dataset.create_layer(LayerOptions {
                    name: &layer_name,
                    srs: Some(&srs),
                    ty,
                    ..Default::default()
                })?;
                layer.create_defn_fields(&[
                    ("id", OGRFieldType::OFTInteger64),
                    ("unit_id", OGRFieldType::OFTInteger64),
                    ("weight", OGRFieldType::OFTReal),
                    ("stem_role", OGRFieldType::OFTString),
                    ("snap_set", OGRFieldType::OFTString),
                    ("bbox_xmin", OGRFieldType::OFTReal),
                    ("bbox_ymin", OGRFieldType::OFTReal),
                    ("bbox_xmax", OGRFieldType::OFTReal),
                    ("bbox_ymax", OGRFieldType::OFTReal),
                ])?;
                layer.set_metadata_item("DESCRIPTION", &declaration.description, "")?;
                layer.set_metadata_item("SNAP_SET", &declaration.name, "")?;
                layer.set_metadata_item("WEIGHT_SEMANTICS", &declaration.weight_semantics, "")?;
                // Decimal integers in brackets form an unambiguous JSON array.
                let levels = declaration
                    .references_levels
                    .iter()
                    .map(i16::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                layer.set_metadata_item("REFERENCES_LEVELS", &format!("[{levels}]"), "")?;
            }
            layers.insert(declaration.name.clone(), [first, first + 1]);
        }
        // Materialize GDAL's deferred layer/schema changes before beginning the
        // row transaction. Native GPKG SQL keeps the journal on disk and avoids
        // a SQLite commit/fsync for each feature without buffering source rows.
        dataset.flush_cache()?;
        dataset.execute_sql("BEGIN", None, Dialect::DEFAULT)?;
        Ok(Self { dataset, layers })
    }

    /// Write one complete, unclipped source geometry and its attributes.
    /// Original bounds occupy nullable `bbox_xmin`, `bbox_ymin`, `bbox_xmax`,
    /// and `bbox_ymax` fields. Missing bounds are never inferred from geometry.
    ///
    /// # Errors
    /// Returns an error for an undeclared set, unsupported geometry, or GDAL
    /// decoding/writing failure. No source row is silently dropped.
    #[instrument(skip_all, level = "trace", fields(snap_set = %set.name, id = row.target.id().get()))]
    pub fn write(&mut self, set: &SnapDecl, row: &SnapTargetRow) -> Result<(), SnapExportError> {
        let bounds = row.bbox.map(|bbox| bbox.bounds());
        let row = &row.target;
        let indices = self
            .layers
            .get(&set.name)
            .ok_or_else(|| SnapExportError::UnknownSet {
                name: set.name.clone(),
            })?;
        let geometry = Geometry::from_wkb(row.geometry().as_bytes())?;
        let index = match geometry.geometry_type() {
            OGRwkbGeometryType::wkbPoint => indices[0],
            OGRwkbGeometryType::wkbLineString => indices[1],
            _ => {
                return Err(SnapExportError::UnsupportedGeometry {
                    name: set.name.clone(),
                    id: row.id().get(),
                    geometry: geometry.geometry_name(),
                });
            }
        };
        let layer = self.dataset.layer(index)?;
        let mut feature = Feature::new(layer.defn())?;
        feature.set_field_integer64(0, row.id().get())?;
        feature.set_field_integer64(1, row.unit_id().get())?;
        feature.set_field_double(2, f64::from(row.weight().get()))?;
        if let Some(role) = row.stem_role() {
            feature.set_field_string(3, &role.to_string())?;
        } else {
            feature.set_field_null(3)?;
        }
        feature.set_field_string(4, &set.name)?;
        for offset in 0..4 {
            if let Some(bounds) = bounds {
                feature.set_field_double(5 + offset, f64::from(bounds[offset]))?;
            } else {
                feature.set_field_null(5 + offset)?;
            }
        }
        feature.set_geometry(geometry)?;
        feature.create(&layer)?;
        Ok(())
    }

    /// Flush and close the dataset before the caller atomically publishes it.
    ///
    /// # Errors
    /// Propagates GDAL flush and close failures. A failure must prevent publication.
    #[instrument(skip_all)]
    pub fn finish(mut self) -> Result<(), SnapExportError> {
        self.dataset.execute_sql("COMMIT", None, Dialect::DEFAULT)?;
        self.dataset.flush_cache()?;
        self.dataset.close()?;
        Ok(())
    }
}
