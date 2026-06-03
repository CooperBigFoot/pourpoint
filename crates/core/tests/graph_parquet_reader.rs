use std::path::Path;
use std::sync::Arc;

use arrow::array::{
    BinaryBuilder, Float32Builder, Float64Builder, Int16Array, Int16Builder, Int64Array,
    Int64Builder, ListBuilder, RecordBatch,
};
use arrow::datatypes::{DataType, Field, Schema};
use hfx_core::UnitId;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use shed_core::error::SessionError;
use shed_core::reader::graph::load_graph;
use shed_core::session::DatasetSession;
use shed_core::testutil::DatasetBuilder;

#[test]
fn reads_graph_parquet_with_level_and_upstream_ids() {
    let (_dir, root) = DatasetBuilder::new(3).build();

    let graph = load_graph(&root.join("graph.parquet")).expect("graph.parquet should load");

    let row = graph.get(UnitId::new(3).unwrap()).expect("unit 3 row");
    assert_eq!(row.level().get(), 0);
    assert_eq!(row.upstream_ids(), &[UnitId::new(2).unwrap()]);
}

#[test]
fn rejects_graph_parquet_missing_bbox_column_for_stats_contract() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(3, 0, &[2]),
        ],
        BboxColumns::OmitMaxY,
    );

    let err = load_graph(&root.join("graph.parquet")).unwrap_err();

    assert!(matches!(
        err,
        SessionError::GraphMissingBboxColumn {
            column: "bbox_maxy"
        }
    ));
}

#[test]
fn realistic_graph_fixture_carries_bbox_columns_for_row_group_stats() {
    let (_dir, root) = DatasetBuilder::new(3).with_dag().build();

    let file = std::fs::File::open(root.join("graph.parquet")).unwrap();
    let builder =
        parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let schema = builder.schema();

    // Fixture-equivalent assertion for Step 3: HFX_SPEC.md requires Parquet
    // row-group statistics on graph bbox_* columns, so fixture graphs must
    // physically carry them. Step 6's ignored GRIT tier must repeat this
    // assertion against real graph.parquet data.
    for column in ["bbox_minx", "bbox_miny", "bbox_maxx", "bbox_maxy"] {
        assert!(
            schema.field_with_name(column).is_ok(),
            "graph.parquet missing {column}"
        );
    }
}

#[test]
fn graph_missing_catchment_row_is_referential_integrity_error() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[GraphRow::headwater(1), GraphRow::new(2, 0, &[1])],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "row count");
}

#[test]
fn graph_extra_row_is_referential_integrity_error() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(3, 0, &[2]),
            GraphRow::new(4, 0, &[3]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "row count");
}

#[test]
fn graph_duplicate_row_is_referential_integrity_error() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(2, 0, &[1]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "duplicate graph unit");
}

#[test]
fn graph_row_id_must_exist_as_catchment() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(4, 0, &[2]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "no corresponding catchment row");
}

#[test]
fn graph_upstream_id_must_exist_as_catchment() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(3, 0, &[99]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "references upstream unit 99");
}

#[test]
fn graph_row_level_must_match_catchment_level() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_catchments_parquet(&root, &[(1, 0), (2, 1), (3, 0)]);
    write_graph_parquet(
        &root,
        &[
            GraphRow::headwater(1),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(3, 0, &[2]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "differs from catchment level");
}

#[test]
fn graph_edge_must_not_cross_levels() {
    let (_dir, root) = DatasetBuilder::new(3).build();
    write_catchments_parquet(&root, &[(1, 1), (2, 0), (3, 0)]);
    write_graph_parquet(
        &root,
        &[
            GraphRow::new(1, 1, &[]),
            GraphRow::new(2, 0, &[1]),
            GraphRow::new(3, 0, &[2]),
        ],
        BboxColumns::All,
    );

    assert_graph_integrity_error(&root, "crosses levels");
}

fn assert_graph_integrity_error(root: &Path, expected_reason: &str) {
    let err = DatasetSession::open_path(root).unwrap_err();
    assert!(
        matches!(
            err,
            SessionError::GraphReferentialIntegrity { ref reason }
                if reason.contains(expected_reason)
        ),
        "expected GraphReferentialIntegrity containing {expected_reason:?}, got {err}"
    );
}

#[derive(Clone, Copy)]
enum BboxColumns {
    All,
    OmitMaxY,
}

struct GraphRow {
    id: i64,
    level: i16,
    upstream_ids: Vec<i64>,
}

impl GraphRow {
    fn headwater(id: i64) -> Self {
        Self::new(id, 0, &[])
    }

    fn new(id: i64, level: i16, upstream_ids: &[i64]) -> Self {
        Self {
            id,
            level,
            upstream_ids: upstream_ids.to_vec(),
        }
    }
}

fn write_graph_parquet(root: &Path, rows: &[GraphRow], bbox_columns: BboxColumns) {
    let mut fields = vec![
        Field::new("id", DataType::Int64, false),
        Field::new("level", DataType::Int16, false),
        Field::new(
            "upstream_ids",
            DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
            false,
        ),
        Field::new("bbox_minx", DataType::Float32, false),
        Field::new("bbox_miny", DataType::Float32, false),
        Field::new("bbox_maxx", DataType::Float32, false),
    ];
    if matches!(bbox_columns, BboxColumns::All) {
        fields.push(Field::new("bbox_maxy", DataType::Float32, false));
    }
    let schema = Arc::new(Schema::new(fields));

    let id_arr = Int64Array::from_iter(rows.iter().map(|row| row.id));
    let level_arr = Int16Array::from_iter(rows.iter().map(|row| row.level));
    let mut upstream_builder = ListBuilder::new(Int64Builder::new());
    let mut minx_b = Float32Builder::new();
    let mut miny_b = Float32Builder::new();
    let mut maxx_b = Float32Builder::new();
    let mut maxy_b = Float32Builder::new();

    for (idx, row) in rows.iter().enumerate() {
        for &upstream_id in &row.upstream_ids {
            upstream_builder.values().append_value(upstream_id);
        }
        upstream_builder.append(true);

        let minx = 0.5f32 + idx as f32;
        minx_b.append_value(minx);
        miny_b.append_value(0.0);
        maxx_b.append_value(minx + 0.4);
        maxy_b.append_value(0.4);
    }

    let mut columns: Vec<Arc<dyn arrow::array::Array>> = vec![
        Arc::new(id_arr),
        Arc::new(level_arr),
        Arc::new(upstream_builder.finish()),
        Arc::new(minx_b.finish()),
        Arc::new(miny_b.finish()),
        Arc::new(maxx_b.finish()),
    ];
    if matches!(bbox_columns, BboxColumns::All) {
        columns.push(Arc::new(maxy_b.finish()));
    }

    let batch = RecordBatch::try_new(schema.clone(), columns).unwrap();
    let file = std::fs::File::create(root.join("graph.parquet")).unwrap();
    let props = WriterProperties::builder()
        .set_statistics_enabled(EnabledStatistics::Chunk)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn write_catchments_parquet(root: &Path, rows: &[(i64, i16)]) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("level", DataType::Int16, false),
        Field::new("parent_id", DataType::Int64, true),
        Field::new("area_km2", DataType::Float32, false),
        Field::new("up_area_km2", DataType::Float32, true),
        Field::new("outlet_lon", DataType::Float64, false),
        Field::new("outlet_lat", DataType::Float64, false),
        Field::new("bbox_minx", DataType::Float32, false),
        Field::new("bbox_miny", DataType::Float32, false),
        Field::new("bbox_maxx", DataType::Float32, false),
        Field::new("bbox_maxy", DataType::Float32, false),
        Field::new("geometry", DataType::Binary, false),
    ]));

    let mut id_b = Int64Builder::new();
    let mut level_b = Int16Builder::new();
    let mut parent_id_b = Int64Builder::new();
    let mut area_b = Float32Builder::new();
    let mut up_area_b = Float32Builder::new();
    let mut outlet_lon_b = Float64Builder::new();
    let mut outlet_lat_b = Float64Builder::new();
    let mut minx_b = Float32Builder::new();
    let mut miny_b = Float32Builder::new();
    let mut maxx_b = Float32Builder::new();
    let mut maxy_b = Float32Builder::new();
    let mut geom_b = BinaryBuilder::new();

    for (idx, &(id, level)) in rows.iter().enumerate() {
        let minx = 0.5 + idx as f64;
        let miny = 0.0;
        let maxx = minx + 0.4;
        let maxy = 0.4;

        id_b.append_value(id);
        level_b.append_value(level);
        parent_id_b.append_null();
        area_b.append_value(10.0);
        up_area_b.append_null();
        outlet_lon_b.append_value((minx + maxx) / 2.0);
        outlet_lat_b.append_value((miny + maxy) / 2.0);
        minx_b.append_value(minx as f32);
        miny_b.append_value(miny as f32);
        maxx_b.append_value(maxx as f32);
        maxy_b.append_value(maxy as f32);
        geom_b.append_value(minimal_wkb_polygon(minx, miny, maxx, maxy));
    }

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_b.finish()),
            Arc::new(level_b.finish()),
            Arc::new(parent_id_b.finish()),
            Arc::new(area_b.finish()),
            Arc::new(up_area_b.finish()),
            Arc::new(outlet_lon_b.finish()),
            Arc::new(outlet_lat_b.finish()),
            Arc::new(minx_b.finish()),
            Arc::new(miny_b.finish()),
            Arc::new(maxx_b.finish()),
            Arc::new(maxy_b.finish()),
            Arc::new(geom_b.finish()),
        ],
    )
    .unwrap();

    let file = std::fs::File::create(root.join("catchments.parquet")).unwrap();
    let props = WriterProperties::builder()
        .set_statistics_enabled(EnabledStatistics::Chunk)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn minimal_wkb_polygon(minx: f64, miny: f64, maxx: f64, maxy: f64) -> Vec<u8> {
    let mut wkb = Vec::new();
    wkb.push(1u8);
    wkb.extend_from_slice(&3u32.to_le_bytes());
    wkb.extend_from_slice(&1u32.to_le_bytes());
    wkb.extend_from_slice(&5u32.to_le_bytes());
    for (x, y) in [
        (minx, miny),
        (maxx, miny),
        (maxx, maxy),
        (minx, maxy),
        (minx, miny),
    ] {
        wkb.extend_from_slice(&x.to_le_bytes());
        wkb.extend_from_slice(&y.to_le_bytes());
    }
    wkb
}
