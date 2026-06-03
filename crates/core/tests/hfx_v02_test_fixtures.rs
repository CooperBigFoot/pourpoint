use serde_json::Value;
use shed_core::testutil::DatasetBuilder;

#[test]
fn fixture_builder_emits_v021_manifest_and_core_artifacts() {
    let (_dir, root) = DatasetBuilder::new(2).with_snap().with_rasters().build();

    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();

    assert_eq!(manifest["format_version"], "0.2.1");
    assert_eq!(manifest["unit_count"], 2);
    assert!(root.join("catchments.parquet").is_file());
    assert!(root.join("graph.parquet").is_file());
    assert!(root.join("snap.parquet").is_file());
    assert!(manifest["auxiliary"].as_array().unwrap().len() >= 2);
}
