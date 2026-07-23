use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use geo::{Area, BoundingRect, Coord, MapCoords, MultiPolygon};
use pourpoint_core::algo::SnapThreshold;
use pourpoint_core::algo::coord::GeoCoord;
use pourpoint_core::algo::{Crs, canonical_wkb_multi_polygon, forward};
use pourpoint_core::session::DatasetSession;
use pourpoint_core::test_raster_source::LocalTiffRasterSource;
use pourpoint_core::{
    AppliedRefinementReason, DelineationOptions, Engine, LevelSelection, PreMergeDrainageUnit,
    PreMergeDrainageUnits, RefinementMode, RefinementOutcome, RefinementProvenance,
    RefinementStrategyName, ResolutionMethod, ResolverConfig, SearchRadiusMetres, SessionError,
    TerminalRefinement,
};
use serde::{Deserialize, Serialize};

const PARITY_FIXTURE_DIR: &str = "tests/fixtures/parity";
const V021_SYNTHETIC_REFINED_DIR: &str = "v021_synthetic_refined";
const PROJECTED_GRASS_DIR: &str = "tiny-with-aux-d8-projected-grass";
const PROJECTED_GRASS_GOLDEN: &str =
    "goldens/tiny-with-aux-d8-projected-grass/projected_grass_refined.json";
const PROJECTED_GRASS_CAPTURE_PREFIX: &str = "POURPOINT_PROJECTED_GRASS_CAPTURE:";
const PROJECTED_GRASS_CAPTURE_PROCESSES: usize = 20;
const PROJECTED_GRASS_INTEGRALITY_TOLERANCE: f64 = 1e-6;
const PROJECTED_GRASS_MINIMUM_CELL_COUNT: i64 = 16;
const M1_SYNTHETIC_REFINED_GOLDEN: &str =
    "goldens/v01_synthetic_refined/oracle_b_synthetic_refined.json";
const REAL_MERIT_V020_URL: &str = "https://basin-delineations-public.upstream.tech/merit/0.2.0/";
const REAL_D8_ENV: &str = "POURPOINT_HFX_V02_REAL_D8_REFINEMENT";
const REAL_MERIT_SEARCH_RADIUS_M: f64 = 5_000.0;
const EXPECTED_REAL_MERIT_D8_DECLS: usize = 60;
const EXPECTED_REAL_MERIT_SNAP_DECLS: usize = 1;
const EXTENT_HEADER_RANGE_BYTES: u64 = 256 * 1024;

#[test]
fn projected_grass_capture_child() {
    if std::env::var("POURPOINT_PROJECTED_GRASS_CAPTURE_CHILD").as_deref() != Ok("1") {
        return;
    }
    let lon = capture_coordinate("POURPOINT_PROJECTED_GRASS_CAPTURE_LON");
    let lat = capture_coordinate("POURPOINT_PROJECTED_GRASS_CAPTURE_LAT");
    let capture = capture_projected_grass(GeoCoord::new(lon, lat));
    println!(
        "{PROJECTED_GRASS_CAPTURE_PREFIX}{}",
        serde_json::to_string(&capture).expect("capture payload should serialize")
    );
}

#[test]
fn search_projected_grass_capture_candidates() {
    if std::env::var("POURPOINT_PROJECTED_GRASS_CAPTURE_SEARCH").as_deref() != Ok("1") {
        return;
    }
    let longitudes = [
        0.0833333333333333,
        0.25,
        0.4166666666666667,
        0.5833333333333333,
        0.75,
        0.9166666666666667,
        0.9833333333333333,
        1.0833333333333333,
        1.25,
        1.4166666666666667,
        1.5833333333333333,
        1.75,
        1.9166666666666667,
    ];
    let latitudes = [
        0.0833333333333333,
        0.25,
        0.4166666666666667,
        0.5833333333333333,
        0.75,
        0.9166666666666666,
        0.9166666666666667,
        1.0833333333333333,
        1.25,
        1.4166666666666667,
        1.5833333333333333,
        1.75,
        1.9166666666666667,
    ];

    for lat in latitudes {
        for lon in longitudes {
            let outlet = GeoCoord::new(lon, lat);
            let Ok(group) = capture_projected_grass_group(outlet, CaptureMode::Baseline) else {
                continue;
            };
            println!(
                "PROJECTED_GRASS_SELECTED:{}",
                serde_json::to_string(&group[0]).expect("selected capture should serialize")
            );
            println!(
                "PROJECTED_GRASS_BASELINE_SHA256:{}",
                sha256_bytes(&decode_hex(&group[0].canonical_wkb_hex))
            );
            return;
        }
    }
    panic!("fixed literal candidate set produced no qualifying projected GRASS outlet");
}

#[test]
fn projected_grass_mutation_probe() {
    if std::env::var("POURPOINT_PROJECTED_GRASS_MUTATION_PROBE").as_deref() != Ok("1") {
        return;
    }
    let outlet = GeoCoord::new(
        capture_coordinate("POURPOINT_PROJECTED_GRASS_CAPTURE_LON"),
        capture_coordinate("POURPOINT_PROJECTED_GRASS_CAPTURE_LAT"),
    );
    let group = capture_projected_grass_group(outlet, CaptureMode::MutationProbe)
        .expect("mutation-probe capture group should be internally stable");
    println!(
        "PROJECTED_GRASS_MUTATION_SHA256:{}",
        sha256_bytes(&decode_hex(&group[0].canonical_wkb_hex))
    );
    println!(
        "PROJECTED_GRASS_MUTATION_ROUNDED_COUNT:{}",
        group[0].rounded_derived_carved_cell_count
    );
}

#[test]
fn projected_grass_offline_golden_is_applied_and_stable() {
    let golden = read_projected_golden();
    let outlet = GeoCoord::new(golden.input_outlet.lon, golden.input_outlet.lat);
    let session = DatasetSession::open_path(&parity_fixture_path(PROJECTED_GRASS_DIR))
        .expect("projected GRASS parity fixture should open");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource::with_encoding(
            hfx::FlowDirEncoding::Grass,
        ))
        .build();
    let options = projected_grass_options();
    let result = engine
        .delineate(outlet, &options)
        .expect("projected GRASS delineation should succeed");
    let canonical = canonical_wkb_multi_polygon(result.geometry())
        .expect("projected GRASS geometry should canonicalize");
    let refined_outlet = golden
        .refined_outlet
        .as_ref()
        .expect("Applied golden should contain a refined outlet");

    assert_eq!(result.input_outlet(), outlet);
    assert_outlet_close(
        result.resolved_outlet(),
        &golden.resolved_outlet,
        golden.comparison_policy.coordinate_abs_epsilon,
    );
    assert_eq!(result.terminal_unit_id().get(), golden.terminal_id);
    assert_eq!(
        resolution_method_string(result.resolution_method()),
        golden.resolution_method
    );
    assert_eq!(golden.resolver_config.search_radius_m, 1000.0);
    assert_eq!(canonical, decode_hex(&golden.canonical_wkb_hex));
    assert_area_within_golden_policy(result.area_km2().as_f64(), golden.area_km2, &golden);
    let mut upstream_ids = result
        .upstream_unit_ids()
        .iter()
        .map(|id| id.get())
        .collect::<Vec<_>>();
    upstream_ids.sort_unstable();
    assert_eq!(upstream_ids, golden.upstream_ids);
    let RefinementOutcome::Applied {
        refined_outlet: actual_refined_outlet,
        provenance,
    } = result.refinement()
    else {
        panic!("projected refinement should be Applied");
    };
    assert_outlet_close(
        *actual_refined_outlet,
        refined_outlet,
        golden.comparison_policy.coordinate_abs_epsilon,
    );
    assert_eq!(
        provenance,
        &RefinementProvenance::Applied {
            strategy: RefinementStrategyName::BuiltInD8,
            why: AppliedRefinementReason::D8AuxMatchedTerminalBbox {
                declaration_index: 0,
            },
        }
    );

    let staged = capture_projected_grass(outlet);
    assert_eq!(
        staged.rounded_derived_carved_cell_count,
        golden
            .carve_measurement
            .as_ref()
            .expect("projected golden should contain carve measurement")
            .derived_carved_cell_count
    );
    assert_integral_derived_count(&staged, true).expect("golden carve should be non-vacuous");
    let group = capture_projected_grass_group(outlet, CaptureMode::Baseline)
        .expect("twenty-process projected GRASS capture should be stable");
    assert_eq!(group.len(), PROJECTED_GRASS_CAPTURE_PROCESSES);
    assert_eq!(decode_hex(&group[0].canonical_wkb_hex), canonical);

    let disabled = engine
        .delineate(
            outlet,
            &projected_grass_options().with_refinement_mode(RefinementMode::Disabled),
        )
        .expect("disabled projected delineation should succeed");
    let disabled_canonical = canonical_wkb_multi_polygon(disabled.geometry())
        .expect("disabled projected geometry should canonicalize");
    assert_ne!(disabled_canonical, canonical);
}

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
            refined_outlet: {
                let refined = golden
                    .refined_outlet
                    .as_ref()
                    .expect("refined golden should contain a refined outlet");
                GeoCoord::new(refined.lon, refined.lat)
            },
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
#[ignore = "network-gated MERIT v0.2.0 D8 refinement readiness proof; set POURPOINT_HFX_V02_REAL_D8_REFINEMENT=1"]
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
    let _bench_net = ScopedEnvVar::set("POURPOINT_BENCH_NET", "1");

    let probe_session =
        DatasetSession::open(REAL_MERIT_V020_URL).expect("real MERIT v0.2.0 should open");
    assert_real_merit_manifest(&probe_session);
    let options = real_merit_options();
    let terminal_geometry = {
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
            .clone()
    };

    let session =
        DatasetSession::open(REAL_MERIT_V020_URL).expect("real MERIT v0.2.0 should reopen");
    assert_real_merit_manifest(&session);
    assert!(
        session.http_stats().is_some(),
        "POURPOINT_BENCH_NET should expose remote request counters"
    );

    let selected_index = match session.select_d8_raster_for_terminal(&terminal_geometry) {
        Ok((handle, _)) => handle.declaration_index(),
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

fn read_projected_golden() -> GoldenRecord {
    read_golden(PROJECTED_GRASS_GOLDEN)
}

fn b_oracle_options() -> DelineationOptions {
    DelineationOptions::default().with_snap_threshold(SnapThreshold::new(500))
}

fn projected_grass_options() -> DelineationOptions {
    DelineationOptions::default()
        .with_resolver_config(
            ResolverConfig::new().with_search_radius(
                SearchRadiusMetres::new(1_000.0)
                    .expect("projected fixture search radius should be valid"),
            ),
        )
        .with_snap_threshold(SnapThreshold::new(500))
}

fn capture_coordinate(name: &str) -> f64 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} should be set for capture child"))
        .parse()
        .unwrap_or_else(|error| panic!("{name} should contain a coordinate: {error}"))
}

fn capture_projected_grass(outlet: GeoCoord) -> ProjectedGrassCapture {
    let session = DatasetSession::open_path(&parity_fixture_path(PROJECTED_GRASS_DIR))
        .expect("projected GRASS parity fixture should open");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource::with_encoding(
            hfx::FlowDirEncoding::Grass,
        ))
        .build();
    let options = projected_grass_options();
    let result = engine
        .delineate(outlet, &options)
        .expect("projected GRASS delineation should succeed");
    let canonical_wkb = canonical_wkb_multi_polygon(result.geometry())
        .expect("projected GRASS geometry should canonicalize");

    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("projected fixture finest level should resolve");
    let resolved = engine
        .resolve_outlet_at_level(outlet, selected, options.resolver_config())
        .expect("projected fixture outlet should resolve");
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("projected same-level traversal should succeed");
    let pre_merge = engine
        .produce_pre_merge_units(&upstream)
        .expect("projected pre-merge records should materialize");
    let terminal_geometry = pre_merge
        .terminal_unit()
        .expect("projected terminal record should exist")
        .geometry();
    let terminal_native = project_to_epsg8857(terminal_geometry);
    let terminal_bbox = terminal_native
        .bounding_rect()
        .expect("projected terminal geometry should have a bbox");
    let refinement = engine
        .refine_terminal_placeholder(&resolved, &pre_merge, &options)
        .expect("projected D8 refinement should apply");
    let TerminalRefinement::Applied {
        refined_outlet,
        geometry,
        provenance,
    } = refinement
    else {
        panic!("expected applied projected D8 refinement, got {refinement:?}");
    };
    assert_eq!(
        provenance,
        RefinementProvenance::Applied {
            strategy: RefinementStrategyName::BuiltInD8,
            why: AppliedRefinementReason::D8AuxMatchedTerminalBbox {
                declaration_index: 0,
            },
        }
    );
    let native_carve = project_to_epsg8857(geometry.polygon());
    let raw_count = native_carve.unsigned_area() / 1_000_000.0;
    let rounded_count = raw_count.round() as i64;
    let mut upstream_ids = result
        .upstream_unit_ids()
        .iter()
        .map(|id| id.get())
        .collect::<Vec<_>>();
    upstream_ids.sort_unstable();

    ProjectedGrassCapture {
        canonical_wkb_hex: encode_hex(&canonical_wkb),
        raw_derived_carved_cell_count: raw_count,
        rounded_derived_carved_cell_count: rounded_count,
        input_outlet: CaptureOutlet::from(outlet),
        resolved_outlet: CaptureOutlet::from(result.resolved_outlet()),
        refined_outlet: CaptureOutlet::from(refined_outlet),
        terminal_id: result.terminal_unit_id().get(),
        upstream_ids,
        area_km2: result.area_km2().as_f64(),
        resolution_method: resolution_method_string(result.resolution_method()),
        native_terminal_bbox: NativeBbox {
            min_x: terminal_bbox.min().x,
            min_y: terminal_bbox.min().y,
            max_x: terminal_bbox.max().x,
            max_y: terminal_bbox.max().y,
        },
    }
}

fn project_to_epsg8857(geometry: &MultiPolygon<f64>) -> MultiPolygon<f64> {
    geometry.map_coords(|Coord { x, y }| {
        let native = forward(Crs::Epsg8857, GeoCoord::new(x, y));
        Coord {
            x: native.x(),
            y: native.y(),
        }
    })
}

fn resolution_method_string(method: &ResolutionMethod) -> String {
    match method {
        ResolutionMethod::PointInPolygon {
            candidates_considered,
            tie_break,
        } => format!(
            "point-in-polygon(candidates_considered={candidates_considered},tie_break={})",
            tie_break
                .map(|value| format!("{value:?}"))
                .unwrap_or_else(|| "none".to_string())
        ),
        other => format!("{other:?}"),
    }
}

fn capture_projected_grass_group(
    outlet: GeoCoord,
    mode: CaptureMode,
) -> Result<Vec<ProjectedGrassCapture>, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("current test executable should resolve: {error}"))?;
    let mut captures = Vec::with_capacity(PROJECTED_GRASS_CAPTURE_PROCESSES);
    for _ in 0..PROJECTED_GRASS_CAPTURE_PROCESSES {
        let output = Command::new(&executable)
            .arg("projected_grass_capture_child")
            .args(["--exact", "--nocapture"])
            .env("POURPOINT_PROJECTED_GRASS_CAPTURE_CHILD", "1")
            .env(
                "POURPOINT_PROJECTED_GRASS_CAPTURE_LON",
                outlet.lon.to_string(),
            )
            .env(
                "POURPOINT_PROJECTED_GRASS_CAPTURE_LAT",
                outlet.lat.to_string(),
            )
            .output()
            .map_err(|error| format!("capture child should launch: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "capture child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|error| format!("capture child stdout should be utf8: {error}"))?;
        let marked = stdout
            .lines()
            .filter_map(|line| line.strip_prefix(PROJECTED_GRASS_CAPTURE_PREFIX))
            .collect::<Vec<_>>();
        if marked.len() != 1 {
            return Err(format!(
                "capture child should emit exactly one marked line, got {}",
                marked.len()
            ));
        }
        captures.push(
            serde_json::from_str(marked[0])
                .map_err(|error| format!("capture payload should deserialize: {error}"))?,
        );
    }
    let first = captures
        .first()
        .ok_or_else(|| "capture group should not be empty".to_string())?;
    for capture in &captures {
        assert_integral_derived_count(capture, mode == CaptureMode::Baseline)?;
        if capture.canonical_wkb_hex != first.canonical_wkb_hex {
            return Err("capture group canonical WKB bytes differ".to_string());
        }
        if capture.rounded_derived_carved_cell_count != first.rounded_derived_carved_cell_count {
            return Err("capture group rounded derived cell counts differ".to_string());
        }
    }
    Ok(captures)
}

fn assert_integral_derived_count(
    capture: &ProjectedGrassCapture,
    require_minimum: bool,
) -> Result<(), String> {
    let residual = (capture.raw_derived_carved_cell_count
        - capture.rounded_derived_carved_cell_count as f64)
        .abs();
    if residual > PROJECTED_GRASS_INTEGRALITY_TOLERANCE {
        return Err(format!(
            "derived cell count residual {residual} exceeds tolerance"
        ));
    }
    if require_minimum
        && capture.rounded_derived_carved_cell_count < PROJECTED_GRASS_MINIMUM_CELL_COUNT
    {
        return Err(format!(
            "rounded derived cell count {} is below minimum",
            capture.rounded_derived_carved_cell_count
        ));
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut child = Command::new("shasum")
        .args(["-a", "256"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("shasum should launch");
    child
        .stdin
        .take()
        .expect("shasum stdin should be piped")
        .write_all(bytes)
        .expect("canonical WKB should write to shasum");
    let output = child.wait_with_output().expect("shasum should finish");
    assert!(output.status.success(), "shasum should succeed");
    String::from_utf8(output.stdout)
        .expect("shasum output should be utf8")
        .split_whitespace()
        .next()
        .expect("shasum should output a hash")
        .to_string()
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn assert_outlet_close(actual: GeoCoord, expected: &GoldenOutlet, epsilon: f64) {
    assert!((actual.lon - expected.lon).abs() <= epsilon);
    assert!((actual.lat - expected.lat).abs() <= epsilon);
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

fn assert_no_root_raster_reads(snapshot: &pourpoint_core::source_telemetry::HttpStatsSnapshot) {
    for path in snapshot.per_path.keys() {
        assert!(
            !path.ends_with("merit/0.2.0/flow_dir.tif")
                && !path.ends_with("merit/0.2.0/flow_acc.tif"),
            "D8 readiness must not read legacy root raster path {path}"
        );
    }
}

fn assert_only_extent_headers_for_all_d8(
    snapshot: &pourpoint_core::source_telemetry::HttpStatsSnapshot,
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

fn d8_bytes_in(snapshot: &pourpoint_core::source_telemetry::HttpStatsSnapshot) -> u64 {
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
        // SAFETY: this ignored test mutates POURPOINT_BENCH_NET before creating
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
    resolved_outlet: GoldenOutlet,
    refined_outlet: Option<GoldenOutlet>,
    terminal_id: i64,
    upstream_ids: Vec<i64>,
    resolution_method: String,
    resolver_config: GoldenResolverConfig,
    #[serde(default)]
    carve_measurement: Option<GoldenCarveMeasurement>,
    comparison_policy: GoldenComparisonPolicy,
}

#[derive(Debug, Deserialize)]
struct GoldenOutlet {
    lon: f64,
    lat: f64,
}

#[derive(Debug, Deserialize)]
struct GoldenComparisonPolicy {
    coordinate_abs_epsilon: f64,
    area_km2_abs_epsilon: f64,
    area_km2_rel_epsilon: f64,
}

#[derive(Debug, Deserialize)]
struct GoldenResolverConfig {
    search_radius_m: f64,
}

#[derive(Debug, Deserialize)]
struct GoldenCarveMeasurement {
    derived_carved_cell_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectedGrassCapture {
    canonical_wkb_hex: String,
    raw_derived_carved_cell_count: f64,
    rounded_derived_carved_cell_count: i64,
    input_outlet: CaptureOutlet,
    resolved_outlet: CaptureOutlet,
    refined_outlet: CaptureOutlet,
    terminal_id: i64,
    upstream_ids: Vec<i64>,
    area_km2: f64,
    resolution_method: String,
    native_terminal_bbox: NativeBbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureOutlet {
    lon: f64,
    lat: f64,
}

impl From<GeoCoord> for CaptureOutlet {
    fn from(value: GeoCoord) -> Self {
        Self {
            lon: value.lon,
            lat: value.lat,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NativeBbox {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureMode {
    Baseline,
    MutationProbe,
}
