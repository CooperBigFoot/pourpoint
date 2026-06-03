//! Graph reader — loads HFX v0.2.1 graph.parquet into a DrainageGraph.

use std::path::Path;

use arrow::array::{Array, Int16Array, Int64Array, LargeListArray, ListArray};
use arrow::datatypes::DataType;
use hfx_core::{AdjacencyRow, DrainageGraph, Level, UnitId};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use tracing::{debug, info, instrument};

use crate::error::SessionError;

const ARTIFACT: &str = "graph.parquet";
const BBOX_COLUMNS: [&str; 4] = ["bbox_minx", "bbox_miny", "bbox_maxx", "bbox_maxy"];

/// Load `graph.parquet` from `path` and return a [`DrainageGraph`].
///
/// # Errors
///
/// | Condition | Error variant |
/// |-----------|---------------|
/// | File cannot be opened | [`SessionError::Io`] |
/// | File is not valid Parquet | [`SessionError::ParquetParse`] |
/// | Schema missing or wrong column type | [`SessionError::GraphSchema`] |
/// | A row contains an invalid unit ID or level | [`SessionError::InvalidRow`] |
/// | Graph domain validation fails | [`SessionError::GraphDomain`] |
#[instrument(skip_all, fields(path = %path.display()))]
pub fn load_graph(path: &Path) -> Result<DrainageGraph, SessionError> {
    let file = std::fs::File::open(path).map_err(|e| SessionError::io(ARTIFACT, e))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|source| SessionError::ParquetParse {
            artifact: ARTIFACT,
            source,
        })?;
    read_graph_from_builder(builder)
}

/// Load `graph.parquet` bytes and return a [`DrainageGraph`].
///
/// # Errors
///
/// See [`load_graph`].
#[instrument(skip_all, fields(byte_len = bytes.len()))]
pub fn load_graph_from_bytes(bytes: bytes::Bytes) -> Result<DrainageGraph, SessionError> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)
        .map_err(|source| SessionError::ParquetParse {
            artifact: ARTIFACT,
            source: source.into(),
        })?;
    read_graph_from_builder(builder)
}

fn read_graph_from_builder<R>(
    builder: ParquetRecordBatchReaderBuilder<R>,
) -> Result<DrainageGraph, SessionError>
where
    R: parquet::file::reader::ChunkReader + 'static,
{
    validate_schema(builder.schema())?;
    debug!("graph.parquet schema validated, reading record batches");

    let reader = builder
        .build()
        .map_err(|source| SessionError::ParquetParse {
            artifact: ARTIFACT,
            source,
        })?;

    let mut rows = Vec::new();
    let mut global_row = 0usize;
    for batch_result in reader {
        let batch = batch_result.map_err(|source| SessionError::ParquetParse {
            artifact: ARTIFACT,
            source: source.into(),
        })?;
        let num_rows = batch.num_rows();

        let id_arr = batch
            .column_by_name("id")
            .and_then(|col| col.as_any().downcast_ref::<Int64Array>())
            .ok_or_else(|| SessionError::graph_schema("column \"id\" is not Int64"))?;
        let level_arr = batch
            .column_by_name("level")
            .and_then(|col| col.as_any().downcast_ref::<Int16Array>())
            .ok_or_else(|| SessionError::graph_schema("column \"level\" is not Int16"))?;
        let upstream_col = batch.column_by_name("upstream_ids").ok_or_else(|| {
            SessionError::graph_schema("column \"upstream_ids\" missing from record batch")
        })?;

        for i in 0..num_rows {
            let row_idx = global_row + i;
            if id_arr.is_null(i) {
                return Err(SessionError::invalid_row(
                    ARTIFACT,
                    row_idx,
                    "null value in non-nullable column \"id\"",
                ));
            }
            if level_arr.is_null(i) {
                return Err(SessionError::invalid_row(
                    ARTIFACT,
                    row_idx,
                    "null value in non-nullable column \"level\"",
                ));
            }

            let raw_id = id_arr.value(i);
            let id = UnitId::new(raw_id).map_err(|e| {
                SessionError::invalid_row(ARTIFACT, row_idx, format!("invalid unit id {raw_id}: {e}"))
            })?;
            let raw_level = level_arr.value(i);
            let level = Level::new(raw_level).map_err(|e| {
                SessionError::invalid_row(
                    ARTIFACT,
                    row_idx,
                    format!("invalid level {raw_level}: {e}"),
                )
            })?;
            let upstream_ids = extract_upstream(upstream_col, i, row_idx)?;
            rows.push(AdjacencyRow::new(id, level, upstream_ids));
        }

        global_row += num_rows;
    }

    let row_count = rows.len();
    let graph = DrainageGraph::new(rows).map_err(|source| SessionError::GraphDomain { source })?;
    info!(row_count, "graph.parquet loaded");
    Ok(graph)
}

fn validate_schema(schema: &arrow::datatypes::SchemaRef) -> Result<(), SessionError> {
    require_graph_column(schema, "id", &DataType::Int64)?;
    require_graph_column(schema, "level", &DataType::Int16)?;
    match schema.field_with_name("upstream_ids") {
        Ok(field) if is_list_int64(field.data_type()) => {}
        Ok(field) => {
            return Err(SessionError::graph_schema(format!(
                "column \"upstream_ids\" has type {:?}, expected List(Int64) or LargeList(Int64)",
                field.data_type()
            )));
        }
        Err(_) => {
            return Err(SessionError::graph_schema(
                "missing required column \"upstream_ids\" (expected List(Int64))",
            ));
        }
    }
    for column in BBOX_COLUMNS {
        if schema.field_with_name(column).is_err() {
            return Err(SessionError::GraphMissingBboxColumn { column });
        }
    }
    Ok(())
}

fn require_graph_column(
    schema: &arrow::datatypes::SchemaRef,
    name: &'static str,
    expected: &DataType,
) -> Result<(), SessionError> {
    match schema.field_with_name(name) {
        Ok(field) if field.data_type() == expected => Ok(()),
        Ok(field) => Err(SessionError::graph_schema(format!(
            "column {name:?} has type {:?}, expected {expected:?}",
            field.data_type()
        ))),
        Err(_) => Err(SessionError::graph_schema(format!(
            "missing required column {name:?} (expected {expected:?})"
        ))),
    }
}

fn is_list_int64(dt: &DataType) -> bool {
    matches!(
        dt,
        DataType::List(f) | DataType::LargeList(f) if f.data_type() == &DataType::Int64
    )
}

fn extract_upstream(
    col: &dyn Array,
    i: usize,
    row_idx: usize,
) -> Result<Vec<UnitId>, SessionError> {
    if let Some(list_arr) = col.as_any().downcast_ref::<ListArray>() {
        if list_arr.is_null(i) {
            return Err(SessionError::invalid_row(
                ARTIFACT,
                row_idx,
                "null value in non-nullable column \"upstream_ids\"",
            ));
        }
        let values = list_arr.value(i);
        let int_arr = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| {
                SessionError::graph_schema("inner values of \"upstream_ids\" are not Int64")
            })?;
        convert_upstream_values(int_arr.values(), row_idx)
    } else if let Some(list_arr) = col.as_any().downcast_ref::<LargeListArray>() {
        if list_arr.is_null(i) {
            return Err(SessionError::invalid_row(
                ARTIFACT,
                row_idx,
                "null value in non-nullable column \"upstream_ids\"",
            ));
        }
        let values = list_arr.value(i);
        let int_arr = values
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| {
                SessionError::graph_schema("inner values of \"upstream_ids\" are not Int64")
            })?;
        convert_upstream_values(int_arr.values(), row_idx)
    } else {
        Err(SessionError::graph_schema(
            "upstream_ids column has unexpected type after schema validation",
        ))
    }
}

fn convert_upstream_values(
    values: &arrow::buffer::ScalarBuffer<i64>,
    row_idx: usize,
) -> Result<Vec<UnitId>, SessionError> {
    values
        .iter()
        .map(|&raw| {
            UnitId::new(raw).map_err(|e| {
                SessionError::invalid_row(
                    ARTIFACT,
                    row_idx,
                    format!("invalid upstream unit id {raw}: {e}"),
                )
            })
        })
        .collect()
}
