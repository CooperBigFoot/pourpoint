//! Typed, GDAL-free transforms for the coordinate reference systems used by pourpoint.

use std::f64::consts::{FRAC_PI_2, PI};

use tracing::instrument;

use crate::algo::coord::GeoCoord;

const SEMI_MAJOR_AXIS_METRES: f64 = 6_378_137.0;
const RECIPROCAL_FLATTENING: f64 = 298.257_223_563;
const A1: f64 = 1.340_264;
const A2: f64 = -0.081_106;
const A3: f64 = 0.000_893;
const A4: f64 = 0.003_796;
const MAX_NEWTON_UPDATES: usize = 12;
const THETA_UPDATE_TOLERANCE: f64 = 1e-14;
const GEODETIC_RESIDUAL_TOLERANCE: f64 = 1e-15;
const AUTHALIC_SINE_CLAMP_ALLOWANCE: f64 = 2.0 * f64::EPSILON;
const LONGITUDE_CLAMP_ALLOWANCE: f64 = 1e-12;

/// A coordinate reference system supported by pourpoint's projection boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crs {
    /// Geographic WGS 84 coordinates in longitude and latitude degrees.
    Epsg4326,
    /// WGS 84 / Equal Earth Greenwich coordinates in metres.
    Epsg8857,
}

impl TryFrom<u32> for Crs {
    type Error = ProjectionError;

    fn try_from(epsg: u32) -> Result<Self, Self::Error> {
        match epsg {
            4326 => Ok(Self::Epsg4326),
            8857 => Ok(Self::Epsg8857),
            epsg => Err(ProjectionError::UnsupportedCrs { epsg }),
        }
    }
}

/// A coordinate expressed in the native units of a selected [`Crs`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeCoord {
    x: f64,
    y: f64,
}

impl NativeCoord {
    /// Create a native coordinate from its x and y ordinates.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Return the native x ordinate.
    pub fn x(self) -> f64 {
        self.x
    }

    /// Return the native y ordinate.
    pub fn y(self) -> f64 {
        self.y
    }
}

/// The inverse iteration stage that failed to converge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InverseStage {
    /// Recovery of the Equal Earth parametric latitude.
    Theta,
    /// Recovery of geodetic latitude from authalic latitude.
    GeodeticLatitude,
}

/// Errors from inverse projection and CRS selection.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProjectionError {
    /// Returned when an EPSG code has no built-in transform.
    #[error("unsupported coordinate reference system EPSG:{epsg}")]
    UnsupportedCrs {
        /// The unsupported EPSG code.
        epsg: u32,
    },
    /// Returned when an inverse Newton iteration exhausts its update limit.
    #[error("inverse projection did not converge during {stage:?} recovery")]
    NonConvergence {
        /// The inverse stage that did not converge.
        stage: InverseStage,
    },
    /// Returned when inverse input is non-finite or outside the EPSG:8857 domain.
    #[error("projected coordinate ({x}, {y}) is outside the projection domain")]
    OutOfDomain {
        /// The rejected native x ordinate.
        x: f64,
        /// The rejected native y ordinate.
        y: f64,
    },
}

/// Transform an EPSG:4326 geographic coordinate into the selected CRS.
#[instrument]
pub fn forward(crs: Crs, coordinate: GeoCoord) -> NativeCoord {
    match crs {
        Crs::Epsg4326 => NativeCoord::new(coordinate.lon, coordinate.lat),
        Crs::Epsg8857 => equal_earth_forward(coordinate),
    }
}

/// Transform a native coordinate from the selected CRS into EPSG:4326.
///
/// # Errors
///
/// Returns [`ProjectionError::OutOfDomain`] for non-finite or out-of-domain
/// EPSG:8857 coordinates, or [`ProjectionError::NonConvergence`] if either
/// mandated inverse iteration exhausts its update limit.
#[instrument]
pub fn inverse(crs: Crs, coordinate: NativeCoord) -> Result<GeoCoord, ProjectionError> {
    match crs {
        Crs::Epsg4326 => Ok(GeoCoord::new(coordinate.x, coordinate.y)),
        Crs::Epsg8857 => equal_earth_inverse(coordinate),
    }
}

fn m() -> f64 {
    3.0_f64.sqrt() / 2.0
}

fn q(phi: f64) -> f64 {
    let f = 1.0 / RECIPROCAL_FLATTENING;
    let e2 = f * (2.0 - f);
    let e = e2.sqrt();
    let s = phi.sin();
    let den = 1.0 - e2 * s * s;
    (1.0 - e2) * (s / den + (e * s).atanh() / e)
}

fn q_p() -> f64 {
    q(FRAC_PI_2)
}

fn authalic_radius() -> f64 {
    SEMI_MAJOR_AXIS_METRES * (q_p() / 2.0).sqrt()
}

fn theta_max() -> f64 {
    PI / 3.0
}

fn y_max() -> f64 {
    let theta = theta_max();
    let theta2 = theta * theta;
    let theta6 = theta2 * theta2 * theta2;
    authalic_radius() * theta * (A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2))
}

fn derivative_polynomial(theta2: f64, theta6: f64) -> f64 {
    A1 + 3.0 * A2 * theta2 + theta6 * (7.0 * A3 + 9.0 * A4 * theta2)
}

fn equal_earth_forward(coordinate: GeoCoord) -> NativeCoord {
    let phi = coordinate.lat.to_radians();
    let lambda = coordinate.lon.to_radians();
    let beta = (q(phi) / q_p()).clamp(-1.0, 1.0).asin();
    let theta = (m() * beta.sin()).asin();
    let theta2 = theta * theta;
    let theta6 = theta2 * theta2 * theta2;
    let d = derivative_polynomial(theta2, theta6);
    let rq = authalic_radius();
    let x = rq * lambda * theta.cos() / (m() * d);
    let y = rq * theta * (A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2));
    NativeCoord::new(x, y)
}

fn equal_earth_inverse(coordinate: NativeCoord) -> Result<GeoCoord, ProjectionError> {
    let x = coordinate.x;
    let y = coordinate.y;
    if !x.is_finite() || !y.is_finite() || y.abs() > y_max() {
        return Err(ProjectionError::OutOfDomain { x, y });
    }

    let rq = authalic_radius();
    let theta = recover_theta(y, rq)?;
    let mut s_beta = theta.sin() / m();
    if s_beta > 1.0 {
        if s_beta - 1.0 <= AUTHALIC_SINE_CLAMP_ALLOWANCE {
            s_beta = 1.0;
        } else {
            return Err(ProjectionError::OutOfDomain { x, y });
        }
    } else if s_beta < -1.0 {
        if -1.0 - s_beta <= AUTHALIC_SINE_CLAMP_ALLOWANCE {
            s_beta = -1.0;
        } else {
            return Err(ProjectionError::OutOfDomain { x, y });
        }
    }
    let beta = s_beta.asin();

    let theta2 = theta * theta;
    let theta6 = theta2 * theta2 * theta2;
    let d = derivative_polynomial(theta2, theta6);
    let mut lambda = m() * (x / rq) * d / theta.cos();
    if lambda.abs() > PI + LONGITUDE_CLAMP_ALLOWANCE {
        return Err(ProjectionError::OutOfDomain { x, y });
    }
    lambda = lambda.clamp(-PI, PI);

    let phi = recover_geodetic_latitude(beta)?;
    Ok(GeoCoord::new(lambda.to_degrees(), phi.to_degrees()))
}

fn recover_theta(y: f64, rq: f64) -> Result<f64, ProjectionError> {
    let mut theta = y / (rq * A1);
    for _ in 0..MAX_NEWTON_UPDATES {
        let theta2 = theta * theta;
        let theta6 = theta2 * theta2 * theta2;
        let polynomial = A1 + A2 * theta2 + theta6 * (A3 + A4 * theta2);
        let dtheta = (theta * polynomial - y / rq) / derivative_polynomial(theta2, theta6);
        theta -= dtheta;
        if dtheta.abs() <= THETA_UPDATE_TOLERANCE {
            return Ok(theta);
        }
    }
    Err(ProjectionError::NonConvergence {
        stage: InverseStage::Theta,
    })
}

fn recover_geodetic_latitude(beta: f64) -> Result<f64, ProjectionError> {
    if beta == FRAC_PI_2 {
        return Ok(FRAC_PI_2);
    }
    if beta == -FRAC_PI_2 {
        return Ok(-FRAC_PI_2);
    }

    let target = q_p() * beta.sin();
    let f = 1.0 / RECIPROCAL_FLATTENING;
    let e2 = f * (2.0 - f);
    let mut phi = beta;
    for _ in 0..MAX_NEWTON_UPDATES {
        let residual = q(phi) - target;
        if residual.abs() <= GEODETIC_RESIDUAL_TOLERANCE {
            return Ok(phi);
        }
        let sin_phi = phi.sin();
        let denominator = 1.0 - e2 * sin_phi * sin_phi;
        let derivative = 2.0 * (1.0 - e2) * phi.cos() / (denominator * denominator);
        phi -= residual / derivative;
    }

    if (q(phi) - target).abs() <= GEODETIC_RESIDUAL_TOLERANCE {
        Ok(phi)
    } else {
        Err(ProjectionError::NonConvergence {
            stage: InverseStage::GeodeticLatitude,
        })
    }
}
