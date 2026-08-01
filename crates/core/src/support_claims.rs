//! reader support claims : declared reader values -> production support obligations.
//!
//! Claims pair canonical on-disk declarations with the HFX domain values used
//! by production. Stable claim IDs are correspondence keys for independent
//! shipped-path evidence.

use hfx::{Crs, FormatVersion};

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

/// Implemented declarations that govern admission of the HFX core manifest.
pub const CORE_MANIFEST_SUPPORT_CLAIMS: &[ReaderSupportClaim] =
    &[FORMAT_VERSION_V0_3_0, DATASET_CRS_EPSG_4326];

/// Returns the typed format version for an implemented declaration.
pub fn claimed_format_version(declaration: &str) -> Option<FormatVersion> {
    if declaration != FORMAT_VERSION_V0_3_0.canonical_declaration() {
        return None;
    }
    match FORMAT_VERSION_V0_3_0.value() {
        ReaderSupportValue::FormatVersion(value) => Some(*value),
        ReaderSupportValue::DatasetCrs(_) => None,
    }
}

/// Returns the typed dataset CRS for an implemented declaration.
pub fn claimed_dataset_crs(declaration: &str) -> Option<Crs> {
    if declaration != DATASET_CRS_EPSG_4326.canonical_declaration() {
        return None;
    }
    match DATASET_CRS_EPSG_4326.value() {
        ReaderSupportValue::DatasetCrs(value) => Some(*value),
        ReaderSupportValue::FormatVersion(_) => None,
    }
}
