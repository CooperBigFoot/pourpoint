//! Isolated GDAL parity proof for committed raster fixtures.

use geo::Rect;
use shed_core::algo::RasterSource;
use shed_core::test_raster_source::LocalTiffRasterSource;
use shed_gdal::GdalRasterSource;

const FIXTURE_ROOT: &str = "../core/tests/fixtures/parity/v01_synthetic_refined";

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

fn assert_f32_tiles_equal(left: &[f32], right: &[f32]) {
    assert_eq!(left.len(), right.len());
    for (idx, (&a, &b)) in left.iter().zip(right).enumerate() {
        assert!(
            (a.is_nan() && b.is_nan()) || a == b,
            "f32 tile mismatch at {idx}: local={a:?} gdal={b:?}"
        );
    }
}
