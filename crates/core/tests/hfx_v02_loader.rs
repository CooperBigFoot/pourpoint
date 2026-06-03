use shed_core::error::SessionError;
use shed_core::session::DatasetSession;
use shed_core::testutil::DatasetBuilder;

#[test]
fn loads_minimal_v021_dataset() {
    let (_dir, root) = DatasetBuilder::new(3).build();

    let session = DatasetSession::open_path(&root).expect("v0.2.1 fixture should load");

    assert_eq!(session.manifest().format_version().to_string(), "0.2.1");
    assert_eq!(session.manifest().unit_count().get(), 3);
    assert_eq!(session.graph().len(), 3);
}

#[test]
fn rejects_v01_manifest_before_missing_v021_fields() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("manifest.json"),
        r#"{"format_version":"0.1","fabric_name":"testfabric"}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("graph.parquet"), []).unwrap();
    std::fs::write(dir.path().join("catchments.parquet"), []).unwrap();

    let err = DatasetSession::open_path(dir.path()).unwrap_err();
    assert!(matches!(
        err,
        SessionError::UnsupportedFormatVersion { ref found, .. } if found == "0.1"
    ));
}
