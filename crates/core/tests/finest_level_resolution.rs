use std::collections::BTreeSet;
use std::sync::Arc;

use arrow::array::{Array, BinaryArray, Float32Array, Int16Array, Int64Array, LargeBinaryArray};
use geo::{Contains, Geometry, Intersects};
use geozero::ToGeo;
use geozero::wkb::Wkb;
use hfx_core::{Level, UnitId, WkbGeometry};
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use parquet::arrow::ProjectionMask;
use parquet::arrow::async_reader::{ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use parquet::file::statistics::Statistics;
use shed_core::algo::coord::GeoCoord;
use shed_core::reader::graph::max_level_from_row_group_statistics;
use shed_core::reader::manifest::read_manifest_from_bytes;
use shed_core::resolver::ResolutionMethod;
use shed_core::session::DatasetSession;
use shed_core::source::DatasetSource;
use shed_core::testutil::DatasetBuilder;
use shed_core::{DelineationOptions, Engine, LevelSelection};

const REAL_GRIT_V200_URL: &str = "https://basin-delineations-public.upstream.tech/grit/2.0.0/";
const REAL_GRIT_ZURICH_OUTLET: GeoCoord = GeoCoord {
    lon: 8.5417,
    lat: 47.3769,
};

#[test]
fn finest_level_selection_returns_max_fixture_level() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();

    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");

    assert_eq!(selected.level(), Level::new(1).expect("fixture level"));
}

#[test]
fn selected_level_public_api_only_resolves_existing_finest_level() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let missing_level = Level::new(7).expect("syntactically valid missing level");
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();

    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");

    assert_ne!(selected.level(), missing_level);
}

#[test]
fn session_level_index_answers_known_and_unknown_units() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");

    assert_eq!(
        session.level_of(UnitId::new(1).expect("fixture unit id")),
        Some(Level::new(0).expect("fixture level"))
    );
    assert_eq!(
        session.level_of(UnitId::new(20).expect("fixture unit id")),
        Some(Level::new(1).expect("fixture level"))
    );
    assert_eq!(
        session.level_of(UnitId::new(999).expect("unknown unit id")),
        None
    );
    assert_eq!(
        session.levels(),
        vec![
            Level::new(0).expect("fixture level"),
            Level::new(1).expect("fixture level")
        ]
    );
    assert_eq!(
        session.max_level(),
        Some(Level::new(1).expect("fixture level"))
    );
}

#[test]
fn graph_row_group_level_statistics_agree_with_stored_session_index() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");

    let stats_max = max_level_from_row_group_statistics(&root.join("graph.parquet"))
        .expect("graph row-group level statistics should read");

    assert_eq!(stats_max, session.max_level());
}

#[test]
fn default_finest_pip_resolution_prefers_nested_l1_child_over_larger_l0_parent() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let outlet = GeoCoord::new(0.5, -0.5);

    let result = engine
        .delineate(outlet, &DelineationOptions::default())
        .expect("nested finest-level outlet should delineate");

    assert_eq!(
        result.terminal_unit_id(),
        UnitId::new(10).expect("fixture child unit id"),
        "default finest resolution should choose the L1 child, not the larger L0 parent"
    );
    assert_eq!(
        result.resolution_method(),
        &ResolutionMethod::PointInPolygon {
            candidates_considered: 1,
            tie_break: None,
        },
        "the L0 parent has larger area/upstream area but must be filtered out before tie-break"
    );
}

#[test]
#[ignore = "network-gated GRIT v2.0.0 finest-level proof; set SHED_HFX_V02_REAL_R2_DELINEATION=1"]
fn grit_v200_default_finest_resolves_zurich_to_l1_by_bounded_reads() {
    if std::env::var("SHED_HFX_V02_REAL_R2_DELINEATION").as_deref() != Ok("1") {
        println!(
            "skipping real GRIT v2.0.0 finest-level proof; set SHED_HFX_V02_REAL_R2_DELINEATION=1 to enable"
        );
        return;
    }

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should start");
    runtime.block_on(async {
        let RemoteDataset { store, root } = open_real_grit_source();
        let manifest_bytes = store
            .get(&remote_artifact_path(&root, "manifest.json"))
            .await
            .expect("real GRIT manifest should be reachable")
            .bytes()
            .await
            .expect("real GRIT manifest bytes should read");
        let parsed =
            read_manifest_from_bytes(&manifest_bytes).expect("real GRIT manifest should parse");
        assert_eq!(parsed.manifest.format_version().to_string(), "0.3.0");
        assert_eq!(parsed.manifest.unit_count().get(), 22_337_300);

        let graph_path = remote_artifact_path(&root, "graph.parquet");
        let levels = graph_levels_from_row_group_statistics(Arc::clone(&store), graph_path).await;
        assert_eq!(
            levels,
            BTreeSet::from([0, 1]),
            "GRIT v2.0.0 default finest should come from graph footer level statistics"
        );
        let finest = *levels
            .iter()
            .max()
            .expect("row-group statistics should expose at least one level");
        assert_eq!(finest, 1);

        let catchment_path = remote_artifact_path(&root, "catchments.parquet");
        let hits = bounded_containing_catchments(
            Arc::clone(&store),
            catchment_path,
            REAL_GRIT_ZURICH_OUTLET,
        )
        .await;
        assert!(
            hits.iter().any(|hit| hit.level == 0),
            "Zurich bounded read should observe the containing L0 parent candidate"
        );
        let selected = hits
            .iter()
            .filter(|hit| hit.level == finest)
            .max_by(|a, b| {
                a.up_area_km2
                    .total_cmp(&b.up_area_km2)
                    .then_with(|| a.area_km2.total_cmp(&b.area_km2))
                    .then_with(|| b.unit_id.cmp(&a.unit_id))
            })
            .expect("Zurich should be contained by a finest-level GRIT catchment");
        assert_eq!(selected.level, 1);

        println!("outlet=zurich lon={} lat={}", REAL_GRIT_ZURICH_OUTLET.lon, REAL_GRIT_ZURICH_OUTLET.lat);
        println!("levels_from=graph.parquet row-group level statistics; levels={levels:?}");
        println!(
            "bounded_pip_resolution=catchments.parquet bbox rows; selected_unit={} selected_level={} containing_candidates={}",
            selected.unit_id,
            selected.level,
            hits.len()
        );
        println!("full_dataset_session_open=false");
    });
}

struct RemoteDataset {
    store: Arc<dyn ObjectStore>,
    root: ObjectPath,
}

#[derive(Debug)]
struct CatchmentHit {
    unit_id: i64,
    level: i16,
    area_km2: f32,
    up_area_km2: f32,
}

fn open_real_grit_source() -> RemoteDataset {
    match DatasetSource::parse(REAL_GRIT_V200_URL).expect("public R2 source should parse") {
        DatasetSource::Remote { store, root, .. } => RemoteDataset { store, root },
        DatasetSource::Local(_) => panic!("real GRIT URL should parse as a remote source"),
    }
}

fn remote_artifact_path(root: &ObjectPath, artifact: &str) -> ObjectPath {
    if root.as_ref().is_empty() {
        ObjectPath::from(artifact)
    } else {
        ObjectPath::from(format!(
            "{}/{artifact}",
            root.as_ref().trim_end_matches('/')
        ))
    }
}

async fn graph_levels_from_row_group_statistics(
    store: Arc<dyn ObjectStore>,
    path: ObjectPath,
) -> BTreeSet<i16> {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT graph.parquet footer should load");
    let level_col = builder
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == "level")
        .expect("graph level column should exist");

    builder
        .metadata()
        .row_groups()
        .iter()
        .flat_map(|row_group| {
            let stats = row_group.column(level_col).statistics();
            [int16_stat_min(stats), int16_stat_max(stats)]
        })
        .flatten()
        .collect()
}

async fn bounded_containing_catchments(
    store: Arc<dyn ObjectStore>,
    path: ObjectPath,
    outlet: GeoCoord,
) -> Vec<CatchmentHit> {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT catchments.parquet footer should load");
    let parquet_schema = builder.parquet_schema();
    let query_bbox = search_bbox(outlet, 100.0);
    let bbox_indices = [
        column_index(parquet_schema, "bbox_minx"),
        column_index(parquet_schema, "bbox_miny"),
        column_index(parquet_schema, "bbox_maxx"),
        column_index(parquet_schema, "bbox_maxy"),
    ];
    let row_groups = builder
        .metadata()
        .row_groups()
        .iter()
        .enumerate()
        .filter_map(|(index, row_group)| {
            let minx = f32_stat_min(row_group.column(bbox_indices[0]).statistics())?;
            let miny = f32_stat_min(row_group.column(bbox_indices[1]).statistics())?;
            let maxx = f32_stat_max(row_group.column(bbox_indices[2]).statistics())?;
            let maxy = f32_stat_max(row_group.column(bbox_indices[3]).statistics())?;
            bbox_intersects(query_bbox, (minx, miny, maxx, maxy)).then_some(index)
        })
        .collect::<Vec<_>>();
    assert!(
        !row_groups.is_empty(),
        "Zurich query bbox should intersect at least one catchment row group"
    );

    let projection = ProjectionMask::roots(
        parquet_schema,
        [
            "id",
            "level",
            "area_km2",
            "up_area_km2",
            "geometry",
            "bbox_minx",
            "bbox_miny",
            "bbox_maxx",
            "bbox_maxy",
        ]
        .into_iter()
        .map(|name| column_index(parquet_schema, name))
        .collect::<Vec<_>>(),
    );
    let mut stream = builder
        .with_projection(projection)
        .with_row_groups(row_groups)
        .with_batch_size(2048)
        .build()
        .expect("real GRIT catchment bbox row groups should stream");

    let point = geo::Point::new(outlet.lon, outlet.lat);
    let mut hits = Vec::new();
    while let Some(reader) = stream
        .next_row_group()
        .await
        .expect("real GRIT catchment row group should read")
    {
        for batch in reader {
            let batch = batch.expect("real GRIT catchment batch should decode");
            let ids = int64_column(&batch, "id");
            let levels = int16_column(&batch, "level");
            let areas = f32_column(&batch, "area_km2");
            let upstream_areas = f32_column(&batch, "up_area_km2");
            let minx = f32_column(&batch, "bbox_minx");
            let miny = f32_column(&batch, "bbox_miny");
            let maxx = f32_column(&batch, "bbox_maxx");
            let maxy = f32_column(&batch, "bbox_maxy");
            let geometry = batch
                .column_by_name("geometry")
                .expect("catchment geometry should be projected");

            for row in 0..batch.num_rows() {
                if !bbox_intersects(
                    query_bbox,
                    (
                        minx.value(row),
                        miny.value(row),
                        maxx.value(row),
                        maxy.value(row),
                    ),
                ) {
                    continue;
                }
                let geom = decode_geometry(geometry.as_ref(), row);
                let contains = match geom {
                    Geometry::Polygon(poly) => poly.contains(&point) || poly.intersects(&point),
                    Geometry::MultiPolygon(poly) => {
                        poly.contains(&point) || poly.intersects(&point)
                    }
                    _ => false,
                };
                if contains {
                    hits.push(CatchmentHit {
                        unit_id: ids.value(row),
                        level: levels.value(row),
                        area_km2: areas.value(row),
                        up_area_km2: if upstream_areas.is_null(row) {
                            0.0
                        } else {
                            upstream_areas.value(row)
                        },
                    });
                }
            }
        }
    }

    hits
}

fn search_bbox(center: GeoCoord, radius_m: f64) -> (f32, f32, f32, f32) {
    let lat_rad = center.lat.to_radians();
    let cos_lat = lat_rad.cos().abs().max(1e-10);
    let dlat = radius_m / 110_540.0;
    let dlon = radius_m / (111_320.0 * cos_lat);
    (
        ((center.lon - dlon).max(-180.0)) as f32,
        ((center.lat - dlat).max(-90.0)) as f32,
        ((center.lon + dlon).min(180.0)) as f32,
        ((center.lat + dlat).min(90.0)) as f32,
    )
}

fn bbox_intersects(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 <= b.2 && a.2 >= b.0 && a.1 <= b.3 && a.3 >= b.1
}

fn column_index(schema: &parquet::schema::types::SchemaDescriptor, name: &str) -> usize {
    schema
        .columns()
        .iter()
        .position(|column| column.name() == name)
        .unwrap_or_else(|| panic!("missing parquet column {name}"))
}

fn int64_column<'a>(batch: &'a arrow::record_batch::RecordBatch, name: &str) -> &'a Int64Array {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Int64Array>())
        .unwrap_or_else(|| panic!("column {name} should decode as Int64"))
}

fn int16_column<'a>(batch: &'a arrow::record_batch::RecordBatch, name: &str) -> &'a Int16Array {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Int16Array>())
        .unwrap_or_else(|| panic!("column {name} should decode as Int16"))
}

fn f32_column<'a>(batch: &'a arrow::record_batch::RecordBatch, name: &str) -> &'a Float32Array {
    batch
        .column_by_name(name)
        .and_then(|column| column.as_any().downcast_ref::<Float32Array>())
        .unwrap_or_else(|| panic!("column {name} should decode as Float32"))
}

fn decode_geometry(column: &dyn Array, row: usize) -> Geometry<f64> {
    let bytes = if let Some(binary) = column.as_any().downcast_ref::<BinaryArray>() {
        assert!(!binary.is_null(row), "geometry should be non-null");
        binary.value(row).to_vec()
    } else if let Some(binary) = column.as_any().downcast_ref::<LargeBinaryArray>() {
        assert!(!binary.is_null(row), "geometry should be non-null");
        binary.value(row).to_vec()
    } else {
        panic!("geometry should decode as Binary or LargeBinary");
    };
    let wkb = WkbGeometry::new(bytes).expect("WKB bytes should be structurally valid");
    Wkb(wkb.as_bytes())
        .to_geo()
        .expect("catchment WKB should decode")
}

fn int16_stat_min(stats: Option<&Statistics>) -> Option<i16> {
    match stats? {
        Statistics::Int32(typed) => typed.min_opt().and_then(|value| i16::try_from(*value).ok()),
        _ => None,
    }
}

fn int16_stat_max(stats: Option<&Statistics>) -> Option<i16> {
    match stats? {
        Statistics::Int32(typed) => typed.max_opt().and_then(|value| i16::try_from(*value).ok()),
        _ => None,
    }
}

fn f32_stat_min(stats: Option<&Statistics>) -> Option<f32> {
    match stats? {
        Statistics::Float(typed) => typed.min_opt().copied(),
        _ => None,
    }
}

fn f32_stat_max(stats: Option<&Statistics>) -> Option<f32> {
    match stats? {
        Statistics::Float(typed) => typed.max_opt().copied(),
        _ => None,
    }
}
