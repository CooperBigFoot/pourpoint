# Changelog

All notable changes to `pourpoint` (the CLI binary) and `pourpoint-core` (the engine crate) are documented here.

## Unreleased

### Rust API migration

- `resolve_outlet` and `resolve_outlet_at_level` remain available as deprecated
  wrappers returning the original public-field `ResolvedOutlet` struct. New code
  must use `resolve_outlet_authority` and `resolve_outlet_authority_at_level` to
  receive the typed `OutletResolution` authority sum.
- `TerminalRefinementInput.resolved_outlet` is intentionally replaced by
  `outlet_authority`. Custom strategy implementations must choose
  `OutletAuthority::VectorPoint(coord)` or `OutletAuthority::UnitOnly(coord)`;
  implicit coordinate conversion is not supported because it would erase the
  invariant this release adds.
- `algo::refine_terminal` and `algo::refine_terminal_from_source` now require
  an explicit `RasterOutlet`. Replace a former bare `NativeCoord` argument with
  `RasterOutlet::UnitOnly(coord)` for containment behavior or
  `RasterOutlet::VectorPoint(coord)` for authoritative vector behavior. The old
  implicit conversion is intentionally unavailable because it could silently
  recreate second resolution.
- `AppliedRefinementReason::D8AuxMatchedTerminalBbox` remains as a deprecated
  source bridge. Engine-produced results use `VectorOutletQuantized` or
  `RasterOutletRanked`; exhaustive matches must add those variants.
- `BestEffortSkipReason` adds `CoarseUnitOnlyNoD8AuxDeclared` and
  `VectorOutletGuardFailed`. Exhaustive matches must add both. The skip and
  aggregate provenance types now provide `PartialEq`, not `Eq`, because guard
  evidence includes raw `f32` accumulation values.
- `RefinementOutcome`, `TerminalRefinement`, and `TerminalRefinementDecision`
  now carry `AppliedRefinementProvenance` or
  `BestEffortRefinementProvenance` in their matching variants. Replace nested
  `RefinementProvenance::{Applied, BestEffortSkipped}` patterns with the typed
  wrapper's `strategy()` and `why()` accessors. The aggregate
  `RefinementProvenance` enum remains deprecated for record migration only.
- `Engine::refine_terminal` is the stable staged refinement method.
  `refine_terminal_placeholder` remains as a deprecated forwarding shim.

### Added

- Added typed vector-point versus unit-only outlet authority, raster seed-kind
  provenance, and rich vector-cell guard failures with threshold, mapped-cell,
  and measured-accumulation evidence.
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

- Vector-cell guarding now accepts HFX GRASS code 0 sinks and signed coverage
  exits as defined terminal semantics while retaining ESRI code 0 behavior.
- Vector-resolved outlets now remain authoritative through D8 refinement. They
  quantize only to their unique containing cell and never fall back to raster
  ranking. Unit-only containment retains the existing deterministic raster
  candidate rule.
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
