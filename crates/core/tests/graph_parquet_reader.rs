use hfx_core::UnitId;
use shed_core::reader::graph::load_graph;
use shed_core::testutil::DatasetBuilder;

#[test]
fn reads_graph_parquet_with_level_and_upstream_ids() {
    let (_dir, root) = DatasetBuilder::new(3).build();

    let graph = load_graph(&root.join("graph.parquet")).expect("graph.parquet should load");

    let row = graph.get(UnitId::new(3).unwrap()).expect("unit 3 row");
    assert_eq!(row.level().get(), 0);
    assert_eq!(row.upstream_ids(), &[UnitId::new(2).unwrap()]);
}
