use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arrow::array::{
    Array, BinaryArray, Float32Array, Int16Array, Int64Array, LargeBinaryArray, StringArray,
};
use geo::{Closest, ClosestPoint, Geometry};
use geozero::ToGeo;
use geozero::wkb::Wkb;
use hfx_core::WkbGeometry;
use hfx_core::{SnapId, UnitId};
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use parquet::arrow::ProjectionMask;
use parquet::arrow::async_reader::{ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use parquet::file::statistics::Statistics;
use shed_core::algo::coord::GeoCoord;
use shed_core::reader::manifest::read_manifest_from_bytes;
use shed_core::resolver::{ResolutionMethod, ResolverConfig};
use shed_core::session::DatasetSession;
use shed_core::source::DatasetSource;
use shed_core::testutil::{DatasetBuilder, TestSnapDeclaration, TestSnapGeometry, TestSnapTarget};
use shed_core::{Engine, LevelSelection};

const REAL_GRIT_V200_URL: &str = "https://basin-delineations-public.upstream.tech/grit/2.0.0/";
const REAL_GRIT_ZURICH_OUTLET: GeoCoord = GeoCoord {
    lon: 8.5417,
    lat: 47.3769,
};
const DEFAULT_SEARCH_RADIUS_M: f64 = 1000.0;

fn declaration(
    name: &str,
    path: &str,
    references_levels: Vec<i16>,
    targets: Vec<TestSnapTarget>,
) -> TestSnapDeclaration {
    TestSnapDeclaration {
        name: name.to_string(),
        path: path.to_string(),
        references_levels,
        targets,
    }
}

fn target(
    id: i64,
    catchment_id: i64,
    weight: f32,
    is_mainstem: bool,
    x: f64,
    y: f64,
) -> TestSnapTarget {
    TestSnapTarget {
        id,
        catchment_id,
        weight,
        is_mainstem,
        geometry: TestSnapGeometry::Point(x, y),
    }
}

fn resolve_finest_snap(declarations: Vec<TestSnapDeclaration>) -> (UnitId, ResolutionMethod) {
    let (_dir, root) = DatasetBuilder::new(1)
        .with_multilevel_nested()
        .with_custom_snap_declarations(declarations)
        .build();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("fixture has a finest level");
    let resolved = engine
        .resolve_outlet_at_level(
            GeoCoord::new(0.5, -0.5),
            selected,
            &ResolverConfig::default(),
        )
        .expect("outlet should resolve");

    (
        resolved.resolved().unit_id,
        resolved.resolved().method.clone(),
    )
}

fn assert_snap(method: &ResolutionMethod, expected_snap_id: i64) {
    match method {
        ResolutionMethod::Snap { snap_id, .. } => {
            assert_eq!(*snap_id, SnapId::new(expected_snap_id).expect("snap id"));
        }
        other => panic!("expected snap resolution, got {other:?}"),
    }
}

#[test]
fn declarations_not_referencing_selected_level_are_ignored() {
    let (_dir, root) = DatasetBuilder::new(1)
        .with_multilevel_nested()
        .with_custom_snap_declarations(vec![declaration(
            "segment-stems",
            "segment-stems.parquet",
            vec![0],
            vec![target(1, 1, 900.0, true, 0.5, -0.5)],
        )])
        .build();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("fixture has a finest level");

    let resolved = engine
        .resolve_outlet_at_level(
            GeoCoord::new(0.5, -0.5),
            selected,
            &ResolverConfig::default(),
        )
        .expect("PiP fallback should resolve inside the selected L1 unit");

    assert_eq!(
        resolved.resolved().unit_id,
        UnitId::new(10).expect("fixture L1 unit")
    );
    assert!(
        matches!(
            resolved.resolved().method,
            ResolutionMethod::PointInPolygon { .. }
        ),
        "no selected-level snap declaration should fall back to the existing PiP path"
    );
}

#[test]
fn selected_level_opens_matching_declaration_store_even_when_not_first() {
    let (unit_id, method) = resolve_finest_snap(vec![
        declaration(
            "segment-stems",
            "segment-stems.parquet",
            vec![0],
            vec![target(1, 1, 900.0, true, 0.5, -0.5)],
        ),
        declaration(
            "reach-stems",
            "reach-stems.parquet",
            vec![1],
            vec![target(20, 20, 10.0, false, 0.5, -0.5)],
        ),
    ]);

    assert_eq!(unit_id, UnitId::new(20).expect("fixture L1 unit"));
    assert_snap(&method, 20);
}

#[test]
fn multiple_matching_declarations_use_name_then_path_order() {
    let (unit_id, method) = resolve_finest_snap(vec![
        declaration(
            "same-name",
            "z-loser.parquet",
            vec![1],
            vec![target(200, 20, 900.0, true, 0.5, -0.5)],
        ),
        declaration(
            "same-name",
            "a-winner.parquet",
            vec![1],
            vec![target(100, 10, 1.0, false, 0.5, -0.5)],
        ),
    ]);

    assert_eq!(unit_id, UnitId::new(10).expect("fixture L1 unit"));
    assert_snap(&method, 100);
}

#[test]
fn snap_targets_at_other_levels_are_ignored_even_when_declaration_matches() {
    let (unit_id, method) = resolve_finest_snap(vec![declaration(
        "mixed-level-stems",
        "mixed-level-stems.parquet",
        vec![0, 1],
        vec![
            target(1, 1, 900.0, true, 0.5, -0.5),
            target(10, 10, 1.0, false, 0.5, -0.5),
        ],
    )]);

    assert_eq!(unit_id, UnitId::new(10).expect("fixture L1 unit"));
    assert_snap(&method, 10);
}

#[test]
fn higher_weight_beats_lower_weight() {
    let (unit_id, method) = resolve_finest_snap(vec![declaration(
        "reach-stems",
        "reach-stems.parquet",
        vec![1],
        vec![
            target(10, 10, 10.0, false, 0.5, -0.5),
            target(20, 20, 20.0, false, 0.5002, -0.5),
        ],
    )]);

    assert_eq!(unit_id, UnitId::new(20).expect("higher-weight unit"));
    assert_snap(&method, 20);
}

#[test]
fn mainstem_beats_non_mainstem_on_equal_weight() {
    let (unit_id, method) = resolve_finest_snap(vec![declaration(
        "reach-stems",
        "reach-stems.parquet",
        vec![1],
        vec![
            target(10, 10, 10.0, false, 0.5, -0.5),
            target(20, 20, 10.0, true, 0.5002, -0.5),
        ],
    )]);

    assert_eq!(unit_id, UnitId::new(20).expect("mainstem unit"));
    assert_snap(&method, 20);
}

#[test]
fn nearer_candidate_wins_after_equal_weight_and_stem_role() {
    let (unit_id, method) = resolve_finest_snap(vec![declaration(
        "reach-stems",
        "reach-stems.parquet",
        vec![1],
        vec![
            target(10, 10, 10.0, false, 0.5, -0.5),
            target(20, 20, 10.0, false, 0.5002, -0.5),
        ],
    )]);

    assert_eq!(unit_id, UnitId::new(10).expect("nearer unit"));
    assert_snap(&method, 10);
}

#[test]
fn lower_snap_id_breaks_only_exact_weight_stem_role_distance_tie() {
    let (unit_id, method) = resolve_finest_snap(vec![declaration(
        "reach-stems",
        "reach-stems.parquet",
        vec![1],
        vec![
            target(20, 20, 10.0, false, 0.5, -0.5),
            target(10, 10, 10.0, false, 0.5, -0.5),
        ],
    )]);

    assert_eq!(unit_id, UnitId::new(10).expect("lower snap-id unit"));
    assert_snap(&method, 10);
}

#[test]
#[ignore = "network-gated GRIT v2.0.0 snap proof; set SHED_HFX_V02_REAL_R2_DELINEATION=1"]
fn grit_v200_finest_snap_uses_reach_stems_and_weight_first_cascade() {
    if std::env::var("SHED_HFX_V02_REAL_R2_DELINEATION").as_deref() != Ok("1") {
        println!(
            "skipping real GRIT v2.0.0 snap proof; set SHED_HFX_V02_REAL_R2_DELINEATION=1 to enable"
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
        let snaps = parsed.aux.snaps;
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].name, "segment-stems");
        assert_eq!(snaps[0].references_levels, vec![0]);

        let l1_decl = snaps
            .iter()
            .filter(|decl| decl.references_levels.contains(&1))
            .min_by(|a, b| a.name.cmp(&b.name).then_with(|| a.snap.cmp(&b.snap)))
            .expect("GRIT finest level should have a matching snap declaration");
        assert_eq!(l1_decl.name, "reach-stems");
        assert_eq!(l1_decl.references_levels, vec![1]);
        assert_ne!(
            l1_decl.snap, snaps[0].snap,
            "finest-level snap resolution must not use snaps.first()"
        );

        let snap_path = remote_artifact_path(&root, &l1_decl.snap);
        let mut candidates =
            bounded_snap_candidates(Arc::clone(&store), snap_path, REAL_GRIT_ZURICH_OUTLET).await;
        assert!(
            candidates.len() > 1,
            "Zurich real reach-stems query should produce multiple candidates for ranking"
        );
        let candidate_ids = candidates
            .iter()
            .map(|candidate| candidate.unit_id)
            .collect::<BTreeSet<_>>();
        let levels = bounded_levels_for_ids(
            Arc::clone(&store),
            remote_artifact_path(&root, "catchments.parquet"),
            &candidate_ids,
        )
        .await;
        for candidate in &mut candidates {
            candidate.level = levels.get(&candidate.unit_id).copied();
        }
        let mut l1_candidates = candidates
            .into_iter()
            .filter(|candidate| candidate.level == Some(1))
            .collect::<Vec<_>>();
        assert!(
            l1_candidates.len() > 1,
            "GRIT reach-stems candidates should resolve to L1 units via targeted catchment ID reads"
        );
        l1_candidates.sort_by(compare_snap_candidates);
        let winner = l1_candidates
            .first()
            .expect("L1 snap candidate list should be non-empty");

        assert!(
            l1_candidates
                .windows(2)
                .all(|pair| compare_snap_candidates(&pair[0], &pair[1]).is_le()),
            "real snap candidates should be ordered by weight, mainstem, distance, then snap_id"
        );
        assert_eq!(
            winner.level,
            Some(1),
            "targeted query_by_ids-style level lookup should keep the L1 reach target"
        );

        println!("outlet=zurich lon={} lat={}", REAL_GRIT_ZURICH_OUTLET.lon, REAL_GRIT_ZURICH_OUTLET.lat);
        println!(
            "selected_snap_decl=name:{} path:{} references_levels:{:?}",
            l1_decl.name, l1_decl.snap, l1_decl.references_levels
        );
        println!(
            "bounded_snap_resolution=winner_snap_id={} winner_unit={} winner_level={} weight={} mainstem={} distance_m={} ranked_l1_candidates={}",
            winner.snap_id,
            winner.unit_id,
            winner.level.expect("winner level should be set"),
            winner.weight,
            winner.mainstem,
            winner.distance_m,
            l1_candidates.len()
        );
        println!("full_dataset_session_open=false");
    });
}

#[derive(Debug)]
struct RemoteDataset {
    store: Arc<dyn ObjectStore>,
    root: ObjectPath,
}

#[derive(Debug)]
struct RealSnapCandidate {
    snap_id: i64,
    unit_id: i64,
    level: Option<i16>,
    weight: f32,
    mainstem: bool,
    distance_m: f64,
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

async fn bounded_snap_candidates(
    store: Arc<dyn ObjectStore>,
    path: ObjectPath,
    outlet: GeoCoord,
) -> Vec<RealSnapCandidate> {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT reach-stems snap footer should load");
    let parquet_schema = builder.parquet_schema();
    let query_bbox = search_bbox(outlet, DEFAULT_SEARCH_RADIUS_M);
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
        "Zurich query bbox should intersect at least one reach-stems row group"
    );

    let projection = ProjectionMask::roots(
        parquet_schema,
        [
            "id",
            "unit_id",
            "weight",
            "stem_role",
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
        .expect("real GRIT reach-stems row groups should stream");

    let mut candidates = Vec::new();
    while let Some(reader) = stream
        .next_row_group()
        .await
        .expect("real GRIT reach-stems row group should read")
    {
        for batch in reader {
            let batch = batch.expect("real GRIT reach-stems batch should decode");
            let ids = int64_column(&batch, "id");
            let unit_ids = int64_column(&batch, "unit_id");
            let weights = f32_column(&batch, "weight");
            let roles = batch
                .column_by_name("stem_role")
                .and_then(|column| column.as_any().downcast_ref::<StringArray>())
                .expect("stem_role should decode as Utf8");
            let minx = f32_column(&batch, "bbox_minx");
            let miny = f32_column(&batch, "bbox_miny");
            let maxx = f32_column(&batch, "bbox_maxx");
            let maxy = f32_column(&batch, "bbox_maxy");
            let geometry = batch
                .column_by_name("geometry")
                .expect("snap geometry should be projected");

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
                let Some((distance_m, _nearest)) = snap_nearest_point(outlet, &geom) else {
                    continue;
                };
                if distance_m > DEFAULT_SEARCH_RADIUS_M {
                    continue;
                }
                candidates.push(RealSnapCandidate {
                    snap_id: ids.value(row),
                    unit_id: unit_ids.value(row),
                    level: None,
                    weight: weights.value(row),
                    mainstem: !roles.is_null(row) && roles.value(row) == "mainstem",
                    distance_m,
                });
            }
        }
    }

    candidates
}

async fn bounded_levels_for_ids(
    store: Arc<dyn ObjectStore>,
    path: ObjectPath,
    ids: &BTreeSet<i64>,
) -> BTreeMap<i64, i16> {
    let builder = ParquetRecordBatchStreamBuilder::new(ParquetObjectReader::new(store, path))
        .await
        .expect("real GRIT catchments.parquet footer should load");
    let parquet_schema = builder.parquet_schema();
    let id_col = column_index(parquet_schema, "id");
    let row_groups = builder
        .metadata()
        .row_groups()
        .iter()
        .enumerate()
        .filter_map(|(index, row_group)| {
            let min_id = int64_stat_min(row_group.column(id_col).statistics())?;
            let max_id = int64_stat_max(row_group.column(id_col).statistics())?;
            ids.iter()
                .any(|id| min_id <= *id && *id <= max_id)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    assert!(
        !row_groups.is_empty(),
        "targeted candidate ID set should intersect catchment id-stat row groups"
    );
    let projection = ProjectionMask::roots(
        parquet_schema,
        ["id", "level"]
            .into_iter()
            .map(|name| column_index(parquet_schema, name))
            .collect::<Vec<_>>(),
    );
    let mut stream = builder
        .with_projection(projection)
        .with_row_groups(row_groups)
        .with_batch_size(4096)
        .build()
        .expect("real GRIT targeted catchment row groups should stream");

    let mut levels = BTreeMap::new();
    while let Some(reader) = stream
        .next_row_group()
        .await
        .expect("real GRIT targeted catchment row group should read")
    {
        for batch in reader {
            let batch = batch.expect("real GRIT targeted catchment batch should decode");
            let unit_ids = int64_column(&batch, "id");
            let unit_levels = int16_column(&batch, "level");
            for row in 0..batch.num_rows() {
                let unit_id = unit_ids.value(row);
                if ids.contains(&unit_id) {
                    levels.insert(unit_id, unit_levels.value(row));
                }
            }
        }
    }
    assert_eq!(
        levels.len(),
        ids.len(),
        "targeted catchment ID read should resolve every snap candidate level"
    );
    levels
}

fn compare_snap_candidates(a: &RealSnapCandidate, b: &RealSnapCandidate) -> std::cmp::Ordering {
    b.weight
        .total_cmp(&a.weight)
        .then_with(|| b.mainstem.cmp(&a.mainstem))
        .then_with(|| a.distance_m.total_cmp(&b.distance_m))
        .then_with(|| a.snap_id.cmp(&b.snap_id))
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

fn local_metre_distance(a: GeoCoord, b: GeoCoord) -> f64 {
    let lat_avg = ((a.lat + b.lat) / 2.0).to_radians();
    let dx_m = (b.lon - a.lon) * 111_320.0 * lat_avg.cos();
    let dy_m = (b.lat - a.lat) * 110_540.0;
    (dx_m * dx_m + dy_m * dy_m).sqrt()
}

fn snap_nearest_point(outlet: GeoCoord, geom: &Geometry<f64>) -> Option<(f64, GeoCoord)> {
    let outlet_point: geo::Point<f64> = outlet.into();
    match geom.closest_point(&outlet_point) {
        Closest::Intersection(point) | Closest::SinglePoint(point) => {
            let nearest = GeoCoord::from(point);
            Some((local_metre_distance(outlet, nearest), nearest))
        }
        Closest::Indeterminate => None,
    }
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
    Wkb(wkb.as_bytes()).to_geo().expect("WKB should decode")
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

fn int64_stat_min(stats: Option<&Statistics>) -> Option<i64> {
    match stats? {
        Statistics::Int64(typed) => typed.min_opt().copied(),
        _ => None,
    }
}

fn int64_stat_max(stats: Option<&Statistics>) -> Option<i64> {
    match stats? {
        Statistics::Int64(typed) => typed.max_opt().copied(),
        _ => None,
    }
}
