# Changelog

All notable changes to `pourpoint` (the CLI binary) and `pourpoint-core` (the engine crate) are documented here.

## Unreleased

### Rust API migration

- `LevelResolvedOutlet::resolved()` remains available as a deprecated legacy
  view returning `&ResolvedOutlet`, so existing public-field access and struct
  patterns continue to compile. New staged callers use
  `LevelResolvedOutlet::authority()` to receive `&OutletResolution` with typed
  vector-versus-containment provenance and terminal-unit authority, not fixed
  raster-cell authority.
- `resolve_outlet` and `resolve_outlet_at_level` remain available as deprecated
  wrappers returning the original public-field `ResolvedOutlet` struct. New code
  must use `resolve_outlet_authority` and `resolve_outlet_authority_at_level` to
  receive typed `OutletResolution` with terminal-unit and reference provenance.
- `TerminalRefinementInput.outlet_authority` is replaced by `outlet_reference`.
  Custom strategies use `OutletReference::VectorPoint(coord)` or
  `OutletReference::UnitOnly(coord)` to identify the proximity reference source.
  `OutletAuthority` remains a deprecated type alias. Callers migrating from the
  older `resolved_outlet` field must also select the reference source explicitly.
- `algo::refine_terminal` and `algo::refine_terminal_from_source` now take a
  raster-native `NativeCoord` proximity reference directly. Replace
  `RasterOutlet::{VectorPoint, UnitOnly}(coord)` with `coord`; `RasterOutlet` is
  removed. Both resolution paths use terminal-constrained raster ranking.
- `RasterSeedKind` now has only `RasterRanked`. Remove `VectorQuantized` matches.
  `AppliedRefinementReason::VectorOutletQuantized` and
  `D8AuxMatchedTerminalBbox` remain deprecated historical compatibility variants.
  Built-in results always use `RasterOutletRanked`, including vector resolution.
- `algo::RefinementError::VectorOutletUnusable` and `VectorOutletGuardFailure`
  are removed; update exhaustive matches and custom guard handling.
  `VectorOutletGuardFailureKind` remains for historical skip evidence.
  `snap_pour_point` accepts finite references outside the raster window and adds
  `SnapError::NonFiniteReference`; update exhaustive matches. The old
  `OutletOutOfBounds` variant remains for compatibility.
- The vector-cell guard no longer participates in refinement. Legacy
  `BestEffortSkipReason::VectorOutletGuardFailed` evidence remains readable but
  is not emitted by the built-in engine. No-candidate failures use normal raster
  selection diagnostics, with coarse fallback only in best-effort mode.
- `RefinementOutcome`, `TerminalRefinement`, and `TerminalRefinementDecision`
  now carry `AppliedRefinementProvenance` or
  `BestEffortRefinementProvenance` in their matching variants. Replace nested
  `RefinementProvenance::{Applied, BestEffortSkipped}` patterns with the typed
  wrapper's `strategy()` and `why()` accessors. The aggregate
  `RefinementProvenance` enum remains deprecated for record migration only.
- `Engine::refine_terminal` is the stable staged refinement method.
  `refine_terminal_placeholder` remains as a deprecated forwarding shim.

### Added

- Added typed vector-point versus containment outlet references and explicit
  raster-ranked seed provenance.
- Added an ignored, explicitly blessed local-current-HFX MERIT recapture target
  that rejects stale D8 v1 input and records exact HFX and adapter versions
  without publishing licensed raster or geometry data.
- Accepted public R2 custom-domain dataset roots at
  `https://basin-delineations-public.upstream.tech/...`.
- Reader floor: pourpoint 0.3.0 for the GRIT address offered by this repository,
  derived from the 0.3.0 format and GRASS decoding entries in
  `crates/core/src/support_claims.rs`.
- Documented remote HFX dataset locations backed by the object-store
  integration, including local paths, `file://`, `s3://`, Cloudflare R2 HTTPS
  URLs, manifest/graph cache behavior, `HFX_CACHE_DIR`, and parquet range
  reads.

### Changed

- D8 candidate selection preserves HFX GRASS code 0 sinks and signed coverage
  exits as defined terminal semantics while retaining ESRI code 0 behavior.
- Raster refinement now ranks usable threshold-qualified cells throughout the
  selected terminal unit against the vector snap point, or request point under
  containment. Ties use higher accumulation, then row-major order. References
  outside the mask or window remain valid proximity references. There is no
  containing-cell shortcut, new search radius, or branch constraint.
  `resolved_outlet` remains the vector/request reference; applied `refined_outlet`
  is the selected cell center. Terminal and upstream units remain unchanged.
  Best-effort failures visibly retain the coarse terminal; required D8 errors.
  This supersedes the unreleased fixed-vector-cell behavior, not vector ranking
  or disabled-refinement semantics. Historical evidence is unchanged.
- Best-effort refinement now distinguishes and carries the first retained
  unreadable D8-family schema. The new public, exhaustive
  `BestEffortSkipReason::UnreadableD8AuxDeclared` variant is a breaking Rust
  source change for downstream exhaustive matches.
- Read out-of-line TIFF ASCII metadata through the remote COG reader with a
  fixed 256-byte ceiling, enabling GDAL nodata values such as `-128` and
  `-2147483648`.
- Filled unwritten U8/I8 remote COG window cells with each raster's declared
  nodata sentinel instead of direction code `0`, so missing tile coverage
  remains detectable.
- Raised the fixed, file-independent decoded COG chunk ceiling from 1 MiB to
  8 MiB, covering a 1024 x 1024 float64 tile while retaining positive
  headroom above 512 x 512 F32 tiles.
- The built-in D8 refinement strategy
  (`D8RasterRefinementStrategy::refine_terminal`) now rejects degenerate input
  terminal geometry with `RefinementError::DegenerateTerminalPolygon` before
  attempting D8 declaration selection, so an unsupported or out-of-range
  declared CRS no longer masks it. A dataset that declares no D8 auxiliary
  still reports `SessionError::MissingRequiredD8Aux` through
  `Engine::delineate`, which short-circuits before refinement.
- Every built-in D8-path failure is now a diagnosable typed skip under
  `BestEffort`, while `RequireD8` remains fatal. CLI and Python debug-formatted
  provenance now includes the complete source diagnostic.
- Rejected flow-direction rasters before refinement when their header nodata
  byte decodes as a legal direction under the declared encoding, with a typed
  diagnostic carrying the byte and encoding.
- Made checked raster probes return explicit absence outside tile bounds, so
  directional nodata sentinels cannot turn nonexistent neighbors into upstream
  cells during D8 tracing.
- Made the HFX D8 declaration's `flow_dir_encoding` the sole decoding
  authority by passing it through `RasterSource::load_flow_direction`.
  Removed reader-configured ESRI defaults and encoding constructors. This is
  a breaking change for external `RasterSource` implementors.

## 0.1.56 — 2026-04-20

### Changed

- Default snap strategy flipped from `SnapStrategy::DistanceFirst` to `SnapStrategy::WeightFirst` to align with HFX v0.2. This fixes a small-basin correctness bug where outlets coincident with a tiny tributary stub's first vertex resolved to a ~0.08 km² headwater instead of the ~9000 km² mainstem.
- Bumped `hfx-core` pin from `=0.1.26` to `=0.2.0`.

### Opt-out

- Legacy distance-first behavior remains available via `--snap-strategy distance-first` (CLI) or `snap_strategy="distance-first"` (Python). Use for datasets whose `weight` column is not hydrologically rank-meaningful.
