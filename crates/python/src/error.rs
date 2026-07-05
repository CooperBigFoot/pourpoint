//! Python exception types for pourpoint.

use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::PyException;

// The first argument to `create_exception!` sets the `__module__` attribute
// that appears in Python tracebacks. We use `pourpoint` (not `_pourpoint`) so that
// users see `pourpoint.DatasetError` rather than `pourpoint._pourpoint.DatasetError`.
// The exception types are registered in the `_pourpoint` compiled extension and
// re-exported by `pourpoint/__init__.py`, but their `__module__` stays `pourpoint`.
create_exception!(pourpoint, PourpointError, PyException);
create_exception!(pourpoint, DatasetError, PourpointError);
create_exception!(pourpoint, ResolutionError, PourpointError);
create_exception!(pourpoint, PyAssemblyError, PourpointError);

/// Map a dataset/session error to a Python [`DatasetError`].
pub fn dataset_err(e: impl std::fmt::Display) -> PyErr {
    DatasetError::new_err(e.to_string())
}

/// Map a [`pourpoint_core::EngineError`] to the most specific Python exception.
pub fn engine_err_to_py(e: pourpoint_core::EngineError) -> PyErr {
    use pourpoint_core::EngineError;
    match e {
        EngineError::Resolution { .. } => ResolutionError::new_err(e.to_string()),
        EngineError::Traversal { .. } => PourpointError::new_err(e.to_string()),
        EngineError::TerminalCatchmentFetch { .. } => DatasetError::new_err(e.to_string()),
        EngineError::TerminalCatchmentDecode { .. } => DatasetError::new_err(e.to_string()),
        EngineError::RasterLocalize { .. } => DatasetError::new_err(e.to_string()),
        EngineError::D8Selection { .. } => DatasetError::new_err(e.to_string()),
        EngineError::RequiredD8RasterSourceMissing { .. } => DatasetError::new_err(e.to_string()),
        EngineError::Refinement { .. } => PourpointError::new_err(e.to_string()),
        EngineError::Assembly { .. } => PyAssemblyError::new_err(e.to_string()),
        EngineError::SessionLevelIndexEmpty { .. } => DatasetError::new_err(e.to_string()),
        EngineError::SameLevelInvariant { .. } => DatasetError::new_err(e.to_string()),
        EngineError::PreMergeCatchmentFetch { .. } => DatasetError::new_err(e.to_string()),
        EngineError::PreMergeCatchmentDecode { .. } => DatasetError::new_err(e.to_string()),
    }
}

/// Map a core export error to a Python exception.
pub fn export_err_to_py(e: pourpoint_core::export::ExportError) -> PyErr {
    use pourpoint_core::export::ExportError;
    match e {
        ExportError::InvalidBasinId { .. }
        | ExportError::MissingFabricVersion { .. }
        | ExportError::NegativeDefaultBasinId { .. }
        | ExportError::DefaultBasinIdCollision { .. }
        | ExportError::EmptyInput
        | ExportError::EmptyUnitBundle
        | ExportError::DuplicateRow { .. }
        | ExportError::DuplicateUnitBundleRow { .. } => {
            pyo3::exceptions::PyValueError::new_err(e.to_string())
        }
        ExportError::BboxFailure { .. }
        | ExportError::CentroidFailure { .. }
        | ExportError::RowGroupPlanningFailure { .. }
        | ExportError::GeometryEncodingFailure { .. }
        | ExportError::UnitGeometryEncodingFailure { .. }
        | ExportError::ArrowWriteFailure { .. }
        | ExportError::ParquetWriteFailure { .. }
        | ExportError::FooterMetadataFailure { .. } => PourpointError::new_err(e.to_string()),
    }
}
