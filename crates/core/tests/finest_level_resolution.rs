use hfx_core::{Level, UnitId};
use shed_core::algo::coord::GeoCoord;
use shed_core::reader::graph::max_level_from_row_group_statistics;
use shed_core::resolver::ResolutionMethod;
use shed_core::session::DatasetSession;
use shed_core::testutil::DatasetBuilder;
use shed_core::{DelineationOptions, Engine, LevelSelection};

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
