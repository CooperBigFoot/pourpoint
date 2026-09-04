//! refineStrategy : TerminalPolygon × OutletAuthority × D8Pantry → TerminalRefinementDecision

use geo::{BoundingRect, Coord, LineString, MultiPolygon, Polygon};
use hfx::{D8RasterMetadataV2, EpsgCode, FlowAccumulationUnits, FlowDirEncoding, UnitId};
use object_store::path::Path as ObjectPath;

use crate::algo::coord::GeoCoord;
use crate::algo::projection::{Crs, NativeCoord, ProjectionError, forward, inverse};
use crate::algo::{
    GridCoord, RasterOutlet, RasterSeedKind, RasterSource, RasterSourceError, RefinementError,
    SnapThreshold, VectorOutletGuardFailureKind, refine_terminal_from_source,
};
use crate::error::SessionError;
use crate::resolver::OutletResolution;
use crate::session::{DatasetSession, RasterKind};
use crate::telemetry::{
    Stage, StageGuard, record_bytes, record_cache_status, record_path, record_requests,
};

/// Runs terminal-only geometry refinement using typed engine context.
pub trait TerminalRefinementStrategy: Send + Sync {
    /// Refine the terminal geometry or return a visible best-effort skip.
    ///
    /// # Errors
    ///
    /// | Variant | When |
    /// |---|---|
    /// | [`TerminalRefinementError::EmptyContainedTerminalGeometry`] | A D8 carve returns an empty geometry |
    /// | [`TerminalRefinementError::Algorithm`] | The underlying refinement algorithm fails |
    /// | [`TerminalRefinementError::RasterSource`] | A required raster source is absent |
    fn refine_terminal(
        &self,
        input: TerminalRefinementInput<'_>,
        pantry: &D8RefinementPantry<'_>,
    ) -> Result<TerminalRefinementDecision, TerminalRefinementError>;
}

/// Outlet authority narrowed to the coordinate information needed by refinement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutletAuthority {
    /// A vector feature chose this point and refinement may only quantize it.
    VectorPoint(GeoCoord),
    /// Containment chose only a unit and refinement may use the raster ranker.
    UnitOnly(GeoCoord),
}

impl From<&OutletResolution> for OutletAuthority {
    fn from(value: &OutletResolution) -> Self {
        match value {
            OutletResolution::VectorPoint { vector_coord, .. } => Self::VectorPoint(*vector_coord),
            OutletResolution::UnitContainment { input_coord, .. } => Self::UnitOnly(*input_coord),
        }
    }
}

/// Terminal-only input for a refinement strategy.
#[derive(Debug, Clone, Copy)]
pub struct TerminalRefinementInput<'a> {
    /// Terminal drainage-unit ID.
    pub terminal_unit: UnitId,
    /// Pre-merge whole-terminal geometry decoded by the staged path.
    pub terminal_geometry: &'a MultiPolygon<f64>,
    /// Typed outlet authority chosen before refinement.
    pub outlet_authority: OutletAuthority,
    /// Minimum flow accumulation used for ranking or the vector-cell guard.
    pub snap_threshold: SnapThreshold,
}

/// D8-specific context for terminal refinement.
///
/// This deliberately exposes only the dataset session and optional raster
/// source needed by the built-in D8 path. General auxiliary binding and custom
/// strategy authoring are outside the current runtime surface.
#[derive(Clone, Copy)]
pub struct D8RefinementPantry<'a> {
    /// Dataset session that owns declared HFX artifacts.
    pub session: &'a DatasetSession,
    /// Engine-attached raster source, if available.
    pub raster_source: Option<&'a (dyn RasterSource + Send + Sync)>,
}

/// Built-in terminal refinement strategy for declared HFX blessed-D8 rasters.
#[derive(Debug, Default, Clone, Copy)]
pub struct D8RasterRefinementStrategy;

impl TerminalRefinementStrategy for D8RasterRefinementStrategy {
    fn refine_terminal(
        &self,
        input: TerminalRefinementInput<'_>,
        pantry: &D8RefinementPantry<'_>,
    ) -> Result<TerminalRefinementDecision, TerminalRefinementError> {
        input
            .terminal_geometry
            .bounding_rect()
            .ok_or(TerminalRefinementError::Algorithm {
                unit_id: input.terminal_unit.get(),
                source: RefinementError::DegenerateTerminalPolygon,
            })?;

        let (handle, native_terminal) = pantry
            .session
            .select_d8_raster_for_terminal(input.terminal_geometry)
            .map_err(|source| TerminalRefinementError::D8Selection {
                unit_id: input.terminal_unit.get(),
                source,
            })?;
        let native_bbox =
            native_terminal
                .bounding_rect()
                .ok_or(TerminalRefinementError::Algorithm {
                    unit_id: input.terminal_unit.get(),
                    source: RefinementError::DegenerateTerminalPolygon,
                })?;

        let Some(raster_source) = pantry.raster_source else {
            return Ok(TerminalRefinementDecision::BestEffortSkipped {
                provenance: RefinementProvenance::BestEffortSkipped {
                    strategy: RefinementStrategyName::BestEffortD8IfPresent,
                    why: BestEffortSkipReason::NoRasterSourceProvided,
                },
            });
        };

        let flow_dir = {
            let _guard = StageGuard::enter(Stage::RasterLocalizeFlowDir);
            let flow_dir = pantry
                .session
                .localize_d8_raster_window(&handle, RasterKind::FlowDir, native_bbox)
                .map_err(|source| TerminalRefinementError::RasterLocalize {
                    unit_id: input.terminal_unit.get(),
                    kind: RasterKind::FlowDir,
                    source,
                })?;
            let bytes = flow_dir.header_bytes() + flow_dir.tile_bytes();
            record_bytes(bytes);
            record_requests(flow_dir.tile_count() as u64);
            record_cache_status(if bytes == 0 { "no_fetch" } else { "fetched" });
            record_path(flow_dir.path());
            flow_dir
        };
        let flow_acc = {
            let _guard = StageGuard::enter(Stage::RasterLocalizeFlowAcc);
            let flow_acc = pantry
                .session
                .localize_d8_raster_window(&handle, RasterKind::FlowAcc, native_bbox)
                .map_err(|source| TerminalRefinementError::RasterLocalize {
                    unit_id: input.terminal_unit.get(),
                    kind: RasterKind::FlowAcc,
                    source,
                })?;
            let bytes = flow_acc.header_bytes() + flow_acc.tile_bytes();
            record_bytes(bytes);
            record_requests(flow_acc.tile_count() as u64);
            record_cache_status(if bytes == 0 { "no_fetch" } else { "fetched" });
            record_path(flow_acc.path());
            flow_acc
        };
        tracing::debug!(
            flow_dir_cog_header_bytes = flow_dir.header_bytes(),
            flow_dir_cog_tile_bytes = flow_dir.tile_bytes(),
            flow_dir_cog_tile_count = flow_dir.tile_count(),
            flow_dir_window_pixels = flow_dir.window_pixels(),
            flow_acc_cog_header_bytes = flow_acc.header_bytes(),
            flow_acc_cog_tile_bytes = flow_acc.tile_bytes(),
            flow_acc_cog_tile_count = flow_acc.tile_count(),
            flow_acc_window_pixels = flow_acc.window_pixels(),
            declaration_index = handle.declaration_index(),
            "localized selected D8 raster windows for refinement"
        );
        let flow_dir_uri = flow_dir.path().to_string_lossy();
        let flow_acc_uri = flow_acc.path().to_string_lossy();
        let flow_dir_encoding = handle.flow_dir_encoding();
        let selected_crs = handle.projection_crs();
        let epsg = handle.epsg();
        let flow_accumulation_units = handle.flow_accumulation_units();
        let native_outlet = match input.outlet_authority {
            OutletAuthority::VectorPoint(vector_coord) => {
                RasterOutlet::VectorPoint(forward(selected_crs, vector_coord))
            }
            OutletAuthority::UnitOnly(input_coord) => {
                RasterOutlet::UnitOnly(forward(selected_crs, input_coord))
            }
        };

        let refinement_result = {
            let _refine_guard = StageGuard::enter(Stage::TerminalRefine);
            refine_terminal_from_source(
                raster_source,
                flow_dir_uri.as_ref(),
                flow_acc_uri.as_ref(),
                &native_terminal,
                native_outlet,
                input.snap_threshold,
                flow_accumulation_units,
                epsg,
                flow_dir_encoding,
            )
            .map_err(|source| TerminalRefinementError::Algorithm {
                unit_id: input.terminal_unit.get(),
                source,
            })?
        };

        let refined_outlet =
            inverse(selected_crs, refinement_result.snapped_coord()).map_err(|source| {
                TerminalRefinementError::Algorithm {
                    unit_id: input.terminal_unit.get(),
                    source: RefinementError::InverseProjection { epsg, source },
                }
            })?;
        let seed_kind = refinement_result.seed_kind();
        let geographic_polygon = inverse_terminal(&refinement_result.into_polygon(), selected_crs)
            .map_err(|source| TerminalRefinementError::Algorithm {
                unit_id: input.terminal_unit.get(),
                source: RefinementError::InverseProjection { epsg, source },
            })?;
        let geometry = ContainedTerminalPolygon::new_unchecked_from_d8_carve(geographic_polygon)
            .map_err(|_source| TerminalRefinementError::Algorithm {
                unit_id: input.terminal_unit.get(),
                source: RefinementError::EmptyPolygonization,
            })?;

        Ok(TerminalRefinementDecision::Applied {
            refined_outlet,
            geometry,
            provenance: RefinementProvenance::Applied {
                strategy: RefinementStrategyName::BuiltInD8,
                why: match seed_kind {
                    RasterSeedKind::VectorQuantized => {
                        AppliedRefinementReason::VectorOutletQuantized {
                            declaration_index: handle.declaration_index(),
                        }
                    }
                    RasterSeedKind::RasterRanked => AppliedRefinementReason::RasterOutletRanked {
                        declaration_index: handle.declaration_index(),
                    },
                },
            },
        })
    }
}

/// Typed handle for one selected blessed-D8 raster declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D8RasterHandle {
    declaration_index: usize,
    flow_dir_uri: String,
    flow_acc_uri: String,
    remote_flow_dir_path: Option<ObjectPath>,
    remote_flow_acc_path: Option<ObjectPath>,
    metadata: D8RasterMetadataV2,
    projection_crs: Crs,
    epsg: u32,
}

impl D8RasterHandle {
    /// Construct a D8 handle after path resolution and coverage checks pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        declaration_index: usize,
        flow_dir_uri: String,
        flow_acc_uri: String,
        remote_flow_dir_path: Option<ObjectPath>,
        remote_flow_acc_path: Option<ObjectPath>,
        metadata: D8RasterMetadataV2,
        projection_crs: Crs,
        epsg: u32,
    ) -> Self {
        Self {
            declaration_index,
            flow_dir_uri,
            flow_acc_uri,
            remote_flow_dir_path,
            remote_flow_acc_path,
            metadata,
            projection_crs,
            epsg,
        }
    }

    /// Return the zero-based declaration index from manifest order.
    pub fn declaration_index(&self) -> usize {
        self.declaration_index
    }

    /// Return the resolved flow-direction raster URI.
    pub fn flow_dir_uri(&self) -> &str {
        &self.flow_dir_uri
    }

    /// Return the resolved flow-accumulation raster URI.
    pub fn flow_acc_uri(&self) -> &str {
        &self.flow_acc_uri
    }

    /// Return the selected remote flow-direction object-store path, if remote.
    pub fn remote_flow_dir_path(&self) -> Option<&ObjectPath> {
        self.remote_flow_dir_path.as_ref()
    }

    /// Return the selected remote flow-accumulation object-store path, if remote.
    pub fn remote_flow_acc_path(&self) -> Option<&ObjectPath> {
        self.remote_flow_acc_path.as_ref()
    }

    /// Return the declared flow-direction encoding.
    pub fn flow_dir_encoding(&self) -> FlowDirEncoding {
        self.metadata.flow_dir_encoding()
    }

    /// Return the declared raster CRS.
    pub fn crs(&self) -> &EpsgCode {
        self.metadata.crs()
    }

    /// Return the parsed built-in projection selected at the session boundary.
    pub fn projection_crs(&self) -> Crs {
        self.projection_crs
    }

    /// Return the numeric declared EPSG identifier.
    pub fn epsg(&self) -> u32 {
        self.epsg
    }

    /// Return the declared flow-accumulation units.
    pub fn flow_accumulation_units(&self) -> FlowAccumulationUnits {
        self.metadata.flow_acc_units()
    }
}

fn inverse_terminal(
    terminal: &MultiPolygon<f64>,
    crs: Crs,
) -> Result<MultiPolygon<f64>, ProjectionError> {
    terminal
        .0
        .iter()
        .map(|polygon| {
            Ok(Polygon::new(
                inverse_ring(polygon.exterior(), crs)?,
                polygon
                    .interiors()
                    .iter()
                    .map(|ring| inverse_ring(ring, crs))
                    .collect::<Result<Vec<_>, ProjectionError>>()?,
            ))
        })
        .collect::<Result<Vec<_>, ProjectionError>>()
        .map(MultiPolygon::new)
}

fn inverse_ring(ring: &LineString<f64>, crs: Crs) -> Result<LineString<f64>, ProjectionError> {
    ring.0
        .iter()
        .map(|coordinate| {
            inverse(crs, NativeCoord::new(coordinate.x, coordinate.y)).map(|geographic| Coord {
                x: geographic.lon,
                y: geographic.lat,
            })
        })
        .collect::<Result<Vec<_>, ProjectionError>>()
        .map(LineString::new)
}

/// Refined terminal polygon produced by the built-in D8 carve.
///
/// The wrapper documents the shrink contract at the type boundary. It does not
/// enforce strict vector containment because polygonized raster output can
/// legitimately extend a fraction of a cell past the source terminal boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainedTerminalPolygon {
    polygon: MultiPolygon<f64>,
}

impl ContainedTerminalPolygon {
    /// Wrap a D8-carved terminal polygon after proving it is non-empty.
    ///
    /// # Errors
    ///
    /// | Variant | When |
    /// |---|---|
    /// | [`TerminalRefinementError::EmptyContainedTerminalGeometry`] | The carve produced no polygons |
    pub fn new_unchecked_from_d8_carve(
        polygon: MultiPolygon<f64>,
    ) -> Result<Self, TerminalRefinementError> {
        if polygon.0.is_empty() {
            return Err(TerminalRefinementError::EmptyContainedTerminalGeometry);
        }
        Ok(Self { polygon })
    }

    /// Return the wrapped polygon.
    pub fn polygon(&self) -> &MultiPolygon<f64> {
        &self.polygon
    }

    /// Consume the wrapper and return the wrapped polygon.
    pub fn into_polygon(self) -> MultiPolygon<f64> {
        self.polygon
    }
}

/// Strategy-level decision before engine policy is applied.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalRefinementDecision {
    /// A strategy produced a terminal override.
    Applied {
        /// Refined outlet coordinate at the selected raster seed cell center.
        refined_outlet: GeoCoord,
        /// Refined terminal geometry.
        geometry: ContainedTerminalPolygon,
        /// Provenance explaining why refinement ran.
        provenance: RefinementProvenance,
    },
    /// Best-effort refinement was visibly skipped.
    BestEffortSkipped {
        /// Provenance explaining why refinement was skipped.
        provenance: RefinementProvenance,
    },
}

/// Provenance for the terminal refinement stage.
#[derive(Debug, Clone, PartialEq)]
pub enum RefinementProvenance {
    /// Refinement was disabled by caller policy.
    Disabled,
    /// A strategy produced a terminal override.
    Applied {
        /// Strategy that produced the override.
        strategy: RefinementStrategyName,
        /// Reason the strategy applied.
        why: AppliedRefinementReason,
    },
    /// Best-effort refinement was visibly skipped.
    BestEffortSkipped {
        /// Strategy that was considered.
        strategy: RefinementStrategyName,
        /// Reason the strategy skipped.
        why: BestEffortSkipReason,
    },
}

/// Stable names for terminal refinement strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefinementStrategyName {
    /// Built-in D8 strategy.
    BuiltInD8,
    /// Convenience D8-if-present strategy.
    BestEffortD8IfPresent,
}

/// Reasons an applied refinement ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedRefinementReason {
    /// An authoritative vector point was mapped to its unique containing cell.
    VectorOutletQuantized {
        /// Zero-based declaration index in manifest order.
        declaration_index: usize,
    },
    /// Unit-only containment used the fixed raster candidate ranker.
    RasterOutletRanked {
        /// Zero-based declaration index in manifest order.
        declaration_index: usize,
    },
}

/// Operator-facing category for a best-effort D8 refinement skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestEffortSkipCategory {
    /// A required D8 declaration, artifact, cache object, or reader capability was unavailable.
    Availability,
    /// The D8 declaration or raster metadata contradicts the supported contract.
    MisDeclaration,
    /// Raster data, geometry, numerical projection, or an internal invariant rejected refinement.
    DataGeometryIntegrity,
}

/// Typed source family for a best-effort D8 refinement skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestEffortSkipSource {
    /// Selecting a covering D8 declaration failed.
    D8Selection,
    /// Localizing a selected raster window failed.
    RasterLocalization,
    /// Opening, reading, or constructing a raster tile failed.
    RasterLoad,
    /// The raster refinement algorithm rejected its inputs or output.
    RefinementAlgorithm,
    /// A strategy returned an empty contained-terminal geometry.
    ContainedTerminalGeometry,
    /// A strategy required an attached raster source.
    RasterSource,
}

/// Reasons best-effort refinement skipped.
#[derive(Debug, Clone, PartialEq)]
pub enum BestEffortSkipReason {
    /// Vector authority is retained coarsely because no blessed D8 pair is declared.
    NoD8AuxDeclared,
    /// Unit-only containment stays coarse because no blessed D8 pair is declared.
    CoarseUnitOnlyNoD8AuxDeclared,
    /// An unreadable declaration names the D8 auxiliary family.
    ///
    /// Eligibility is determined only by the declared name starting with
    /// `hfx.aux.d8_raster.`; no code verified that the referenced artifact is a
    /// D8 raster. `schema` is the first matching unreadable declaration in
    /// manifest order.
    UnreadableD8AuxDeclared {
        /// First matching unreadable schema name in manifest order.
        schema: String,
    },
    /// The engine has no raster source attached.
    NoRasterSourceProvided,
    /// The containing raster cell for vector authority failed a strict usability guard.
    VectorOutletGuardFailed {
        /// Failed guard conjunct.
        kind: VectorOutletGuardFailureKind,
        /// Requested threshold in upstream cells.
        requested_threshold: SnapThreshold,
        /// Effective threshold in the declared accumulation units.
        effective_threshold: f32,
        /// Declared accumulation units.
        units: FlowAccumulationUnits,
        /// Mapped cell when mapping succeeded.
        mapped_cell: Option<GridCoord>,
        /// Measured accumulation when the raster carried one.
        measured_accumulation: Option<f32>,
    },
    /// A required declaration, artifact, cache object, or reader capability was unavailable.
    Availability {
        /// Typed family in which the unavailable input was encountered.
        source: BestEffortSkipSource,
        /// Complete rendering of the underlying source error.
        diagnostic: String,
    },
    /// A declaration or raster metadata contradicted the supported D8 contract.
    MisDeclaration {
        /// Typed family in which the contract contradiction was encountered.
        source: BestEffortSkipSource,
        /// Complete rendering of the underlying source error.
        diagnostic: String,
    },
    /// Raster data, geometry, numerical projection, or an internal invariant rejected refinement.
    DataGeometryIntegrity {
        /// Typed family in which the data, geometry, or integrity failure was encountered.
        source: BestEffortSkipSource,
        /// Complete rendering of the underlying source error.
        diagnostic: String,
    },
}

impl BestEffortSkipReason {
    /// Return the operator-facing category of this skip.
    pub fn category(&self) -> BestEffortSkipCategory {
        match self {
            Self::NoD8AuxDeclared
            | Self::CoarseUnitOnlyNoD8AuxDeclared
            | Self::NoRasterSourceProvided
            | Self::Availability { .. } => BestEffortSkipCategory::Availability,
            Self::UnreadableD8AuxDeclared { .. } | Self::MisDeclaration { .. } => {
                BestEffortSkipCategory::MisDeclaration
            }
            Self::VectorOutletGuardFailed { .. } | Self::DataGeometryIntegrity { .. } => {
                BestEffortSkipCategory::DataGeometryIntegrity
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BestEffortSkipCategory, BestEffortSkipReason};

    #[test]
    fn best_effort_skip_categories_distinguish_unreadable_d8_from_absence() {
        assert_eq!(
            BestEffortSkipReason::UnreadableD8AuxDeclared {
                schema: "hfx.aux.d8_raster.v1".to_owned(),
            }
            .category(),
            BestEffortSkipCategory::MisDeclaration
        );
        assert_eq!(
            BestEffortSkipReason::NoD8AuxDeclared.category(),
            BestEffortSkipCategory::Availability
        );
    }
}

/// Errors from the terminal-refinement strategy seam.
#[derive(Debug, thiserror::Error)]
pub enum TerminalRefinementError {
    /// Fired when the D8 carve returns no terminal polygons.
    #[error("D8 terminal refinement produced an empty terminal geometry")]
    EmptyContainedTerminalGeometry,

    /// Fired when a required raster source is absent.
    #[error(
        "terminal refinement strategy {strategy:?} requires a raster source for unit {unit_id}"
    )]
    RasterSource {
        /// Terminal drainage-unit ID.
        unit_id: i64,
        /// Strategy that required raster access.
        strategy: RefinementStrategyName,
    },

    /// Fired when selecting the covering D8 declaration fails.
    #[error("failed to select D8 raster for terminal refinement (unit {unit_id}): {source}")]
    D8Selection {
        /// Terminal drainage-unit ID.
        unit_id: i64,
        /// Underlying session error.
        source: SessionError,
    },

    /// Fired when a selected D8 raster window cannot be localized.
    #[error(
        "failed to localize selected D8 {kind:?} raster for terminal refinement (unit {unit_id}): {source}"
    )]
    RasterLocalize {
        /// Terminal drainage-unit ID.
        unit_id: i64,
        /// Raster kind being localized.
        kind: RasterKind,
        /// Underlying session error.
        source: SessionError,
    },

    /// Fired when the underlying D8 refinement algorithm fails.
    #[error("terminal refinement algorithm failed for unit {unit_id}: {source}")]
    Algorithm {
        /// Terminal drainage-unit ID.
        unit_id: i64,
        /// Underlying algorithm error.
        source: RefinementError,
    },
}

pub(crate) fn best_effort_skip_reason(error: &TerminalRefinementError) -> BestEffortSkipReason {
    match error {
        TerminalRefinementError::EmptyContainedTerminalGeometry => {
            BestEffortSkipReason::DataGeometryIntegrity {
                source: BestEffortSkipSource::ContainedTerminalGeometry,
                diagnostic: error.to_string(),
            }
        }
        TerminalRefinementError::RasterSource { .. } => BestEffortSkipReason::Availability {
            source: BestEffortSkipSource::RasterSource,
            diagnostic: error.to_string(),
        },
        TerminalRefinementError::D8Selection { source, .. } => {
            best_effort_session_skip(source, BestEffortSkipSource::D8Selection)
        }
        TerminalRefinementError::RasterLocalize { source, .. } => {
            best_effort_session_skip(source, BestEffortSkipSource::RasterLocalization)
        }
        TerminalRefinementError::Algorithm { source, .. } => best_effort_refinement_skip(source),
    }
}

fn best_effort_session_skip(
    error: &SessionError,
    source: BestEffortSkipSource,
) -> BestEffortSkipReason {
    let diagnostic = error.to_string();
    match error {
        SessionError::MissingRequiredD8Aux
        | SessionError::NoCoveringD8Tile { .. }
        | SessionError::AmbiguousD8Coverage { .. }
        | SessionError::TerminalSpansD8Tiles { .. }
        | SessionError::CogExtentHeaderRead { .. }
        | SessionError::Cache(_) => BestEffortSkipReason::Availability { source, diagnostic },
        SessionError::D8CrsIdentifierOutOfRange { .. } | SessionError::UnsupportedD8Crs { .. } => {
            BestEffortSkipReason::MisDeclaration { source, diagnostic }
        }
        SessionError::IntegrityViolation { .. } => {
            BestEffortSkipReason::DataGeometryIntegrity { source, diagnostic }
        }
        _ => BestEffortSkipReason::DataGeometryIntegrity { source, diagnostic },
    }
}

fn best_effort_refinement_skip(error: &RefinementError) -> BestEffortSkipReason {
    let diagnostic = error.to_string();
    match error {
        RefinementError::VectorOutletUnusable { failure } => {
            BestEffortSkipReason::VectorOutletGuardFailed {
                kind: failure.kind,
                requested_threshold: failure.requested_threshold,
                effective_threshold: failure.effective_threshold,
                units: failure.units,
                mapped_cell: failure.mapped_cell,
                measured_accumulation: failure.measured_accumulation,
            }
        }
        RefinementError::DimensionMismatch { .. }
        | RefinementError::GeoTransformMismatch { .. }
        | RefinementError::GeographicKm2Unsupported { .. } => {
            BestEffortSkipReason::MisDeclaration {
                source: BestEffortSkipSource::RefinementAlgorithm,
                diagnostic,
            }
        }
        RefinementError::DegenerateTerminalPolygon
        | RefinementError::EmptyRasterMask { .. }
        | RefinementError::MaskFailed { .. }
        | RefinementError::SnapFailed { .. }
        | RefinementError::EmptyPolygonization
        | RefinementError::InverseProjection { .. } => {
            BestEffortSkipReason::DataGeometryIntegrity {
                source: BestEffortSkipSource::RefinementAlgorithm,
                diagnostic,
            }
        }
        RefinementError::RasterLoad { source } => {
            let diagnostic = source.to_string();
            match source {
                RasterSourceError::FileNotFound { .. }
                | RasterSourceError::OpenFailed { .. }
                | RasterSourceError::ReadFailed { .. }
                | RasterSourceError::EmptyWindow { .. } => BestEffortSkipReason::Availability {
                    source: BestEffortSkipSource::RasterLoad,
                    diagnostic,
                },
                RasterSourceError::InvalidFlowDirectionNodata { .. } => {
                    BestEffortSkipReason::MisDeclaration {
                        source: BestEffortSkipSource::RasterLoad,
                        diagnostic,
                    }
                }
                RasterSourceError::TileConstruction { .. } => {
                    BestEffortSkipReason::DataGeometryIntegrity {
                        source: BestEffortSkipSource::RasterLoad,
                        diagnostic,
                    }
                }
            }
        }
    }
}
