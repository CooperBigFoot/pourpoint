# Changelog

All notable changes to `pourpoint` (the CLI binary) and `pourpoint-core` (the engine crate) are documented here.

## Unreleased

### Added

- Accepted public R2 custom-domain dataset roots at
  `https://basin-delineations-public.upstream.tech/...`.
- Documented remote HFX dataset locations backed by the object-store
  integration, including local paths, `file://`, `s3://`, Cloudflare R2 HTTPS
  URLs, manifest/graph cache behavior, `HFX_CACHE_DIR`, and parquet range
  reads.

### Changed

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
