//! SnapTargets : HFX snap selection → GeoPackage | GeoDataFrame.
//! Python owns output paths and optional GeoPandas imports; core streams source rows.

use std::path::{Path, PathBuf};

use gdal::DriverManager;
use pourpoint_core::error::SessionError;
use pourpoint_core::snap_targets::SnapTargets;
use pourpoint_gdal::snap_export::{SnapExportError, SnapGeoPackage};
use pyo3::exceptions::{PyImportError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use crate::error::dataset_err;

/// A reusable selection of complete, declared HFX snap geometries.
#[pyclass(name = "SnapTargets")]
pub struct PySnapTargets {
    pub(crate) inner: SnapTargets,
}

#[pymethods]
impl PySnapTargets {
    /// Stream this selection to a GeoPackage, replacing the destination only on success.
    ///
    /// # Errors
    /// Raises ValueError for a non-.gpkg path and DatasetError for read or write failures.
    #[tracing::instrument(skip_all, fields(path = %path.display()))]
    fn write(&self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gpkg"))
        {
            return Err(PyValueError::new_err(
                "snap target output must have a .gpkg extension",
            ));
        }
        py.allow_threads(|| {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let temporary = tempfile::Builder::new()
                .prefix(".pourpoint-snap-")
                .tempdir_in(parent)
                .map_err(dataset_err)?;
            let temporary_path = temporary.path().join("snap-targets.gpkg");
            let dataset = DriverManager::get_driver_by_name("GPKG")
                .and_then(|driver| driver.create_vector_only(&temporary_path))
                .map_err(dataset_err)?;
            let mut writer =
                SnapGeoPackage::new(dataset, self.inner.metadata()).map_err(dataset_err)?;
            self.inner
                .visit::<SnapExportError>(|set, row| writer.write(set, row))
                .map_err(dataset_err)?;
            writer.finish().map_err(dataset_err)?;
            std::fs::rename(&temporary_path, &path).map_err(dataset_err)?;
            Ok(())
        })
    }

    /// Materialize the selection as an EPSG:4326 GeoDataFrame with source metadata in attrs.
    ///
    /// # Errors
    /// Raises ImportError without GeoPandas, or DatasetError for invalid source data.
    #[tracing::instrument(skip_all)]
    fn to_geodataframe(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let geopandas = py.import("geopandas").map_err(|error| {
            if error.is_instance_of::<PyImportError>(py) {
                PyImportError::new_err(
                    "to_geodataframe() requires GeoPandas; install pourpoint[geopandas]",
                )
            } else {
                error
            }
        })?;
        let rows = py
            .allow_threads(|| {
                let mut rows = Vec::new();
                self.inner.visit::<SessionError>(|set, row| {
                    rows.push((set.name.clone(), row.clone()));
                    Ok(())
                })?;
                Ok::<_, SessionError>(rows)
            })
            .map_err(dataset_err)?;
        let data = PyDict::new(py);
        let sets = PyList::empty(py);
        let ids = PyList::empty(py);
        let units = PyList::empty(py);
        let weights = PyList::empty(py);
        let roles = PyList::empty(py);
        let bounds = PyList::empty(py);
        let wkb = PyList::empty(py);
        for (set, source_row) in rows {
            bounds.append(source_row.bbox.map(|bbox| {
                let [xmin, ymin, xmax, ymax] = bbox.bounds();
                (xmin, ymin, xmax, ymax)
            }))?;
            let row = source_row.target;
            sets.append(set)?;
            ids.append(row.id().get())?;
            units.append(row.unit_id().get())?;
            weights.append(row.weight().get())?;
            roles.append(row.stem_role().map(|role| role.to_string()))?;
            wkb.append(PyBytes::new(py, row.geometry().as_bytes()))?;
        }
        data.set_item("snap_set", sets)?;
        data.set_item("id", ids)?;
        data.set_item("unit_id", units)?;
        data.set_item("weight", weights)?;
        data.set_item("stem_role", roles)?;
        data.set_item("bbox", bounds)?;
        // Keep the exact original bytes, including their WKB byte order.
        data.set_item("geometry_wkb", &wkb)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("crs", "EPSG:4326")?;
        let geometry =
            geopandas
                .getattr("GeoSeries")?
                .call_method("from_wkb", (wkb,), Some(&kwargs))?;
        kwargs.set_item("geometry", geometry)?;
        let frame = geopandas
            .getattr("GeoDataFrame")?
            .call((data,), Some(&kwargs))?;
        let metadata = self.inner.metadata();
        let declarations: Vec<_> = metadata
            .declarations
            .iter()
            .map(|set| {
                serde_json::json!({
                    "name": set.name,
                    "description": set.description,
                    "references_levels": set.references_levels,
                    "weight_semantics": set.weight_semantics,
                    "snap": set.snap,
                })
            })
            .collect();
        let attrs = serde_json::json!({"snap_sets": declarations, "manifest": metadata.manifest});
        let attrs = py
            .import("json")?
            .call_method1("loads", (attrs.to_string(),))?;
        frame.setattr("attrs", attrs)?;
        Ok(frame.unbind())
    }
}
