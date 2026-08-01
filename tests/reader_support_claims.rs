use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use pourpoint_core::support_claims::{
    CORE_MANIFEST_SUPPORT_CLAIMS, DATASET_CRS_EPSG_4326, FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS,
    FORMAT_VERSION_V0_3_0, ReaderSupportValue,
};
use pourpoint_core::testutil::DatasetBuilder;
use serde_json::{Value, json};

fn pourpoint() -> Command {
    Command::cargo_bin("pourpoint").unwrap()
}

fn invoke_delineate(root: &Path) -> (bool, Value) {
    let output = pourpoint()
        .args([
            "delineate",
            "--dataset",
            root.to_str().unwrap(),
            "--lat",
            "0.20",
            "--lon",
            "1.70",
            "--no-refine",
            "--json",
        ])
        .output()
        .expect("failed to execute the shipped pourpoint binary");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    let json = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("CLI stdout should be JSON: {error}\nstdout={stdout}"));
    (output.status.success(), json)
}

fn invoke_refined_delineate(root: &Path) -> Value {
    let output = pourpoint()
        .args([
            "delineate",
            "--dataset",
            root.to_str().unwrap(),
            "--lat",
            "0.4166666666666667",
            "--lon",
            "0.9833333333333333",
            "--snap-radius",
            "1000",
            "--snap-threshold",
            "500",
        ])
        .output()
        .expect("failed to execute the shipped pourpoint binary");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout should be UTF-8");
    assert!(
        output.status.success(),
        "refined delineation should succeed: {stdout}"
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("CLI stdout should be JSON: {error}\nstdout={stdout}"))
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir(destination).expect("fixture copy directory should be creatable");
    for entry in fs::read_dir(source).expect("fixture directory should be readable") {
        let entry = entry.expect("fixture directory entry should be readable");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("fixture entry type should be readable")
            .is_dir()
        {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("fixture file should be copied");
        }
    }
}

fn projected_grass_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass")
}

fn assert_rasters_are_byte_identical(source: &Path, copy: &Path) {
    for raster in [
        "aux/d8/projected/flow_dir.tif",
        "aux/d8/projected/flow_acc.tif",
    ] {
        assert_eq!(
            fs::read(source.join(raster)).expect("tracked raster should be readable"),
            fs::read(copy.join(raster)).expect("copied raster should be readable")
        );
    }
}

fn fixture_copy_with_flow_direction_encoding(declaration: &str) -> (tempfile::TempDir, PathBuf) {
    let source = projected_grass_fixture();
    let directory = tempfile::tempdir().expect("temporary fixture directory should be creatable");
    let root = directory.path().join("dataset");
    copy_directory(&source, &root);
    assert_rasters_are_byte_identical(&source, &root);

    let manifest_path = root.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).expect("copied fixture manifest should be readable"),
    )
    .expect("copied fixture manifest should be valid JSON");
    manifest["auxiliary"][0]["metadata"]["flow_dir_encoding"] =
        Value::String(declaration.to_owned());
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("edited manifest should serialize"),
    )
    .expect("edited fixture manifest should be writable");

    assert_rasters_are_byte_identical(&source, &root);
    (directory, root)
}

fn assert_reference_outcomes_are_pairwise_distinct() {
    let outcomes = [
        ("applied(lon=0.986445, lat=0.416385)", json!([13, 1289])),
        ("applied(lon=0.986445, lat=0.416385)", json!([13, 13])),
        (
            "best_effort_skipped(BestEffortSkipped { strategy: BestEffortD8IfPresent, why: MisDeclaration { source: RasterLoad, diagnostic: \"flow-direction nodata byte 128 decodes as a legal direction under Esri encoding\" } })",
            json!([25]),
        ),
    ];
    assert_ne!(outcomes[0], outcomes[1]);
    assert_ne!(outcomes[0], outcomes[2]);
    assert_ne!(outcomes[1], outcomes[2]);
}

fn assert_encoding_outcome(
    declaration: &str,
    expected_refinement: Value,
    expected_area_km2: f64,
    expected_ring_vertex_counts: Value,
) {
    let (_directory, root) = fixture_copy_with_flow_direction_encoding(declaration);
    let output = invoke_refined_delineate(&root);
    let features = output["features"]
        .as_array()
        .expect("CLI output should contain a feature array");
    assert_eq!(features.len(), 1);
    let feature = &features[0];
    assert_eq!(feature["properties"]["refinement"], expected_refinement);
    let actual_area_km2 = feature["properties"]["area_km2"]
        .as_f64()
        .expect("area_km2 should be numeric");
    assert!((actual_area_km2 - expected_area_km2).abs() <= 0.025_f64);

    let ring_vertex_counts = Value::Array(
        feature["geometry"]["coordinates"]
            .as_array()
            .expect("geometry coordinates should contain polygons")
            .iter()
            .flat_map(|polygon| {
                polygon
                    .as_array()
                    .expect("polygon coordinates should contain rings")
                    .iter()
            })
            .map(|ring| {
                Value::from(
                    ring.as_array()
                        .expect("ring coordinates should contain vertices")
                        .len(),
                )
            })
            .collect(),
    );
    assert_eq!(ring_vertex_counts, expected_ring_vertex_counts);
}

fn replace_manifest_field(root: &Path, field: &str, replacement: &str) {
    let path = root.join("manifest.json");
    let bytes = std::fs::read(&path).expect("fixture manifest should be readable");
    let mut manifest: Value =
        serde_json::from_slice(&bytes).expect("fixture manifest should be valid JSON");
    let value = manifest
        .get_mut(field)
        .unwrap_or_else(|| panic!("fixture manifest should contain {field}"));
    *value = Value::String(replacement.to_owned());
    std::fs::write(
        path,
        serde_json::to_vec(&manifest).expect("edited manifest should serialize"),
    )
    .expect("edited fixture manifest should be writable");
}

#[test]
fn core_manifest_inventory_is_typed() {
    assert_eq!(CORE_MANIFEST_SUPPORT_CLAIMS.len(), 2);

    let format_version = &CORE_MANIFEST_SUPPORT_CLAIMS[0];
    assert_eq!(format_version.id().as_str(), "core-format-version-0.3.0");
    assert_eq!(format_version.canonical_declaration(), "0.3.0");
    assert!(matches!(
        format_version.value(),
        ReaderSupportValue::FormatVersion(hfx::FormatVersion::V0_3_0)
    ));

    let dataset_crs = &CORE_MANIFEST_SUPPORT_CLAIMS[1];
    assert_eq!(dataset_crs.id().as_str(), "core-dataset-crs-epsg-4326");
    assert_eq!(dataset_crs.canonical_declaration(), "EPSG:4326");
    assert!(matches!(
        dataset_crs.value(),
        ReaderSupportValue::DatasetCrs(hfx::Crs::Epsg4326)
    ));
}

#[test]
fn format_version_claim_has_shipped_cli_evidence() {
    assert_eq!(
        FORMAT_VERSION_V0_3_0.id().as_str(),
        "core-format-version-0.3.0"
    );
    let (_directory, root) = DatasetBuilder::new(3).build();

    let (succeeded, json) = invoke_delineate(&root);
    assert!(succeeded, "claimed format version should succeed: {json}");
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_field(&root, "format_version", "0.2.1");
    let (succeeded, json) = invoke_delineate(&root);
    assert!(!succeeded, "unsupported format version should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: unsupported HFX format version \"0.2.1\", expected \"0.3.0\""
    );
}

#[test]
fn dataset_crs_claim_has_shipped_cli_evidence() {
    assert_eq!(
        DATASET_CRS_EPSG_4326.id().as_str(),
        "core-dataset-crs-epsg-4326"
    );
    let (_directory, root) = DatasetBuilder::new(3).build();

    let (succeeded, json) = invoke_delineate(&root);
    assert!(succeeded, "claimed dataset CRS should succeed: {json}");
    assert_eq!(json["successes"].as_array().map(Vec::len), Some(1));

    replace_manifest_field(&root, "crs", "EPSG:3857");
    let (succeeded, json) = invoke_delineate(&root);
    assert!(!succeeded, "unsupported dataset CRS should fail");
    assert_eq!(
        json["error"],
        "failed to open HFX dataset session: unsupported CRS \"EPSG:3857\", expected \"EPSG:4326\""
    );
}

#[test]
fn grass_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    assert_reference_outcomes_are_pairwise_distinct();
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[2];
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-grass");
    assert_eq!(claim.canonical_declaration(), "grass");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Grass)
    ));
    assert_encoding_outcome(
        "grass",
        json!("applied(lon=0.986445, lat=0.416385)"),
        24986.140564067347,
        json!([13, 1289]),
    );
}

#[test]
fn taudem_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    assert_reference_outcomes_are_pairwise_distinct();
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[1];
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-taudem");
    assert_eq!(claim.canonical_declaration(), "taudem");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Taudem)
    ));
    assert_encoding_outcome(
        "taudem",
        json!("applied(lon=0.986445, lat=0.416385)"),
        24613.14053443639,
        json!([13, 13]),
    );
}

#[test]
fn esri_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence() {
    assert_reference_outcomes_are_pairwise_distinct();
    let claim = &FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS[0];
    assert_eq!(claim.id().as_str(), "core-flow-dir-encoding-esri");
    assert_eq!(claim.canonical_declaration(), "esri");
    assert!(matches!(
        claim.value(),
        ReaderSupportValue::FlowDirectionEncoding(hfx::FlowDirEncoding::Esri)
    ));
    assert_encoding_outcome(
        "esri",
        json!(
            "best_effort_skipped(BestEffortSkipped { strategy: BestEffortD8IfPresent, why: MisDeclaration { source: RasterLoad, diagnostic: \"flow-direction nodata byte 128 decodes as a legal direction under Esri encoding\" } })"
        ),
        36922.8059387193,
        json!([25]),
    );
}
