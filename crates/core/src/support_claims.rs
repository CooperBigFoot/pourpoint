//! reader support claims : declared reader values -> production support obligations.
//!
//! Claims pair canonical on-disk declarations with the HFX domain values used
//! by production. Stable claim IDs are correspondence keys for independent
//! shipped-path evidence.

use hfx::{Crs, FlowAccumulationUnits, FlowDirEncoding, FormatVersion};

use crate::algo::projection::Crs as D8Crs;

/// Stable correspondence key for a reader support claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReaderSupportClaimId(&'static str);

impl ReaderSupportClaimId {
    /// Creates a stable support-claim ID.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// Returns the stable ID text.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Typed production value implemented for a canonical declaration.
///
/// Later reader-support steps extend this vocabulary rather than defining
/// separate field-name and string-value catalogs.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderSupportValue {
    /// An implemented HFX format version.
    FormatVersion(FormatVersion),
    /// An implemented dataset-level coordinate reference system.
    DatasetCrs(Crs),
    /// An implemented D8 flow-direction encoding.
    FlowDirectionEncoding(FlowDirEncoding),
    /// An implemented D8 raster coordinate reference system.
    D8Crs(D8Crs),
    /// Implemented D8 flow-accumulation units.
    D8FlowAccumulationUnits(FlowAccumulationUnits),
}

/// One exact on-disk declaration implemented by the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderSupportClaim {
    id: ReaderSupportClaimId,
    canonical_declaration: &'static str,
    value: ReaderSupportValue,
}

impl ReaderSupportClaim {
    /// Creates an immutable reader support claim.
    pub const fn new(
        id: ReaderSupportClaimId,
        canonical_declaration: &'static str,
        value: ReaderSupportValue,
    ) -> Self {
        Self {
            id,
            canonical_declaration,
            value,
        }
    }

    /// Returns the stable correspondence key.
    pub const fn id(&self) -> ReaderSupportClaimId {
        self.id
    }

    /// Returns the exact supported on-disk declaration.
    pub const fn canonical_declaration(&self) -> &'static str {
        self.canonical_declaration
    }

    /// Returns the typed value consumed by production.
    pub const fn value(&self) -> &ReaderSupportValue {
        &self.value
    }
}

/// Support claim for HFX `format_version = "0.3.0"`.
pub const FORMAT_VERSION_V0_3_0: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-format-version-0.3.0"),
    "0.3.0",
    ReaderSupportValue::FormatVersion(FormatVersion::V0_3_0),
);

/// Support claim for dataset `crs = "EPSG:4326"`.
pub const DATASET_CRS_EPSG_4326: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-dataset-crs-epsg-4326"),
    "EPSG:4326",
    ReaderSupportValue::DatasetCrs(Crs::Epsg4326),
);

/// Support claim for D8 `flow_dir_encoding = "esri"`.
pub const FLOW_DIR_ENCODING_ESRI: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-flow-dir-encoding-esri"),
    "esri",
    ReaderSupportValue::FlowDirectionEncoding(FlowDirEncoding::Esri),
);

/// Support claim for D8 `flow_dir_encoding = "taudem"`.
pub const FLOW_DIR_ENCODING_TAUDEM: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-flow-dir-encoding-taudem"),
    "taudem",
    ReaderSupportValue::FlowDirectionEncoding(FlowDirEncoding::Taudem),
);

/// Support claim for D8 `flow_dir_encoding = "grass"`.
pub const FLOW_DIR_ENCODING_GRASS: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-flow-dir-encoding-grass"),
    "grass",
    ReaderSupportValue::FlowDirectionEncoding(FlowDirEncoding::Grass),
);

/// Implemented declarations for D8 flow-direction encoding.
pub const FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS: &[ReaderSupportClaim] = &[
    FLOW_DIR_ENCODING_ESRI,
    FLOW_DIR_ENCODING_TAUDEM,
    FLOW_DIR_ENCODING_GRASS,
];

/// Implemented declarations that govern admission of the HFX core manifest.
pub const CORE_MANIFEST_SUPPORT_CLAIMS: &[ReaderSupportClaim] =
    &[FORMAT_VERSION_V0_3_0, DATASET_CRS_EPSG_4326];

/// Support claim for D8 raster `crs = "EPSG:4326"`.
pub const D8_CRS_EPSG_4326: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-d8-crs-epsg-4326"),
    "EPSG:4326",
    ReaderSupportValue::D8Crs(D8Crs::Epsg4326),
);

/// Support claim for D8 raster `crs = "EPSG:8857"`.
pub const D8_CRS_EPSG_8857: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-d8-crs-epsg-8857"),
    "EPSG:8857",
    ReaderSupportValue::D8Crs(D8Crs::Epsg8857),
);

/// Support claim for D8 raster `flow_acc_units = "cells"`.
pub const D8_FLOW_ACCUMULATION_UNITS_CELLS: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-d8-flow-acc-units-cells"),
    "cells",
    ReaderSupportValue::D8FlowAccumulationUnits(FlowAccumulationUnits::Cells),
);

/// Support claim for D8 raster `flow_acc_units = "km2"`.
pub const D8_FLOW_ACCUMULATION_UNITS_KM2: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("core-d8-flow-acc-units-km2"),
    "km2",
    ReaderSupportValue::D8FlowAccumulationUnits(FlowAccumulationUnits::Km2),
);

/// Implemented exact declarations for D8 raster CRS and accumulation metadata.
pub const D8_METADATA_SUPPORT_CLAIMS: &[ReaderSupportClaim] = &[
    D8_CRS_EPSG_4326,
    D8_CRS_EPSG_8857,
    D8_FLOW_ACCUMULATION_UNITS_CELLS,
    D8_FLOW_ACCUMULATION_UNITS_KM2,
];

/// Returns the typed format version for an implemented declaration.
pub fn claimed_format_version(declaration: &str) -> Option<FormatVersion> {
    if declaration != FORMAT_VERSION_V0_3_0.canonical_declaration() {
        return None;
    }
    match FORMAT_VERSION_V0_3_0.value() {
        ReaderSupportValue::FormatVersion(value) => Some(*value),
        ReaderSupportValue::DatasetCrs(_)
        | ReaderSupportValue::FlowDirectionEncoding(_)
        | ReaderSupportValue::D8Crs(_)
        | ReaderSupportValue::D8FlowAccumulationUnits(_) => None,
    }
}

/// Returns the typed dataset CRS for an implemented declaration.
pub fn claimed_dataset_crs(declaration: &str) -> Option<Crs> {
    if declaration != DATASET_CRS_EPSG_4326.canonical_declaration() {
        return None;
    }
    match DATASET_CRS_EPSG_4326.value() {
        ReaderSupportValue::DatasetCrs(value) => Some(*value),
        ReaderSupportValue::FormatVersion(_)
        | ReaderSupportValue::FlowDirectionEncoding(_)
        | ReaderSupportValue::D8Crs(_)
        | ReaderSupportValue::D8FlowAccumulationUnits(_) => None,
    }
}

/// Returns the typed D8 CRS for an implemented exact declaration.
pub fn claimed_d8_crs(declaration: &str) -> Option<D8Crs> {
    let claim = if declaration == D8_CRS_EPSG_4326.canonical_declaration() {
        &D8_CRS_EPSG_4326
    } else if declaration == D8_CRS_EPSG_8857.canonical_declaration() {
        &D8_CRS_EPSG_8857
    } else {
        return None;
    };
    match claim.value() {
        ReaderSupportValue::D8Crs(value) => Some(*value),
        ReaderSupportValue::FormatVersion(_)
        | ReaderSupportValue::DatasetCrs(_)
        | ReaderSupportValue::FlowDirectionEncoding(_)
        | ReaderSupportValue::D8FlowAccumulationUnits(_) => None,
    }
}

/// Returns the typed D8 flow-accumulation units for an implemented exact declaration.
pub fn claimed_d8_flow_accumulation_units(declaration: &str) -> Option<FlowAccumulationUnits> {
    let claim = if declaration == D8_FLOW_ACCUMULATION_UNITS_CELLS.canonical_declaration() {
        &D8_FLOW_ACCUMULATION_UNITS_CELLS
    } else if declaration == D8_FLOW_ACCUMULATION_UNITS_KM2.canonical_declaration() {
        &D8_FLOW_ACCUMULATION_UNITS_KM2
    } else {
        return None;
    };
    match claim.value() {
        ReaderSupportValue::D8FlowAccumulationUnits(value) => Some(*value),
        ReaderSupportValue::FormatVersion(_)
        | ReaderSupportValue::DatasetCrs(_)
        | ReaderSupportValue::FlowDirectionEncoding(_)
        | ReaderSupportValue::D8Crs(_) => None,
    }
}

/// Returns whether two exact D8 metadata declarations form a supported pair.
pub fn d8_pair_is_compatible(
    crs_declaration: &str,
    flow_accumulation_units_declaration: &str,
) -> bool {
    matches!(
        (
            claimed_d8_crs(crs_declaration),
            claimed_d8_flow_accumulation_units(flow_accumulation_units_declaration),
        ),
        (Some(D8Crs::Epsg4326), Some(FlowAccumulationUnits::Cells))
            | (Some(D8Crs::Epsg8857), Some(FlowAccumulationUnits::Cells))
            | (Some(D8Crs::Epsg8857), Some(FlowAccumulationUnits::Km2))
    )
}
