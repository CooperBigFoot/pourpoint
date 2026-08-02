//! Manifest reader — parses manifest.json into an hfx::Manifest plus
//! pourpoint-side auxiliary declarations.
//!
//! HFX v0.3.0 hard-cut: only `format_version == "0.3.0"` and `crs ==
//! "EPSG:4326"` are accepted. The version check runs first so a v0.1 manifest
//! is rejected with a typed [`SessionError::UnsupportedFormatVersion`] before
//! any required-field parsing. Presence of snap/raster data is expressed
//! through `auxiliary[]` declarations.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use hfx::{
    AuxiliaryDecl, AuxiliarySchemaId, BoundingBox, D8RasterMetadataV2, Manifest, ManifestBuilder,
    Topology, UnitCount,
};
use tracing::instrument;

use crate::error::SessionError;
use crate::support_claims::{
    DATASET_CRS_EPSG_4326, FORMAT_VERSION_V0_3_0, ReaderSupportValue, claimed_auxiliary_schema,
    claimed_dataset_crs, claimed_format_version,
};

/// Parsed metadata for a blessed `hfx.aux.d8_raster.v2` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D8RasterDecl {
    /// Relative path (dataset-root-relative) to the flow-direction raster.
    pub flow_dir: String,
    /// Relative path (dataset-root-relative) to the flow-accumulation raster.
    pub flow_acc: String,
    /// Required typed D8 raster v2 metadata.
    pub metadata: D8RasterMetadataV2,
}

/// Parsed metadata for a blessed `hfx.aux.snap.v2` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapDecl {
    /// Kebab-case name, unique across snap declarations in the dataset.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Relative path (dataset-root-relative) to the snap-feature Parquet file.
    pub snap: String,
    /// Non-empty list of HFX levels this snap file may reference.
    pub references_levels: Vec<i16>,
    /// Producer documentation for how `weight` values should be interpreted.
    pub weight_semantics: String,
}

/// A generic (non-blessed) auxiliary declaration retained as a raw handle.
///
/// pourpoint performs structural checks only on these (path resolution + presence);
/// it does NOT parse their metadata semantically. This is the reverse-DNS /
/// provisional handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericAuxDecl {
    /// The raw schema ID string.
    pub schema: String,
    /// Artifact key → resolved dataset-root-relative path.
    pub artifacts: BTreeMap<String, String>,
    /// Raw metadata retained without semantic parsing.
    pub metadata: serde_json::Value,
}

/// `retain : unrecognized hfx.aux declaration -> raw declaration`.
///
/// This carrier is diagnostic-only, is not inserted into [`hfx::Manifest`],
/// and does not assert that its artifact paths are safe, present, or usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableAuxDecl {
    /// The complete raw schema name.
    pub schema: String,
    /// The raw artifact key-to-path mapping.
    pub artifacts: BTreeMap<String, String>,
    /// Raw metadata retained without interpretation.
    pub metadata: serde_json::Value,
}

/// pourpoint-side classified auxiliary declarations parsed from `manifest.auxiliary[]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuxDeclarations {
    /// Blessed D8 raster declarations.
    pub d8_rasters: Vec<D8RasterDecl>,
    /// Blessed snap declarations.
    pub snaps: Vec<SnapDecl>,
    /// Provisional / third-party declarations retained as raw handles.
    pub generic: Vec<GenericAuxDecl>,
    /// Unrecognized `hfx.aux.*` declarations retained for diagnostics only.
    pub unreadable: Vec<UnreadableAuxDecl>,
}

/// A parsed manifest plus its classified auxiliary declarations.
#[derive(Debug, Clone)]
pub struct ParsedManifest {
    /// The validated core manifest.
    pub manifest: Manifest,
    /// pourpoint-side classified auxiliary declarations.
    pub aux: AuxDeclarations,
}

/// Raw serde struct for deserializing manifest.json.
///
/// All fields are `Option<T>` so that field-level error reporting (rather than
/// a serde-layer failure) drives missing-required-field diagnostics.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct RawManifest {
    pub format_version: Option<String>,
    pub fabric_name: Option<String>,
    pub fabric_version: Option<String>,
    pub crs: Option<String>,
    pub has_up_area: Option<bool>,
    pub topology: Option<String>,
    pub region: Option<String>,
    pub bbox: Option<Vec<f64>>,
    pub unit_count: Option<u64>,
    pub created_at: Option<String>,
    pub adapter_version: Option<String>,
    #[serde(default)]
    pub auxiliary: Vec<RawAuxiliary>,
}

/// Raw serde struct for one `auxiliary[]` entry.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct RawAuxiliary {
    pub schema: Option<String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Reads and validates `manifest.json` at `path`, returning a [`ParsedManifest`].
///
/// # Errors
///
/// | Variant | Condition |
/// |---|---|
/// | [`SessionError::Io`] | File cannot be read |
/// | [`SessionError::UnsupportedFormatVersion`] | `format_version` is not `"0.3.0"` |
/// | [`SessionError::UnsupportedCrs`] | `crs` is not `"EPSG:4326"` |
/// | [`SessionError::ManifestJsonParse`] | Bytes are not valid JSON or do not match the expected shape |
/// | [`SessionError::ManifestFieldMissing`] | A required field is absent |
/// | [`SessionError::ManifestFieldInvalid`] | A field is present but its value is invalid |
/// | [`SessionError::AuxiliaryDeclParse`] | An `auxiliary[]` entry is malformed |
#[instrument(skip_all, fields(path = %path.display()))]
pub fn read_manifest(path: &Path) -> Result<ParsedManifest, SessionError> {
    let bytes = std::fs::read(path).map_err(|e| SessionError::io("manifest.json", e))?;
    read_manifest_from_bytes(&bytes)
}

/// Reads and validates `manifest.json` bytes, returning a [`ParsedManifest`].
///
/// # Errors
///
/// See [`read_manifest`].
#[instrument(skip_all, fields(byte_len = bytes.len()))]
pub fn read_manifest_from_bytes(bytes: &[u8]) -> Result<ParsedManifest, SessionError> {
    let raw = serde_json::from_slice::<RawManifest>(bytes)
        .map_err(|source| SessionError::ManifestJsonParse { source })?;

    build_manifest(raw)
}

/// Converts a [`RawManifest`] into a validated [`ParsedManifest`].
fn build_manifest(raw: RawManifest) -> Result<ParsedManifest, SessionError> {
    // --- Format version is checked FIRST, before any required-field parsing. ---
    let format_version_str = raw
        .format_version
        .ok_or(SessionError::ManifestFieldMissing {
            field: "format_version",
        })?;
    let format_version = claimed_format_version(&format_version_str).ok_or_else(|| {
        SessionError::UnsupportedFormatVersion {
            found: format_version_str,
            expected: FORMAT_VERSION_V0_3_0.canonical_declaration(),
        }
    })?;

    let fabric_name = raw.fabric_name.ok_or(SessionError::ManifestFieldMissing {
        field: "fabric_name",
    })?;

    let crs_str = raw
        .crs
        .ok_or(SessionError::ManifestFieldMissing { field: "crs" })?;
    let crs = claimed_dataset_crs(&crs_str).ok_or_else(|| SessionError::UnsupportedCrs {
        found: crs_str,
        expected: DATASET_CRS_EPSG_4326.canonical_declaration(),
    })?;

    let topology_str = raw
        .topology
        .ok_or(SessionError::ManifestFieldMissing { field: "topology" })?;
    let topology = Topology::from_str(&topology_str).map_err(|_| {
        SessionError::manifest_field_invalid(
            "topology",
            format!("unsupported topology {topology_str:?}, expected \"tree\" or \"dag\""),
        )
    })?;

    let bbox_raw = raw
        .bbox
        .ok_or(SessionError::ManifestFieldMissing { field: "bbox" })?;
    if bbox_raw.len() != 4 {
        return Err(SessionError::manifest_field_invalid(
            "bbox",
            format!(
                "expected 4 elements [minx, miny, maxx, maxy], got {}",
                bbox_raw.len()
            ),
        ));
    }
    let bbox = BoundingBox::new(
        bbox_raw[0] as f32,
        bbox_raw[1] as f32,
        bbox_raw[2] as f32,
        bbox_raw[3] as f32,
    )
    .map_err(|e| SessionError::manifest_field_invalid("bbox", e.to_string()))?;

    let unit_count_raw = raw.unit_count.ok_or(SessionError::ManifestFieldMissing {
        field: "unit_count",
    })?;
    let unit_count = UnitCount::new(unit_count_raw)
        .map_err(|e| SessionError::manifest_field_invalid("unit_count", e.to_string()))?;

    let created_at = raw.created_at.ok_or(SessionError::ManifestFieldMissing {
        field: "created_at",
    })?;

    let adapter_version = raw
        .adapter_version
        .ok_or(SessionError::ManifestFieldMissing {
            field: "adapter_version",
        })?;

    // --- Auxiliary declarations ---
    let mut aux = AuxDeclarations::default();
    let mut aux_decls: Vec<AuxiliaryDecl> = Vec::with_capacity(raw.auxiliary.len());
    for entry in raw.auxiliary {
        match parse_auxiliary(entry)? {
            ParsedAuxiliary::Readable { decl, classified } => {
                aux_decls.push(decl);
                match classified {
                    ClassifiedAux::D8(d8) => aux.d8_rasters.push(d8),
                    ClassifiedAux::Snap(snap) => aux.snaps.push(snap),
                    ClassifiedAux::Generic(g) => aux.generic.push(g),
                }
            }
            ParsedAuxiliary::Unreadable(decl) => aux.unreadable.push(decl),
        }
    }

    // --- Build core manifest ---
    let mut builder = ManifestBuilder::new(
        format_version,
        fabric_name,
        crs,
        topology,
        bbox,
        unit_count,
        created_at,
        adapter_version,
    )
    .map_err(|source| SessionError::ManifestDomain { source })?;

    if raw.has_up_area.unwrap_or(false) {
        builder = builder.with_up_area();
    }
    if let Some(v) = raw.fabric_version {
        builder = builder.with_fabric_version(v);
    }
    if let Some(v) = raw.region {
        builder = builder.with_region(v);
    }
    for decl in aux_decls {
        builder = builder.with_auxiliary(decl);
    }

    Ok(ParsedManifest {
        manifest: builder.build(),
        aux,
    })
}

/// Classified, metadata-parsed auxiliary variant.
enum ClassifiedAux {
    D8(D8RasterDecl),
    Snap(SnapDecl),
    Generic(GenericAuxDecl),
}

/// Result of attempting to interpret one raw auxiliary declaration.
enum ParsedAuxiliary {
    Readable {
        decl: AuxiliaryDecl,
        classified: ClassifiedAux,
    },
    Unreadable(UnreadableAuxDecl),
}

/// Parse one `auxiliary[]` entry into a readable declaration or raw diagnostic.
fn parse_auxiliary(raw: RawAuxiliary) -> Result<ParsedAuxiliary, SessionError> {
    let schema_str = raw.schema.ok_or_else(|| SessionError::AuxiliaryDeclParse {
        schema: "<missing>".to_string(),
        reason: "auxiliary entry is missing required \"schema\" field".to_string(),
    })?;

    let schema_id = match AuxiliarySchemaId::parse(&schema_str) {
        Ok(schema_id) => schema_id,
        Err(_) if schema_str.starts_with("hfx.aux.") => {
            return Ok(ParsedAuxiliary::Unreadable(UnreadableAuxDecl {
                schema: schema_str,
                artifacts: raw.artifacts,
                metadata: raw.metadata,
            }));
        }
        Err(error) => {
            return Err(SessionError::AuxiliaryDeclParse {
                schema: schema_str,
                reason: error.to_string(),
            });
        }
    };

    if raw.artifacts.is_empty() {
        return Err(SessionError::AuxiliaryDeclParse {
            schema: schema_str,
            reason: "auxiliary \"artifacts\" mapping must be non-empty".to_string(),
        });
    }

    let decl = AuxiliaryDecl::new(schema_id.clone(), raw.artifacts.clone()).map_err(|e| {
        SessionError::AuxiliaryDeclParse {
            schema: schema_str.clone(),
            reason: e.to_string(),
        }
    })?;

    let claim = claimed_auxiliary_schema(&schema_id);
    let classified = match claim.value() {
        ReaderSupportValue::AuxiliarySchemaD8RasterV2
            if schema_str == claim.canonical_declaration() =>
        {
            ClassifiedAux::D8(parse_d8_metadata(
                &schema_str,
                &raw.artifacts,
                &raw.metadata,
            )?)
        }
        ReaderSupportValue::AuxiliarySchemaSnapV2
            if schema_str == claim.canonical_declaration() =>
        {
            ClassifiedAux::Snap(parse_snap_metadata(
                &schema_str,
                &raw.artifacts,
                &raw.metadata,
            )?)
        }
        ReaderSupportValue::AuxiliarySchemaGeneric => {
            // Generic handle: raw path + metadata only, no semantic parsing.
            ClassifiedAux::Generic(GenericAuxDecl {
                schema: schema_str,
                artifacts: raw.artifacts,
                metadata: raw.metadata,
            })
        }
        ReaderSupportValue::AuxiliarySchemaD8RasterV2
        | ReaderSupportValue::AuxiliarySchemaSnapV2 => {
            return Err(SessionError::AuxiliaryDeclParse {
                schema: schema_str,
                reason: format!(
                    "parsed auxiliary schema does not match support claim {:?}",
                    claim.canonical_declaration()
                ),
            });
        }
        ReaderSupportValue::FormatVersion(_)
        | ReaderSupportValue::DatasetCrs(_)
        | ReaderSupportValue::FlowDirectionEncoding(_)
        | ReaderSupportValue::D8Crs(_)
        | ReaderSupportValue::D8FlowAccumulationUnits(_)
        | ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported => {
            return Err(SessionError::AuxiliaryDeclParse {
                schema: schema_str,
                reason: "auxiliary schema resolved to a non-routable support claim".to_string(),
            });
        }
    };

    Ok(ParsedAuxiliary::Readable { decl, classified })
}

/// Parse the metadata block for an `hfx.aux.d8_raster.v2` declaration.
fn parse_d8_metadata(
    schema: &str,
    artifacts: &BTreeMap<String, String>,
    metadata: &serde_json::Value,
) -> Result<D8RasterDecl, SessionError> {
    let flow_dir = require_artifact(schema, artifacts, "flow_dir")?;
    let flow_acc = require_artifact(schema, artifacts, "flow_acc")?;

    let metadata_object = metadata
        .as_object()
        .ok_or_else(|| SessionError::AuxiliaryDeclParse {
            schema: schema.to_string(),
            reason: "metadata block must be an object".to_string(),
        })?;
    let allowed_keys = ["crs", "flow_dir_encoding", "flow_acc_units"];
    if let Some(additional) = metadata_object
        .keys()
        .find(|key| !allowed_keys.contains(&key.as_str()))
    {
        return Err(SessionError::AuxiliaryDeclParse {
            schema: schema.to_string(),
            reason: format!("metadata contains forbidden additional property {additional:?}"),
        });
    }
    let parsed_metadata = D8RasterMetadataV2::parse(
        metadata_object
            .get("crs")
            .and_then(serde_json::Value::as_str),
        metadata_object
            .get("flow_dir_encoding")
            .and_then(serde_json::Value::as_str),
        metadata_object
            .get("flow_acc_units")
            .and_then(serde_json::Value::as_str),
    )
    .map_err(|source| SessionError::AuxiliaryDeclParse {
        schema: schema.to_string(),
        reason: source.to_string(),
    })?;

    Ok(D8RasterDecl {
        flow_dir,
        flow_acc,
        metadata: parsed_metadata,
    })
}

/// Parse the metadata block for an `hfx.aux.snap.v2` declaration.
fn parse_snap_metadata(
    schema: &str,
    artifacts: &BTreeMap<String, String>,
    metadata: &serde_json::Value,
) -> Result<SnapDecl, SessionError> {
    let snap = require_artifact(schema, artifacts, "snap")?;

    let meta_obj = metadata
        .as_object()
        .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
            name: "<unknown>".to_string(),
            reason: "metadata block must be an object".to_string(),
        })?;

    let name = meta_obj
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
            name: "<unknown>".to_string(),
            reason: "metadata.name must be a non-empty string".to_string(),
        })?
        .to_string();
    if name.is_empty() {
        return Err(SessionError::SnapAuxMetadataInvalid {
            name: "<unknown>".to_string(),
            reason: "metadata.name must be a non-empty string".to_string(),
        });
    }

    let description = meta_obj
        .get("description")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
            name: name.clone(),
            reason: "metadata.description must be a string".to_string(),
        })?
        .to_string();

    let weight_semantics = meta_obj
        .get("weight_semantics")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
            name: name.clone(),
            reason: "metadata.weight_semantics must be a string".to_string(),
        })?
        .to_string();

    let levels_raw = meta_obj
        .get("references_levels")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
            name: name.clone(),
            reason: "metadata.references_levels must be a non-empty array".to_string(),
        })?;
    if levels_raw.is_empty() {
        return Err(SessionError::SnapAuxMetadataInvalid {
            name: name.clone(),
            reason: "metadata.references_levels must be non-empty".to_string(),
        });
    }
    let mut references_levels = Vec::with_capacity(levels_raw.len());
    for v in levels_raw {
        let n = v
            .as_i64()
            .ok_or_else(|| SessionError::SnapAuxMetadataInvalid {
                name: name.clone(),
                reason: "metadata.references_levels entries must be integers".to_string(),
            })?;
        if !(0..=i64::from(i16::MAX)).contains(&n) {
            return Err(SessionError::SnapAuxMetadataInvalid {
                name: name.clone(),
                reason: format!(
                    "metadata.references_levels entry {n} out of range [0, {}]",
                    i16::MAX
                ),
            });
        }
        references_levels.push(n as i16);
    }

    Ok(SnapDecl {
        name,
        description,
        snap,
        references_levels,
        weight_semantics,
    })
}

/// Return the artifact path for `key`, erroring if it is absent.
fn require_artifact(
    schema: &str,
    artifacts: &BTreeMap<String, String>,
    key: &str,
) -> Result<String, SessionError> {
    artifacts
        .get(key)
        .cloned()
        .ok_or_else(|| SessionError::AuxiliaryDeclParse {
            schema: schema.to_string(),
            reason: format!("missing required artifact key {key:?}"),
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use hfx::{Crs, FlowAccumulationUnits, FlowDirEncoding, FormatVersion, Topology};
    use serde_json::json;
    use tempfile::TempDir;

    use super::{UnreadableAuxDecl, read_manifest};
    use crate::error::SessionError;

    fn write_manifest(dir: &TempDir, value: &serde_json::Value) -> std::path::PathBuf {
        let path = dir.path().join("manifest.json");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(value.to_string().as_bytes()).unwrap();
        path
    }

    fn minimal_json() -> serde_json::Value {
        json!({
            "format_version": "0.3.0",
            "fabric_name": "testfabric",
            "crs": "EPSG:4326",
            "topology": "tree",
            "bbox": [-10.0, -5.0, 10.0, 5.0],
            "unit_count": 100,
            "created_at": "2026-01-01T00:00:00Z",
            "adapter_version": "hfx-adapter-v1"
        })
    }

    #[test]
    fn test_valid_minimal_manifest() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(&dir, &minimal_json());

        let parsed = read_manifest(&path).unwrap();
        let manifest = parsed.manifest;

        assert_eq!(manifest.format_version(), FormatVersion::V0_3_0);
        assert_eq!(manifest.fabric_name(), "testfabric");
        assert_eq!(manifest.crs(), Crs::Epsg4326);
        assert_eq!(manifest.topology(), Topology::Tree);
        assert_eq!(manifest.unit_count().get(), 100);
        assert_eq!(manifest.created_at(), "2026-01-01T00:00:00Z");
        assert_eq!(manifest.adapter_version(), "hfx-adapter-v1");
        assert!(parsed.aux.snaps.is_empty());
        assert!(parsed.aux.d8_rasters.is_empty());
    }

    #[test]
    fn test_v01_format_version_rejected_before_missing_fields() {
        let dir = TempDir::new().unwrap();
        // v0.1 manifest: also omits unit_count, but version check must fire first.
        let value = json!({
            "format_version": "0.1",
            "fabric_name": "testfabric"
        });
        let path = write_manifest(&dir, &value);
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, SessionError::UnsupportedFormatVersion { ref found, .. } if found == "0.1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_v021_format_version_rejected_before_missing_fields() {
        let dir = TempDir::new().unwrap();
        let value = json!({
            "format_version": "0.2.1",
            "fabric_name": "testfabric"
        });
        let path = write_manifest(&dir, &value);
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, SessionError::UnsupportedFormatVersion { ref found, .. } if found == "0.2.1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_unsupported_crs() {
        let dir = TempDir::new().unwrap();
        let mut value = minimal_json();
        value["crs"] = json!("EPSG:32632");
        let path = write_manifest(&dir, &value);
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, SessionError::UnsupportedCrs { .. }),
            "got {err}"
        );
    }

    #[test]
    fn test_d8_and_snap_auxiliary_parsed() {
        let dir = TempDir::new().unwrap();
        let mut value = minimal_json();
        value["auxiliary"] = json!([
            {
                "schema": "hfx.aux.d8_raster.v2",
                "artifacts": { "flow_dir": "flow_dir.tif", "flow_acc": "flow_acc.tif" },
                "metadata": {
                    "crs": "EPSG:4326",
                    "flow_dir_encoding": "esri",
                    "flow_acc_units": "cells"
                }
            },
            {
                "schema": "hfx.aux.snap.v2",
                "artifacts": { "snap": "snap/segment_stems.parquet" },
                "metadata": {
                    "name": "segment-stems",
                    "description": "Segment stems.",
                    "references_levels": [0],
                    "weight_semantics": "higher is stronger"
                }
            }
        ]);
        let path = write_manifest(&dir, &value);
        let parsed = read_manifest(&path).unwrap();
        assert_eq!(parsed.aux.d8_rasters.len(), 1);
        assert_eq!(parsed.aux.d8_rasters[0].flow_dir, "flow_dir.tif");
        assert_eq!(
            parsed.aux.d8_rasters[0].metadata.flow_dir_encoding(),
            FlowDirEncoding::Esri
        );
        assert_eq!(
            parsed.aux.d8_rasters[0].metadata.crs().as_str(),
            "EPSG:4326"
        );
        assert_eq!(
            parsed.aux.d8_rasters[0].metadata.flow_acc_units(),
            FlowAccumulationUnits::Cells
        );
        assert_eq!(parsed.aux.snaps.len(), 1);
        assert_eq!(parsed.aux.snaps[0].name, "segment-stems");
        assert_eq!(parsed.aux.snaps[0].references_levels, vec![0]);
    }

    #[test]
    fn test_unreadable_hfx_auxiliary_retained_outside_typed_manifest() {
        let dir = TempDir::new().unwrap();
        let mut value = minimal_json();
        value["auxiliary"] = json!([
            {
                "schema": "hfx.aux.d8_raster.v1",
                "artifacts": {
                    "flow_dir": "missing/v1-dir.tif",
                    "flow_acc": "missing/v1-acc.tif"
                },
                "metadata": {
                    "flow_dir_encoding": "esri",
                    "producer": "legacy-v1"
                }
            },
            {
                "schema": "hfx.aux.d8_raster.v3",
                "artifacts": {
                    "flow_dir": "missing/v3-dir.tif",
                    "flow_acc": "missing/v3-acc.tif"
                },
                "metadata": {"purpose": "future-d8"}
            },
            {
                "schema": "hfx.aux.d8_raster.v1",
                "artifacts": {
                    "flow_dir": "missing/duplicate-v1-dir.tif",
                    "flow_acc": "missing/duplicate-v1-acc.tif"
                },
                "metadata": {
                    "flow_dir_encoding": "grass",
                    "producer": "duplicate-v1"
                }
            },
            {
                "schema": "hfx.aux.snap.v2",
                "artifacts": {"snap": "snap.parquet"},
                "metadata": {
                    "name": "synthetic-outlet-snap",
                    "description": "Synthetic snap target at the unit 3 outlet.",
                    "weight_semantics": "higher is preferred",
                    "references_levels": [0]
                }
            }
        ]);
        let path = write_manifest(&dir, &value);

        let parsed = read_manifest(&path).unwrap();

        assert_eq!(parsed.manifest.auxiliary().len(), 1);
        assert_eq!(
            parsed.manifest.auxiliary()[0].schema().to_string(),
            "hfx.aux.snap.v2"
        );
        assert!(parsed.aux.d8_rasters.is_empty());
        assert!(parsed.aux.generic.is_empty());
        assert_eq!(parsed.aux.snaps.len(), 1);
        assert_eq!(parsed.aux.snaps[0].name, "synthetic-outlet-snap");
        assert_eq!(parsed.aux.snaps[0].snap, "snap.parquet");
        assert_eq!(parsed.aux.snaps[0].references_levels, vec![0]);
        assert_eq!(
            parsed.aux.unreadable,
            vec![
                UnreadableAuxDecl {
                    schema: "hfx.aux.d8_raster.v1".to_string(),
                    artifacts: BTreeMap::from([
                        ("flow_acc".to_string(), "missing/v1-acc.tif".to_string()),
                        ("flow_dir".to_string(), "missing/v1-dir.tif".to_string()),
                    ]),
                    metadata: json!({
                        "flow_dir_encoding": "esri",
                        "producer": "legacy-v1"
                    }),
                },
                UnreadableAuxDecl {
                    schema: "hfx.aux.d8_raster.v3".to_string(),
                    artifacts: BTreeMap::from([
                        ("flow_acc".to_string(), "missing/v3-acc.tif".to_string()),
                        ("flow_dir".to_string(), "missing/v3-dir.tif".to_string()),
                    ]),
                    metadata: json!({"purpose": "future-d8"}),
                },
                UnreadableAuxDecl {
                    schema: "hfx.aux.d8_raster.v1".to_string(),
                    artifacts: BTreeMap::from([
                        (
                            "flow_acc".to_string(),
                            "missing/duplicate-v1-acc.tif".to_string(),
                        ),
                        (
                            "flow_dir".to_string(),
                            "missing/duplicate-v1-dir.tif".to_string(),
                        ),
                    ]),
                    metadata: json!({
                        "flow_dir_encoding": "grass",
                        "producer": "duplicate-v1"
                    }),
                },
            ]
        );
    }

    #[test]
    fn test_missing_required_field() {
        let dir = TempDir::new().unwrap();
        let mut value = minimal_json();
        value.as_object_mut().unwrap().remove("unit_count");
        let path = write_manifest(&dir, &value);
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(
                err,
                SessionError::ManifestFieldMissing {
                    field: "unit_count"
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_invalid_json() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(&path, b"{broken").unwrap();
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, SessionError::ManifestJsonParse { .. }),
            "got {err}"
        );
    }

    #[test]
    fn test_generic_aux_retained_as_handle() {
        let dir = TempDir::new().unwrap();
        let mut value = minimal_json();
        value["auxiliary"] = json!([
            {
                "schema": "org.example.custom.v1",
                "artifacts": { "data": "extra/custom.bin" },
                "metadata": { "anything": 42 }
            }
        ]);
        let path = write_manifest(&dir, &value);
        let parsed = read_manifest(&path).unwrap();
        assert_eq!(parsed.aux.generic.len(), 1);
        assert_eq!(parsed.aux.generic[0].schema, "org.example.custom.v1");
    }
}
