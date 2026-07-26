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
