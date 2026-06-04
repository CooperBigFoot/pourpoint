# M5 Basin GeoParquet Export Plan

This folds basin GeoParquet export into **M5 - Rust Core API Cutover And Campaign Boundary**. It is additive to the Rust core public API stabilization work and should land after `DelineationResult` and provenance accessor names are stable.

This revision incorporates the critique in `docs/hfx-v02-redesign/m5-export-critique.md`. The two escalations are resolved for executor planning:

- `delineation` identity uses source **fabric data version** from `Manifest::fabric_version()`, not `adapter_version()`.
- Hilbert ordering remains required, but shed must own and document explicit curve parameters because HFX v0.2.1 does not define them.

## Orientation Findings

- M5 stabilizes Rust core result types, public terminology, telemetry/error names, and migration docs before the campaign closes.
- `DelineationResult` exposes the export surface: `terminal_unit_id()`, `resolved_outlet()`, `resolution_method()`, `upstream_unit_ids()`, `refinement()`, `geometry()`, `area_km2()`, and `geometry_wkb()`.
- `RefinementOutcome` already has `Applied`, `BestEffortSkipped`, and `Disabled`, so provenance maps directly to documented string states.
- `encode_wkb_multi_polygon` writes 2D WKB and `DelineationResult` stores `MultiPolygon<f64>`, so `geometry_types: ["MultiPolygon"]` is honest.
- `DatasetSession::manifest()` exposes both `fabric_version() -> Option<&str>` and `adapter_version() -> &str`; these are distinct and must not be aliased.
- The reusable Parquet write precedent is the `WriterProperties` plus `EnabledStatistics::Chunk` pattern in fixture writers, including the test helper in `crates/core/src/assembly.rs`. There is no production basin-export writer to mirror.
- HFX v0.2.1 requires bbox columns, row-group statistics, and Hilbert-style spatial ordering in prose, but its validator currently defers Hilbert curve parameters. Shed must not claim conformance to an HFX-defined curve.

## Format Status

This is a documented `shed` export format, not a versioned open spec. It must be code-against-able and accepted by standard GeoParquet readers, but M5 does not add a spec crate, validator, conformance suite, or external version negotiation.

If another producer or external conformer appears, elevate `docs/basin-geoparquet-export.md` into a versioned open spec with compatibility rules, fixtures, and conformance tests.

## GeoParquet Schema

One row represents one `(basin_id, delineation)` pair: a caller-owned basin catalog entry delineated by one source fabric/method.

| Column | Arrow type | Nullable | Required | Description |
|---|---|---:|---:|---|
| `basin_id` | `Utf8` | No | Yes | Caller-supplied basin identity, unique per basin within the run and filesystem-safe for HDX `basin=<id>` directories. A terminal-unit default exists only for narrow single-fabric catalogs. |
| `delineation` | `Utf8` | No | Yes | Label for the source fabric data version and method that produced the geometry, for example `grit/2.0.0/d8-best-effort`. |
| `geometry` | `Binary` | No | Yes | OGC WKB `MultiPolygon`, 2D, EPSG:4326. The dissolved result from `DelineationResult::geometry_wkb()`. |
| `outlet_lon` | `Float64` | No | Yes | Resolved outlet longitude in EPSG:4326. |
| `outlet_lat` | `Float64` | No | Yes | Resolved outlet latitude in EPSG:4326. |
| `area_km2` | `Float64` | No | Yes | Final geodesic drainage area from the dissolved watershed. |
| `bbox_minx` | `Float32` | No | Yes | Geometry bounding box west, rounded outward. |
| `bbox_miny` | `Float32` | No | Yes | Geometry bounding box south, rounded outward. |
| `bbox_maxx` | `Float32` | No | Yes | Geometry bounding box east, rounded outward. |
| `bbox_maxy` | `Float32` | No | Yes | Geometry bounding box north, rounded outward. |
| `resolution_method` | `Utf8` | Yes | Optional provenance | Exported display/debug form of `ResolutionMethod`. |
| `refinement_status` | `Utf8` | Yes | Optional provenance | One of `applied`, `best_effort_skipped`, or `disabled`. |
| `upstream_unit_ids` | `List<Int64>` | Yes | Optional provenance | Dataset-local HFX unit IDs included in traversal, including the terminal. Audit-only; not cross-fabric identity. |
| `adapter_version` | `Utf8` | Yes | Optional provenance | Adapter/tooling version from `Manifest::adapter_version()`. This is provenance, not the identity-bearing fabric data version. |

The default documented profile includes the optional provenance columns. A minimal profile may omit all optional provenance columns. The file must not store `hilbert_index`; it is only a sort key.

Empty input is an error in M5. A zero-row GeoParquet file creates ambiguous metadata and no HDX value for the "delineate once" use case. Sharded empty outputs can be revisited later if a real pipeline needs them.

## GeoParquet Footer Metadata

The writer must put the `geo` JSON into Parquet file-level `key_value_metadata`, not only Arrow schema metadata.

Required mechanism in parquet 58:

- build a `parquet::format::KeyValue` or equivalent with `key = "geo"` and `value = Some(serialized_geo_json)`;
- attach it through `WriterProperties::builder().set_key_value_metadata(Some(vec![geo_kv]))` before constructing `ArrowWriter`, or call `ArrowWriter::append_key_value_metadata(geo_kv)` before `close()`;
- after `close()`, tests must reopen the written Parquet file, read `file_metadata().key_value_metadata()`, and assert the `geo` entry from the footer.

The `geo` JSON declares:

- `version`: `1.1.0`
- `primary_column`: `geometry`
- `columns.geometry.encoding`: `WKB`
- `columns.geometry.geometry_types`: `["MultiPolygon"]`
- `columns.geometry.crs`: PROJJSON for EPSG:4326 with `id.authority = "EPSG"` and `id.code = 4326`
- `columns.geometry.bbox`: dataset-level `[minx, miny, maxx, maxy]` in f64, covering all row geometries

Do not rely on absent-CRS default semantics. `orientation` is omitted intentionally in M5; winding is unspecified in the export contract unless a later gate proves the dissolved geometry is consistently RFC7946-oriented.

Do not declare GeoParquet `covering.bbox` in M5. The export keeps HFX-style flat `bbox_minx`/`bbox_miny`/`bbox_maxx`/`bbox_maxy` columns as ordinary attributes with Parquet statistics. GeoParquet 1.1 covering metadata is optional and its canonical examples use struct child paths; adding flat-column covering pointers without target-reader proof risks invalidating otherwise usable `geo` metadata.

## Basin Identity And Delineation Semantics

`basin_id` is parsed at the boundary into a `BasinId` newtype. Use an allowlist, not a blocklist:

- regex: `^[A-Za-z0-9._-]+$`
- length: 1-128 bytes
- reject `.` and `..`
- reject case-insensitive Windows device names: `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`
- reject trailing `.` or trailing space
- reject `=` by construction through the regex, because HDX uses `basin=<id>` path segments

`basin_id` must be unique per physical basin within a run. Duplicate `(basin_id, delineation)` rows are always an error. Reusing the same `basin_id` with different `delineation` labels is valid and is the expected multi-fabric representation for the same physical basin.

Defaulting is intentionally narrow:

- only allowed for an explicitly single-fabric export mode;
- uses `DelineationResult::terminal_unit_id()` formatted as a decimal string;
- because `UnitId` is signed `i64`, defaulting rejects negative terminal IDs rather than emitting a leading `-`;
- only safe when the catalog has at most one outlet per terminal unit.

If two defaulted rows collide, the writer must raise a specific default-ID collision error that names both originating outlets and tells the caller to supply explicit `basin_id` values. It must not surface as a generic duplicate row.

`delineation` is sourced from source fabric data identity:

- `fabric_name = manifest.fabric_name()`
- `fabric_version = manifest.fabric_version()`
- `method = ExportMethod`

Default label format: `{fabric_name}/{fabric_version}/{method}`. If `fabric_version()` is `None`, the default label constructor errors and requires the caller to provide an explicit `DelineationLabel` or explicit fabric data version. `adapter_version()` may be exported as optional provenance but must not drive `delineation`.

`delineation` is a column value, not a filesystem path segment. It is not validated with the `BasinId` filesystem allowlist, and labels may contain separators such as `/` because HDX does not partition on `delineation` in this contract. If a future layout partitions by delineation, it needs a separate path-safe label type.

## Rust Core API Shape

Place the export API in `crates/core/src/export/` and re-export stable public entry points from `shed_core` only after M5 result names have settled.

Proposed types:

- `BasinId`: parsed newtype enforcing the allowlist identity rules.
- `DelineationLabel`: parsed newtype for the `delineation` column.
- `FabricIdentity`: value object sourced from `hfx_core::Manifest`, carrying `fabric_name`, required `fabric_version` for default labels, and optional `adapter_version` provenance.
- `ExportMethod`: enum or parsed label for the method portion of `delineation`.
- `ExportOrigin`: caller/outlet context used in diagnostics, especially default-ID collisions.
- `BasinExportInput<'a>`: explicit `BasinId` plus borrowed `DelineationResult`, identity, method, and origin; or a single-fabric default-ID variant.
- `BasinGeoParquetWriter`: batch writer that accepts parsed inputs and writes one GeoParquet file.
- `ExportOptions`: provenance inclusion, row-group target, optional explicit label override, and CLI/test toggles. Defaults: provenance included, target row group 8,192 rows.
- `ExportError`: `thiserror` enum with doc-commented named-field variants for invalid IDs, missing fabric version, default-ID collision, duplicate rows, geometry encoding, bbox/centroid failure, Arrow/Parquet write failure, empty input, footer-metadata failure, and row-group planning failure.

The API is batch-oriented and separate from `Engine::delineate()`. The hot delineation path returns results; export is a persistence step.

## Module Placement

Add `crates/core/src/export/mod.rs` with small submodules only if needed:

- `identity.rs`: `BasinId`, `DelineationLabel`, `FabricIdentity`, `ExportMethod`, `ExportOrigin`.
- `schema.rs`: Arrow schema and GeoParquet footer metadata construction.
- `spatial.rs`: bbox, outward f32 rounding, centroid, and shed-owned Hilbert index.
- `row_groups.rs`: balanced 4,096-8,192 row-group planning.
- `writer.rs`: row materialization, sorting, Arrow array construction, footer metadata, and Parquet write.

Keep this module outside `engine.rs`, `assembly.rs`, `resolver`, `refinement`, and staged contracts. The engine should not know whether its result is later written. A CLI command may be added only after the core writer is green. Python/pyshed exposure is deferred.

## Spatial Ordering, Bbox, And Row Groups

### Bbox Columns

Compute each row bbox from the dissolved `MultiPolygon<f64>`. Convert to `Float32` with outward rounding:

- `bbox_minx` and `bbox_miny`: round toward negative infinity at f32 precision;
- `bbox_maxx` and `bbox_maxy`: round toward positive infinity at f32 precision.

This prevents row-group pruning from dropping a geometry whose true f64 bounds were narrowed by nearest f32 conversion. The dataset-level `geo.bbox` remains f64 and must cover all true row bboxes.

### Shed Hilbert Parameters

HFX v0.2.1 does not define Hilbert curve parameters. M5 therefore documents a shed-owned ordering:

- curve extent: fixed global EPSG:4326 extent `[-180.0, -90.0, 180.0, 90.0]`;
- input point: centroid of the dissolved `MultiPolygon`;
- bit depth: 16 bits per axis;
- axis mapping: longitude maps to x, latitude maps to y;
- normalization: clamp finite lon/lat to the global extent before quantization;
- quantization: map min extent to `0`, max extent to `(2^16 - 1)`;
- sort key: `HilbertIndex(u32)`;
- tie-breaks: `(hilbert_index ASC, basin_id ASC, delineation ASC)`.

This is not "mirroring" HFX. It is a documented shed export choice that preserves basin-stable ordering across runs because it does not normalize against the file-level bbox. Add a small test with hand-computed expected indices for known centroids.

### Row Groups

For fewer than 4,096 rows, write exactly one row group.

For 4,096 or more rows, plan balanced row groups where every group is between 4,096 and 8,192 rows, including the final group. Use:

- `group_count = ceil(row_count / 8192)`;
- distribute rows evenly across `group_count` groups, with the first `remainder` groups receiving one extra row;
- feed one Arrow `RecordBatch` per planned group and call `ArrowWriter::flush()` after each group, or otherwise prove the written metadata has the planned boundaries.

This avoids a 50k-row export ending with an 848-row tail. It also reconciles the literal HFX row-group sentence without inventing an untested final-group exemption.

Enable `EnabledStatistics::Chunk` in `WriterProperties`, adapting the existing fixture-writer pattern, and assert bbox column statistics in the written file metadata.

## Documented Format Deliverable

Add `docs/basin-geoparquet-export.md` during implementation. Outline:

1. Status: documented shed format, not a versioned spec.
2. Purpose: delineate once, extract many; one dataset-level outlines file for HDX.
3. Required schema table with Arrow types and nullability.
4. Optional provenance columns and default profile.
5. Geometry and CRS: WKB MultiPolygon, EPSG:4326, GeoParquet footer metadata.
6. `basin_id` allowlist, caller ownership, default terminal-unit fallback, and collision warning.
7. `delineation` label from `fabric_name`, `fabric_version`, and method; `adapter_version` as provenance only; label is a column value, not a path segment.
8. Bbox columns with outward rounding.
9. Shed-owned Hilbert parameters and deterministic tie-breaks.
10. Balanced row-group sizing and bbox statistics.
11. Minimal example with two rows for the same `basin_id` and different `delineation`.
12. Elevation path to a versioned open spec.

## M5 Step Decomposition

These steps slot into M5 after result type stabilization and before final migration-doc closure. Each step must end green and must not touch the M1 canonicalizer/goldens, M3 staged contracts, or M4 carve/refinement boundary.

### Step M5.E1 - Document Export Contract

Create `docs/basin-geoparquet-export.md` from this plan. Do not add implementation.

Gate:

```bash
rg "fabric_version|adapter_version|not a versioned spec|GeoParquet footer|Hilbert" docs/basin-geoparquet-export.md
```

### Step M5.E2 - Add Export Identity Types

Add `BasinId`, `DelineationLabel`, `FabricIdentity`, `ExportMethod`, and `ExportOrigin` under `crates/core/src/export/`, with parsing at boundaries and no writer yet.

Gate:

```bash
cargo test -p shed-core export_identity
```

Required tests: allowlist acceptance, rejection of unsafe/reserved IDs, rejection of `=`, rejection of trailing dot, caller-supplied valid ID, missing `fabric_version()` default-label error, adapter-version-as-provenance only, default terminal-unit ID formatting, negative terminal-unit default rejection, and same `BasinId` allowed with distinct `DelineationLabel` values.

### Step M5.E3 - Add Spatial Utility

Add bbox, outward f32 rounding, centroid, and shed-owned Hilbert utility as internal export helpers. Keep them independent of Parquet.

Gate:

```bash
cargo test -p shed-core export_spatial
```

Required tests: bbox correctness, outward rounding contains a non-representable f64 bbox, deterministic Hilbert order, hand-computed expected Hilbert indices for known centroids, tie-break stability, fixed-global-extent stability when unrelated rows are added, and explicit error for empty or centroid-less geometry.

### Step M5.E4 - Add Schema And Footer Metadata Builder

Add Arrow schema construction and GeoParquet `geo` footer metadata JSON construction. Do not include a `covering.bbox` block in M5.

Gate:

```bash
cargo test -p shed-core export_schema
```

Required tests: exact field names/order/types/nullability, optional provenance inclusion/exclusion, serialized `geo` JSON with primary geometry column, WKB encoding, `MultiPolygon` geometry type, EPSG:4326 PROJJSON id, dataset-level bbox, no `covering.bbox` block, and omitted orientation.

### Step M5.E5 - Add Row-Group Planner

Add balanced row-group planning independent of Parquet writing.

Gate:

```bash
cargo test -p shed-core export_row_groups
```

Required tests: tiny file produces one group, 4,096 produces one group, 8,193 produces two legal groups, about 9,000 produces legal balanced groups, and 50,000 produces no short tail.

### Step M5.E6 - Add Batch GeoParquet Writer

Implement `BasinGeoParquetWriter` using `ArrowWriter`, footer `geo` key/value metadata, chunk statistics, explicit row-group flushing, and sorted rows.

Gate:

```bash
cargo test -p shed-core export_writer
```

Required tests: write/read round trip asserting schema and values, footer-level `geo` metadata read back through Parquet metadata, bbox column statistics present, planned row-group sizes appear in written metadata, Hilbert-sorted output order, empty input error, duplicate `(basin_id, delineation)` rejection, duplicate `basin_id` with different `delineation` accepted, and default-ID collision emits the specific two-origin diagnostic.

### Step M5.E7 - Add Small Golden Export Fixture

Commit a tiny deterministic GeoParquet fixture under `crates/core/tests/fixtures/export/` or the nearest existing fixture convention. Keep it synthetic and small.

Gate:

```bash
cargo test -p shed-core export_golden
```

Required tests: fixture reads back with standard Parquet/Arrow path, footer `geo` metadata is present, required values match expected JSON or inline constants, Hilbert order is stable, bbox values cover true geometry bounds, and no M1/M3/M4 fixture paths are touched.

### Step M5.E8 - Optional CLI Emit Command

Only if it is a thin wrapper over the core writer, add a CLI command under existing `src/main.rs` command structure. If the caller catalog shape is not already settled, defer this step.

Gate:

```bash
cargo test --workspace --exclude pyshed cli_export
```

Required tests if implemented: parse all `BasinId` values before the expensive delineation loop, map `ExportError` into `anyhow` with useful context, reject unsafe IDs before delineation, write one output file, and preserve the core writer schema and footer metadata. This step must be skipped rather than invented if it requires a new catalog format.

### Step M5.E9 - M5 Closure Update

Update M5 migration docs to mention the additive export surface and deferrals: no versioned spec, no conformance suite, no pyshed export API, no Python-authored strategies.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test -p shed-core
rg "atom|Atom" crates/core/src crates/core/README.md docs/hfx-v02-redesign
rg "basin GeoParquet|pyshed.*deferred|not a versioned spec" docs
```

The `atom` grep keeps M5's existing allowlist requirement. Export docs must not introduce new public atom terminology.

## Test Strategy

- Round-trip: write a small export, read it back with Parquet/Arrow, assert schema, row count, values, and footer `geo` metadata.
- GeoParquet proof: parse the written footer `geo` key and assert `version`, `primary_column`, `encoding`, `geometry_types`, EPSG:4326 PROJJSON id, dataset-level bbox, absence of `covering.bbox`, and omitted orientation.
- Golden fixture: commit one small deterministic GeoParquet export fixture and assert it remains readable and semantically stable.
- Hilbert ordering: assert hand-computed index values and persisted row order.
- Bbox correctness: assert per-row bbox columns outwardly contain true f64 geometry bounds and dataset-level GeoParquet bbox covers all rows.
- Row groups/statistics: inspect Parquet metadata for planned row-group sizes and bbox column statistics, including an awkward realistic count.
- Basin IDs: test caller-supplied IDs, allowlist rejection, default terminal-unit fallback, negative default rejection, default collision diagnostic, duplicate rejection, and same physical basin represented by same `basin_id` across two delineation labels.
- Provenance: assert optional columns serialize `resolution_method`, `refinement_status`, `upstream_unit_ids`, and `adapter_version` when enabled and disappear cleanly when disabled.

## Boundaries

- Do not alter delineation behavior, staged contracts, canonicalizer/goldens, or M4 carve/refinement logic.
- Do not add a versioned spec, validator, or conformance suite in M5.
- Do not expose Python/pyshed export in M5 unless it is mechanically free after the Rust writer exists; default stance is deferred.
- Do not claim the Hilbert curve is HFX-defined. It is a shed-owned documented export parameter until HFX specifies one.
- Every implementation commit still needs the mandatory patch bump, staged `Cargo.toml`/`Cargo.lock` as applicable, conventional commit, and tag.
