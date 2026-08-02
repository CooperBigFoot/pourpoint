# Changelog

All notable changes to `pourpoint` are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to
[PEP 440](https://peps.python.org/pep-0440/) versioning (decoupled from the workspace's
per-commit Rust crate versioning).

## [Unreleased]

### Added

- Added the top-level `pourpoint.BestEffortSkipReason` export and the typed
  `DelineationResult.refinement_skip_reason` accessor.

## [0.3.0] - 2026-07-31

### Added

- The public core surface now exposes prepared remote-window coverage through
  `LocalizedRasterWindow::coverage`, including the source raster grid, window
  offsets, ordered covered-tile indexes, covered-tile coordinates, and whether
  the selected tiles cross the X axis, Y axis, both axes, or neither.

### Changed

- The caller-supplied HFX D8 declaration's `flow_dir_encoding` is now the sole
  decoding authority. `RasterSource::load_flow_direction` receives
  `FlowDirEncoding`; reader-configured ESRI defaults and the encoding
  constructors were removed. External Rust `RasterSource` implementors and
  callers must update.
- Every built-in D8-path failure now produces a typed, diagnosable skip under
  `BestEffort`, while `RequireD8` remains fatal. Debug-formatted provenance
  from both the CLI and Python surfaces includes the complete source
  diagnostic.
- The fixed, file-independent decoded COG chunk ceiling increased from exactly
  `1_048_576` bytes to exactly `8_388_608` bytes. This is guard-headroom
  sizing: the ceiling is numerically large enough for 1024 x 1024 x 8 bytes
  while retaining positive headroom above 512 x 512 F32 tiles. It does not add
  F64 support to the shipped reader.
- The shipped remote COG reader now resolves bounded out-of-line TIFF ASCII
  metadata under a fixed 256-byte ceiling. This enables nodata declarations
  such as `-128` and `-2147483648`; longer declarations fail with typed
  `CacheError::RemoteTiffAsciiTooLong`. The reader normalizes the bounded
  declaration when writing the localized TIFF, and synthesizes `"-1"` when an
  F32 source has no nodata declaration.

### Fixed

- Out-of-bounds checked raster probes now return explicit absence instead of
  the tile's nodata value. A directional nodata sentinel can no longer turn a
  nonexistent neighbor into an upstream cell during D8 tracing, which can
  change delineation geometry.
- Flow-direction rasters are rejected before refinement when their header
  nodata byte decodes as a legal direction under the declared encoding. The
  typed diagnostic carries both the byte and encoding.
- Built-in D8 refinement now reports typed
  `RefinementError::DegenerateTerminalPolygon` before D8 declaration selection.
  Unsupported or out-of-range declared CRS errors no longer mask a degenerate
  terminal. A dataset with no D8 auxiliary still reports
  `SessionError::MissingRequiredD8Aux` through `Engine::delineate`, which
  short-circuits before refinement.
- Unwritten U8/I8 remote COG window cells are now filled with the declared
  nodata byte instead of legal direction code `0`, keeping missing tile
  coverage detectable.
- On every U8/I8 `decode_window` call, `direction_nodata_byte` parses the
  nodata declaration before window allocation and regardless of whether any
  output cell is unwritten. An unrepresentable declaration such as `"255.0"`
  therefore fails with `CacheError::UnsupportedCog` even when resolved tiles
  cover every output cell.

### Reader-fix scope authority

The production reader widening was a deliberate, reviewed change: commit
[`fa266ec`](https://github.com/CooperBigFoot/pourpoint/commit/fa266ecba836d164738cc8c2b04476e88379f143),
`feat(core): read bounded out-of-line TIFF ASCII (#108)`, implements the
support in the shipped reader and is contained in the `9d3e5cb` release
endpoint. The widening was authorized during the release as a mid-release
scope change. It fixes a capability defect in the released reader and
therefore belongs to the 0.3.0 release surface.

### Rust source compatibility

This inventory was computed across the literal range `68ac80c..9d3e5cb` over
`crates/core/src`, `crates/gdal/src`, and `crates/python/src`, with complete
items and enum blocks inspected at both endpoints and module/re-export
reachability traced. The approved graph's `crates/cli` path was not scanned
because it does not exist. The CLI is the root package, its `src/` contains
only `main.rs`, and it has no library target, so it exposes no Rust API to
downstream consumers.

The table contains 50 public-delta entries when definitions, enum variants,
methods, changed declarations, removals, and re-export entries are counted
separately. Derive-generated and handwritten trait implementations are
excluded from this count. The range adds one handwritten downstream-public
trait implementation,
`impl From<FlowDirectionTileError> for RasterSourceError`, and no others. The
range's only other added trait implementation is confined to a `#[cfg(test)]`
module.

| Public item | Delta | 0.2.1 | 0.3.0 | Consumer action |
|---|---|---|---|---|
| `pourpoint_core::algo::flow_direction_tile::FlowDirectionTileError` | Added | Absent | Public enum | Handle construction failure where applicable. |
| `FlowDirectionTileError::DirectionalNodata { nodata: u8, encoding: FlowDirEncoding }` | Added | Absent | Public variant | Exhaustive matches must add the variant. |
| `pourpoint_core::algo::FlowDirectionTileError` | Added re-export | Absent | Re-exported from `algo` | The shorter public path is available. |
| `FlowDirectionTile::from_raw` | **BREAKING** signature change | `(RasterTile<u8>, FlowDirEncoding) -> Self` | `(RasterTile<u8>, FlowDirEncoding) -> Result<Self, FlowDirectionTileError>` | Propagate or handle the result. |
| `RasterTile::get_checked` | **BREAKING** signature change | `(&self, isize, isize) -> T` | `(&self, isize, isize) -> Option<T>` | Handle explicit out-of-bounds absence. |
| `refine_terminal_from_source` | **BREAKING** signature change | Eight arguments ending in `epsg: u32` | Adds ninth argument `flow_dir_encoding: FlowDirEncoding` | Pass the HFX declaration's encoding. |
| `RasterSourceError::InvalidFlowDirectionNodata { nodata: u8, encoding: FlowDirEncoding }` | Added | Absent | Public variant | Exhaustive matches must add the variant. |
| `RasterSource::load_flow_direction` | **BREAKING** trait-method signature change | `(&self, &str, &Rect<f64>) -> Result<FlowDirectionTile<Raw>, RasterSourceError>` | Adds `encoding: FlowDirEncoding` before the return type | External implementors and callers must update. |
| `CacheError::RemoteTiffAsciiTooLong { path, tag, length, limit }` | Added | Absent | Public variant | Exhaustive matches must add the variant. |
| `CrossedTileAxes` | Added | Absent | Public enum | Use for prepared-window axis coverage. |
| `CrossedTileAxes::{Neither, X, Y, XAndY}` (4 entries) | Added | Absent | Four public variants | Exhaustive matches must cover all four. |
| `RasterWindowCoverage` | Added | Absent | Public struct with private fields | Read coverage through its accessors. |
| `RasterWindowCoverage::{origin_x, origin_y, pixel_width, pixel_height, raster_width, raster_height, tile_width, tile_height, window_col_off, window_row_off, covered_tile_indexes, covered_tile_col_row, crossed_axes}` (13 entries) | Added | Absent | Public accessors | No migration; use the needed observation. |
| `LocalizedRasterWindow::coverage` | Added | Absent | `(&self) -> Option<&RasterWindowCoverage>` | Account for `None` on legacy/internal cached construction. |
| Root re-exports `pourpoint_core::{CrossedTileAxes, RasterWindowCoverage}` (2 entries) | Added re-exports | Absent | Available at the crate root | Import from the crate root. |
| `BestEffortSkipCategory` | Added | Absent | Public enum | Use for operator-facing skip grouping. |
| `BestEffortSkipCategory::{Availability, MisDeclaration, DataGeometryIntegrity}` (3 entries) | Added | Absent | Three public variants | Exhaustive matches must cover all three. |
| `BestEffortSkipSource` | Added | Absent | Public enum | Use for the typed failure-source family. |
| `BestEffortSkipSource::{D8Selection, RasterLocalization, RasterLoad, RefinementAlgorithm, ContainedTerminalGeometry, RasterSource}` (6 entries) | Added | Absent | Six public variants | Exhaustive matches must cover all six. |
| `BestEffortSkipReason::{Availability { source, diagnostic }, MisDeclaration { source, diagnostic }, DataGeometryIntegrity { source, diagnostic }}` (3 entries) | Added variants | Enum had only `NoD8AuxDeclared` and `NoRasterSourceProvided` | Three diagnostic variants added | Exhaustive matches must add all three. |
| `BestEffortSkipReason::category` | Added | Absent | `(&self) -> BestEffortSkipCategory` | No migration; use for stable classification. |
| `TerminalRefinement::best_effort_skipped` | Added | Absent | `(BestEffortSkipReason) -> Self` | No migration; use to construct classified skips. |
| `EncodedLocalTiffRasterSource` | **BREAKING** removal | Public test-fixture source behind `test-fixtures` | Removed | Use `LocalTiffRasterSource` and pass encoding to `load_flow_direction`. |
| `LocalTiffRasterSource::with_encoding` | **BREAKING** removal | `(FlowDirEncoding) -> EncodedLocalTiffRasterSource` | Removed | Use the unit source directly and pass encoding per load. |
| `GdalRasterSource::with_encoding` | **BREAKING** removal | `(FlowDirEncoding) -> Self` | Removed | Configure GDAL separately and pass encoding per load. |

> No enum under `crates/` is `#[non_exhaustive]`, so exhaustive downstream Rust matches may require source changes.

#### Public-delta file audit

The checklist below follows every entry in the `git diff --name-status` file
list restricted to `crates/core/src`, `crates/gdal/src`, and `crates/python/src`.
Counts are syntactic public-delta entries and sum to the table's 50.

| Changed source file | Endpoint finding | Count |
|---|---|---:|
| `crates/core/src/algo/accumulation_tile.rs` | No added, removed, or signature-changed downstream public item; only adaptation to `RasterTile::get_checked`. | 0 |
| `crates/core/src/algo/flow_direction_tile.rs` | Added `FlowDirectionTileError`, its `DirectionalNodata` variant, and changed `FlowDirectionTile::from_raw`. | 3 |
| `crates/core/src/algo/mod.rs` | Added the `algo::FlowDirectionTileError` re-export. | 1 |
| `crates/core/src/algo/raster_tile.rs` | Changed `RasterTile::get_checked`. | 1 |
| `crates/core/src/algo/refine.rs` | Changed `refine_terminal_from_source`. | 1 |
| `crates/core/src/algo/trace.rs` | No added, removed, or signature-changed downstream public item; changes are regression proofs. | 0 |
| `crates/core/src/algo/traits.rs` | Added `RasterSourceError::InvalidFlowDirectionNodata` and changed `RasterSource::load_flow_direction`; the added `From<FlowDirectionTileError>` implementation is excluded by the stated methodology. | 2 |
| `crates/core/src/cog.rs` | Added `LocalizedRasterWindow::coverage`, `CrossedTileAxes` and four variants, `RasterWindowCoverage`, and its 13 public accessors. | 20 |
| `crates/core/src/engine.rs` | No added, removed, or signature-changed downstream public item; runtime handling and tests changed. | 0 |
| `crates/core/src/error.rs` | Added `CacheError::RemoteTiffAsciiTooLong`. | 1 |
| `crates/core/src/lib.rs` | Added two root re-exports: `CrossedTileAxes` and `RasterWindowCoverage`. | 2 |
| `crates/core/src/raster_cache.rs` | No added, removed, or signature-changed downstream public item; the module remains crate-private. | 0 |
| `crates/core/src/refinement.rs` | Added two enums, their nine variants, three `BestEffortSkipReason` variants, and `BestEffortSkipReason::category`. | 15 |
| `crates/core/src/staged.rs` | Added `TerminalRefinement::best_effort_skipped`. | 1 |
| `crates/core/src/test_raster_source.rs` | Removed `EncodedLocalTiffRasterSource` and `LocalTiffRasterSource::with_encoding` from the feature-gated public fixture surface. | 2 |
| `crates/gdal/src/raster_reader.rs` | Removed `GdalRasterSource::with_encoding`; the trait-implementation signature follows the trait row already counted in `traits.rs`. | 1 |
| **Total** | **Matches the compatibility table.** | **50** |

### Release-range audit

- Production behavior/API: `cd5aec9`, `594089f`, `55a41c8`, `766640d`,
  `8f4e71e`, `d5ed6d1`, `8c56f4d`, `fa266ec`, and `b947ceb`; each is
  represented in the release bullets or compatibility table above.
- Regression proof with no independent shipped behavior: `9303a91`,
  `d169162`, `447a7fb`, `0de969a`, `ef822fa`, and `6beacc5`.
- Documentation-only with no independent runtime bullet: `9ec267c`,
  `37a9bc8`, `ae1fc10`, and `ad2bd5d`.
- Merge-only with no independent runtime bullet: `03ecd1c`, `95603fa`,
  `fb24e5a`, `945e732`, `88b51f7`, `eca9274`, `9514d10`, `a3b9e81`,
  `1100e0d`, and `9d3e5cb`.

## [0.2.1] - 2026-07-26

Pourpoint 0.2.0 shipped on 2026-07-24 under tag `pourpoint-v0.2.0` and was
live-fired in production. Version 0.2.1 was subsequently cut under tag
`pourpoint-v0.2.1` at commit `68ac80c`.

### Changed

- Remote classic-TIFF and BigTIFF metadata reads no longer scale with the
  raster's total tile count. Extent selection reads only the required
  georeferencing tags, and planned-window reads resolve only the tile-index
  entries needed by the requested window.

### Fixed

- Bounded owned DEFLATE chunk decoding now handles predictor 1 without
  horizontal differencing and enforces the planned compressed, covered,
  decoded, and output-allocation limits.
- Projected-CRS and canonical-GRIT documentation now describes the shipped
  EPSG:4326 and EPSG:8857 behavior and the available D8 raster auxiliaries.

### Rust source compatibility

The release-preparation commit contains no Rust source changes. Relative to
`pourpoint-v0.2.0`, the public-enum diff through base commit
`b7e4dc3daf86aee7fff6e09ebd197ddd9bd0066c` contains no added variants and
exactly one removed variant:

| Public enum | Status | Added variants | Removed variants |
|---|---|---|---|
| `SessionError` | existing | None | `SessionError::CogExtentHeaderTooLarge` (REMOVED) |

No enum under `crates/` is marked `#[non_exhaustive]`. Downstream Rust code
with exhaustive matches may therefore require source changes.

## [0.2.0] - 2026-07-23

### Changed

- `hfx.aux.d8_raster.v1` is de-blessed. Opening a v1 dataset now fails with an
  error directing the user to recompile it with a v2-emitting adapter.
- Terminal refinement supports EPSG:8857 Equal Earth rasters without raster
  reprojection. Selection, carving, and snapping stay in the raster's native
  CRS; only the refined result is converted back to EPSG:4326.
- Public snap thresholds remain cell counts. For accumulation declared in
  `km2`, the comparison threshold is converted using projected pixel area.
  EPSG:4326 with `km2` is rejected rather than approximated.

### Fixed

- pourpoint 0.1.0 could return non-reproducible carve geometry for
  multi-component terminals, including different canonical geometry and
  polygon counts when components shared a vertex. At the fix's base commit, the
  separated-component probe had 15/15 repeats differ from the first, while the
  diagonal probe had 199/199 differing outputs, two canonical WKBs, and polygon
  counts of 6 or 7. Version 0.2.0 uses ordered ring-origin selection; both
  probes converged to one raw geometry across 200 in-process calls and 15
  separate processes.

### Rust source compatibility

The release-prep change contains no Rust source changes, so the public enum
surface at the prepared commit equals the surface derived at base commit
`d0637d41d3cb4b5421121dd31c604dfbec16f1ae` against
`pourpoint-v0.1.0`:

| Public enum | Status | Added variants |
|---|---|---|
| `RefinementError` | existing | `GeographicKm2Unsupported`, `InverseProjection` |
| `SessionError` | existing | `UnsupportedD8RasterV1`, `D8CrsIdentifierOutOfRange`, `UnsupportedD8Crs` |
| `ProjectionError` | new | `UnsupportedCrs`, `NonConvergence`, `OutOfDomain` |
| `Crs` | new | `Epsg4326`, `Epsg8857` |
| `InverseStage` | new | `Theta`, `GeodeticLatitude` |

None of these enums, or any enum under `crates/`, is marked
`#[non_exhaustive]`. Downstream Rust code with exhaustive matches may therefore
require source changes.

## [0.1.0] - 2026-07-07

- Established the first public `pourpoint` release and the compatibility
  baseline used for the 0.2.0 enum derivation.

## Legacy `pyshed` history

The entries below predate the `pourpoint` release stream. Their original
historical bullets are retained, while their headings are explicitly namespaced
to prevent their version numbers from being read as current pourpoint releases.

### [pyshed 0.3.0] - 2026-06-28

- **Requires HFX v0.3.0; older HFX format versions no longer load.** HFX v0.3.0
  stores catchment and snap bounding boxes as a GeoParquet 1.1 bbox covering
  struct (`bbox.{xmin,ymin,xmax,ymax}`) in place of four flat `bbox_*` columns,
  making HFX datasets first-class GeoParquet that standard spatial tools query
  with automatic row-group pruning.
- shed reads the covering struct's leaf row-group statistics directly via a
  predicate edit in the catchment and snap readers; the concurrency-64 range-read
  orchestration and the footer/row-group caches are unchanged. The optional
  geoarrow-rs catchments-geometry delegation was evaluated and deferred — its
  no-regression benchmark needs a live R2 covering dataset that was unavailable
  for this release — so 0.3.0 ships the predicate-read path alone.
- The hosted GRIT `2.0.0` example dataset is re-hosted at HFX v0.3.0 in lockstep
  with this release.

### [pyshed 0.2.4] - 2026-06-18

- fix: terminals covered by multiple overlapping per-Pfaf-02 D8 raster
  declarations (e.g. the MERIT v0.2.x global fabric) now select the
  manifest-first covering tile and carve under default refinement, instead of
  failing with `AmbiguousD8Coverage`. Overlapping declarations are windows of a
  single coherent D8 fabric and agree in the overlap, so the choice is
  immaterial.

### [pyshed 0.2.3] - 2026-06-07

- fix: `bench_trace` now flushes trace output on exit.
- typing: added `bench_trace` to the packaged type stubs.
- docs: README/API corrected to HFX v0.2.1 reality (grit/2.0.0, honest cold/warm
  open performance, `HFX_CACHE_DIR` + reuse-the-Engine guidance, geometry vs
  area-only delineation).

### [pyshed 0.2.2] - 2026-06-06

- perf: rebuild against the core validation sidecar and id-index reuse fix, so a
  warm open skips the full referential re-scan. Repeat opens are substantially
  faster once the cache is populated; first/cold open is unchanged because it
  populates the cache. No API or input-contract change; requires HFX v0.2.1
  datasets (unchanged from 0.2.1).

### [pyshed 0.2.1] - 2026-06-06

- perf: dataset open no longer reads the full catchment `geometry` column for
  referential validation (id/level-only projection). Cold open on large datasets
  is substantially faster — e.g. ~17s → ~9s on a 2.9M-unit local dataset, and a
  larger proportional win on remote datasets where the geometry column is many GB.
  The remaining open cost is the (still uncached) id-index build. No API or
  input-contract change; requires HFX v0.2.1 datasets (unchanged from 0.2.0).

### [pyshed 0.2.0] - 2026-06-05

- requires HFX v0.2.1; v0.1 no longer loads
- new: staged sub-function API + unit-bundle GeoParquet export.
- Added `LevelSelection.FINEST` and the explicit
  `Engine.select_level(selection=...)` parameter.

### [pyshed 0.2.0rc3] - 2026-06-05

- requires HFX v0.2.1; v0.1 no longer loads
- new: staged sub-function API + unit-bundle GeoParquet export.

### [pyshed 0.1.11] - 2026-05-06

### Changed

- Enabled remote parquet cache defaults for faster repeated startup and range
  access against hosted HFX datasets.
- Added persistent ID indexes with validation sidecars, plus row-group and
  footer caching to reduce repeated parquet metadata work.
- Improved benchmark harness telemetry for outlet selection and search-radius
  behavior.
- Defaulted the `repair_geometry` kwarg to clean topology and selected the
  dissolve strategy from benchmark results.

### [pyshed 0.1.10] - 2026-05-04

### Changed

- Updated the root README examples to use the canonical public GRIT HFX
  v1.0.0 dataset at
  `https://basin-delineations-public.upstream.tech/grit/1.0.0/`.

### Added

- Added `AreaOnlyResult` via `Engine.delineate(..., geometry=False)` for callers
  that only need scalar delineation metadata and area.
- Added `DelineationResult.geometry_bbox` and cached repeated
  `DelineationResult.geometry_wkb` property access.
- Documented `pyshed.Engine(...)` dataset strings for local paths, `file://`,
  `s3://`, and Cloudflare R2 HTTPS URLs, plus remote manifest/graph caching via
  `HFX_CACHE_DIR` and parquet range-read behavior.

### [pyshed 0.1.7] - 2026-04-21

### Changed

- Reverted the experimental Linux manylinux wheel setup. `pyshed` is again
  published as an Apple Silicon macOS-only wheel while Linux support is left
  open for future community contribution.

### [pyshed 0.1.6] - 2026-04-21

### Fixed

- Quoted the Linux `LDFLAGS` assignment in the cibuildwheel environment so the
  manylinux job parses correctly. This fixes the immediate `Malformed
  environment option` failure seen in `0.1.5` before the Linux build even
  started.

### [pyshed 0.1.5] - 2026-04-21

### Fixed

- Corrected the Linux manylinux wheel stack builder to handle `lib64` installs
  from CMake projects like PROJ while still preferring `lib` where explicitly
  requested. This fixes the failed `0.1.4` Linux wheel build before wheel
  repair.

### [pyshed 0.1.4] - 2026-04-21

### Added

- Added Linux x86_64 wheel builds via cibuildwheel's `manylinux2014` image,
  alongside the existing Apple Silicon macOS wheel.
- Added Linux wheel verification with `auditwheel show`, an `ldd` dependency
  check against the repaired wheel, and a clean-container import smoke test.

### Changed

- Documented Linux x86_64 as a supported wheel platform in the package README
  and metadata.

### [pyshed 0.1.3] - 2026-04-20

### Changed

- Default `snap_strategy` is now `"weight-first"` (was `"distance-first"`). Fixes small-basin correctness where an outlet coincident with a tiny tributary stub's first vertex resolved to a ~0.08 km² headwater instead of the ~9000 km² mainstem. Aligns pyshed with the HFX v0.2 weight contract.

### Opt-out

- Pass `snap_strategy="distance-first"` to `Engine(...)` or `Engine.delineate(...)` to keep the v0.1.2 behavior.

### [pyshed 0.1.2] - 2026-04-18

### Added
- Shipped PEP 561 typing metadata in the wheel via `pyshed/__init__.pyi` and
  `pyshed/py.typed`, so IDE hover, autocomplete, and static type checking now
  work against the public Python API.
- Added a developer-oriented API reference in `crates/python/API.md`
  documenting the exported classes, return types, properties, and exceptions.

### Changed
- Corrected the batch-delineation README example to match the real API shape:
  `Engine.delineate_batch()` accepts outlet dicts with `"lat"` and `"lon"`
  keys.

### [pyshed 0.1.1] - 2026-04-17

### Changed
- Locked GDAL's cmake dependency discovery to the wheel build prefix and passed
  explicit PROJ, TIFF, SQLite, GEOS, and curl hints to reduce accidental
  linkage against runner-local libraries.
- Added a delocate preflight step that inspects install names with `otool`
  before repair, plus an unrepaired-wheel `delocate-listdeps` dump ahead of
  `delocate-wheel`.
- Seeded bundled `GDAL_DATA` and `PROJ_DATA` in `pyshed.__init__` before
  importing `_pyshed`, while keeping the existing PyO3 runtime injection as a
  belt-and-suspenders fallback. `_set_proj_data()` now also sets the `PROJ_DATA`
  GDAL config option before calling `OSRSetPROJSearchPaths`.

### [pyshed 0.1.0] - 2026-04-17

First public release on PyPI. Apple Silicon macOS only (`macosx_11_0_arm64`);
community contributions for Linux / Intel / Windows are welcome — see
[CONTRIBUTING.md](https://github.com/CooperBigFoot/shed/blob/main/CONTRIBUTING.md).

### Added
- `pyshed.Engine(path).delineate(lat, lon)` and `.delineate_batch(outlets)`.
- `DelineationResult` with `geometry_wkb`, `to_geojson()`, area, and snap info.
- Typed exception hierarchy rooted at `ShedError` (`DatasetError`,
  `ResolutionError`, `AssemblyError`).
- Bundled native stack inside the wheel: GDAL 3.12.1, PROJ 9.7.1, GEOS 3.14.1,
  libtiff 4.7.1, SQLite, zlib, libcurl, nghttp2, OpenSSL, libpng, jpeg-turbo,
  zstd, libdeflate, xz. All 14 licenses shipped under
  `pyshed-0.1.0.dist-info/licenses/`.
- Runtime injection of bundled `GDAL_DATA` and `proj.db` via `CPLSetConfigOption`
  and `OSRSetPROJSearchPaths` at module import time.

### `pyshed` pre-release history

#### [pyshed 0.1.0rc4] - 2026-04-17
Dropped `PROJ_RENAME_SYMBOLS` — PROJ's cmake renames its own symbols but not
libgeod's, so GDAL's preprocessor rewrote `geod_init` → `internal_geod_init`
against a PROJ that didn't export the renamed names.

#### [pyshed 0.1.0rc3] - 2026-04-17
Fixed build order: `build_tiff` must run before `build_proj`; PROJ 9.7's cmake
requires TIFF.

#### [pyshed 0.1.0rc2] - 2026-04-17
Removed a top-level `permissions: actions: read` block that was stripping
`contents: read` and causing `actions/checkout` to fail on the private repo.

#### [pyshed 0.1.0rc1] - 2026-04-17
Initial TestPyPI dry run.
