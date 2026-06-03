use hfx_core::BoundingBox;
use shed_core::session::DatasetSession;
use shed_core::testutil::DatasetBuilder;

#[test]
fn manifest_selected_snap_aux_loads_and_queries() {
    let (_dir, root) = DatasetBuilder::new(2).with_snap().build();
    let session = DatasetSession::open_path(&root).expect("snap aux fixture should load");
    let snap = session.snap().expect("snap store should be present");

    let bbox = BoundingBox::new(-180.0, -1.0, 180.0, 1.0).unwrap();
    let results = snap.query_by_bbox(&bbox).expect("snap query should succeed");

    assert!(!results.is_empty());
    assert_eq!(results[0].stem_role().map(|role| role.to_string()).as_deref(), Some("mainstem"));
}
