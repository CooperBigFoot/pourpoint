//! Isolated GDAL parity proof for committed raster fixtures.

use gdal::DriverManager;
use gdal::raster::Buffer;
use geo::Rect;
use geozero::ToGeo;
use geozero::wkb::Wkb;
use hfx::{FlowDirEncoding, UnitId};
use pourpoint_core::algo::{
    GridCoord, GridDims, NativeCoord, RasterSource, SnapThreshold, canonical_wkb_multi_polygon,
    refine_terminal_from_source,
};
use pourpoint_core::session::DatasetSession;
use pourpoint_core::test_raster_source::LocalTiffRasterSource;
use pourpoint_gdal::GdalRasterSource;
use serde::Deserialize;

const FIXTURE_ROOT: &str = "../core/tests/fixtures/parity/v01_synthetic_refined";
const MERIT_URL: &str = "https://basin-delineations-public.upstream.tech/merit-basins/0.1.0/";
const MERIT_GOLDEN: &str =
    "../core/tests/fixtures/parity/goldens/v01_merit_refined/oracle_c_merit_refined.json";
const MERIT_WINDOW_ROOT: &str = "merit_basins/0.1.0/raster-windows";

#[test]
fn signed_tiff_samples_match_local_and_gdal_normalization() {
    let directory = tempfile::tempdir().expect("signed TIFF temp directory should be created");
    let flow_dir_path = directory.path().join("flow_dir_int8.tif");
    let flow_acc_path = directory.path().join("flow_acc_int32.tif");
    write_signed_flow_direction(&flow_dir_path);
    write_signed_accumulation(&flow_acc_path);

    let bbox = Rect::new(
        geo::coord! { x: 0.0, y: 0.0 },
        geo::coord! { x: 2.0, y: 2.0 },
    );
    let local = LocalTiffRasterSource::with_encoding(FlowDirEncoding::Grass);
    let gdal = GdalRasterSource::with_encoding(FlowDirEncoding::Grass);

    let local_fd = local
        .load_flow_direction(&flow_dir_path.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode int8 flow direction");
    let gdal_fd = gdal
        .load_flow_direction(&flow_dir_path.to_string_lossy(), &bbox)
        .expect("GDAL source should decode int8 flow direction");
    assert_eq!(local_fd.dims(), GridDims::new(2, 2));
    assert_eq!(gdal_fd.dims(), GridDims::new(2, 2));
    assert_eq!(local_fd.inner().data(), &[1_u8, 254, 8, 0]);
    assert_eq!(local_fd.inner().data(), gdal_fd.inner().data());
    assert_eq!(local_fd.geo(), gdal_fd.geo());
    assert_eq!(local_fd.geo().origin_x(), 0.0);
    assert_eq!(local_fd.geo().origin_y(), 2.0);
    assert_eq!(local_fd.geo().pixel_width(), 1.0);
    assert_eq!(local_fd.geo().pixel_height(), -1.0);
    assert_eq!(
        local_fd.get(GridCoord::new(0, 0)),
        Some(pourpoint_core::algo::FlowDir::Northeast)
    );
    assert_eq!(local_fd.get(GridCoord::new(0, 1)), None);
    assert_eq!(
        local_fd.get(GridCoord::new(1, 0)),
        Some(pourpoint_core::algo::FlowDir::East)
    );
    assert_eq!(local_fd.get(GridCoord::new(1, 1)), None);

    let local_acc = local
        .load_accumulation(&flow_acc_path.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode int32 accumulation");
    let gdal_acc = gdal
        .load_accumulation(&flow_acc_path.to_string_lossy(), &bbox)
        .expect("GDAL source should decode int32 accumulation");
    assert_eq!(local_acc.dims(), GridDims::new(2, 2));
    assert_eq!(gdal_acc.dims(), GridDims::new(2, 2));
    assert_f32_tiles_equal(local_acc.inner().data(), &[1.0_f32, f32::NAN, 3.0, 4.0]);
    assert_f32_tiles_equal(local_acc.inner().data(), gdal_acc.inner().data());
    assert!(local_acc.inner().nodata().is_nan());
    assert!(gdal_acc.inner().nodata().is_nan());
    assert_eq!(local_acc.geo(), gdal_acc.geo());
}

fn write_signed_flow_direction(path: &std::path::Path) {
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver should exist");
    let mut dataset = driver
        .create_with_band_type::<i8, _>(path, 2, 2, 1)
        .expect("int8 flow-direction TIFF should be created");
    dataset
        .set_geo_transform(&[0.0, 1.0, 0.0, 2.0, 0.0, -1.0])
        .expect("flow-direction geotransform should be written");
    let mut band = dataset
        .rasterband(1)
        .expect("flow-direction band should exist");
    band.set_no_data_value(Some(-1.0))
        .expect("flow-direction nodata should be written");
    let mut buffer = Buffer::new((2, 2), vec![1_i8, -2, 8, 0]);
    band.write((0, 0), (2, 2), &mut buffer)
        .expect("flow-direction samples should be written");
}

fn write_signed_accumulation(path: &std::path::Path) {
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver should exist");
    let mut dataset = driver
        .create_with_band_type::<i32, _>(path, 2, 2, 1)
        .expect("int32 accumulation TIFF should be created");
    dataset
        .set_geo_transform(&[0.0, 1.0, 0.0, 2.0, 0.0, -1.0])
        .expect("accumulation geotransform should be written");
    let mut band = dataset
        .rasterband(1)
        .expect("accumulation band should exist");
    band.set_no_data_value(Some(-9999.0))
        .expect("accumulation nodata should be written");
    let mut buffer = Buffer::new((2, 2), vec![1_i32, -9999, 3, 4]);
    band.write((0, 0), (2, 2), &mut buffer)
        .expect("accumulation samples should be written");
}

#[test]
#[ignore = "requires GDAL runtime"]
fn synthetic_b_tiff_matches_gdal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let flow_dir_path = root.join("flow_dir.tif");
    let flow_acc_path = root.join("flow_acc.tif");
    let bbox = Rect::new(
        geo::coord! { x: 0.0, y: -5.0 },
        geo::coord! { x: 5.0, y: 0.0 },
    );

    let local = LocalTiffRasterSource;
    let gdal = GdalRasterSource::new();

    let local_fd = local
        .load_flow_direction(&flow_dir_path.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode flow_dir");
    let gdal_fd = gdal
        .load_flow_direction(&flow_dir_path.to_string_lossy(), &bbox)
        .expect("GDAL source should decode flow_dir");
    assert_eq!(local_fd.inner().data(), gdal_fd.inner().data());
    assert_eq!(local_fd.inner().nodata(), gdal_fd.inner().nodata());
    assert_eq!(local_fd.geo(), gdal_fd.geo());

    let local_acc = local
        .load_accumulation(&flow_acc_path.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode flow_acc");
    let gdal_acc = gdal
        .load_accumulation(&flow_acc_path.to_string_lossy(), &bbox)
        .expect("GDAL source should decode flow_acc");
    assert_f32_tiles_equal(local_acc.inner().data(), gdal_acc.inner().data());
    assert!(local_acc.inner().nodata().is_nan());
    assert!(gdal_acc.inner().nodata().is_nan());
    assert_eq!(local_acc.geo(), gdal_acc.geo());
}

#[test]
#[ignore = "requires network-materialized MERIT C windows and GDAL runtime"]
fn merit_c_windows_tiff_match_gdal() {
    assert_eq!(
        std::env::var("POURPOINT_PARITY_R2_CAPTURE").as_deref(),
        Ok("1"),
        "POURPOINT_PARITY_R2_CAPTURE=1 is required for the MERIT C decode proof"
    );

    let root = hfx_cache_root().join(MERIT_WINDOW_ROOT);
    let pairs = merit_window_pairs(&root);
    assert!(
        !pairs.is_empty(),
        "no MERIT windows found in {}; run the core Step 4 capture first",
        root.display()
    );

    let local = LocalTiffRasterSource;
    let gdal = GdalRasterSource::new();
    for pair in pairs {
        assert_raster_pair_matches(&local, &gdal, &pair);
    }

    let root = hfx_cache_root().join(MERIT_WINDOW_ROOT);
    let pairs = merit_window_pairs(&root);
    let session = DatasetSession::open(MERIT_URL).expect("MERIT session should open");
    for record in merit_c_records() {
        let terminal_polygon = terminal_polygon(&session, record.terminal_id);
        assert_direct_terminal_carve_matches_gdal(
            &local,
            &gdal,
            &pairs,
            &terminal_polygon,
            &record,
        );
    }
}

fn assert_raster_pair_matches(
    local: &LocalTiffRasterSource,
    gdal: &GdalRasterSource,
    pair: &MeritWindowPair,
) {
    let bbox = Rect::new(
        geo::coord! { x: -180.0, y: -60.0 },
        geo::coord! { x: 180.0, y: 60.0 },
    );
    let local_fd = local
        .load_flow_direction(&pair.flow_dir.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode MERIT flow_dir window");
    let gdal_fd = gdal
        .load_flow_direction(&pair.flow_dir.to_string_lossy(), &bbox)
        .expect("GDAL source should decode MERIT flow_dir window");
    assert_eq!(local_fd.inner().data(), gdal_fd.inner().data());
    assert_eq!(local_fd.inner().nodata(), gdal_fd.inner().nodata());
    assert_eq!(local_fd.geo(), gdal_fd.geo());

    let local_acc = local
        .load_accumulation(&pair.flow_acc.to_string_lossy(), &bbox)
        .expect("local TIFF source should decode MERIT flow_acc window");
    let gdal_acc = gdal
        .load_accumulation(&pair.flow_acc.to_string_lossy(), &bbox)
        .expect("GDAL source should decode MERIT flow_acc window");
    assert_f32_tiles_equal(local_acc.inner().data(), gdal_acc.inner().data());
    assert!(local_acc.inner().nodata().is_nan());
    assert!(gdal_acc.inner().nodata().is_nan());
    assert_eq!(local_acc.geo(), gdal_acc.geo());
}

fn assert_direct_terminal_carve_matches_gdal(
    local: &LocalTiffRasterSource,
    gdal: &GdalRasterSource,
    pairs: &[MeritWindowPair],
    terminal_polygon: &geo::MultiPolygon<f64>,
    record: &MeritGoldenRecord,
) {
    let mut last_error = None;
    for pair in pairs {
        let local_result = refine_terminal_from_source(
            local,
            &pair.flow_dir.to_string_lossy(),
            &pair.flow_acc.to_string_lossy(),
            terminal_polygon,
            record.resolved_outlet.into(),
            SnapThreshold::DEFAULT,
        );
        let gdal_result = refine_terminal_from_source(
            gdal,
            &pair.flow_dir.to_string_lossy(),
            &pair.flow_acc.to_string_lossy(),
            terminal_polygon,
            record.resolved_outlet.into(),
            SnapThreshold::DEFAULT,
        );
        match (local_result, gdal_result) {
            (Ok(local_result), Ok(gdal_result)) => {
                assert_eq!(local_result.snapped_coord(), gdal_result.snapped_coord());
                assert_eq!(
                    canonical_wkb_multi_polygon(local_result.polygon())
                        .expect("local carve should canonicalize"),
                    canonical_wkb_multi_polygon(gdal_result.polygon())
                        .expect("GDAL carve should canonicalize")
                );
                return;
            }
            (Err(local_error), Err(gdal_error)) => {
                last_error = Some(format!(
                    "local={local_error}; gdal={gdal_error}; pair={:?}",
                    pair
                ));
            }
            (local_result, gdal_result) => {
                panic!(
                    "{} direct carve had divergent success: local={:?}, gdal={:?}, pair={:?}",
                    record.case_name, local_result, gdal_result, pair
                );
            }
        }
    }

    panic!(
        "{} direct terminal carve did not succeed for any cached window pair; last_error={:?}",
        record.case_name, last_error
    );
}

fn merit_c_records() -> Vec<MeritGoldenRecord> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(MERIT_GOLDEN);
    serde_json::from_str(&std::fs::read_to_string(path).expect("MERIT C golden should be readable"))
        .expect("MERIT C golden should match the proof schema")
}

fn terminal_polygon(session: &DatasetSession, terminal_id: i64) -> geo::MultiPolygon<f64> {
    let unit_id = UnitId::new(terminal_id).expect("terminal id should be valid");
    let unit = session
        .catchments()
        .query_by_ids(&[unit_id])
        .expect("terminal catchment should query by id")
        .into_iter()
        .next()
        .expect("terminal catchment should exist");
    match Wkb(unit.geometry().as_bytes())
        .to_geo()
        .expect("terminal WKB should decode")
    {
        geo::Geometry::MultiPolygon(multipolygon) => multipolygon,
        geo::Geometry::Polygon(polygon) => geo::MultiPolygon::new(vec![polygon]),
        other => panic!("expected terminal MultiPolygon WKB, got {other:?}"),
    }
}

#[derive(Debug, Deserialize)]
struct MeritGoldenRecord {
    case_name: String,
    resolved_outlet: Outlet,
    terminal_id: i64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Outlet {
    lon: f64,
    lat: f64,
}

impl From<Outlet> for NativeCoord {
    fn from(outlet: Outlet) -> Self {
        NativeCoord::new(outlet.lon, outlet.lat)
    }
}

#[derive(Debug)]
struct MeritWindowPair {
    flow_dir: std::path::PathBuf,
    flow_acc: std::path::PathBuf,
}

fn merit_window_pairs(root: &std::path::Path) -> Vec<MeritWindowPair> {
    let entries = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("MERIT raster-window cache should be readable: {error}"));
    let mut flow_dir = std::collections::BTreeMap::new();
    let mut flow_acc = std::collections::BTreeMap::new();
    for entry in entries {
        let path = entry
            .expect("MERIT raster-window cache entry should be readable")
            .path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some(fragment) = name.strip_prefix("flow-dir.") {
            flow_dir.insert(window_fragment(fragment), path);
        } else if let Some(fragment) = name.strip_prefix("flow-acc.") {
            flow_acc.insert(window_fragment(fragment), path);
        }
    }
    flow_dir
        .into_iter()
        .filter_map(|(fragment, flow_dir)| {
            flow_acc
                .get(&fragment)
                .cloned()
                .map(|flow_acc| MeritWindowPair { flow_dir, flow_acc })
        })
        .collect()
}

fn window_fragment(name_without_kind: &str) -> String {
    name_without_kind
        .split_once('.')
        .map(|(_, fragment)| fragment)
        .unwrap_or(name_without_kind)
        .trim_end_matches(".tif")
        .to_string()
}

fn hfx_cache_root() -> std::path::PathBuf {
    std::env::var_os("HFX_CACHE_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache/hfx"))
        })
        .expect("HFX cache root should be available")
}

fn assert_f32_tiles_equal(left: &[f32], right: &[f32]) {
    assert_eq!(left.len(), right.len());
    for (idx, (&a, &b)) in left.iter().zip(right).enumerate() {
        assert!(
            (a.is_nan() && b.is_nan()) || a == b,
            "f32 tile mismatch at {idx}: local={a:?} gdal={b:?}"
        );
    }
}
