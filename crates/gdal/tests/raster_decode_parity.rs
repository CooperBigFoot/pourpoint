//! Isolated GDAL parity proof for committed raster fixtures.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use gdal::DriverManager;
use gdal::raster::{Buffer, GdalDataType, RasterCreationOptions};
use gdal::spatial_ref::SpatialRef;
use geo::{Area, BoundingRect, Rect};
use geozero::ToGeo;
use geozero::wkb::Wkb;
use hfx::{FlowAccumulationUnits, FlowDirEncoding, UnitId};
use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use pourpoint_core::algo::{
    Crs, GeoCoord, GridCoord, GridDims, NativeCoord, RasterOutlet, RasterSource, RasterSourceError,
    RefinementError, SnapThreshold, canonical_wkb_multi_polygon, forward,
    refine_terminal_from_source,
};
use pourpoint_core::session::{DatasetSession, RasterKind};
use pourpoint_core::test_raster_source::LocalTiffRasterSource;
use pourpoint_core::{
    CrossedTileAxes, DelineationOptions, Engine, LevelSelection, LocalizedRasterWindow,
    RefinementMode, ResolverConfig, SearchRadiusMetres, TerminalRefinement,
};
use pourpoint_gdal::GdalRasterSource;
use serde::Deserialize;
use tempfile::TempDir;
use tiff::decoder::Decoder;
use tiff::tags::Tag;
use url::Url;

const FIXTURE_ROOT: &str = "../core/tests/fixtures/parity/v01_synthetic_refined";
const MERIT_URL: &str = "https://basin-delineations-public.upstream.tech/merit-basins/0.1.0/";
const MERIT_GOLDEN: &str =
    "../core/tests/fixtures/parity/goldens/v01_merit_refined/oracle_c_merit_refined.json";
const MERIT_WINDOW_ROOT: &str = "merit_basins/0.1.0/raster-windows";
const PROJECTED_GRASS_ROOT: &str = "../core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass";
const PROJECTED_GRASS_GOLDEN: &str = "../core/tests/fixtures/parity/goldens/tiny-with-aux-d8-projected-grass/projected_grass_refined.json";
const PROJECTED_GRASS_FLOW_DIR: &str =
    "../core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass/aux/d8/projected/flow_dir.tif";
const PROJECTED_GRASS_FLOW_ACC: &str =
    "../core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass/aux/d8/projected/flow_acc.tif";
const REMOTE_ROOT: &str = "m3-s2/projected-grass";
const PROBE_ROOT: &str = "m3-s2/projected-grass-placement-probe";
const REMOTE_URL: &str = "s3://pourpoint-test/m3-s2/projected-grass/";
const PROBE_URL: &str = "s3://pourpoint-test/m3-s2/projected-grass-placement-probe/";
const PROBE_FABRIC_NAME: &str = "conformance-tiny-projected-grass-placement-probe";
const SOURCE_SIDE: usize = 256;
const RASTER_SIDE: usize = 1024;
const TILE_SIDE: usize = 512;
const RASTER_PIXELS: usize = RASTER_SIDE * RASTER_SIDE;
const TILE_PIXELS: usize = TILE_SIDE * TILE_SIDE;
const PROJECTED_TRANSFORM: [f64; 6] = [0.0, 1000.0, 0.0, 256000.0, 0.0, -1000.0];

static CACHE_ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
struct ProjectedGrassGolden {
    canonical_wkb_hex: String,
    input_outlet: Outlet,
    resolved_outlet: Outlet,
    terminal_id: i64,
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(
        value.len() % 2,
        0,
        "projected GRASS golden hex must have an even length"
    );
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("projected GRASS golden hex must be ASCII");
            u8::from_str_radix(pair, 16)
                .expect("projected GRASS golden must contain hexadecimal bytes")
        })
        .collect()
}

struct CacheEnv {
    _guard: MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl CacheEnv {
    fn set(path: &Path) -> Self {
        let guard = CACHE_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::env::var_os("HFX_CACHE_DIR");
        // SAFETY: this test binary serializes HFX_CACHE_DIR changes with
        // CACHE_ENV_LOCK and restores the prior value before releasing it.
        unsafe {
            std::env::set_var("HFX_CACHE_DIR", path);
        }
        Self {
            _guard: guard,
            previous,
        }
    }
}

impl Drop for CacheEnv {
    fn drop(&mut self) {
        // SAFETY: CACHE_ENV_LOCK remains held while the prior value is restored.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("HFX_CACHE_DIR", value),
                None => std::env::remove_var("HFX_CACHE_DIR"),
            }
        }
    }
}

fn creation_options() -> RasterCreationOptions {
    RasterCreationOptions::from_iter([
        "TILED=YES",
        "BLOCKXSIZE=512",
        "BLOCKYSIZE=512",
        "COMPRESS=DEFLATE",
        "PREDICTOR=2",
    ])
}

fn read_source_i8(path: &Path) -> Vec<i8> {
    let dataset = gdal::Dataset::open(path).expect("committed FlowDir source should open");
    assert_eq!(dataset.raster_size(), (SOURCE_SIDE, SOURCE_SIDE));
    assert_eq!(dataset.raster_count(), 1);
    assert_eq!(
        dataset.geo_transform().expect("source geotransform"),
        PROJECTED_TRANSFORM
    );
    assert_eq!(
        dataset
            .spatial_ref()
            .expect("source CRS")
            .authority()
            .expect("source CRS authority"),
        "EPSG:8857"
    );
    let band = dataset.rasterband(1).expect("source FlowDir band");
    assert_eq!(band.band_type(), GdalDataType::Int8);
    assert_eq!(band.no_data_value(), Some(-128.0));
    band.read_as::<i8>(
        (0, 0),
        (SOURCE_SIDE, SOURCE_SIDE),
        (SOURCE_SIDE, SOURCE_SIDE),
        None,
    )
    .expect("source FlowDir samples should read")
    .data()
    .to_vec()
}

fn read_source_i32(path: &Path) -> Vec<i32> {
    let dataset = gdal::Dataset::open(path).expect("committed FlowAcc source should open");
    assert_eq!(dataset.raster_size(), (SOURCE_SIDE, SOURCE_SIDE));
    assert_eq!(dataset.raster_count(), 1);
    assert_eq!(
        dataset.geo_transform().expect("source geotransform"),
        PROJECTED_TRANSFORM
    );
    assert_eq!(
        dataset
            .spatial_ref()
            .expect("source CRS")
            .authority()
            .expect("source CRS authority"),
        "EPSG:8857"
    );
    let band = dataset.rasterband(1).expect("source FlowAcc band");
    assert_eq!(band.band_type(), GdalDataType::Int32);
    assert_eq!(band.no_data_value(), Some(-2_147_483_648.0));
    band.read_as::<i32>(
        (0, 0),
        (SOURCE_SIDE, SOURCE_SIDE),
        (SOURCE_SIDE, SOURCE_SIDE),
        None,
    )
    .expect("source FlowAcc samples should read")
    .data()
    .to_vec()
}

fn write_i8_raster(path: &Path, samples: &[i8]) {
    assert_eq!(samples.len(), RASTER_PIXELS);
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver should exist");
    let mut dataset = driver
        .create_with_band_type_with_options::<i8, _>(
            path,
            RASTER_SIDE,
            RASTER_SIDE,
            1,
            &creation_options(),
        )
        .expect("tiled Int8 TIFF should be created");
    dataset
        .set_geo_transform(&PROJECTED_TRANSFORM)
        .expect("Int8 geotransform should be written");
    dataset
        .set_spatial_ref(&SpatialRef::from_epsg(8857).expect("EPSG:8857 should resolve"))
        .expect("Int8 CRS should be written");
    {
        let mut band = dataset.rasterband(1).expect("Int8 band should exist");
        band.set_no_data_value(Some(-128.0))
            .expect("Int8 nodata should be written");
        let mut buffer = Buffer::new((RASTER_SIDE, RASTER_SIDE), samples.to_vec());
        band.write((0, 0), (RASTER_SIDE, RASTER_SIDE), &mut buffer)
            .expect("Int8 samples should be written");
    }
    dataset.flush_cache().expect("Int8 TIFF should flush");
}

fn write_i32_raster(path: &Path, samples: &[i32]) {
    assert_eq!(samples.len(), RASTER_PIXELS);
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver should exist");
    let mut dataset = driver
        .create_with_band_type_with_options::<i32, _>(
            path,
            RASTER_SIDE,
            RASTER_SIDE,
            1,
            &creation_options(),
        )
        .expect("tiled Int32 TIFF should be created");
    dataset
        .set_geo_transform(&PROJECTED_TRANSFORM)
        .expect("Int32 geotransform should be written");
    dataset
        .set_spatial_ref(&SpatialRef::from_epsg(8857).expect("EPSG:8857 should resolve"))
        .expect("Int32 CRS should be written");
    {
        let mut band = dataset.rasterband(1).expect("Int32 band should exist");
        band.set_no_data_value(Some(-2_147_483_648.0))
            .expect("Int32 nodata should be written");
        let mut buffer = Buffer::new((RASTER_SIDE, RASTER_SIDE), samples.to_vec());
        band.write((0, 0), (RASTER_SIDE, RASTER_SIDE), &mut buffer)
            .expect("Int32 samples should be written");
    }
    dataset.flush_cache().expect("Int32 TIFF should flush");
}

fn validate_i8_raster(path: &Path, expected: &[i8]) -> Vec<i8> {
    let dataset = gdal::Dataset::open(path).expect("generated Int8 TIFF should reopen");
    assert_generated_layout(&dataset, GdalDataType::Int8, -128.0);
    let actual = dataset
        .rasterband(1)
        .expect("generated Int8 band")
        .read_as::<i8>(
            (0, 0),
            (RASTER_SIDE, RASTER_SIDE),
            (RASTER_SIDE, RASTER_SIDE),
            None,
        )
        .expect("generated Int8 samples should read");
    assert_eq!(actual.data(), expected);
    actual.data().to_vec()
}

fn validate_i32_raster(path: &Path, expected: &[i32]) -> Vec<i32> {
    let dataset = gdal::Dataset::open(path).expect("generated Int32 TIFF should reopen");
    assert_generated_layout(&dataset, GdalDataType::Int32, -2_147_483_648.0);
    let actual = dataset
        .rasterband(1)
        .expect("generated Int32 band")
        .read_as::<i32>(
            (0, 0),
            (RASTER_SIDE, RASTER_SIDE),
            (RASTER_SIDE, RASTER_SIDE),
            None,
        )
        .expect("generated Int32 samples should read");
    assert_eq!(actual.data(), expected);
    actual.data().to_vec()
}

fn assert_generated_layout(dataset: &gdal::Dataset, data_type: GdalDataType, nodata: f64) {
    assert_eq!(dataset.raster_size(), (RASTER_SIDE, RASTER_SIDE));
    assert_eq!(dataset.raster_count(), 1);
    assert_eq!(
        dataset.geo_transform().expect("generated geotransform"),
        PROJECTED_TRANSFORM
    );
    assert_eq!(
        dataset
            .spatial_ref()
            .expect("generated CRS")
            .authority()
            .expect("generated CRS authority"),
        "EPSG:8857"
    );
    let band = dataset.rasterband(1).expect("generated band should exist");
    assert_eq!(band.block_size(), (TILE_SIDE, TILE_SIDE));
    assert_eq!(band.band_type(), data_type);
    assert_eq!(band.no_data_value(), Some(nodata));
}

fn pristine_i8(source: &[i8]) -> Vec<i8> {
    assert_eq!(source.len(), SOURCE_SIDE * SOURCE_SIDE);
    let mut expanded = vec![-128_i8; RASTER_PIXELS];
    for y in 0..SOURCE_SIDE {
        let source_row = &source[y * SOURCE_SIDE..(y + 1) * SOURCE_SIDE];
        expanded[y * RASTER_SIDE..y * RASTER_SIDE + SOURCE_SIDE].copy_from_slice(source_row);
    }
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            if x < SOURCE_SIDE && y < SOURCE_SIDE {
                assert_eq!(expanded[y * RASTER_SIDE + x], source[y * SOURCE_SIDE + x]);
            } else {
                assert_eq!(expanded[y * RASTER_SIDE + x], -128);
            }
        }
    }
    expanded
}

fn pristine_i32(source: &[i32]) -> Vec<i32> {
    assert_eq!(source.len(), SOURCE_SIDE * SOURCE_SIDE);
    let mut expanded = vec![-2_147_483_648_i32; RASTER_PIXELS];
    for y in 0..SOURCE_SIDE {
        let source_row = &source[y * SOURCE_SIDE..(y + 1) * SOURCE_SIDE];
        expanded[y * RASTER_SIDE..y * RASTER_SIDE + SOURCE_SIDE].copy_from_slice(source_row);
    }
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            if x < SOURCE_SIDE && y < SOURCE_SIDE {
                assert_eq!(expanded[y * RASTER_SIDE + x], source[y * SOURCE_SIDE + x]);
            } else {
                assert_eq!(expanded[y * RASTER_SIDE + x], -2_147_483_648);
            }
        }
    }
    expanded
}

fn probe_i8() -> Vec<i8> {
    let mut samples = Vec::with_capacity(RASTER_PIXELS);
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            let bx = x / TILE_SIDE;
            let by = y / TILE_SIDE;
            let b = 2 * by + bx;
            let u = x % TILE_SIDE;
            let v = y % TILE_SIDE;
            samples.push(if u == 0 && v == 0 {
                -128
            } else {
                (1 + ((b + u + 3 * v) % 8)) as i8
            });
        }
    }
    samples
}

fn probe_i32() -> Vec<i32> {
    let mut samples = Vec::with_capacity(RASTER_PIXELS);
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            let bx = x / TILE_SIDE;
            let by = y / TILE_SIDE;
            let b = 2 * by + bx;
            let u = x % TILE_SIDE;
            let v = y % TILE_SIDE;
            samples.push(if u == 0 && v == 0 {
                -2_147_483_648
            } else {
                (b * 1_000_000 + v * TILE_SIDE + u + 1) as i32
            });
        }
    }
    samples
}

fn block_samples<T: Copy>(samples: &[T], block: usize) -> Vec<T> {
    let bx = block % 2;
    let by = block / 2;
    let mut result = Vec::with_capacity(TILE_PIXELS);
    for v in 0..TILE_SIDE {
        for u in 0..TILE_SIDE {
            let x = bx * TILE_SIDE + u;
            let y = by * TILE_SIDE + v;
            result.push(samples[y * RASTER_SIDE + x]);
        }
    }
    result
}

fn assert_distinct_blocks<T: Copy + Eq + std::fmt::Debug>(samples: &[T]) {
    let blocks = [
        block_samples(samples, 0),
        block_samples(samples, 1),
        block_samples(samples, 2),
        block_samples(samples, 3),
    ];
    for (left, right) in [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)] {
        assert_ne!(blocks[left], blocks[right], "blocks {left} and {right}");
    }
}

fn localized_nodata(path: &Path) -> String {
    let file = std::fs::File::open(path).expect("localized TIFF should open");
    let mut decoder = Decoder::new(file).expect("localized TIFF should decode");
    decoder
        .get_tag_ascii_string(Tag::GdalNodata)
        .expect("localized TIFF should declare GDAL nodata")
}

fn assert_remote_window_coverage(
    localized: &LocalizedRasterWindow,
    expected_offsets: (u32, u32),
    expected_flat_indexes: &[u32],
    expected_axes: CrossedTileAxes,
) {
    let coverage = localized
        .coverage()
        .expect("remote localization should expose prepared COG coverage");
    assert_eq!(coverage.origin_x(), 0.0);
    assert_eq!(coverage.origin_y(), 256000.0);
    assert_eq!(coverage.pixel_width(), 1000.0);
    assert_eq!(coverage.pixel_height(), -1000.0);
    assert_eq!(coverage.raster_width(), 1024);
    assert_eq!(coverage.raster_height(), 1024);
    assert_eq!(coverage.tile_width(), 512);
    assert_eq!(coverage.tile_height(), 512);
    assert_eq!(
        (coverage.window_col_off(), coverage.window_row_off()),
        expected_offsets
    );
    assert_eq!(coverage.covered_tile_indexes(), expected_flat_indexes);
    for &flat_index in expected_flat_indexes {
        let expected_col_row = match flat_index {
            0 => (0, 0),
            1 => (1, 0),
            2 => (0, 1),
            3 => (1, 1),
            _ => panic!("fixture flat tile index {flat_index} has no expected mapping"),
        };
        assert_eq!(
            coverage.covered_tile_col_row(flat_index),
            Some(expected_col_row)
        );
    }
    assert_eq!(coverage.covered_tile_col_row(4), None);
    assert_eq!(coverage.crossed_axes(), expected_axes);
}

async fn put_staged(store: &Arc<dyn ObjectStore>, root: &str, key: &str, bytes: &[u8]) {
    let path = ObjectPath::from(format!("{root}/{key}"));
    store
        .put(&path, PutPayload::from(bytes.to_vec()))
        .await
        .unwrap_or_else(|error| panic!("failed to stage {path}: {error}"));
}

#[test]
fn remote_four_tile_localization_preserves_source_placement_and_gdal_engine_parity() {
    let directory = TempDir::new().expect("multi-tile proof temp directory should be created");
    let _cache_env = CacheEnv::set(&directory.path().join("cache"));
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_ROOT);
    let source_flow_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_FLOW_DIR);
    let source_flow_acc = Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_FLOW_ACC);
    let source_dir_samples = read_source_i8(&source_flow_dir);
    let source_acc_samples = read_source_i32(&source_flow_acc);

    let pristine_dir_samples = pristine_i8(&source_dir_samples);
    let pristine_acc_samples = pristine_i32(&source_acc_samples);
    let probe_dir_samples = probe_i8();
    let probe_acc_samples = probe_i32();
    let pristine_dir_path = directory.path().join("pristine-flow-dir.tif");
    let pristine_acc_path = directory.path().join("pristine-flow-acc.tif");
    let probe_dir_path = directory.path().join("probe-flow-dir.tif");
    let probe_acc_path = directory.path().join("probe-flow-acc.tif");

    write_i8_raster(&pristine_dir_path, &pristine_dir_samples);
    write_i32_raster(&pristine_acc_path, &pristine_acc_samples);
    write_i8_raster(&probe_dir_path, &probe_dir_samples);
    write_i32_raster(&probe_acc_path, &probe_acc_samples);
    drop(validate_i8_raster(
        &pristine_dir_path,
        &pristine_dir_samples,
    ));
    drop(validate_i32_raster(
        &pristine_acc_path,
        &pristine_acc_samples,
    ));
    let reopened_probe_dir = validate_i8_raster(&probe_dir_path, &probe_dir_samples);
    let reopened_probe_acc = validate_i32_raster(&probe_acc_path, &probe_acc_samples);
    assert_distinct_blocks(&reopened_probe_dir);
    assert_distinct_blocks(&reopened_probe_acc);

    let manifest = std::fs::read(fixture_root.join("manifest.json"))
        .expect("projected manifest should be readable");
    let graph = std::fs::read(fixture_root.join("graph.parquet"))
        .expect("projected graph should be readable");
    let catchments = std::fs::read(fixture_root.join("catchments.parquet"))
        .expect("projected catchments should be readable");
    let pristine_dir_bytes =
        std::fs::read(&pristine_dir_path).expect("pristine FlowDir should be readable");
    let pristine_acc_bytes =
        std::fs::read(&pristine_acc_path).expect("pristine FlowAcc should be readable");
    let probe_dir_bytes = std::fs::read(&probe_dir_path).expect("probe FlowDir should be readable");
    let probe_acc_bytes = std::fs::read(&probe_acc_path).expect("probe FlowAcc should be readable");
    let mut probe_manifest: serde_json::Value =
        serde_json::from_slice(&manifest).expect("projected manifest JSON should parse");
    assert_eq!(
        probe_manifest["fabric_name"],
        serde_json::Value::String("conformance-tiny-projected-grass".to_string())
    );
    probe_manifest["fabric_name"] = serde_json::Value::String(PROBE_FABRIC_NAME.to_string());
    let probe_manifest =
        serde_json::to_vec(&probe_manifest).expect("probe manifest should serialize");

    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let runtime = tokio::runtime::Runtime::new().expect("staging runtime should be created");
    runtime.block_on(async {
        for (root, staged_manifest, flow_dir, flow_acc) in [
            (
                REMOTE_ROOT,
                manifest.as_slice(),
                pristine_dir_bytes.as_slice(),
                pristine_acc_bytes.as_slice(),
            ),
            (
                PROBE_ROOT,
                probe_manifest.as_slice(),
                probe_dir_bytes.as_slice(),
                probe_acc_bytes.as_slice(),
            ),
        ] {
            put_staged(&store, root, "manifest.json", staged_manifest).await;
            put_staged(&store, root, "graph.parquet", &graph).await;
            put_staged(&store, root, "catchments.parquet", &catchments).await;
            put_staged(&store, root, "aux/d8/projected/flow_dir.tif", flow_dir).await;
            put_staged(&store, root, "aux/d8/projected/flow_acc.tif", flow_acc).await;
        }
    });
    drop(runtime);

    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_GOLDEN);
    let golden: ProjectedGrassGolden = serde_json::from_str(
        &std::fs::read_to_string(golden_path).expect("projected GRASS golden should be readable"),
    )
    .expect("projected GRASS golden should match the proof schema");
    let probe_root = ObjectPath::from(PROBE_ROOT);
    let probe_url = Url::parse(PROBE_URL).expect("probe URL should parse");
    let probe_session =
        DatasetSession::open_remote_with_store(store.clone(), &probe_root, &probe_url)
            .expect("placement-probe remote session should open");
    let geographic_terminal = terminal_polygon(&probe_session, golden.terminal_id);
    let (handle, _) = probe_session
        .select_d8_raster_for_terminal(&geographic_terminal)
        .expect("declared projected GRASS rasters should cover the terminal");
    assert_eq!(handle.flow_dir_encoding(), FlowDirEncoding::Grass);
    let full_bbox = Rect::new(
        geo::coord! { x: 0.0, y: -768000.0 },
        geo::coord! { x: 1024000.0, y: 256000.0 },
    );
    let localized_dir = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, full_bbox)
        .expect("four-tile FlowDir window should localize");
    assert_remote_window_coverage(
        &localized_dir,
        (0, 0),
        &[0, 1, 2, 3],
        CrossedTileAxes::XAndY,
    );
    let cached_localized_dir = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, full_bbox)
        .expect("cached four-tile FlowDir window should localize");
    assert_eq!(cached_localized_dir.path(), localized_dir.path());
    assert_eq!(cached_localized_dir.header_bytes(), 0);
    assert_eq!(cached_localized_dir.tile_bytes(), 0);
    assert_eq!(cached_localized_dir.tile_count(), 0);
    assert_eq!(cached_localized_dir.window_pixels(), 0);
    assert_remote_window_coverage(
        &cached_localized_dir,
        (0, 0),
        &[0, 1, 2, 3],
        CrossedTileAxes::XAndY,
    );
    let localized_acc = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, full_bbox)
        .expect("four-tile FlowAcc window should localize");
    assert_remote_window_coverage(
        &localized_acc,
        (0, 0),
        &[0, 1, 2, 3],
        CrossedTileAxes::XAndY,
    );
    assert_eq!(localized_dir.tile_count(), 4);
    assert_eq!(localized_acc.tile_count(), 4);
    assert_eq!(localized_dir.window_pixels(), 1_048_576);
    assert_eq!(localized_acc.window_pixels(), 1_048_576);
    assert_eq!(localized_nodata(localized_dir.path()), "128");
    assert_eq!(localized_nodata(localized_acc.path()), "nan");

    // One-pixel padding makes this x-seam request a (510, 1, 4, 4) pixel window.
    let x_only_bbox = Rect::new(
        geo::coord! { x: 511000.0, y: 252000.0 },
        geo::coord! { x: 513000.0, y: 254000.0 },
    );
    let x_only_dir = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, x_only_bbox)
        .expect("x-only FlowDir window should localize");
    let x_only_acc = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, x_only_bbox)
        .expect("x-only FlowAcc window should localize");
    assert_remote_window_coverage(&x_only_dir, (510, 1), &[0, 1], CrossedTileAxes::X);
    assert_remote_window_coverage(&x_only_acc, (510, 1), &[0, 1], CrossedTileAxes::X);

    // One-pixel padding makes this y-seam request a (0, 510, 4, 4) pixel window.
    let y_only_bbox = Rect::new(
        geo::coord! { x: 1000.0, y: -257000.0 },
        geo::coord! { x: 3000.0, y: -255000.0 },
    );
    let y_only_dir = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, y_only_bbox)
        .expect("y-only FlowDir window should localize");
    let y_only_acc = probe_session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, y_only_bbox)
        .expect("y-only FlowAcc window should localize");
    assert_remote_window_coverage(&y_only_dir, (0, 510), &[0, 2], CrossedTileAxes::Y);
    assert_remote_window_coverage(&y_only_acc, (0, 510), &[0, 2], CrossedTileAxes::Y);

    let local = LocalTiffRasterSource;
    let gdal = GdalRasterSource::new();
    let local_dir = local
        .load_flow_direction(
            &localized_dir.path().to_string_lossy(),
            &full_bbox,
            handle.flow_dir_encoding(),
        )
        .expect("local reader should decode localized FlowDir");
    let gdal_dir = gdal
        .load_flow_direction(
            &localized_dir.path().to_string_lossy(),
            &full_bbox,
            handle.flow_dir_encoding(),
        )
        .expect("GDAL reader should decode localized FlowDir");
    assert_eq!(local_dir.dims(), GridDims::new(RASTER_SIDE, RASTER_SIDE));
    assert_eq!(gdal_dir.dims(), GridDims::new(RASTER_SIDE, RASTER_SIDE));
    assert_eq!(local_dir.inner().nodata(), 128);
    assert_eq!(gdal_dir.inner().nodata(), 128);
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            let bx = x / TILE_SIDE;
            let by = y / TILE_SIDE;
            let b = 2 * by + bx;
            let u = x % TILE_SIDE;
            let v = y % TILE_SIDE;
            let s = y * RASTER_SIDE + x;
            let i = (TILE_SIDE * by + v) * RASTER_SIDE + (TILE_SIDE * bx + u);
            let expected = probe_dir_samples[s] as u8;
            assert_eq!(
                local_dir.inner().data()[i],
                expected,
                "local FlowDir mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
            );
            assert_eq!(
                gdal_dir.inner().data()[i],
                expected,
                "GDAL FlowDir mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
            );
        }
    }
    assert_eq!(local_dir.inner().data(), gdal_dir.inner().data());
    assert_eq!(local_dir.geo(), gdal_dir.geo());
    assert_eq!(local_dir.geo().origin_x(), 0.0);
    assert_eq!(local_dir.geo().origin_y(), 256000.0);
    assert_eq!(local_dir.geo().pixel_width(), 1000.0);
    assert_eq!(local_dir.geo().pixel_height(), -1000.0);
    assert_eq!(gdal_dir.geo().origin_x(), 0.0);
    assert_eq!(gdal_dir.geo().origin_y(), 256000.0);
    assert_eq!(gdal_dir.geo().pixel_width(), 1000.0);
    assert_eq!(gdal_dir.geo().pixel_height(), -1000.0);

    let local_acc = local
        .load_accumulation(&localized_acc.path().to_string_lossy(), &full_bbox)
        .expect("local reader should decode localized FlowAcc");
    let gdal_acc = gdal
        .load_accumulation(&localized_acc.path().to_string_lossy(), &full_bbox)
        .expect("GDAL reader should decode localized FlowAcc");
    assert_eq!(local_acc.dims(), GridDims::new(RASTER_SIDE, RASTER_SIDE));
    assert_eq!(gdal_acc.dims(), GridDims::new(RASTER_SIDE, RASTER_SIDE));
    assert!(local_acc.inner().nodata().is_nan());
    assert!(gdal_acc.inner().nodata().is_nan());
    let expected_nan_indices = std::collections::BTreeSet::from([0, 512, 524288, 524800]);
    let local_nan_indices = local_acc
        .inner()
        .data()
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.is_nan().then_some(index))
        .collect::<std::collections::BTreeSet<_>>();
    let gdal_nan_indices = gdal_acc
        .inner()
        .data()
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.is_nan().then_some(index))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(local_nan_indices, expected_nan_indices);
    assert_eq!(gdal_nan_indices, expected_nan_indices);
    for y in 0..RASTER_SIDE {
        for x in 0..RASTER_SIDE {
            let bx = x / TILE_SIDE;
            let by = y / TILE_SIDE;
            let b = 2 * by + bx;
            let u = x % TILE_SIDE;
            let v = y % TILE_SIDE;
            let s = y * RASTER_SIDE + x;
            let i = (TILE_SIDE * by + v) * RASTER_SIDE + (TILE_SIDE * bx + u);
            let raw = probe_acc_samples[s];
            if raw == -2_147_483_648 {
                assert!(
                    local_acc.inner().data()[i].is_nan(),
                    "local FlowAcc nodata mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
                );
                assert!(
                    gdal_acc.inner().data()[i].is_nan(),
                    "GDAL FlowAcc nodata mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
                );
            } else {
                let expected = raw as f32;
                assert_eq!(
                    local_acc.inner().data()[i].to_bits(),
                    expected.to_bits(),
                    "local FlowAcc mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
                );
                assert_eq!(
                    gdal_acc.inner().data()[i].to_bits(),
                    expected.to_bits(),
                    "GDAL FlowAcc mismatch x={x} y={y} b={b} u={u} v={v} s={s} i={i}"
                );
            }
        }
    }
    assert_f32_tiles_equal(local_acc.inner().data(), gdal_acc.inner().data());
    assert_eq!(local_acc.geo(), gdal_acc.geo());
    assert_eq!(local_acc.geo().origin_x(), 0.0);
    assert_eq!(local_acc.geo().origin_y(), 256000.0);
    assert_eq!(local_acc.geo().pixel_width(), 1000.0);
    assert_eq!(local_acc.geo().pixel_height(), -1000.0);
    assert_eq!(gdal_acc.geo().origin_x(), 0.0);
    assert_eq!(gdal_acc.geo().origin_y(), 256000.0);
    assert_eq!(gdal_acc.geo().pixel_width(), 1000.0);
    assert_eq!(gdal_acc.geo().pixel_height(), -1000.0);

    let pristine_root = ObjectPath::from(REMOTE_ROOT);
    let pristine_url = Url::parse(REMOTE_URL).expect("pristine URL should parse");
    let session = DatasetSession::open_remote_with_store(store, &pristine_root, &pristine_url)
        .expect("pristine remote session should open");
    let engine = Engine::builder(session)
        .with_raster_source(GdalRasterSource::new())
        .build();
    let options = DelineationOptions::default()
        .with_resolver_config(
            ResolverConfig::new().with_search_radius(
                SearchRadiusMetres::new(1_000.0)
                    .expect("projected fixture search radius should be valid"),
            ),
        )
        .with_snap_threshold(SnapThreshold::new(500))
        .with_refinement_mode(RefinementMode::RequireD8);
    let input_outlet = GeoCoord::new(golden.input_outlet.lon, golden.input_outlet.lat);
    let selected = engine
        .select_level(LevelSelection::Finest)
        .expect("projected fixture finest level should resolve");
    let resolved = engine
        .resolve_outlet_at_level(input_outlet, selected, options.resolver_config())
        .expect("projected fixture outlet should resolve");
    let upstream = engine
        .traverse_upstream_at_level(&resolved)
        .expect("projected same-level traversal should succeed");
    let units = engine
        .produce_pre_merge_units(&upstream)
        .expect("projected pre-merge units should materialize");
    let refinement = engine
        .refine_terminal_placeholder(&resolved, &units, &options)
        .expect("required projected D8 refinement should complete");
    assert!(
        matches!(&refinement, TerminalRefinement::Applied { .. }),
        "required D8 refinement must be applied, got {refinement:?}"
    );
    let dissolved = engine
        .dissolve_watershed(&units, &refinement, &options)
        .expect("refined projected watershed should dissolve");
    let result = engine.compose_result(resolved, upstream, &units, refinement, dissolved);
    let actual_wkb = canonical_wkb_multi_polygon(result.geometry())
        .expect("remote GDAL engine geometry should canonicalize");
    assert_eq!(actual_wkb, decode_hex(&golden.canonical_wkb_hex));
}

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
    let local = LocalTiffRasterSource;
    let gdal = GdalRasterSource::new();

    let local_fd = local
        .load_flow_direction(
            &flow_dir_path.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Grass,
        )
        .expect("local TIFF source should decode int8 flow direction");
    let gdal_fd = gdal
        .load_flow_direction(
            &flow_dir_path.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Grass,
        )
        .expect("GDAL source should decode int8 flow direction");
    assert_eq!(local_fd.dims(), GridDims::new(2, 2));
    assert_eq!(gdal_fd.dims(), GridDims::new(2, 2));
    assert_eq!(local_fd.inner().data(), &[1_u8, 254, 8, 0]);
    assert_eq!(local_fd.inner().data(), gdal_fd.inner().data());
    assert_eq!(local_fd.inner().nodata(), gdal_fd.inner().nodata());
    assert_eq!(local_fd.inner().nodata(), 255);
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

#[test]
fn directional_uint8_nodata_is_rejected_by_local_and_gdal_sources() {
    let directory = tempfile::tempdir().expect("UInt8 TIFF temp directory should be created");
    let path = directory.path().join("flow_dir_uint8_nodata_one.tif");
    let driver = DriverManager::get_driver_by_name("GTiff").expect("GTiff driver should exist");
    let mut dataset = driver
        .create_with_band_type::<u8, _>(&path, 2, 2, 1)
        .expect("UInt8 flow-direction TIFF should be created");
    dataset
        .set_geo_transform(&[0.0, 1.0, 0.0, 2.0, 0.0, -1.0])
        .expect("flow-direction geotransform should be written");
    {
        let mut band = dataset
            .rasterband(1)
            .expect("flow-direction band should exist");
        band.set_no_data_value(Some(1.0))
            .expect("flow-direction nodata should be written");
        let mut buffer = Buffer::new((2, 2), vec![1_u8, 2, 4, 8]);
        band.write((0, 0), (2, 2), &mut buffer)
            .expect("flow-direction samples should be written");
    }
    drop(dataset);

    let bbox = Rect::new(
        geo::coord! { x: 0.0, y: 0.0 },
        geo::coord! { x: 2.0, y: 2.0 },
    );
    let local_err = LocalTiffRasterSource
        .load_flow_direction(&path.to_string_lossy(), &bbox, FlowDirEncoding::Esri)
        .expect_err("local reader must reject directional header nodata");
    let gdal_err = GdalRasterSource::new()
        .load_flow_direction(&path.to_string_lossy(), &bbox, FlowDirEncoding::Esri)
        .expect_err("GDAL reader must reject directional header nodata");

    assert!(matches!(
        local_err,
        RasterSourceError::InvalidFlowDirectionNodata {
            nodata: 1,
            encoding: FlowDirEncoding::Esri,
        }
    ));
    assert!(matches!(
        gdal_err,
        RasterSourceError::InvalidFlowDirectionNodata {
            nodata: 1,
            encoding: FlowDirEncoding::Esri,
        }
    ));
}

#[test]
fn projected_grass_declaration_drives_gdal_and_changes_geometry() {
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_ROOT);
    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(PROJECTED_GRASS_GOLDEN);
    let golden: ProjectedGrassGolden = serde_json::from_str(
        &std::fs::read_to_string(golden_path).expect("projected GRASS golden should be readable"),
    )
    .expect("projected GRASS golden should match the proof schema");
    let input_outlet = GeoCoord::new(golden.input_outlet.lon, golden.input_outlet.lat);
    let options = DelineationOptions::default()
        .with_resolver_config(
            ResolverConfig::new().with_search_radius(
                SearchRadiusMetres::new(1_000.0)
                    .expect("projected fixture search radius should be valid"),
            ),
        )
        .with_snap_threshold(SnapThreshold::new(500));

    let engine_session =
        DatasetSession::open_path(&fixture_root).expect("projected GRASS fixture should open");
    let engine = Engine::builder(engine_session)
        .with_raster_source(GdalRasterSource::new())
        .build();
    let engine_result = engine
        .delineate(input_outlet, &options)
        .expect("declared-GRASS GDAL delineation should succeed");
    let engine_wkb = canonical_wkb_multi_polygon(engine_result.geometry())
        .expect("declared-GRASS engine geometry should canonicalize");

    assert_eq!(
        engine_wkb,
        decode_hex(&golden.canonical_wkb_hex),
        "Engine with a zero-argument GDAL source must reproduce the immutable declared-GRASS golden bytes"
    );

    let direct_session =
        DatasetSession::open_path(&fixture_root).expect("projected GRASS fixture should reopen");
    let geographic_terminal = terminal_polygon(&direct_session, golden.terminal_id);
    let (handle, native_terminal) = direct_session
        .select_d8_raster_for_terminal(&geographic_terminal)
        .expect("projected GRASS declaration should cover the terminal");
    let native_bbox = native_terminal
        .bounding_rect()
        .expect("projected terminal should have native bounds");
    let flow_dir = direct_session
        .localize_d8_raster_window(&handle, RasterKind::FlowDir, native_bbox)
        .expect("projected flow direction should localize");
    let flow_acc = direct_session
        .localize_d8_raster_window(&handle, RasterKind::FlowAcc, native_bbox)
        .expect("projected flow accumulation should localize");
    let native_outlet = forward(
        Crs::Epsg8857,
        GeoCoord::new(golden.resolved_outlet.lon, golden.resolved_outlet.lat),
    );
    let source = GdalRasterSource::new();
    let grass = refine_terminal_from_source(
        &source,
        &flow_dir.path().to_string_lossy(),
        &flow_acc.path().to_string_lossy(),
        &native_terminal,
        native_outlet,
        SnapThreshold::new(500),
        FlowAccumulationUnits::Km2,
        8857_u32,
        FlowDirEncoding::Grass,
    )
    .expect("GRASS counterfactual carve should succeed");
    let esri_err = refine_terminal_from_source(
        &source,
        &flow_dir.path().to_string_lossy(),
        &flow_acc.path().to_string_lossy(),
        &native_terminal,
        native_outlet,
        SnapThreshold::new(500),
        FlowAccumulationUnits::Km2,
        8857_u32,
        FlowDirEncoding::Esri,
    )
    .expect_err("an ESRI declaration over a byte-128 header nodata must be rejected");
    assert!(matches!(
        esri_err,
        RefinementError::RasterLoad {
            source: RasterSourceError::InvalidFlowDirectionNodata {
                nodata: 128,
                encoding: FlowDirEncoding::Esri,
            },
        }
    ));
    let grass_wkb = canonical_wkb_multi_polygon(grass.polygon())
        .expect("native GRASS carve should canonicalize");
    let grass_cells = (grass.polygon().unsigned_area() / 1_000_000.0).round();
    assert!(
        grass_cells >= 100.0,
        "the declared GRASS carve must remain substantial; got {grass_cells}"
    );

    let differential_flow_dir = fixture_root.join("aux/d8/projected/flow_dir_nodata_minus_one.tif");
    let differential_grass = refine_terminal_from_source(
        &source,
        &differential_flow_dir.to_string_lossy(),
        &flow_acc.path().to_string_lossy(),
        &native_terminal,
        native_outlet,
        SnapThreshold::new(500),
        FlowAccumulationUnits::Km2,
        8857_u32,
        FlowDirEncoding::Grass,
    )
    .expect("the nodata-minus-one GRASS differential carve should succeed");
    let differential_esri = refine_terminal_from_source(
        &source,
        &differential_flow_dir.to_string_lossy(),
        &flow_acc.path().to_string_lossy(),
        &native_terminal,
        native_outlet,
        SnapThreshold::new(500),
        FlowAccumulationUnits::Km2,
        8857_u32,
        FlowDirEncoding::Esri,
    )
    .expect("the nodata-minus-one ESRI differential carve should succeed");
    let differential_grass_wkb = canonical_wkb_multi_polygon(differential_grass.polygon())
        .expect("differential GRASS carve should canonicalize");
    let differential_esri_wkb = canonical_wkb_multi_polygon(differential_esri.polygon())
        .expect("differential ESRI carve should canonicalize");
    let differential_grass_cells =
        (differential_grass.polygon().unsigned_area() / 1_000_000.0).round();
    let differential_esri_cells =
        (differential_esri.polygon().unsigned_area() / 1_000_000.0).round();

    assert_eq!(
        differential_grass_wkb, grass_wkb,
        "changing only non-directional nodata metadata must preserve the GRASS geometry"
    );
    assert_ne!(
        differential_grass_wkb, differential_esri_wkb,
        "identical accepted bytes decoded under GRASS and ESRI must produce different canonical geometry"
    );
    assert!(
        differential_esri_cells >= 1.0,
        "the ESRI differential must polygonize at least its snapped seed cell; got {differential_esri_cells}"
    );
    assert!(
        differential_grass_cells >= differential_esri_cells * 100.0,
        "the GRASS differential must be at least two orders of magnitude larger: grass={differential_grass_cells}, esri={differential_esri_cells}"
    );
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
        .load_flow_direction(
            &flow_dir_path.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Esri,
        )
        .expect("local TIFF source should decode flow_dir");
    let gdal_fd = gdal
        .load_flow_direction(
            &flow_dir_path.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Esri,
        )
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
        .load_flow_direction(
            &pair.flow_dir.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Esri,
        )
        .expect("local TIFF source should decode MERIT flow_dir window");
    let gdal_fd = gdal
        .load_flow_direction(
            &pair.flow_dir.to_string_lossy(),
            &bbox,
            FlowDirEncoding::Esri,
        )
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
            RasterOutlet::UnitOnly(NativeCoord::from(record.resolved_outlet)),
            SnapThreshold::DEFAULT,
            FlowAccumulationUnits::Cells,
            4326_u32,
            FlowDirEncoding::Esri,
        );
        let gdal_result = refine_terminal_from_source(
            gdal,
            &pair.flow_dir.to_string_lossy(),
            &pair.flow_acc.to_string_lossy(),
            terminal_polygon,
            RasterOutlet::UnitOnly(NativeCoord::from(record.resolved_outlet)),
            SnapThreshold::DEFAULT,
            FlowAccumulationUnits::Cells,
            4326_u32,
            FlowDirEncoding::Esri,
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
