use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use geo::{BoundingRect, LineString, MultiPolygon, Polygon};
use hfx_core::{AreaKm2, Level, OutletCoord, UnitId};
use rayon::ThreadPoolBuilder;
use shed_core::algo::canonical_wkb_multi_polygon;
use shed_core::algo::coord::GeoCoord;
use shed_core::session::DatasetSession;
use shed_core::testutil::DatasetBuilder;
use shed_core::{
    DelineationOptions, Engine, EngineError, LevelSelection, PreMergeDrainageUnit,
    PreMergeDrainageUnits, RefinementMode, SelectedLevel, TerminalRefinement,
};

const PARITY_FIXTURE_DIR: &str = "tests/fixtures/parity";
const V021_SYNTHETIC_REFINED_DIR: &str = "v021_synthetic_refined";

#[test]
fn staged_level_selection_parses_finest_before_resolution() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();

    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");

    assert_eq!(selected.level(), Level::new(1).expect("fixture level"));
}

#[test]
fn staged_pre_merge_units_are_pristine_terminal_first_records() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");
    let resolved = engine
        .resolve_outlet_at_level(GeoCoord::new(2.5, -0.5), selected, &Default::default())
        .expect("fixture outlet should resolve to terminal L1 unit");
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("same-level traversal should succeed");

    let pre_merge = engine
        .produce_pre_merge_units(&upstream)
        .expect("pre-merge units should materialize");

    assert_eq!(pre_merge.selected_level(), selected);
    assert_eq!(pre_merge.terminal(), resolved.resolved().unit_id);
    assert_eq!(pre_merge.units().len(), 3);
    assert_eq!(
        pre_merge.units()[0].id(),
        resolved.resolved().unit_id,
        "terminal must be first for typed inspection"
    );
    assert_eq!(
        pre_merge.terminal_unit().map(|unit| unit.id()),
        Some(pre_merge.terminal())
    );

    let terminal = pre_merge
        .terminal_unit()
        .expect("terminal record should be available");
    let bbox = terminal
        .geometry()
        .bounding_rect()
        .expect("fixture terminal geometry should have a bbox");
    assert_eq!(bbox.min().x, 2.0);
    assert_eq!(bbox.min().y, -1.0);
    assert_eq!(bbox.max().x, 3.0);
    assert_eq!(bbox.max().y, 0.0);

    for unit in pre_merge.units() {
        assert_eq!(
            unit.level(),
            selected.level(),
            "all pre-merge records must stay at SelectedLevel"
        );
        assert_eq!(unit.area().get(), 10.0);
        assert!(unit.outlet().lon().is_finite());
        assert!(unit.outlet().lat().is_finite());
        assert_eq!(unit.up_area(), None);
        assert!(
            unit.geometry().bounding_rect().is_some(),
            "every record must include decoded whole geometry"
        );
    }
}

#[test]
fn delineate_equals_explicit_staged_composition_refine_off_v021_fixture() {
    let session = DatasetSession::open_path(&parity_fixture_path(V021_SYNTHETIC_REFINED_DIR))
        .expect("v0.2.1 converted parity fixture should open");
    let engine = Engine::builder(session).build();
    let outlet = GeoCoord::new(2.5, -2.5);
    let options = DelineationOptions::default().with_refinement_mode(RefinementMode::Disabled);

    let direct = engine
        .delineate(outlet, &options)
        .expect("direct delineation should succeed");
    let staged = explicit_staged_composition(&engine, outlet, &options)
        .expect("explicit staged composition should succeed");

    assert_delineation_results_equal(&direct, &staged);
}

#[test]
fn delineate_equals_explicit_staged_composition_best_effort_no_rasters() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let outlet = GeoCoord::new(2.5, -0.5);
    let options = DelineationOptions::default();

    let direct = engine
        .delineate(outlet, &options)
        .expect("direct delineation should succeed");
    let staged = explicit_staged_composition(&engine, outlet, &options)
        .expect("explicit staged composition should succeed");

    assert_eq!(
        direct.refinement(),
        &shed_core::RefinementOutcome::NoRastersAvailable
    );
    assert_delineation_results_equal(&direct, &staged);
}

#[test]
fn staged_refine_terminal_placeholder_disabled_reuses_pre_merge_terminal() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let (resolved, pre_merge) = pre_merge_for_nested_fixture(&engine);
    let options = DelineationOptions::default().with_refinement_mode(RefinementMode::Disabled);

    let refinement = engine
        .refine_terminal_placeholder(&resolved, &pre_merge, &options)
        .expect("disabled refinement should resolve without raster work");
    let dissolved = engine
        .dissolve_watershed(&pre_merge, &refinement, &options)
        .expect("refine-off dissolve should succeed");

    assert_eq!(refinement, TerminalRefinement::Disabled);
    assert!(!dissolved.geometry().0.is_empty());
    assert!(dissolved.area_km2().as_f64() > 0.0);
}

#[test]
fn staged_refine_terminal_placeholder_best_effort_no_rasters() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let (resolved, pre_merge) = pre_merge_for_nested_fixture(&engine);
    let options = DelineationOptions::default();

    let refinement = engine
        .refine_terminal_placeholder(&resolved, &pre_merge, &options)
        .expect("best-effort with no rasters should be a typed outcome");
    let dissolved = engine
        .dissolve_watershed(&pre_merge, &refinement, &options)
        .expect("no-raster dissolve should use whole units");

    assert_eq!(refinement, TerminalRefinement::NoRastersAvailable);
    assert!(!dissolved.geometry().0.is_empty());
}

#[test]
fn staged_dissolve_is_byte_identical_across_permuted_four_thread_runs() {
    let (_dir, root) = DatasetBuilder::new(1).with_multilevel_nested().build();
    let session = DatasetSession::open_path(&root).expect("nested fixture should open");
    let engine = Engine::builder(session).build();
    let (_resolved, pre_merge) = pre_merge_for_nested_fixture(&engine);
    let options = DelineationOptions::default().with_refinement_mode(RefinementMode::Disabled);
    let refinement = TerminalRefinement::Disabled;
    let pool = ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .expect("fixed test thread pool should build");

    pool.install(|| {
        assert!(rayon::current_num_threads() > 1);

        let first = canonical_wkb_multi_polygon(
            engine
                .dissolve_watershed(&pre_merge, &refinement, &options)
                .expect("baseline dissolve should succeed")
                .geometry(),
        )
        .expect("baseline geometry should canonicalize");
        for run_index in 1..20 {
            let mut run_units = pre_merge.units().to_vec();
            if run_index % 2 == 1 {
                run_units.reverse();
            } else {
                let shift = run_index % run_units.len();
                run_units.rotate_left(shift);
            }
            let run_pre_merge = PreMergeDrainageUnits::new_for_test(
                pre_merge.terminal(),
                pre_merge.selected_level(),
                run_units,
            );
            let current = canonical_wkb_multi_polygon(
                engine
                    .dissolve_watershed(&run_pre_merge, &refinement, &options)
                    .expect("permuted dissolve should succeed")
                    .geometry(),
            )
            .expect("permuted geometry should canonicalize");
            assert_eq!(
                first, current,
                "canonical WKB changed on staged dissolve run {run_index}"
            );
        }
    });
}

#[test]
fn staged_dissolve_replaces_terminal_with_applied_refinement() {
    let (_dir, root) = DatasetBuilder::new(1).build();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let options = DelineationOptions::default();
    let units = manual_pre_merge_units(
        MultiPolygon::new(vec![rect(10.0, 10.0, 11.0, 11.0)]),
        MultiPolygon::new(vec![rect(0.0, 0.0, 1.0, 1.0)]),
    );
    let refinement = TerminalRefinement::Applied {
        refined_outlet: GeoCoord::new(1.5, 0.5),
        geometry: MultiPolygon::new(vec![rect(1.0, 0.0, 2.0, 1.0)]),
    };

    let dissolved = engine
        .dissolve_watershed(&units, &refinement, &options)
        .expect("applied refinement dissolve should succeed");
    let bbox = dissolved
        .geometry()
        .bounding_rect()
        .expect("dissolved geometry should have bounds");

    assert_close(bbox.min().x, 0.0);
    assert_close(bbox.min().y, 0.0);
    assert_close(bbox.max().x, 2.0);
    assert_close(bbox.max().y, 1.0);
}

#[test]
fn staged_dissolve_rejects_empty_refined_terminal_override() {
    let (_dir, root) = DatasetBuilder::new(1).build();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let options = DelineationOptions::default();
    let units = manual_pre_merge_units(
        MultiPolygon::new(vec![rect(1.0, 0.0, 2.0, 1.0)]),
        MultiPolygon::new(vec![rect(0.0, 0.0, 1.0, 1.0)]),
    );
    let refinement = TerminalRefinement::Applied {
        refined_outlet: GeoCoord::new(1.5, 0.5),
        geometry: MultiPolygon::new(vec![]),
    };

    let err = engine
        .dissolve_watershed(&units, &refinement, &options)
        .expect_err("empty refined terminal override should fail");

    assert!(
        matches!(err, EngineError::Assembly { message, .. } if message.contains("refined terminal geometry"))
    );
}

#[test]
fn staged_dissolve_bypasses_bad_whole_terminal_when_refined_override_exists() {
    let (_dir, root) = DatasetBuilder::new(1).build();
    let session = DatasetSession::open_path(&root).expect("fixture should open");
    let engine = Engine::builder(session).build();
    let options = DelineationOptions::default();
    let units = manual_pre_merge_units(
        MultiPolygon::new(vec![]),
        MultiPolygon::new(vec![rect(0.0, 0.0, 1.0, 1.0)]),
    );
    let refinement = TerminalRefinement::Applied {
        refined_outlet: GeoCoord::new(1.5, 0.5),
        geometry: MultiPolygon::new(vec![rect(1.0, 0.0, 2.0, 1.0)]),
    };

    let dissolved = engine
        .dissolve_watershed(&units, &refinement, &options)
        .expect("applied override should bypass the unusable whole terminal geometry");

    assert!(!dissolved.geometry().0.is_empty());
}

fn pre_merge_for_nested_fixture(
    engine: &Engine,
) -> (shed_core::LevelResolvedOutlet, PreMergeDrainageUnits) {
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");
    let resolved = engine
        .resolve_outlet_at_level(GeoCoord::new(2.5, -0.5), selected, &Default::default())
        .expect("fixture outlet should resolve to terminal L1 unit");
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("same-level traversal should succeed");
    let pre_merge = engine
        .produce_pre_merge_units(&upstream)
        .expect("pre-merge units should materialize");

    (resolved, pre_merge)
}

fn explicit_staged_composition(
    engine: &Engine,
    outlet: GeoCoord,
    options: &DelineationOptions,
) -> Result<shed_core::DelineationResult, EngineError> {
    let selected_level = engine.select_level(LevelSelection::Finest)?;
    let resolved =
        engine.resolve_outlet_at_level(outlet, selected_level, options.resolver_config())?;
    let upstream = engine.traverse_upstream_at_level(&resolved)?;
    let pre_merge = engine.produce_pre_merge_units(&upstream)?;
    let refinement = engine.refine_terminal_placeholder(&resolved, &pre_merge, options)?;
    let dissolved = engine.dissolve_watershed(&pre_merge, &refinement, options)?;

    Ok(engine.compose_result(resolved, upstream, refinement, dissolved))
}

fn assert_delineation_results_equal(
    direct: &shed_core::DelineationResult,
    staged: &shed_core::DelineationResult,
) {
    assert_eq!(direct.terminal_unit_id(), staged.terminal_unit_id());
    assert_eq!(direct.input_outlet(), staged.input_outlet());
    assert_eq!(direct.resolved_outlet(), staged.resolved_outlet());
    assert_eq!(direct.resolution_method(), staged.resolution_method());
    assert_eq!(
        direct
            .upstream_unit_ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        staged
            .upstream_unit_ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(direct.refinement(), staged.refinement());
    assert_eq!(
        canonical_wkb_multi_polygon(direct.geometry())
            .expect("direct geometry should canonicalize"),
        canonical_wkb_multi_polygon(staged.geometry())
            .expect("staged geometry should canonicalize")
    );
    assert_close(direct.area_km2().as_f64(), staged.area_km2().as_f64());
}

fn parity_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(PARITY_FIXTURE_DIR)
        .join(name)
}

fn manual_pre_merge_units(
    terminal_geometry: MultiPolygon<f64>,
    upstream_geometry: MultiPolygon<f64>,
) -> PreMergeDrainageUnits {
    let level = Level::new(0).expect("test level");
    let terminal = UnitId::new(2).expect("terminal id");
    let upstream = UnitId::new(1).expect("upstream id");
    PreMergeDrainageUnits::new_for_test(
        terminal,
        SelectedLevel::from_proven_level_for_test(level),
        vec![
            pre_merge_unit(terminal, level, terminal_geometry),
            pre_merge_unit(upstream, level, upstream_geometry),
        ],
    )
}

fn pre_merge_unit(id: UnitId, level: Level, geometry: MultiPolygon<f64>) -> PreMergeDrainageUnit {
    PreMergeDrainageUnit::new_for_test(
        id,
        level,
        AreaKm2::new(1.0).expect("test area"),
        None,
        OutletCoord::new(0.0, 0.0).expect("test outlet"),
        geometry,
    )
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon<f64> {
    Polygon::new(
        LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]),
        vec![],
    )
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {actual} to be within tolerance of {expected}"
    );
}
