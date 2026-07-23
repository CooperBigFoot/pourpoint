use std::f64::consts::{FRAC_PI_2, PI};

use pourpoint_core::algo::{
    Crs, GeoCoord, InverseStage, NativeCoord, ProjectionError, forward, inverse,
};
use serde::Deserialize;

const A: f64 = 6_378_137.0;
const RECIPROCAL_FLATTENING: f64 = 298.257_223_563;
const A1: f64 = 1.340_264;
const A2: f64 = -0.081_106;
const A3: f64 = 0.000_893;
const A4: f64 = 0.003_796;

#[derive(Deserialize)]
struct Oracle {
    forward_cases: Vec<PositiveCase>,
    inverse_cases: Vec<PositiveCase>,
    grit_grid: GritGrid,
    reference: Reference,
}

#[derive(Deserialize)]
struct PositiveCase {
    name: String,
    geographic_lon_lat_degrees: [f64; 2],
    projected_xy_metres: [f64; 2],
}

#[derive(Deserialize)]
struct GritGrid {
    negative_inverse_cases: Vec<NegativeInverseCase>,
}

#[derive(Deserialize)]
struct NegativeInverseCase {
    name: String,
    projected_xy_metres: [f64; 2],
    expected: String,
}

#[derive(Deserialize)]
struct Reference {
    a1: f64,
    a2: f64,
    a3: f64,
    a4: f64,
    a_metres: f64,
    authalic_radius_metres: f64,
    eccentricity: f64,
    eccentricity_squared: f64,
    flattening: f64,
    m_expression: String,
    polar_authalic_quantity: f64,
    reciprocal_flattening: f64,
    theta_max_radians: f64,
    y_max_metres: f64,
}

fn oracle() -> Oracle {
    serde_json::from_str(include_str!(
        "fixtures/projection/equal_earth_proj_oracle.json"
    ))
    .expect("the committed Equal Earth oracle must deserialize")
}

fn ellipsoid() -> (f64, f64) {
    let flattening = 1.0 / RECIPROCAL_FLATTENING;
    let eccentricity_squared = flattening * (2.0 - flattening);
    (eccentricity_squared, eccentricity_squared.sqrt())
}

fn m() -> f64 {
    3.0_f64.sqrt() / 2.0
}

fn authalic_q(phi: f64) -> f64 {
    let (e2, e) = ellipsoid();
    let s = phi.sin();
    let den = 1.0 - e2 * s * s;
    (1.0 - e2) * (s / den + (e * s).atanh() / e)
}

fn derived_constants() -> (f64, f64, f64) {
    let q_p = authalic_q(FRAC_PI_2);
    let rq = A * (q_p / 2.0).sqrt();
    let theta_max = PI / 3.0;
    let theta2 = theta_max * theta_max;
    let theta6 = theta2 * theta2 * theta2;
    let y_max = rq * theta_max * (A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2));
    (q_p, rq, y_max)
}

fn recover_theta(y: f64, rq: f64) -> f64 {
    let mut theta = y / (rq * A1);
    for _ in 0..12 {
        let theta2 = theta * theta;
        let theta6 = theta2 * theta2 * theta2;
        let polynomial = A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2);
        let derivative = A1 + 3.0 * A2 * theta2 + theta6 * (7.0 * A3 + 9.0 * A4 * theta2);
        let update = (theta * polynomial - y / rq) / derivative;
        theta -= update;
        if update.abs() <= 1e-14 {
            return theta;
        }
    }
    panic!("test reference theta recovery did not converge");
}

fn longitude_difference_degrees(actual: f64, expected: f64) -> f64 {
    let direct = (actual - expected).abs();
    direct
        .min((direct - 360.0).abs())
        .min((direct + 360.0).abs())
}

fn latitude_tolerance(name: &str) -> f64 {
    match name {
        "equator" | "antimeridian_east" | "antimeridian_west" => 1e-12,
        "axis_order_control" | "north_15" | "south_15" => 1.5e-8,
        "north_45" | "south_45" => 5e-9,
        "north_60" | "south_60" => 2e-9,
        "north_89" | "south_89" => 1e-10,
        "north_89_99" | "south_89_99" => 5e-10,
        unknown => panic!("unexpected inverse oracle row {unknown}"),
    }
}

fn next_up(value: f64) -> f64 {
    f64::from_bits(value.to_bits() + 1)
}

#[test]
fn reference_constants_match_normative_binary64_derivation() {
    let oracle = oracle();
    let reference = oracle.reference;
    let flattening = 1.0 / RECIPROCAL_FLATTENING;
    let (e2, e) = ellipsoid();
    let (q_p, rq, y_max) = derived_constants();

    assert_eq!(reference.a1, A1);
    assert_eq!(reference.a2, A2);
    assert_eq!(reference.a3, A3);
    assert_eq!(reference.a4, A4);
    assert_eq!(reference.a_metres, A);
    assert_eq!(reference.reciprocal_flattening, RECIPROCAL_FLATTENING);
    assert_eq!(reference.m_expression, "sqrt(3)/2");
    assert_eq!(reference.flattening, flattening);
    assert_eq!(reference.eccentricity_squared, e2);
    assert_eq!(reference.eccentricity, e);
    assert_eq!(reference.polar_authalic_quantity, q_p);
    assert_eq!(reference.authalic_radius_metres, rq);
    assert_eq!(reference.theta_max_radians, PI / 3.0);
    assert_eq!(reference.y_max_metres, y_max);
}

#[test]
fn identity_and_crs_parsing_are_typed() {
    assert_eq!(Crs::try_from(4326), Ok(Crs::Epsg4326));
    assert_eq!(Crs::try_from(8857), Ok(Crs::Epsg8857));
    assert_eq!(
        Crs::try_from(3857),
        Err(ProjectionError::UnsupportedCrs { epsg: 3857 })
    );

    let case = oracle()
        .forward_cases
        .into_iter()
        .find(|case| case.name == "axis_order_control")
        .expect("axis-order control row must exist");
    let geographic = GeoCoord::new(
        case.geographic_lon_lat_degrees[0],
        case.geographic_lon_lat_degrees[1],
    );
    let native = forward(Crs::Epsg4326, geographic);
    assert_eq!(native.x(), geographic.lon);
    assert_eq!(native.y(), geographic.lat);
    assert_eq!(
        inverse(Crs::Epsg4326, native),
        Ok(GeoCoord::new(geographic.lon, geographic.lat))
    );
}

#[test]
fn forward_matches_every_proj_oracle_row() {
    for case in oracle().forward_cases {
        let result = forward(
            Crs::Epsg8857,
            GeoCoord::new(
                case.geographic_lon_lat_degrees[0],
                case.geographic_lon_lat_degrees[1],
            ),
        );
        assert!(
            (result.x() - case.projected_xy_metres[0]).abs() <= 1e-6,
            "{} x: expected {}, got {}",
            case.name,
            case.projected_xy_metres[0],
            result.x()
        );
        assert!(
            (result.y() - case.projected_xy_metres[1]).abs() <= 1e-6,
            "{} y: expected {}, got {}",
            case.name,
            case.projected_xy_metres[1],
            result.y()
        );
    }
}

#[test]
fn inverse_matches_every_proj_oracle_row() {
    for case in oracle().inverse_cases {
        let result = inverse(
            Crs::Epsg8857,
            NativeCoord::new(case.projected_xy_metres[0], case.projected_xy_metres[1]),
        )
        .unwrap_or_else(|error| panic!("{} inverse failed: {error}", case.name));
        let longitude_error =
            longitude_difference_degrees(result.lon, case.geographic_lon_lat_degrees[0]);
        let latitude_error = (result.lat - case.geographic_lon_lat_degrees[1]).abs();
        assert!(
            longitude_error <= 1e-10,
            "{} longitude error: {longitude_error}",
            case.name
        );
        assert!(
            latitude_error <= latitude_tolerance(&case.name),
            "{} latitude error: {latitude_error}",
            case.name
        );
    }
}

#[test]
fn non_pole_forward_rows_are_near_exactly_self_consistent() {
    let (q_p, rq, _) = derived_constants();
    for case in oracle()
        .forward_cases
        .into_iter()
        .filter(|case| !matches!(case.name.as_str(), "north_pole" | "south_pole"))
    {
        let projected = forward(
            Crs::Epsg8857,
            GeoCoord::new(
                case.geographic_lon_lat_degrees[0],
                case.geographic_lon_lat_degrees[1],
            ),
        );
        let recovered = inverse(Crs::Epsg8857, projected)
            .unwrap_or_else(|error| panic!("{} inverse failed: {error}", case.name));
        let theta = recover_theta(projected.y(), rq);
        let beta = (theta.sin() / m()).asin();
        let residual = (authalic_q(recovered.lat.to_radians()) - q_p * beta.sin()).abs();
        assert!(residual <= 1e-15, "{} q residual: {residual}", case.name);

        let reprojected = forward(Crs::Epsg8857, recovered);
        assert!(
            (reprojected.x() - projected.x()).abs() <= 1e-6,
            "{} round-trip x",
            case.name
        );
        assert!(
            (reprojected.y() - projected.y()).abs() <= 1e-6,
            "{} round-trip y",
            case.name
        );
    }
}

#[test]
fn inverse_domain_and_canonical_poles_are_explicit() {
    let north = forward(Crs::Epsg8857, GeoCoord::new(0.0, 90.0));
    let south = forward(Crs::Epsg8857, GeoCoord::new(0.0, -90.0));
    assert_eq!(inverse(Crs::Epsg8857, north), Ok(GeoCoord::new(0.0, 90.0)));
    assert_eq!(inverse(Crs::Epsg8857, south), Ok(GeoCoord::new(0.0, -90.0)));

    let above_north = NativeCoord::new(0.0, next_up(north.y()));
    assert!(matches!(
        inverse(Crs::Epsg8857, above_north),
        Err(ProjectionError::OutOfDomain { .. })
    ));

    for coordinate in [
        NativeCoord::new(f64::NAN, 0.0),
        NativeCoord::new(f64::INFINITY, 0.0),
        NativeCoord::new(0.0, f64::NEG_INFINITY),
        NativeCoord::new(0.0, f64::NAN),
    ] {
        assert!(matches!(
            inverse(Crs::Epsg8857, coordinate),
            Err(ProjectionError::OutOfDomain { .. })
        ));
    }

    let negative_cases = oracle().grit_grid.negative_inverse_cases;
    let expected_names = ["top_left", "top_right", "bottom_left", "bottom_right"];
    assert_eq!(negative_cases.len(), expected_names.len());
    for expected_name in expected_names {
        let case = negative_cases
            .iter()
            .find(|case| case.name == expected_name)
            .unwrap_or_else(|| panic!("missing negative row {expected_name}"));
        assert_eq!(
            case.expected, "outside_projection_domain",
            "{} marker",
            case.name
        );
        assert!(matches!(
            inverse(
                Crs::Epsg8857,
                NativeCoord::new(case.projected_xy_metres[0], case.projected_xy_metres[1])
            ),
            Err(ProjectionError::OutOfDomain { .. })
        ));
    }
}

fn direct_spherical_forward(longitude_degrees: f64, latitude_degrees: f64) -> NativeCoord {
    let (_, rq, _) = derived_constants();
    let theta = (m() * latitude_degrees.to_radians().sin()).asin();
    let theta2 = theta * theta;
    let theta6 = theta2 * theta2 * theta2;
    let d = A1 + 3.0 * A2 * theta2 + theta6 * (7.0 * A3 + 9.0 * A4 * theta2);
    NativeCoord::new(
        rq * longitude_degrees.to_radians() * theta.cos() / (m() * d),
        rq * theta * (A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2)),
    )
}

#[test]
fn direct_spherical_geodetic_latitude_is_detectably_wrong() {
    let cases = oracle().forward_cases;
    for (name, diagnostic) in [
        ("north_15", 8_117.1),
        ("north_45", 13_670.7),
        ("north_60", 9_537.3),
    ] {
        let case = cases
            .iter()
            .find(|case| case.name == name)
            .unwrap_or_else(|| panic!("missing spherical regression row {name}"));
        let wrong = direct_spherical_forward(
            case.geographic_lon_lat_degrees[0],
            case.geographic_lon_lat_degrees[1],
        );
        let divergence = (wrong.y() - case.projected_xy_metres[1]).abs();
        assert!(divergence > 1_000.0, "{name}: {divergence}");
        assert!(
            (divergence - diagnostic).abs() <= 0.1,
            "{name}: expected about {diagnostic}, got {divergence}"
        );
    }

    let mut peak = (0.0_f64, 0.0_f64);
    for tenth_degree in 0..=900 {
        let latitude = f64::from(tenth_degree) / 10.0;
        let projected = forward(Crs::Epsg8857, GeoCoord::new(0.0, latitude));
        let (_, rq, _) = derived_constants();
        let theta = recover_theta(projected.y(), rq);
        let wrong_latitude = (theta.sin() / m()).asin().to_degrees();
        let divergence = (wrong_latitude - latitude).abs();
        if divergence > peak.1 {
            peak = (latitude, divergence);
        }
    }
    assert!((peak.0 - 45.1).abs() <= 0.1, "peak latitude: {}", peak.0);
    assert!(
        (peak.1 - 0.1283).abs() <= 0.0001,
        "peak divergence: {}",
        peak.1
    );
}

#[test]
fn inverse_stage_variants_remain_distinct() {
    assert_ne!(InverseStage::Theta, InverseStage::GeodeticLatitude);
}
