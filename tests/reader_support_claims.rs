use std::path::Path;

use assert_cmd::Command;
use pourpoint_core::support_claims::{
    CORE_MANIFEST_SUPPORT_CLAIMS, DATASET_CRS_EPSG_4326, FORMAT_VERSION_V0_3_0, ReaderSupportValue,
};
use pourpoint_core::testutil::DatasetBuilder;
use serde_json::Value;

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
