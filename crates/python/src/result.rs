//! Python-exposed [`DelineationResult`] wrapper.

use std::sync::OnceLock;

use geo::BoundingRect;
use pourpoint_core::algo::encode_wkb_multi_polygon;
use pourpoint_core::engine::DelineationAreaOnlyResult;
use pourpoint_core::refinement::{
    AppliedRefinementReason, BestEffortSkipCategory, BestEffortSkipReason,
};
use pourpoint_core::{DelineationResult, RefinementOutcome};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::geojson::result_to_geojson_feature;

/// Python-visible wrapper around [`DelineationResult`].
#[pyclass(name = "DelineationResult")]
pub struct PyDelineationResult {
    inner: DelineationResult,
    geometry_wkb: OnceLock<Vec<u8>>,
}

/// Typed Python view of a best-effort refinement skip reason.
#[pyclass(name = "BestEffortSkipReason", frozen)]
pub struct PyBestEffortSkipReason {
    inner: BestEffortSkipReason,
}

impl PyBestEffortSkipReason {
    fn from_reason(reason: BestEffortSkipReason) -> Self {
        Self { inner: reason }
    }
}

#[pymethods]
impl PyBestEffortSkipReason {
    /// Stable machine-readable reason kind.
    #[getter]
    fn kind(&self) -> &'static str {
        match &self.inner {
            BestEffortSkipReason::UnreadableD8AuxDeclared { .. } => "unreadable_d8_aux_declared",
            BestEffortSkipReason::NoD8AuxDeclared => "no_d8_aux_declared",
            BestEffortSkipReason::CoarseUnitOnlyNoD8AuxDeclared => {
                "coarse_unit_only_no_d8_aux_declared"
            }
            BestEffortSkipReason::NoRasterSourceProvided => "no_raster_source_provided",
            BestEffortSkipReason::VectorOutletGuardFailed { .. } => "vector_outlet_guard_failed",
            BestEffortSkipReason::Availability { .. } => "availability",
            BestEffortSkipReason::MisDeclaration { .. } => "mis_declaration",
            BestEffortSkipReason::DataGeometryIntegrity { .. } => "data_geometry_integrity",
        }
    }

    /// Stable operator-facing category.
    #[getter]
    fn category(&self) -> &'static str {
        match self.inner.category() {
            BestEffortSkipCategory::Availability => "availability",
            BestEffortSkipCategory::MisDeclaration => "mis_declaration",
            BestEffortSkipCategory::DataGeometryIntegrity => "data_geometry_integrity",
        }
    }

    /// Retained unreadable schema, when this is an unreadable-D8 reason.
    #[getter]
    fn schema(&self) -> Option<String> {
        match &self.inner {
            BestEffortSkipReason::UnreadableD8AuxDeclared { schema } => Some(schema.clone()),
            BestEffortSkipReason::NoD8AuxDeclared
            | BestEffortSkipReason::CoarseUnitOnlyNoD8AuxDeclared
            | BestEffortSkipReason::NoRasterSourceProvided
            | BestEffortSkipReason::VectorOutletGuardFailed { .. }
            | BestEffortSkipReason::Availability { .. }
            | BestEffortSkipReason::MisDeclaration { .. }
            | BestEffortSkipReason::DataGeometryIntegrity { .. } => None,
        }
    }

    /// Failed vector guard conjunct, when applicable.
    #[getter]
    fn failure_kind(&self) -> Option<&'static str> {
        let BestEffortSkipReason::VectorOutletGuardFailed { kind, .. } = &self.inner else {
            return None;
        };
        Some(match kind {
            pourpoint_core::algo::VectorOutletGuardFailureKind::GridMapping => "grid_mapping",
            pourpoint_core::algo::VectorOutletGuardFailureKind::OutsideTerminalMask => {
                "outside_terminal_mask"
            }
            pourpoint_core::algo::VectorOutletGuardFailureKind::UndefinedFlowDirection => {
                "undefined_flow_direction"
            }
            pourpoint_core::algo::VectorOutletGuardFailureKind::UndefinedAccumulation => {
                "undefined_accumulation"
            }
            pourpoint_core::algo::VectorOutletGuardFailureKind::BelowThreshold => "below_threshold",
        })
    }

    /// Requested threshold in upstream cells, when applicable.
    #[getter]
    fn requested_threshold(&self) -> Option<u32> {
        match &self.inner {
            BestEffortSkipReason::VectorOutletGuardFailed {
                requested_threshold,
                ..
            } => Some(requested_threshold.pixels()),
            _ => None,
        }
    }

    /// Effective threshold in declared accumulation units, when applicable.
    #[getter]
    fn effective_threshold(&self) -> Option<f32> {
        match &self.inner {
            BestEffortSkipReason::VectorOutletGuardFailed {
                effective_threshold,
                ..
            } => Some(*effective_threshold),
            _ => None,
        }
    }

    /// Declared accumulation units, when applicable.
    #[getter]
    fn units(&self) -> Option<String> {
        match &self.inner {
            BestEffortSkipReason::VectorOutletGuardFailed { units, .. } => Some(units.to_string()),
            _ => None,
        }
    }

    /// Mapped `(row, col)` cell, absent when grid mapping failed.
    #[getter]
    fn mapped_cell(&self) -> Option<(usize, usize)> {
        match &self.inner {
            BestEffortSkipReason::VectorOutletGuardFailed { mapped_cell, .. } => {
                mapped_cell.map(|cell| (cell.row, cell.col))
            }
            _ => None,
        }
    }

    /// Measured accumulation, absent when mapping or data availability prevented measurement.
    #[getter]
    fn measured_accumulation(&self) -> Option<f32> {
        match &self.inner {
            BestEffortSkipReason::VectorOutletGuardFailed {
                measured_accumulation,
                ..
            } => *measured_accumulation,
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        match self.schema() {
            Some(schema) => format!(
                "BestEffortSkipReason(kind='{}', category='{}', schema='{schema}')",
                self.kind(),
                self.category()
            ),
            None => format!(
                "BestEffortSkipReason(kind='{}', category='{}', schema=None)",
                self.kind(),
                self.category()
            ),
        }
    }
}

impl PyDelineationResult {
    /// Wrap a [`DelineationResult`] from the engine.
    pub fn from_result(result: DelineationResult) -> Self {
        Self {
            inner: result,
            geometry_wkb: OnceLock::new(),
        }
    }

    /// Return the wrapped core result for internal Python binding modules.
    pub(crate) fn inner(&self) -> &DelineationResult {
        &self.inner
    }
}

/// Python-visible wrapper around [`DelineationAreaOnlyResult`].
#[pyclass(name = "AreaOnlyResult")]
pub struct PyAreaOnlyResult {
    inner: DelineationAreaOnlyResult,
}

impl PyAreaOnlyResult {
    /// Wrap a [`DelineationAreaOnlyResult`] from the engine.
    pub fn from_result(result: DelineationAreaOnlyResult) -> Self {
        Self { inner: result }
    }
}

/// Light Python value for upstream unit metadata retained on a merged result.
#[pyclass(name = "DelineationUnitMetadata")]
#[derive(Clone)]
pub struct PyDelineationUnitMetadata {
    id: i64,
    level: i16,
    area_km2: f64,
    up_area_km2: Option<f64>,
    outlet: (f64, f64),
}

#[pymethods]
impl PyDelineationResult {
    /// Terminal unit ID that the outlet resolved to.
    #[getter]
    fn terminal_unit_id(&self) -> i64 {
        self.inner.terminal_unit_id().get()
    }

    /// Input outlet coordinate as `(lon, lat)`.
    #[getter]
    fn input_outlet(&self) -> (f64, f64) {
        let c = self.inner.input_outlet();
        (c.lon, c.lat)
    }

    /// Resolved outlet coordinate as `(lon, lat)`.
    #[getter]
    fn resolved_outlet(&self) -> (f64, f64) {
        let c = self.inner.resolved_outlet();
        (c.lon, c.lat)
    }

    /// Refined outlet coordinate as `(lon, lat)`, or `None` if refinement was
    /// not applied.
    #[getter]
    fn refined_outlet(&self) -> Option<(f64, f64)> {
        refined_outlet_tuple(self.inner.refinement())
    }

    /// Typed best-effort refinement skip reason, if refinement was skipped.
    #[getter]
    fn refinement_skip_reason(&self) -> Option<PyBestEffortSkipReason> {
        match self.inner.refinement() {
            RefinementOutcome::BestEffortSkipped { provenance } => Some(
                PyBestEffortSkipReason::from_reason(provenance.why().clone()),
            ),
            RefinementOutcome::Applied { .. } | RefinementOutcome::Disabled => None,
        }
    }

    /// Raster seed decision: `vector_quantized`, `raster_ranked`, `coarse`, or `disabled`.
    #[getter]
    fn refinement_seed_kind(&self) -> &'static str {
        refinement_seed_kind(self.inner.refinement())
    }

    /// Debug string representation of the resolution method.
    #[getter]
    fn resolution_method(&self) -> String {
        format!("{:?}", self.inner.resolution_method())
    }

    /// All upstream unit IDs (including the terminal unit).
    #[getter]
    fn upstream_unit_ids(&self) -> Vec<i64> {
        self.inner
            .upstream_unit_ids()
            .iter()
            .map(|id| id.get())
            .collect()
    }

    /// Light upstream unit metadata without per-unit geometries.
    #[getter]
    fn upstream_units(&self) -> Vec<PyDelineationUnitMetadata> {
        self.inner
            .upstream_units()
            .iter()
            .map(|unit| PyDelineationUnitMetadata {
                id: unit.id().get(),
                level: unit.level().get(),
                area_km2: f64::from(unit.area().get()),
                up_area_km2: unit.up_area().map(|area| f64::from(area.get())),
                outlet: (unit.outlet().lon(), unit.outlet().lat()),
            })
            .collect()
    }

    /// Geodesic watershed area in km².
    #[getter]
    fn area_km2(&self) -> f64 {
        self.inner.area_km2().as_f64()
    }

    /// Watershed geometry bounding box as `(minx, miny, maxx, maxy)`.
    #[getter]
    fn geometry_bbox(&self) -> Option<(f64, f64, f64, f64)> {
        self.inner.geometry().bounding_rect().map(|rect| {
            let min = rect.min();
            let max = rect.max();
            (min.x, min.y, max.x, max.y)
        })
    }

    /// Watershed geometry encoded as OGC WKB bytes (little-endian, 2D).
    #[getter]
    fn geometry_wkb<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        if let Some(bytes) = self.geometry_wkb.get() {
            return Ok(PyBytes::new(py, bytes));
        }

        let encoded = encode_wkb_multi_polygon(self.inner.geometry())
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        match self.geometry_wkb.set(encoded) {
            Ok(()) => match self.geometry_wkb.get() {
                Some(bytes) => Ok(PyBytes::new(py, bytes)),
                None => Err(pyo3::exceptions::PyRuntimeError::new_err(
                    "failed to cache geometry WKB",
                )),
            },
            Err(bytes) => match self.geometry_wkb.get() {
                Some(cached) => Ok(PyBytes::new(py, cached)),
                None => Ok(PyBytes::new(py, &bytes)),
            },
        }
    }

    /// Serialize the result as a GeoJSON Feature string.
    fn to_geojson(&self) -> PyResult<String> {
        result_to_geojson_feature(&self.inner)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    fn __repr__(&self) -> String {
        format!(
            "DelineationResult(terminal_unit_id={}, area_km2={:.2}, upstream_count={})",
            self.inner.terminal_unit_id().get(),
            self.inner.area_km2().as_f64(),
            self.inner.upstream_unit_ids().len(),
        )
    }
}

#[pymethods]
impl PyDelineationUnitMetadata {
    /// Drainage unit ID.
    #[getter]
    fn id(&self) -> i64 {
        self.id
    }

    /// HFX drainage-unit level.
    #[getter]
    fn level(&self) -> i16 {
        self.level
    }

    /// Local drainage area from `catchments.parquet`.
    #[getter]
    fn area_km2(&self) -> f64 {
        self.area_km2
    }

    /// Total upstream drainage area from `catchments.parquet`, if present.
    #[getter]
    fn up_area_km2(&self) -> Option<f64> {
        self.up_area_km2
    }

    /// Declared outlet coordinate as `(lon, lat)`.
    #[getter]
    fn outlet(&self) -> (f64, f64) {
        self.outlet
    }

    fn __repr__(&self) -> String {
        format!(
            "DelineationUnitMetadata(id={}, level={}, area_km2={:.2})",
            self.id, self.level, self.area_km2
        )
    }
}

#[pymethods]
impl PyAreaOnlyResult {
    /// Terminal unit ID that the outlet resolved to.
    #[getter]
    fn terminal_unit_id(&self) -> i64 {
        self.inner.terminal_unit_id().get()
    }

    /// Input outlet coordinate as `(lon, lat)`.
    #[getter]
    fn input_outlet(&self) -> (f64, f64) {
        let c = self.inner.input_outlet();
        (c.lon, c.lat)
    }

    /// Resolved outlet coordinate as `(lon, lat)`.
    #[getter]
    fn resolved_outlet(&self) -> (f64, f64) {
        let c = self.inner.resolved_outlet();
        (c.lon, c.lat)
    }

    /// Refined outlet coordinate as `(lon, lat)`, or `None` if refinement was
    /// not applied.
    #[getter]
    fn refined_outlet(&self) -> Option<(f64, f64)> {
        refined_outlet_tuple(self.inner.refinement())
    }

    /// Debug string representation of the resolution method.
    #[getter]
    fn resolution_method(&self) -> String {
        format!("{:?}", self.inner.resolution_method())
    }

    /// All upstream unit IDs (including the terminal unit).
    #[getter]
    fn upstream_unit_ids(&self) -> Vec<i64> {
        self.inner
            .upstream_unit_ids()
            .iter()
            .map(|id| id.get())
            .collect()
    }

    /// Geodesic watershed area in km².
    #[getter]
    fn area_km2(&self) -> f64 {
        self.inner.area_km2().as_f64()
    }

    fn __repr__(&self) -> String {
        format!(
            "AreaOnlyResult(terminal_unit_id={}, area_km2={:.2}, upstream_count={})",
            self.inner.terminal_unit_id().get(),
            self.inner.area_km2().as_f64(),
            self.inner.upstream_unit_ids().len(),
        )
    }
}

fn refinement_seed_kind(refinement: &RefinementOutcome) -> &'static str {
    match refinement {
        RefinementOutcome::Applied { provenance, .. } => match provenance.why() {
            AppliedRefinementReason::VectorOutletQuantized { .. } => "vector_quantized",
            #[allow(deprecated)]
            AppliedRefinementReason::D8AuxMatchedTerminalBbox { .. }
            | AppliedRefinementReason::RasterOutletRanked { .. } => "raster_ranked",
        },
        RefinementOutcome::BestEffortSkipped { .. } => "coarse",
        RefinementOutcome::Disabled => "disabled",
    }
}

fn refined_outlet_tuple(refinement: &RefinementOutcome) -> Option<(f64, f64)> {
    match refinement {
        RefinementOutcome::Applied { refined_outlet, .. } => {
            Some((refined_outlet.lon, refined_outlet.lat))
        }
        _ => None,
    }
}
