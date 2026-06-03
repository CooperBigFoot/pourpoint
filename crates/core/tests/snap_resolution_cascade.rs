use hfx_core::{SnapId, UnitId};
use shed_core::algo::coord::GeoCoord;
use shed_core::resolver::{ResolutionMethod, ResolverConfig};
use shed_core::session::DatasetSession;
use shed_core::testutil::{DatasetBuilder, TestSnapDeclaration, TestSnapGeometry, TestSnapTarget};
use shed_core::{Engine, LevelSelection};

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
