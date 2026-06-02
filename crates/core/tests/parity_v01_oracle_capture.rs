//! Capture-time v0.1 parity oracle tests.

use geo::{Area, BoundingRect, Rect};
use shed_core::algo::{GeoCoord, SnapThreshold};
use shed_core::session::DatasetSession;
use shed_core::test_raster_source::LocalTiffRasterSource;
use shed_core::{DelineationOptions, Engine, RefinementOutcome};

const FIXTURE_ROOT: &str = "tests/fixtures/parity/v01_synthetic_refined";
const TERMINAL_AREA: f64 = 25.0;

#[test]
fn synthetic_fixture_smoke() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let session = DatasetSession::open_path(&root).expect("v0.1 synthetic refined fixture opens");
    let engine = Engine::builder(session)
        .with_raster_source(LocalTiffRasterSource)
        .build();

    let result = engine
        .delineate(
            GeoCoord::new(2.5, -2.5),
            &DelineationOptions::default().with_snap_threshold(SnapThreshold::new(500)),
        )
        .expect("synthetic refined fixture should delineate");

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

fn rect_contains_rect(outer: &Rect<f64>, inner: &Rect<f64>) -> bool {
    inner.min().x >= outer.min().x
        && inner.max().x <= outer.max().x
        && inner.min().y >= outer.min().y
        && inner.max().y <= outer.max().y
}
