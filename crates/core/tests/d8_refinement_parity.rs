use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use geo::BoundingRect;
use serde::Deserialize;
use shed_core::algo::SnapThreshold;
use shed_core::algo::canonical_wkb_multi_polygon;
use shed_core::algo::coord::GeoCoord;
use shed_core::session::DatasetSession;
use shed_core::test_raster_source::LocalTiffRasterSource;
use shed_core::{
    AppliedRefinementReason, DelineationOptions, Engine, LevelSelection, PreMergeDrainageUnit,
    PreMergeDrainageUnits, RefinementOutcome, RefinementProvenance, RefinementStrategyName,
    ResolverConfig, SearchRadiusMetres, SessionError, TerminalRefinement,
};

const PARITY_FIXTURE_DIR: &str = "tests/fixtures/parity";
const V021_SYNTHETIC_REFINED_DIR: &str = "v021_synthetic_refined";
const M1_SYNTHETIC_REFINED_GOLDEN: &str =
    "goldens/v01_synthetic_refined/oracle_b_synthetic_refined.json";
const REAL_MERIT_V020_URL: &str = "https://basin-delineations-public.upstream.tech/merit/0.2.0/";
const REAL_D8_ENV: &str = "SHED_HFX_V02_REAL_D8_REFINEMENT";
const REAL_MERIT_SEARCH_RADIUS_M: f64 = 5_000.0;
const EXPECTED_REAL_MERIT_D8_DECLS: usize = 60;
const EXPECTED_REAL_MERIT_SNAP_DECLS: usize = 1;
const EXTENT_HEADER_RANGE_BYTES: u64 = 256 * 1024;

#[test]
fn v021_synthetic_d8_refinement_matches_m1_b_golden() {
    let golden = read_golden(M1_SYNTHETIC_REFINED_GOLDEN);
    let session = DatasetSession::open_path(&parity_fixture_path(V021_SYNTHETIC_REFINED_DIR))
        .expect("v0.2.1 converted parity fixture should open");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource)
        .build();
    let outlet = GeoCoord::new(golden.input_outlet.lon, golden.input_outlet.lat);
    let options = b_oracle_options();

    let result = engine
        .delineate(outlet, &options)
        .expect("D8-refined delineation should succeed");
    let canonical = canonical_wkb_multi_polygon(result.geometry())
        .expect("D8-refined geometry should canonicalize");

    assert_eq!(canonical, decode_hex(&golden.canonical_wkb_hex));
    assert_area_within_golden_policy(result.area_km2().as_f64(), golden.area_km2, &golden);
    assert_eq!(result.terminal_unit_id().get(), golden.terminal_id);
    assert_eq!(
        result
            .upstream_unit_ids()
            .iter()
            .map(|id| id.get())
            .collect::<Vec<_>>(),
        golden.upstream_ids
    );
    assert_eq!(
        result.refinement(),
        &RefinementOutcome::Applied {
            refined_outlet: GeoCoord::new(golden.refined_outlet.lon, golden.refined_outlet.lat),
            provenance: RefinementProvenance::Applied {
                strategy: RefinementStrategyName::BuiltInD8,
                why: AppliedRefinementReason::D8AuxMatchedTerminalBbox {
                    declaration_index: 0,
                },
            },
        }
    );
}

#[test]
fn applied_d8_carve_replaces_whole_terminal_in_final_dissolve() {
    let golden = read_golden(M1_SYNTHETIC_REFINED_GOLDEN);
    let session = DatasetSession::open_path(&parity_fixture_path(V021_SYNTHETIC_REFINED_DIR))
        .expect("v0.2.1 converted parity fixture should open");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource)
        .build();
    let outlet = GeoCoord::new(golden.input_outlet.lon, golden.input_outlet.lat);
    let options = b_oracle_options();

    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("finest level should resolve");
    let resolved = engine
        .resolve_outlet_at_level(outlet, selected, options.resolver_config())
        .expect("fixture outlet should resolve");
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("same-level traversal should succeed");
    let pre_merge = engine
        .produce_pre_merge_units(&upstream)
        .expect("pre-merge records should materialize");
    let whole_terminal = pre_merge
        .terminal_unit()
        .expect("terminal record should exist")
        .geometry();

    let refinement = engine
        .refine_terminal_placeholder(&resolved, &pre_merge, &options)
        .expect("D8 refinement should apply");
    let TerminalRefinement::Applied { geometry, .. } = &refinement else {
        panic!("expected applied D8 refinement, got {refinement:?}");
    };
    let dissolved = engine
        .dissolve_watershed(&pre_merge, &refinement, &options)
        .expect("applied D8 dissolve should succeed");
    let whole_terminal_dissolved = engine
        .dissolve_watershed(&pre_merge, &TerminalRefinement::Disabled, &options)
        .expect("whole-terminal dissolve should succeed");
    let replacement_pre_merge =
        pre_merge_with_terminal_geometry(&pre_merge, geometry.polygon().clone());
    let replacement_dissolved = engine
        .dissolve_watershed(
            &replacement_pre_merge,
            &TerminalRefinement::Disabled,
            &options,
        )
        .expect("carved-terminal replacement dissolve should succeed");

    let final_canonical = canonical_wkb_multi_polygon(dissolved.geometry())
        .expect("final geometry should canonicalize");
    let replacement_canonical = canonical_wkb_multi_polygon(replacement_dissolved.geometry())
        .expect("replacement geometry should canonicalize");
    let whole_terminal_dissolved_canonical =
        canonical_wkb_multi_polygon(whole_terminal_dissolved.geometry())
            .expect("whole-terminal dissolved geometry should canonicalize");
    let whole_terminal_canonical = canonical_wkb_multi_polygon(whole_terminal)
        .expect("whole terminal geometry should canonicalize");

    // R3: pre-merge unit records stay pristine. Their area/geometry may
    // intentionally disagree with final refined output; final geometry is
    // assembled only after excluding the whole terminal and inserting the carve.
    assert_ne!(final_canonical, whole_terminal_canonical);
    assert_ne!(final_canonical, whole_terminal_dissolved_canonical);
    assert_eq!(final_canonical, replacement_canonical);
    assert_eq!(final_canonical, decode_hex(&golden.canonical_wkb_hex));
}

#[test]
#[ignore = "network-gated MERIT v0.2.0 D8 refinement readiness proof; set SHED_HFX_V02_REAL_D8_REFINEMENT=1"]
fn merit_v020_d8_refinement_selects_manifest_first_overlapping_pfaf() {
    if std::env::var(REAL_D8_ENV).as_deref() != Ok("1") {
        println!(
            "skipping real MERIT v0.2.0 D8 refinement readiness proof; set {REAL_D8_ENV}=1 to enable"
        );
        return;
    }

    // Real MERIT-Hydro D8 rasters are per-Pfaf-02 basin windows. Irregular
    // basins have overlapping rectangular extents, so a terminal near a basin
    // boundary is fully covered by more than one declaration. hfx.aux.d8_raster.v1
    // requires overlapping entries to be windows of a single coherent D8 fabric
    // (identical values in the overlap), so selection collapses to the
    // manifest-first covering declaration and the carve proceeds rather than
    // surfacing AmbiguousD8Coverage.
    let _bench_net = ScopedEnvVar::set("PYSHED_BENCH_NET", "1");

    let probe_session =
        DatasetSession::open(REAL_MERIT_V020_URL).expect("real MERIT v0.2.0 should open");
    assert_real_merit_manifest(&probe_session);
    let options = real_merit_options();
    let terminal_bbox = {
        let probe_engine = Engine::builder(probe_session).build();
        let selected = probe_engine
            .select_level(LevelSelection::Finest)
            .expect("real MERIT finest level should resolve");
        let resolved = probe_engine
            .resolve_outlet_at_level(
                real_merit_rhine_basel_outlet(),
                selected,
                options.resolver_config(),
            )
            .expect("rhine_basel outlet should resolve in MERIT v0.2.0");
        let upstream = probe_engine
            .traverse_upstream_at_level(&resolved)
            .expect("rhine_basel upstream traversal should succeed");
        let pre_merge = probe_engine
            .produce_pre_merge_units(&upstream)
            .expect("rhine_basel pre-merge units should materialize");
        pre_merge
            .terminal_unit()
            .expect("rhine_basel terminal unit should exist")
            .geometry()
            .bounding_rect()
            .expect("rhine_basel terminal geometry should have a bbox")
    };

    let session =
        DatasetSession::open(REAL_MERIT_V020_URL).expect("real MERIT v0.2.0 should reopen");
    assert_real_merit_manifest(&session);
    assert!(
        session.http_stats().is_some(),
        "PYSHED_BENCH_NET should expose remote request counters"
    );

    let selected_index = match session.select_d8_raster_for_bbox(terminal_bbox) {
        Ok(handle) => handle.declaration_index(),
        Err(SessionError::TerminalSpansD8Tiles {
            declaration_indices,
            ..
        }) => panic!(
            "ESCALATE: rhine_basel spans MERIT v0.2.0 D8 declarations {declaration_indices:?}; mosaicking is not implemented"
        ),
        Err(err) => {
            panic!("real MERIT rhine_basel D8 selection should pick a covering tile: {err}")
        }
    };
    assert!(
        selected_index < EXPECTED_REAL_MERIT_D8_DECLS,
        "selected declaration index should be within the declared D8 set"
    );

    let after_selection = session
        .http_stats()
        .expect("request counters should remain available after selection");
    assert_no_root_raster_reads(&after_selection);
    assert_only_extent_headers_for_all_d8(&after_selection);

    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource)
        .build();
    let result = engine
        .delineate(real_merit_rhine_basel_outlet(), &options)
        .expect("real MERIT rhine_basel should carve after manifest-first D8 selection");
    let RefinementOutcome::Applied { provenance, .. } = result.refinement() else {
        panic!(
            "expected applied D8 refinement, got {:?}",
            result.refinement()
        );
    };
    assert!(matches!(
        provenance,
        RefinementProvenance::Applied {
            strategy: RefinementStrategyName::BuiltInD8,
            ..
        }
    ));
    assert!(
        !result.geometry().0.is_empty(),
        "carved watershed geometry should be non-empty"
    );

    println!(
        "real_merit_d8_boundary declaration_count={} selected_declaration_index={} bounded_d8_header_bytes={} refinement=Applied",
        EXPECTED_REAL_MERIT_D8_DECLS,
        selected_index,
        d8_bytes_in(&after_selection)
    );
}

fn parity_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(PARITY_FIXTURE_DIR)
        .join(name)
}

fn read_golden(name: &str) -> GoldenRecord {
    let path = parity_fixture_path(name);
    serde_json::from_str(&fs::read_to_string(path).expect("golden should be readable"))
        .expect("golden should match test schema")
}

fn b_oracle_options() -> DelineationOptions {
    DelineationOptions::default().with_snap_threshold(SnapThreshold::new(500))
}

fn real_merit_options() -> DelineationOptions {
    DelineationOptions::default().with_resolver_config(
        ResolverConfig::new().with_search_radius(
            SearchRadiusMetres::new(REAL_MERIT_SEARCH_RADIUS_M)
                .expect("real MERIT search radius should be valid"),
        ),
    )
}

fn real_merit_rhine_basel_outlet() -> GeoCoord {
    GeoCoord::new(7.5890, 47.5596)
}

fn pre_merge_with_terminal_geometry(
    pre_merge: &PreMergeDrainageUnits,
    terminal_geometry: geo::MultiPolygon<f64>,
) -> PreMergeDrainageUnits {
    let units = pre_merge
        .units()
        .iter()
        .map(|unit| {
            let geometry = if unit.id() == pre_merge.terminal() {
                terminal_geometry.clone()
            } else {
                unit.geometry().clone()
            };
            PreMergeDrainageUnit::new_for_test(
                unit.id(),
                unit.level(),
                unit.area(),
                unit.up_area(),
                unit.outlet(),
                geometry,
            )
        })
        .collect();
    PreMergeDrainageUnits::new_for_test(pre_merge.terminal(), pre_merge.selected_level(), units)
}

fn assert_area_within_golden_policy(actual: f64, expected: f64, golden: &GoldenRecord) {
    let tolerance = golden
        .comparison_policy
        .area_km2_abs_epsilon
        .max(expected.abs() * golden.comparison_policy.area_km2_rel_epsilon);
    assert!(
        (actual - expected).abs() <= tolerance,
        "area {actual} differs from golden {expected} beyond tolerance {tolerance}"
    );
}

fn decode_hex(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| (hex_digit(pair[0]) << 4) | hex_digit(pair[1]))
        .collect()
}

fn assert_real_merit_manifest(session: &DatasetSession) {
    assert_eq!(session.manifest().format_version().to_string(), "0.3.0");
    assert_eq!(
        session.auxiliary_declarations().d8_rasters.len(),
        EXPECTED_REAL_MERIT_D8_DECLS,
        "MERIT v0.2.0 should declare the expected blessed-D8 raster tiles"
    );
    assert_eq!(
        session.auxiliary_declarations().snaps.len(),
        EXPECTED_REAL_MERIT_SNAP_DECLS,
        "MERIT v0.2.0 should declare the expected snap auxiliary artifact"
    );
    for decl in &session.auxiliary_declarations().d8_rasters {
        assert!(decl.flow_dir.starts_with("aux/d8/"));
        assert!(decl.flow_dir.ends_with("/flow_dir.tif"));
        assert!(decl.flow_acc.starts_with("aux/d8/"));
        assert!(decl.flow_acc.ends_with("/flow_acc.tif"));
    }
}

fn assert_no_root_raster_reads(snapshot: &shed_core::source_telemetry::HttpStatsSnapshot) {
    for path in snapshot.per_path.keys() {
        assert!(
            !path.ends_with("merit/0.2.0/flow_dir.tif")
                && !path.ends_with("merit/0.2.0/flow_acc.tif"),
            "D8 readiness must not read legacy root raster path {path}"
        );
    }
}

fn assert_only_extent_headers_for_all_d8(
    snapshot: &shed_core::source_telemetry::HttpStatsSnapshot,
) {
    for (path, counters) in snapshot
        .per_path
        .iter()
        .filter(|(path, _)| path.contains("merit/0.2.0/aux/d8/"))
    {
        assert!(
            counters.bytes_in <= EXTENT_HEADER_RANGE_BYTES,
            "D8 declaration {path} read {} bytes before ambiguity detection, exceeding the bounded extent-header range; selection must not full-raster download",
            counters.bytes_in
        );
    }
}

fn d8_bytes_in(snapshot: &shed_core::source_telemetry::HttpStatsSnapshot) -> u64 {
    snapshot
        .per_path
        .iter()
        .filter(|(path, _)| path.contains("merit/0.2.0/aux/d8/"))
        .map(|(_, counters)| counters.bytes_in)
        .sum()
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this ignored test mutates PYSHED_BENCH_NET before creating
        // sessions and restores it before returning.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        // SAFETY: restores the process environment value changed by this test.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        other => panic!("invalid hex digit {other}"),
    }
}

#[derive(Debug, Deserialize)]
struct GoldenRecord {
    canonical_wkb_hex: String,
    area_km2: f64,
    input_outlet: GoldenOutlet,
    refined_outlet: GoldenOutlet,
    terminal_id: i64,
    upstream_ids: Vec<i64>,
    comparison_policy: GoldenComparisonPolicy,
}

#[derive(Debug, Deserialize)]
struct GoldenOutlet {
    lon: f64,
    lat: f64,
}

#[derive(Debug, Deserialize)]
struct GoldenComparisonPolicy {
    area_km2_abs_epsilon: f64,
    area_km2_rel_epsilon: f64,
}
