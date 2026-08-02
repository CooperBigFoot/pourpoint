//! reader support claims : declared reader values -> production support obligations and retained
//! unreadable schema names -> D8-family routing decisions.
//!
//! Claims pair canonical on-disk declarations with the HFX domain values used
//! by production. Stable claim IDs are correspondence keys for independent
//! shipped-path evidence.

use hfx::{
    AuxiliarySchemaId, BlessedAuxSchema, Crs, FlowAccumulationUnits, FlowDirEncoding, FormatVersion,
};

use crate::algo::projection::Crs as D8Crs;

pub(crate) const D8_AUXILIARY_SCHEMA_FAMILY_PREFIX: &str = "hfx.aux.d8_raster.";

pub(crate) fn is_unreadable_d8_auxiliary_schema(schema: &str) -> bool {
    schema.starts_with(D8_AUXILIARY_SCHEMA_FAMILY_PREFIX)
}

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
    /// The blessed D8 raster v2 auxiliary schema.
    AuxiliarySchemaD8RasterV2,
    /// The blessed snap v2 auxiliary schema.
    AuxiliarySchemaSnapV2,
    /// A provisional or third-party generic auxiliary schema.
    AuxiliarySchemaGeneric,
    /// The named non-support outcome for the de-blessed D8 raster v1 schema.
    AuxiliarySchemaD8RasterV1Unsupported,
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

/// Support claim for the blessed D8 raster v2 auxiliary schema.
pub const AUXILIARY_SCHEMA_D8_RASTER_V2: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("aux-schema-d8-raster-v2"),
    "hfx.aux.d8_raster.v2",
    ReaderSupportValue::AuxiliarySchemaD8RasterV2,
);

/// Support claim for the blessed snap v2 auxiliary schema.
pub const AUXILIARY_SCHEMA_SNAP_V2: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("aux-schema-snap-v2"),
    "hfx.aux.snap.v2",
    ReaderSupportValue::AuxiliarySchemaSnapV2,
);

/// Support claim for generic provisional and third-party auxiliary schemas.
pub const AUXILIARY_SCHEMA_GENERIC: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("aux-schema-generic"),
    "hfx.x.experimental.v1",
    ReaderSupportValue::AuxiliarySchemaGeneric,
);

/// Named non-support claim for the de-blessed D8 raster v1 auxiliary schema.
pub const AUXILIARY_SCHEMA_D8_RASTER_V1_UNSUPPORTED: ReaderSupportClaim = ReaderSupportClaim::new(
    ReaderSupportClaimId::new("aux-schema-d8-raster-v1-unsupported"),
    "hfx.aux.d8_raster.v1",
    ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported,
);

/// Auxiliary-schema classifications implemented by the manifest reader.
pub const AUXILIARY_SCHEMA_SUPPORT_CLAIMS: &[ReaderSupportClaim] = &[
    AUXILIARY_SCHEMA_D8_RASTER_V2,
    AUXILIARY_SCHEMA_SNAP_V2,
    AUXILIARY_SCHEMA_GENERIC,
    AUXILIARY_SCHEMA_D8_RASTER_V1_UNSUPPORTED,
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

/// Every claim inventory in this module must use a mechanically covered
/// `const` or `static` slice or array form and appear exactly once here.
///
/// The correspondence test mechanically covers only the documented token
/// patterns, so this convention does not extend its source-level boundary.
/// Topology and every other parse/store/expose/log-only field are outside the
/// catalog, which is bounded to production behavior that branches on values.
pub const READER_SUPPORT_CLAIM_INVENTORIES: &[&[ReaderSupportClaim]] = &[
    CORE_MANIFEST_SUPPORT_CLAIMS,
    FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS,
    AUXILIARY_SCHEMA_SUPPORT_CLAIMS,
    D8_METADATA_SUPPORT_CLAIMS,
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
        | ReaderSupportValue::D8FlowAccumulationUnits(_)
        | ReaderSupportValue::AuxiliarySchemaD8RasterV2
        | ReaderSupportValue::AuxiliarySchemaSnapV2
        | ReaderSupportValue::AuxiliarySchemaGeneric
        | ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported => None,
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
        | ReaderSupportValue::D8FlowAccumulationUnits(_)
        | ReaderSupportValue::AuxiliarySchemaD8RasterV2
        | ReaderSupportValue::AuxiliarySchemaSnapV2
        | ReaderSupportValue::AuxiliarySchemaGeneric
        | ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported => None,
    }
}

/// Returns the auxiliary-schema support claim for a parsed HFX schema ID.
pub fn claimed_auxiliary_schema(schema: &AuxiliarySchemaId) -> &'static ReaderSupportClaim {
    match schema {
        AuxiliarySchemaId::Blessed(BlessedAuxSchema::D8RasterV2) => &AUXILIARY_SCHEMA_D8_RASTER_V2,
        AuxiliarySchemaId::Blessed(BlessedAuxSchema::SnapV2) => &AUXILIARY_SCHEMA_SNAP_V2,
        AuxiliarySchemaId::Provisional(_) | AuxiliarySchemaId::ThirdParty(_) => {
            &AUXILIARY_SCHEMA_GENERIC
        }
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
        | ReaderSupportValue::D8FlowAccumulationUnits(_)
        | ReaderSupportValue::AuxiliarySchemaD8RasterV2
        | ReaderSupportValue::AuxiliarySchemaSnapV2
        | ReaderSupportValue::AuxiliarySchemaGeneric
        | ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported => None,
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
        | ReaderSupportValue::D8Crs(_)
        | ReaderSupportValue::AuxiliarySchemaD8RasterV2
        | ReaderSupportValue::AuxiliarySchemaSnapV2
        | ReaderSupportValue::AuxiliarySchemaGeneric
        | ReaderSupportValue::AuxiliarySchemaD8RasterV1Unsupported => None,
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

#[cfg(test)]
mod tests {
    use super::is_unreadable_d8_auxiliary_schema;

    #[test]
    fn unreadable_d8_routing_uses_the_exact_family_prefix() {
        assert!(is_unreadable_d8_auxiliary_schema("hfx.aux.d8_raster.v1"));
        assert!(is_unreadable_d8_auxiliary_schema("hfx.aux.d8_raster.v99"));
        assert!(!is_unreadable_d8_auxiliary_schema("hfx.aux.snap.v99"));
        assert!(!is_unreadable_d8_auxiliary_schema("hfx.aux.bogus.v9"));
        assert!(!is_unreadable_d8_auxiliary_schema("hfx.aux.d8-raster.v1"));
    }
}
