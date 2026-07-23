use std::collections::BTreeSet;
use std::path::Path;

use arrow::array::{Array, Int16Array};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use pourpoint_core::session::DatasetSession;
use pourpoint_core::testutil::DatasetBuilder;
use serde_json::Value;

const PARITY_FIXTURE_ROOT: &str = "tests/fixtures/parity";
const V021_SYNTHETIC_REFINED_DIR: &str = "v021_synthetic_refined";

#[test]
fn fixture_builder_emits_v021_manifest_and_core_artifacts() {
    let (_dir, root) = DatasetBuilder::new(2).with_snap().with_rasters().build();

    let manifest = read_manifest(&root);

    assert_eq!(manifest["format_version"], "0.3.0");
    assert_eq!(manifest["unit_count"], 2);
    assert!(root.join("catchments.parquet").is_file());
    assert!(root.join("graph.parquet").is_file());
    assert_graph_has_bbox_columns(&root.join("graph.parquet"));
}

#[test]
fn fixture_builder_declares_snap_and_d8_aux_artifacts() {
    let (_dir, root) = DatasetBuilder::new(2).with_snap().with_rasters().build();

    let manifest = read_manifest(&root);
    let auxiliary = manifest["auxiliary"].as_array().unwrap();
    let schemas = auxiliary
        .iter()
        .map(|decl| decl["schema"].as_str().unwrap())
        .collect::<BTreeSet<_>>();

    assert!(schemas.contains("hfx.aux.snap.v2"));
    assert!(schemas.contains("hfx.aux.d8_raster.v2"));
    let d8 = auxiliary
        .iter()
        .find(|decl| decl["schema"] == "hfx.aux.d8_raster.v2")
        .unwrap();
    assert_eq!(
        d8["metadata"],
        serde_json::json!({
            "crs": "EPSG:4326",
            "flow_dir_encoding": "esri",
            "flow_acc_units": "cells"
        })
    );
    assert!(root.join("snap.parquet").is_file());
    assert!(root.join("flow_dir.tif").is_file());
    assert!(root.join("flow_acc.tif").is_file());
}

#[test]
fn fixture_builder_emits_distinct_single_multilevel_and_dag_shapes() {
    let (_single_dir, single_root) = DatasetBuilder::new(3).build();
    let single = DatasetSession::open_path(&single_root).unwrap();
    assert_eq!(single.manifest().unit_count().get(), 3);
    assert_eq!(single.topology(), hfx::Topology::Tree);
    assert_eq!(
        graph_levels(&single_root.join("graph.parquet")),
        BTreeSet::from([0])
    );

    let (_nested_dir, nested_root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let nested = DatasetSession::open_path(&nested_root).unwrap();
    assert_eq!(nested.manifest().unit_count().get(), 4);
    assert_eq!(nested.topology(), hfx::Topology::Tree);
    assert_eq!(
        graph_levels(&nested_root.join("graph.parquet")),
        BTreeSet::from([0, 1])
    );
    assert_catchments_have_parent_id(&nested_root.join("catchments.parquet"));

    let (_dag_dir, dag_root) = DatasetBuilder::new(3).with_dag().build();
    let dag = DatasetSession::open_path(&dag_root).unwrap();
    assert_eq!(dag.manifest().unit_count().get(), 7);
    assert_eq!(dag.topology(), hfx::Topology::Dag);
    assert_eq!(
        dag.graph()
            .get(hfx::UnitId::new(6).unwrap())
            .unwrap()
            .upstream_ids()
            .len(),
        2
    );
    assert_eq!(
        dag.graph()
            .get(hfx::UnitId::new(7).unwrap())
            .unwrap()
            .upstream_ids()
            .len(),
        2
    );
}

#[test]
fn converted_parity_fixture_is_separate_v021_d8_fixture() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(PARITY_FIXTURE_ROOT)
        .join(V021_SYNTHETIC_REFINED_DIR);
    let manifest = read_manifest(&root);

    assert_eq!(manifest["format_version"], "0.3.0");
    assert_eq!(manifest["unit_count"], 1);
    assert!(root.join("catchments.parquet").is_file());
    assert!(root.join("graph.parquet").is_file());
    assert!(!root.join("graph.arrow").exists());
    assert_graph_has_bbox_columns(&root.join("graph.parquet"));
    assert!(
        manifest["auxiliary"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decl| {
                decl["schema"] == "hfx.aux.d8_raster.v2"
                    && decl["metadata"]
                        == serde_json::json!({
                            "crs": "EPSG:4326",
                            "flow_dir_encoding": "esri",
                            "flow_acc_units": "cells"
                        })
            })
    );
    assert!(root.join("flow_dir.tif").is_file());
    assert!(root.join("flow_acc.tif").is_file());
    DatasetSession::open_path(&root).expect("converted v0.2.1 parity fixture should open");
}

fn read_manifest(root: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap()
}

fn assert_graph_has_bbox_columns(path: &Path) {
    let schema = parquet_schema(path);
    for column in ["bbox_minx", "bbox_miny", "bbox_maxx", "bbox_maxy"] {
        assert!(
            schema.field_with_name(column).is_ok(),
            "graph.parquet should include {column}"
        );
    }
}

fn assert_catchments_have_parent_id(path: &Path) {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();
    let mut saw_parent = false;
    for batch in reader {
        let batch = batch.unwrap();
        let parent_ids = batch
            .column_by_name("parent_id")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .unwrap();
        saw_parent |= (0..parent_ids.len()).any(|idx| !parent_ids.is_null(idx));
    }
    assert!(
        saw_parent,
        "nested fixture should include child parent_id values"
    );
}

fn graph_levels(path: &Path) -> BTreeSet<i16> {
    let file = std::fs::File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let reader = builder.build().unwrap();
    let mut levels = BTreeSet::new();
    for batch in reader {
        let batch = batch.unwrap();
        let level_arr = batch
            .column_by_name("level")
            .unwrap()
            .as_any()
            .downcast_ref::<Int16Array>()
            .unwrap();
        levels.extend((0..level_arr.len()).map(|idx| level_arr.value(idx)));
    }
    levels
}

fn parquet_schema(path: &Path) -> arrow::datatypes::SchemaRef {
    let file = std::fs::File::open(path).unwrap();
    ParquetRecordBatchReaderBuilder::try_new(file)
        .unwrap()
        .schema()
        .clone()
}
