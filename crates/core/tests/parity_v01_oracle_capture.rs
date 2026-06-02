//! Capture-time v0.1 parity oracle tests.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use geo::{Area, BoundingRect, Rect};
use serde::{Deserialize, Serialize};
use shed_core::algo::{
    CANONICAL_WKB_VERSION, GeoCoord, SnapThreshold, canonical_wkb_multi_polygon,
};
use shed_core::session::DatasetSession;
use shed_core::test_raster_source::LocalTiffRasterSource;
use shed_core::{
    DelineationOptions, DelineationResult, Engine, PipTieBreak, RefinementOutcome,
    ResolutionMethod, ResolverConfig, SnapStrategy,
};

const FIXTURE_ROOT: &str = "tests/fixtures/parity/v01_synthetic_refined";
const GOLDEN_ROOT: &str = "tests/fixtures/parity/goldens/v01_synthetic_refined";
const GOLDEN_FILE: &str = "oracle_b_synthetic_refined.json";
const TERMINAL_AREA: f64 = 25.0;
const SYNTHETIC_OUTLET: GeoCoord = GeoCoord {
    lon: 2.5,
    lat: -2.5,
};
const STABILITY_RUNS: usize = 3;
const COORDINATE_ABS_EPSILON: f64 = 0.000001;
const AREA_KM2_ABS_EPSILON: f64 = 0.000001;
const AREA_KM2_REL_EPSILON: f64 = 0.000001;

#[test]
fn synthetic_fixture_smoke() {
    let result = delineate_synthetic_refined();

    assert!(
        matches!(result.refinement(), RefinementOutcome::Applied { .. }),
        "expected Applied refinement, got {:?}",
        result.refinement()
    );

    let refined_area = result.geometry().unsigned_area();
    assert!(
        refined_area > 0.0 && refined_area < TERMINAL_AREA,
        "expected strict shrink: 0 < refined_area < {TERMINAL_AREA}, got {refined_area}"
    );

    let terminal_bbox = Rect::new(
        geo::coord! { x: 0.0, y: -5.0 },
        geo::coord! { x: 5.0, y: 0.0 },
    );
    let refined_bbox = result
        .geometry()
        .bounding_rect()
        .expect("refined geometry should have a bbox");
    assert!(
        rect_contains_rect(&terminal_bbox, &refined_bbox),
        "refined bbox {refined_bbox:?} must be contained by terminal bbox {terminal_bbox:?}"
    );
}

#[test]
fn synthetic_refined_matches_committed_golden() {
    let golden = read_golden_record();
    let current = capture_synthetic_refined();

    assert_golden_matches_current(&golden, &current);
}

#[test]
fn synthetic_stability_check() {
    let first = capture_synthetic_refined();

    for run_index in 2..=STABILITY_RUNS {
        let current = capture_synthetic_refined();
        assert_golden_matches_current(&first, &current);
        assert_eq!(
            first.canonical_wkb_hex, current.canonical_wkb_hex,
            "canonical WKB changed on stability run {run_index}"
        );
    }
}

#[test]
fn bless_synthetic_refined() {
    if env::var_os("SHED_PARITY_BLESS").is_none() {
        let golden = read_golden_record();
        let current = capture_synthetic_refined();
        assert_golden_matches_current(&golden, &current);
        return;
    }

    synthetic_stability_check();

    let golden = capture_synthetic_refined_for_bless();
    let golden_path = golden_path();
    fs::create_dir_all(
        golden_path
            .parent()
            .expect("golden file should have a parent"),
    )
    .expect("golden directory should be creatable");
    fs::write(
        golden_path,
        serde_json::to_string_pretty(&golden).expect("golden should serialize") + "\n",
    )
    .expect("golden should be writable");
}

fn rect_contains_rect(outer: &Rect<f64>, inner: &Rect<f64>) -> bool {
    inner.min().x >= outer.min().x
        && inner.max().x <= outer.max().x
        && inner.min().y >= outer.min().y
        && inner.max().y <= outer.max().y
}

fn delineate_synthetic_refined() -> DelineationResult {
    let session =
        DatasetSession::open_path(&fixture_root()).expect("v0.1 synthetic refined fixture opens");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource)
        .build();

    engine
        .delineate(SYNTHETIC_OUTLET, &synthetic_options())
        .expect("synthetic refined fixture should delineate")
}

fn synthetic_options() -> DelineationOptions {
    DelineationOptions::default().with_snap_threshold(SnapThreshold::new(500))
}

fn capture_synthetic_refined() -> GoldenRecord {
    GoldenRecord::from_result(
        &delineate_synthetic_refined(),
        FixtureProvenance::not_read(),
    )
}

fn capture_synthetic_refined_for_bless() -> GoldenRecord {
    GoldenRecord::from_result(
        &delineate_synthetic_refined(),
        FixtureProvenance::read_from_fixture(),
    )
}

fn read_golden_record() -> GoldenRecord {
    serde_json::from_str(
        &fs::read_to_string(golden_path()).expect("B golden should be committed and readable"),
    )
    .expect("B golden should match the golden schema")
}

fn assert_golden_matches_current(golden: &GoldenRecord, current: &GoldenRecord) {
    assert_eq!(golden.canonical_wkb_hex, current.canonical_wkb_hex);
    assert_close(
        "area_km2",
        golden.area_km2,
        current.area_km2,
        golden.comparison_policy.area_km2_abs_epsilon,
        golden.comparison_policy.area_km2_rel_epsilon,
    );
    assert_outlet_close(
        "input_outlet",
        &golden.input_outlet,
        &current.input_outlet,
        golden,
    );
    assert_outlet_close(
        "resolved_outlet",
        &golden.resolved_outlet,
        &current.resolved_outlet,
        golden,
    );
    assert_eq!(
        golden.refined_outlet.is_some(),
        current.refined_outlet.is_some()
    );
    if let (Some(golden_refined), Some(current_refined)) =
        (&golden.refined_outlet, &current.refined_outlet)
    {
        assert_outlet_close("refined_outlet", golden_refined, current_refined, golden);
    }
    assert_eq!(golden.terminal_id, current.terminal_id);
    assert_eq!(golden.upstream_ids, current.upstream_ids);
    assert_eq!(golden.resolution_method, current.resolution_method);
    assert_eq!(golden.resolver_config, current.resolver_config);
    assert_eq!(golden.refinement_outcome, current.refinement_outcome);
    assert_eq!(golden.canonicalizer_version, current.canonicalizer_version);
}

fn assert_outlet_close(name: &str, golden: &Outlet, current: &Outlet, record: &GoldenRecord) {
    let epsilon = record.comparison_policy.coordinate_abs_epsilon;
    assert_abs_close(&format!("{name}.lon"), golden.lon, current.lon, epsilon);
    assert_abs_close(&format!("{name}.lat"), golden.lat, current.lat, epsilon);
}

fn assert_close(name: &str, expected: f64, actual: f64, abs_epsilon: f64, rel_epsilon: f64) {
    let diff = (expected - actual).abs();
    let rel_allowed = expected.abs().max(actual.abs()) * rel_epsilon;
    assert!(
        diff <= abs_epsilon.max(rel_allowed),
        "{name} expected {expected}, got {actual}, diff {diff}"
    );
}

fn assert_abs_close(name: &str, expected: f64, actual: f64, epsilon: f64) {
    let diff = (expected - actual).abs();
    assert!(
        diff <= epsilon,
        "{name} expected {expected}, got {actual}, diff {diff}"
    );
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT)
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(GOLDEN_ROOT)
        .join(GOLDEN_FILE)
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct GoldenRecord {
    canonical_wkb_hex: String,
    area_km2: f64,
    input_outlet: Outlet,
    resolved_outlet: Outlet,
    refined_outlet: Option<Outlet>,
    terminal_id: i64,
    upstream_ids: Vec<i64>,
    resolution_method: String,
    resolver_config: ResolverConfigRecord,
    refinement_outcome: RefinementOutcomeRecord,
    canonicalizer_version: String,
    comparison_policy: ComparisonPolicy,
    raster_interpretation: RasterInterpretation,
    fixture_provenance: FixtureProvenance,
    attestation: Attestation,
}

impl GoldenRecord {
    fn from_result(result: &DelineationResult, fixture_provenance: FixtureProvenance) -> Self {
        let mut upstream_ids = result
            .upstream_atom_ids()
            .iter()
            .map(|atom_id| i64::try_from(atom_id.get()).expect("atom id should fit in i64"))
            .collect::<Vec<_>>();
        upstream_ids.sort_unstable();
        upstream_ids.dedup();

        Self {
            canonical_wkb_hex: encode_hex(
                &canonical_wkb_multi_polygon(result.geometry())
                    .expect("engine geometry should canonicalize"),
            ),
            area_km2: result.area_km2().as_f64(),
            input_outlet: Outlet::from(result.input_outlet()),
            resolved_outlet: Outlet::from(result.resolved_outlet()),
            refined_outlet: refined_outlet(result.refinement()),
            terminal_id: i64::try_from(result.terminal_atom_id().get())
                .expect("terminal atom id should fit in i64"),
            upstream_ids,
            resolution_method: resolution_method_label(result.resolution_method()),
            resolver_config: ResolverConfigRecord::from(ResolverConfig::new()),
            refinement_outcome: RefinementOutcomeRecord::from(result.refinement()),
            canonicalizer_version: CANONICAL_WKB_VERSION.to_string(),
            comparison_policy: ComparisonPolicy::default(),
            raster_interpretation: RasterInterpretation::synthetic_refined(),
            fixture_provenance,
            attestation: Attestation::synthetic_refined(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Outlet {
    lon: f64,
    lat: f64,
}

impl From<GeoCoord> for Outlet {
    fn from(coord: GeoCoord) -> Self {
        Self {
            lon: coord.lon,
            lat: coord.lat,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ResolverConfigRecord {
    search_radius_m: f64,
}

impl From<ResolverConfig> for ResolverConfigRecord {
    fn from(config: ResolverConfig) -> Self {
        Self {
            search_radius_m: config.search_radius().as_f64(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RefinementOutcomeRecord {
    status: String,
    reason: Option<String>,
}

impl From<&RefinementOutcome> for RefinementOutcomeRecord {
    fn from(outcome: &RefinementOutcome) -> Self {
        match outcome {
            RefinementOutcome::Applied { .. } => Self {
                status: "Applied".to_string(),
                reason: None,
            },
            RefinementOutcome::NoRastersAvailable => Self {
                status: "NotApplied".to_string(),
                reason: Some("no rasters available".to_string()),
            },
            RefinementOutcome::NoRasterSourceProvided => Self {
                status: "NotApplied".to_string(),
                reason: Some("no raster source provided".to_string()),
            },
            RefinementOutcome::Disabled => Self {
                status: "NotApplied".to_string(),
                reason: Some("refinement disabled".to_string()),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ComparisonPolicy {
    coordinate_abs_epsilon: f64,
    area_km2_abs_epsilon: f64,
    area_km2_rel_epsilon: f64,
}

impl Default for ComparisonPolicy {
    fn default() -> Self {
        Self {
            coordinate_abs_epsilon: COORDINATE_ABS_EPSILON,
            area_km2_abs_epsilon: AREA_KM2_ABS_EPSILON,
            area_km2_rel_epsilon: AREA_KM2_REL_EPSILON,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RasterInterpretation {
    dimensions: RasterDimensions,
    crs: String,
    transform: [f64; 6],
    origin: String,
    pixel_size_degrees: PixelSize,
    extent: RasterExtent,
    pixel_interpretation: String,
    flow_direction: RasterBandInterpretation,
    flow_accumulation: RasterBandInterpretation,
}

impl RasterInterpretation {
    fn synthetic_refined() -> Self {
        Self {
            dimensions: RasterDimensions {
                columns: 5,
                rows: 5,
            },
            crs: "EPSG:4326".to_string(),
            transform: [0.0, 1.0, 0.0, 0.0, 0.0, -1.0],
            origin: "upper-left PixelIsArea corner (0, 0)".to_string(),
            pixel_size_degrees: PixelSize { x: 1.0, y: -1.0 },
            extent: RasterExtent {
                x_min: 0.0,
                x_max: 5.0,
                y_min: -5.0,
                y_max: 0.0,
            },
            pixel_interpretation:
                "GeoTIFF GTRasterTypeGeoKey=PixelIsArea; refinement uses pixel centers".to_string(),
            flow_direction: RasterBandInterpretation {
                sample_type: "uint8".to_string(),
                encoding: "ESRI D8".to_string(),
                nodata: "255".to_string(),
            },
            flow_accumulation: RasterBandInterpretation {
                sample_type: "float32".to_string(),
                encoding: "accumulation".to_string(),
                nodata: "-1 decoded as NaN".to_string(),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RasterDimensions {
    columns: usize,
    rows: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PixelSize {
    x: f64,
    y: f64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RasterExtent {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct RasterBandInterpretation {
    sample_type: String,
    encoding: String,
    nodata: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct FixtureProvenance {
    content_hash_algorithm: String,
    files: Vec<FileProvenance>,
}

impl FixtureProvenance {
    fn not_read() -> Self {
        Self {
            content_hash_algorithm: "sha256".to_string(),
            files: Vec::new(),
        }
    }

    fn read_from_fixture() -> Self {
        Self {
            content_hash_algorithm: "sha256".to_string(),
            files: [
                "manifest.json",
                "catchments.parquet",
                "graph.arrow",
                "flow_dir.tif",
                "flow_acc.tif",
            ]
            .iter()
            .map(|name| FileProvenance::read(name))
            .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct FileProvenance {
    path: String,
    size_bytes: u64,
    sha256: String,
}

impl FileProvenance {
    fn read(name: &str) -> Self {
        let path = fixture_root().join(name);
        Self {
            path: name.to_string(),
            size_bytes: fs::metadata(&path)
                .unwrap_or_else(|error| panic!("fixture file {name} should have metadata: {error}"))
                .len(),
            sha256: sha256_file(&path),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Attestation {
    local_tiff_raster_source_gdal_tile_parity: String,
    proof_command: String,
}

impl Attestation {
    fn synthetic_refined() -> Self {
        Self {
            local_tiff_raster_source_gdal_tile_parity:
                "Step 2 proved LocalTiffRasterSource tile-identical to GdalRasterSource for the B fixture window before this golden was blessed".to_string(),
            proof_command:
                "cargo test -p shed-gdal --test raster_decode_parity synthetic_b_tiff_matches_gdal -- --ignored --nocapture".to_string(),
        }
    }
}

fn refined_outlet(outcome: &RefinementOutcome) -> Option<Outlet> {
    match outcome {
        RefinementOutcome::Applied { refined_outlet } => Some(Outlet::from(*refined_outlet)),
        RefinementOutcome::NoRastersAvailable
        | RefinementOutcome::NoRasterSourceProvided
        | RefinementOutcome::Disabled => None,
    }
}

fn resolution_method_label(method: &ResolutionMethod) -> String {
    match method {
        ResolutionMethod::PointInPolygon {
            candidates_considered,
            tie_break,
        } => format!(
            "point-in-polygon(candidates_considered={candidates_considered},tie_break={})",
            pip_tie_break_label(tie_break.as_ref())
        ),
        ResolutionMethod::Snap {
            strategy,
            snap_id,
            distance_m,
            weight,
            mainstem_status,
            candidates_considered,
        } => format!(
            "snap(strategy={},snap_id={},distance_m={distance_m},weight={},mainstem_status={mainstem_status:?},candidates_considered={candidates_considered})",
            snap_strategy_label(*strategy),
            snap_id.get(),
            weight.get()
        ),
    }
}

fn pip_tie_break_label(tie_break: Option<&PipTieBreak>) -> String {
    match tie_break {
        Some(PipTieBreak::HighestUpstreamArea) => "highest-upstream-area".to_string(),
        Some(PipTieBreak::HighestLocalArea) => "highest-local-area".to_string(),
        Some(PipTieBreak::LowestAtomId) => "lowest-atom-id".to_string(),
        None => "none".to_string(),
    }
}

fn snap_strategy_label(strategy: SnapStrategy) -> &'static str {
    match strategy {
        SnapStrategy::DistanceFirst => "distance-first",
        SnapStrategy::WeightFirst => "weight-first",
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

fn sha256_file(path: &Path) -> String {
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("shasum should run for {path:?}: {error}"));
    assert!(
        output.status.success(),
        "shasum failed for {path:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("shasum output should be utf8")
        .split_whitespace()
        .next()
        .expect("shasum output should include a hash")
        .to_string()
}
