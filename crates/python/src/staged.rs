//! Python-exposed staged delineation intermediates.

use pourpoint_core::algo::encode_wkb_multi_polygon;
use pourpoint_core::staged::{
    DissolvedWatershed, LevelResolvedOutlet, LevelSelection, PreMergeDrainageUnits,
    SameLevelUpstreamUnits, SelectedLevel, TerminalRefinement,
};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Python-visible HFX level selection.
#[pyclass(name = "LevelSelection", eq)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyLevelSelection {
    /// Select the finest loaded HFX drainage-unit level.
    #[pyo3(name = "FINEST")]
    Finest,
}

impl From<PyLevelSelection> for LevelSelection {
    fn from(selection: PyLevelSelection) -> Self {
        match selection {
            PyLevelSelection::Finest => LevelSelection::Finest,
        }
    }
}

/// Python-visible wrapper around a selected HFX level.
#[pyclass(name = "SelectedLevel")]
#[derive(Clone)]
pub struct PySelectedLevel {
    pub(crate) inner: SelectedLevel,
}

impl PySelectedLevel {
    pub(crate) fn from_inner(inner: SelectedLevel) -> Self {
        Self { inner }
    }
}

/// Python-visible wrapper around a level-resolved outlet.
#[pyclass(name = "ResolvedOutlet")]
#[derive(Clone)]
pub struct PyResolvedOutlet {
    pub(crate) inner: LevelResolvedOutlet,
}

impl PyResolvedOutlet {
    pub(crate) fn from_inner(inner: LevelResolvedOutlet) -> Self {
        Self { inner }
    }
}

/// Python-visible wrapper around same-level upstream traversal.
#[pyclass(name = "UpstreamUnits")]
#[derive(Clone)]
pub struct PyUpstreamUnits {
    pub(crate) inner: SameLevelUpstreamUnits,
}

impl PyUpstreamUnits {
    pub(crate) fn from_inner(inner: SameLevelUpstreamUnits) -> Self {
        Self { inner }
    }
}

/// Python-visible wrapper around pre-merge drainage units.
#[pyclass(name = "PreMergeDrainageUnits")]
#[derive(Clone)]
pub struct PyPreMergeDrainageUnits {
    pub(crate) inner: PreMergeDrainageUnits,
}

impl PyPreMergeDrainageUnits {
    pub(crate) fn from_inner(inner: PreMergeDrainageUnits) -> Self {
        Self { inner }
    }
}

/// Python-visible wrapper around terminal refinement output.
#[pyclass(name = "TerminalRefinement")]
#[derive(Clone)]
pub struct PyTerminalRefinement {
    pub(crate) inner: TerminalRefinement,
}

impl PyTerminalRefinement {
    pub(crate) fn from_inner(inner: TerminalRefinement) -> Self {
        Self { inner }
    }
}

/// Python-visible wrapper around dissolved watershed output.
#[pyclass(name = "DissolvedWatershed")]
#[derive(Clone)]
pub struct PyDissolvedWatershed {
    pub(crate) inner: DissolvedWatershed,
}

impl PyDissolvedWatershed {
    pub(crate) fn from_inner(inner: DissolvedWatershed) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PySelectedLevel {
    /// Selected HFX drainage-unit level.
    #[getter]
    fn level(&self) -> i16 {
        self.inner.level().get()
    }

    fn __repr__(&self) -> String {
        format!("SelectedLevel(level={})", self.inner.level().get())
    }
}

#[pymethods]
impl PyResolvedOutlet {
    /// Selected HFX drainage-unit level.
    #[getter]
    fn level(&self) -> i16 {
        self.inner.selected_level().level().get()
    }

    /// Terminal unit ID resolved at the selected level.
    #[getter]
    fn terminal_unit_id(&self) -> i64 {
        self.inner.resolved().unit_id.get()
    }

    /// Original input outlet coordinate as `(lon, lat)`.
    #[getter]
    fn input_outlet(&self) -> (f64, f64) {
        let coord = self.inner.resolved().input_coord;
        (coord.lon, coord.lat)
    }

    /// Resolved outlet coordinate as `(lon, lat)`.
    #[getter]
    fn resolved_outlet(&self) -> (f64, f64) {
        let coord = self.inner.resolved().resolved_coord;
        (coord.lon, coord.lat)
    }

    /// Debug string representation of the resolution method.
    #[getter]
    fn resolution_method(&self) -> String {
        format!("{:?}", self.inner.resolved().method)
    }

    fn __repr__(&self) -> String {
        format!(
            "ResolvedOutlet(level={}, terminal_unit_id={})",
            self.inner.selected_level().level().get(),
            self.inner.resolved().unit_id.get()
        )
    }
}

#[pymethods]
impl PyUpstreamUnits {
    /// Terminal unit ID used for same-level upstream traversal.
    #[getter]
    fn terminal_unit_id(&self) -> i64 {
        self.inner.terminal().get()
    }

    /// Selected HFX drainage-unit level shared by the upstream units.
    #[getter]
    fn level(&self) -> i16 {
        self.inner.selected_level().level().get()
    }

    /// Inclusive upstream unit IDs, including the terminal unit.
    #[getter]
    fn unit_ids(&self) -> Vec<i64> {
        self.inner
            .upstream()
            .iter()
            .map(|unit_id| unit_id.get())
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "UpstreamUnits(terminal_unit_id={}, count={})",
            self.inner.terminal().get(),
            self.inner.upstream().len()
        )
    }
}

#[pymethods]
impl PyPreMergeDrainageUnits {
    /// R3 note: pre-merge units are whole source drainage units, including the
    /// whole terminal unit. Summing or unioning these rows is not the same as
    /// the merged result after terminal refinement.
    #[classattr]
    const R3_NOTE: &'static str = "R3: pre-merge units are whole source drainage units, including the whole terminal; summing or unioning them is not the refined merged watershed.";

    /// Terminal unit ID represented by the first pre-merge record.
    #[getter]
    fn terminal_unit_id(&self) -> i64 {
        self.inner.terminal().get()
    }

    /// Selected HFX drainage-unit level shared by every pre-merge unit.
    #[getter]
    fn level(&self) -> i16 {
        self.inner.selected_level().level().get()
    }

    /// Whole source drainage-unit records before terminal refinement.
    #[getter]
    fn units(&self) -> Vec<PyPreMergeDrainageUnit> {
        self.inner
            .units()
            .iter()
            .map(|unit| PyPreMergeDrainageUnit {
                id: unit.id().get(),
                level: unit.level().get(),
                area_km2: f64::from(unit.area().get()),
                up_area_km2: unit.up_area().map(|area| f64::from(area.get())),
                outlet: (unit.outlet().lon(), unit.outlet().lat()),
            })
            .collect()
    }

    /// Whole source drainage-unit WKB geometries before terminal refinement.
    #[getter]
    fn unit_geometry_wkb<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyBytes>>> {
        self.inner
            .units()
            .iter()
            .map(|unit| {
                let encoded = encode_wkb_multi_polygon(unit.geometry())
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
                Ok(PyBytes::new(py, &encoded))
            })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "PreMergeDrainageUnits(terminal_unit_id={}, count={}, r3_note={:?})",
            self.inner.terminal().get(),
            self.inner.units().len(),
            Self::R3_NOTE
        )
    }
}

/// Light Python value for one pre-merge drainage unit.
#[pyclass(name = "PreMergeDrainageUnit")]
#[derive(Clone)]
pub struct PyPreMergeDrainageUnit {
    id: i64,
    level: i16,
    area_km2: f64,
    up_area_km2: Option<f64>,
    outlet: (f64, f64),
}

#[pymethods]
impl PyPreMergeDrainageUnit {
    #[getter]
    fn id(&self) -> i64 {
        self.id
    }

    #[getter]
    fn level(&self) -> i16 {
        self.level
    }

    #[getter]
    fn area_km2(&self) -> f64 {
        self.area_km2
    }

    #[getter]
    fn up_area_km2(&self) -> Option<f64> {
        self.up_area_km2
    }

    #[getter]
    fn outlet(&self) -> (f64, f64) {
        self.outlet
    }

    fn __repr__(&self) -> String {
        format!(
            "PreMergeDrainageUnit(id={}, level={}, area_km2={:.2})",
            self.id, self.level, self.area_km2
        )
    }
}

#[pymethods]
impl PyTerminalRefinement {
    /// Refinement status: `applied`, `best_effort_skipped`, or `disabled`.
    #[getter]
    fn status(&self) -> &'static str {
        match &self.inner {
            TerminalRefinement::Applied { .. } => "applied",
            TerminalRefinement::BestEffortSkipped { .. } => "best_effort_skipped",
            TerminalRefinement::Disabled => "disabled",
        }
    }

    /// Refined outlet coordinate as `(lon, lat)`, or `None` when not applied.
    #[getter]
    fn refined_outlet(&self) -> Option<(f64, f64)> {
        match &self.inner {
            TerminalRefinement::Applied { refined_outlet, .. } => {
                Some((refined_outlet.lon, refined_outlet.lat))
            }
            _ => None,
        }
    }

    fn __repr__(&self) -> String {
        format!("TerminalRefinement(status={:?})", self.status())
    }
}

#[pymethods]
impl PyDissolvedWatershed {
    /// Geodesic watershed area in km² after terminal refinement and dissolve.
    #[getter]
    fn area_km2(&self) -> f64 {
        self.inner.area_km2().as_f64()
    }

    /// Dissolved watershed geometry encoded as OGC WKB bytes.
    #[getter]
    fn geometry_wkb<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let encoded = encode_wkb_multi_polygon(self.inner.geometry())
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        Ok(PyBytes::new(py, &encoded))
    }

    fn __repr__(&self) -> String {
        format!("DissolvedWatershed(area_km2={:.2})", self.area_km2())
    }
}
