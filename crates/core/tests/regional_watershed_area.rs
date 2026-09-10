//! Regional area regressions on captured and synthetic WGS84 polygons.
use geo::{GeodesicArea, LineString, MultiPolygon, Polygon};
use hfx::WkbGeometry;
use pourpoint_core::algo::watershed_area::WatershedAreaError;
use pourpoint_core::algo::{decode_wkb_multi_polygon, geodesic_area, geodesic_area_multi};

#[test]
fn captured_tiny_components_do_not_contribute_earth_complements() {
    let bytes = include_bytes!("fixtures/regional-area/tiny-components.wkb");
    let geometry = decode_wkb_multi_polygon(&WkbGeometry::new(bytes.to_vec()).unwrap()).unwrap();
    assert_eq!(geometry.0.len(), 2);
    for polygon in &geometry.0 {
        let signed = polygon.geodesic_area_signed();
        let unsigned = polygon.geodesic_area_unsigned();
        assert!(signed < 0.0 && signed.abs() < 0.001, "signed={signed}");
        assert!(unsigned > 5e14, "unsigned={unsigned}");
        let area = geodesic_area(polygon).unwrap().as_f64();
        assert!(
            (0.0..1e-9).contains(&area),
            "regional km2={area}; signed m2={signed}; unsigned m2={unsigned}"
        );
    }
    let area = geodesic_area_multi(&geometry).unwrap().as_f64();
    assert!((0.0..1e-9).contains(&area), "regional km2={area}");
}

fn rect(w: f64, s: f64, e: f64, n: f64) -> Polygon<f64> {
    Polygon::new(
        LineString::from(vec![(w, s), (e, s), (e, n), (w, n), (w, s)]),
        vec![],
    )
}
fn reverse(p: &Polygon<f64>) -> Polygon<f64> {
    Polygon::new(
        LineString::new(p.exterior().0.iter().copied().rev().collect()),
        p.interiors().to_vec(),
    )
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-7,
        "actual={actual} expected={expected}"
    );
}
#[test]
fn winding_independent_shells_and_holes() {
    let shell = rect(0., 0., 1., 1.);
    let hole = rect(0.25, 0.25, 0.75, 0.75);
    // Independent GeographicLib/pyproj WGS84 reference, tolerance 0.1 m².
    for exterior in [shell.clone(), reverse(&shell)] {
        close(
            geodesic_area(&exterior).unwrap().as_f64(),
            12308.778361469452,
        );
        for interior in [hole.clone(), reverse(&hole)] {
            let polygon = Polygon::new(
                exterior.exterior().clone(),
                vec![interior.exterior().clone()],
            );
            let area = geodesic_area(&polygon).unwrap().as_f64();
            assert!(area > 0.0);
            close(area, 9231.61422481486);
            close(
                geodesic_area_multi(&MultiPolygon(vec![polygon]))
                    .unwrap()
                    .as_f64(),
                area,
            );
        }
    }
    let mixed = MultiPolygon(vec![shell, reverse(&rect(10., 0., 11., 1.))]);
    close(
        geodesic_area_multi(&mixed).unwrap().as_f64(),
        24617.556722938905,
    );
}
#[test]
fn short_edge_antimeridian_is_regional() {
    // Only the area helper is certified, not planar assembly or snapping.
    let polygon = rect(179., 0., -179., 1.);
    for p in [polygon.clone(), reverse(&polygon)] {
        close(geodesic_area(&p).unwrap().as_f64(), 24619.443759277205);
    }
}
#[test]
fn holes_exceeding_shell_are_not_made_positive() {
    let shell = rect(0., 0., 1., 1.);
    let hole = rect(-1., -1., 2., 2.);
    for p in [shell.clone(), reverse(&shell), Polygon::empty()] {
        let invalid = Polygon::new(p.exterior().clone(), vec![hole.exterior().clone()]);
        assert!(
            matches!(geodesic_area(&invalid), Err(WatershedAreaError::HoleAreaExceedsShell { shell_m2, holes_m2 }) if holes_m2 > shell_m2)
        );
        assert!(matches!(
            geodesic_area_multi(&MultiPolygon(vec![invalid])),
            Err(WatershedAreaError::HoleAreaExceedsShell { .. })
        ));
    }
}
#[test]
fn empty_and_nonfinite_behavior() {
    assert_eq!(
        geodesic_area_multi(&MultiPolygon(vec![])),
        Err(WatershedAreaError::EmptyGeometry)
    );
    // Retain the single empty polygon's zero-area behavior.
    close(geodesic_area(&Polygon::empty()).unwrap().as_f64(), 0.);
    close(
        geodesic_area_multi(&MultiPolygon(vec![Polygon::empty()]))
            .unwrap()
            .as_f64(),
        0.,
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for p in [
            rect(value, 0., 1., 1.),
            Polygon::new(
                rect(0., 0., 1., 1.).exterior().clone(),
                vec![rect(0.2, value, 0.8, 0.8).exterior().clone()],
            ),
        ] {
            assert!(matches!(
                geodesic_area(&p),
                Err(WatershedAreaError::NonFiniteArea { .. })
            ));
            assert!(matches!(
                geodesic_area_multi(&MultiPolygon(vec![p])),
                Err(WatershedAreaError::NonFiniteArea { .. })
            ));
        }
    }
}

#[test]
#[ignore = "requires explicit paths to retained operational GRIT and TDX WKB, not redistributed"]
fn captured_complete_watersheds() {
    for (key, expected, parts) in [
        ("POURPOINT_GRIT_WKB", 35903.3077027896, 9),
        ("POURPOINT_TDX_WKB", 35571.42020080588, 7),
    ] {
        let path = std::env::var(key).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let geometry = decode_wkb_multi_polygon(&WkbGeometry::new(bytes).unwrap()).unwrap();
        let unchanged = geometry.clone();
        assert_eq!(geometry.0.len(), parts);
        assert!(geometry.0.iter().all(|p| p.interiors().is_empty()));
        let actual = geodesic_area_multi(&geometry).unwrap().as_f64();
        println!("{key}: parts={parts}, area_km2={actual:.14}");
        close(actual, expected);
        assert_eq!(geometry, unchanged);
    }
}
