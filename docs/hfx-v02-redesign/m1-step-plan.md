# M1 Step Plan - Durable v0.1 Parity Oracle

Milestone 1 captures today's working v0.1 engine behavior as inert committed
artifacts before any HFX v0.2.1 loader, naming, dependency, or staged-pipeline
work begins. The oracle captures the code; it must not bend the engine to make a
golden pass.

## Assumptions

- `../hfx/spec/HFX_SPEC.md` remains the canonical contract for later v0.2.1
  fixtures, but M1 captures current shed v0.1 behavior only.
- Current shed is `0.1.109` on `main`, and `cargo build -p shed-core` is green
  before M1 starts.
- The current v0.1 engine path is `resolve -> traverse -> try_refine -> assemble
  -> compose`; M1 must not change that behavior.
- `grit/1.0.0` is a pinned v0.1 remote dataset with `has_rasters=false`, so
  real-data oracle A truthfully captures `RefinementOutcome::NoRastersAvailable`.
- `merit-basins/0.1.0` is a pinned v0.1 remote dataset with real D8 rasters, so
  real-data oracle C can capture `RefinementOutcome::Applied` only when a raster
  source is attached to the current engine.
- Today's production real-raster decode path is `GdalRasterSource`, attached by
  `pyshed`; `shed-core` has no production raster source. M1 keeps the mandatory
  `shed-core` offline gates GDAL-free, but any `tiff`/`cog.rs` test source used
  for B or C must be proven tile-identical to GDAL on representative B and C
  windows before those goldens are blessed.
- `DatasetBuilder::with_rasters()` writes stub bytes and cannot drive a real
  carve. The offline refined oracle B therefore needs a committed synthetic v0.1
  fixture with real `flow_dir.tif` and `flow_acc.tif` bytes.
- `parity_golden_artifacts` must survive M2. It must read committed golden bytes
  only, with no `DatasetBuilder`, no `DatasetSession`, no `Engine`, and no
  `AtomId` or other v0.1 `hfx-core` type dependency.

## Ordered Executor Steps

1. Define the canonical golden schema, geometry normalizer, and offline artifact harness.
2. Add the committed synthetic raster fixture and local TIFF test raster source.
3. Capture offline refined synthetic goldens for oracle B.
4. Capture network-gated pinned real-data goldens for oracles A and C.
5. Harden oracle invariants and close the final M1 gates.

Each step is an independent commit boundary. Every executor step must:

```bash
./scripts/bump-version.sh patch
git add Cargo.toml <step files>
git commit -m "<conventional commit message>"
git tag v$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
```

Do not let tooling create its own commit or tag. M1 does not touch `pyshed`.

## Step 1 - Golden Schema And Canonical Artifact Harness

**Goal**

Define the loader-independent golden contract and the single canonical geometry
normalizer that every later capture and parity comparison must use.

**Scope**

- Add a canonical geometry normalizer in `shed-core` library code, for example
  `crates/core/src/algo/canonical_wkb.rs`, and expose it through the narrowest
  existing module path.
- Add `crates/core/tests/parity_golden_artifacts.rs`.
- Add fixture documentation under `crates/core/tests/fixtures/parity/`.
- Add only lightweight dev dependencies if unavoidable.
- Non-behavioral `serde` derives or thin test-side mappers are allowed for
  golden serialization.

The normalizer must define and test:

- Coordinate precision.
- Ring closure representation.
- Exterior and interior ring orientation.
- Ring start vertex as a total order: lexicographically smallest rounded
  `(x, y)`, then adjacent-vertex sequence as the tie-break when duplicate
  rounded coordinates exist.
- Hole ordering as a total order: normalized ring bbox, then signed area, then
  full rounded vertex sequence.
- Polygon/component ordering as a total order: normalized exterior bbox, then
  area, then hole count, then full rounded exterior and hole vertex sequences.
- Canonical little-endian 2D WKB emission.
- Idempotence: canonical WKB decoded and normalized again must produce identical
  canonical WKB.
- Degenerate idempotence cases: duplicate vertices after rounding and
  equal-primary-key multi-component geometries.

Antimeridian-crossing geometries are out of scope for M1 because the chosen A,
B, and C outlets are far from ±180 degrees.

The golden schema must use version-neutral names: `terminal_id` and
`upstream_ids`, not `terminal_atom_id` or `upstream_atom_ids`. Required golden
fields are canonical final geometry WKB, `area_km2`, input outlet, resolved
outlet, refined outlet when `Applied`, terminal ID, sorted upstream ID set,
resolution method, resolver configuration including search radius, refinement
outcome, canonicalizer version, and scalar comparison policy. The
canonicalizer-version note must state either that goldens were validated on at
least two architectures, or that the chosen coordinate precision has enough
margin that observed pre-rounding divergence cannot flip rounded coordinates;
`area_km2` uses the scalar epsilon policy, not byte-exact equality.

**Out-Of-Scope Guard**

No engine behavior changes, no v0.2 loader work, no `hfx-core` migration, no
atom-to-unit API rename, no production raster backend, and no dependency on
`DatasetBuilder` or the v0.1 loader in `parity_golden_artifacts`.

**Concrete Verification**

Offline:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_golden_artifacts
git status --short
```

The artifact test may start with an empty-golden or tiny seed-golden path, but it
must already prove schema validation, canonical WKB idempotence, and
loader-independent execution.

**Dependencies**

None.

**Commit Obligation**

Patch bump, stage `Cargo.toml` and Step 1 files, conventional commit, tag
`v<version>`.

## Step 2 - Synthetic Raster Fixture And Local TIFF Source

**Goal**

Create the offline refined fixture path: a committed v0.1 dataset with real TIFF
rasters and a test-support raster source that can drive current refinement from
those bytes after tile-identity proof against GDAL.

**Scope**

- Add a small committed synthetic v0.1 fixture under
  `crates/core/tests/fixtures/parity/v01_synthetic_refined/` containing
  `manifest.json`, `catchments.parquet`, `graph.arrow`, `flow_dir.tif`, and
  `flow_acc.tif`.
- Add a `test-fixtures`-gated in-crate local TIFF `RasterSource` adapter that
  reuses `crates/core/src/cog.rs` GeoTIFF metadata/sample interpretation instead
  of duplicating parsing logic.
- Use the existing `tiff = "0.9"` path and `TiffEncoder` where practical.
- Add the self dev-dependency bridge so canonical no-feature integration-test
  commands compile the gated adapter:

```toml
[dev-dependencies.shed-core]
path = "."
features = ["test-fixtures"]
```

- Add a smoke test in `crates/core/tests/parity_v01_oracle_capture.rs` that opens
  the committed fixture from disk, attaches the test TIFF source, runs one
  `Engine::delineate`, and proves the current engine returns
  `RefinementOutcome::Applied` with a strict terminal shrink:
  `0 < refined_area < terminal_area`, plus refined bbox contained by terminal
  bbox.
- Add or plan an isolated `shed-gdal` test target that proves the Step 2 TIFF
  source decodes at least one B fixture window tile-identically to
  `GdalRasterSource`: same `u8` flow-direction tile, same `f32` accumulation
  tile, same nodata handling, and same geotransform. This proof is mandatory for
  blessing B, but it must live outside `shed-core` or behind an explicit GDAL
  feature so `cargo build -p shed-core` and `cargo test -p shed-core ...` remain
  GDAL-free.
- Record raster interpretation metadata in the fixture README: dimensions,
  origin, pixel size, north-up transform, CRS, sample types, nodata, flow-dir
  encoding, and PixelIsArea/PixelIsPoint handling.

**Out-Of-Scope Guard**

Do not use `DatasetBuilder` at read time. Do not use stub `.tif` bytes. Do not
add GDAL to the offline compile graph. Do not mark
`parity_v01_oracle_capture` with `required-features`, because that would skip the
target while reporting green. Do not broaden this into a general production
raster reader. If tile identity with `GdalRasterSource` cannot be proven, stop
and replan B/C capture around an isolated GDAL capture path; do not bless a
`tiff`-reader-only oracle as today's production raster behavior.

**Concrete Verification**

Offline:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test parity_v01_oracle_capture synthetic_fixture_smoke
git status --short
```

The smoke test must fail loudly, not skip, if the TIFF adapter is not compiled
under the canonical command.

GDAL-isolated proof, outside the mandatory offline core gate:

```bash
cargo test -p shed-gdal --test raster_decode_parity synthetic_b_tiff_matches_gdal -- --ignored --nocapture
```

If the exact test name differs, document the exact command in the fixture README.

**Dependencies**

Step 1 green.

**Commit Obligation**

Patch bump, stage `Cargo.toml` and Step 2 files, conventional commit, tag
`v<version>`.

## Step 3 - Offline Refined Synthetic Goldens

**Goal**

Capture oracle B as committed offline goldens by running the unmodified current
v0.1 core engine against the committed synthetic raster-bearing fixture, using a
TIFF source already proven tile-identical to the GDAL production decoder for the
B window.

**Scope**

- Extend `parity_v01_oracle_capture` with a read-only comparison path for B.
- Add an explicit bless mode for generation, for example:

```bash
SHED_PARITY_BLESS=1 cargo test -p shed-core --test parity_v01_oracle_capture bless_synthetic_refined -- --nocapture
```

- Before blessing each B golden, run at least three single-thread
  `delineate()` calls for the same case and assert stable canonical WKB plus
  stable scalar fields under the Step 1 policy. Do not use `delineate_batch*`.
- Commit B golden JSON under
  `crates/core/tests/fixtures/parity/goldens/v01_synthetic_refined/`.
- Include fixture byte sizes and hashes for `manifest.json`,
  `catchments.parquet`, `graph.arrow`, `flow_dir.tif`, and `flow_acc.tif`.
  The v0.1-format input hashes are inert provenance only; the durable artifact
  test must not re-hash `manifest.json`, `catchments.parquet`, or `graph.arrow`
  after capture.
- Include the full required golden field set and the raster interpretation
  metadata from Step 2.
- Extend `parity_golden_artifacts` so it validates B goldens from disk without
  opening the v0.1 dataset.
- Record that the Step 2 TIFF decode was proven tile-identical to GDAL for the B
  fixture window before the B golden was blessed.

**Out-Of-Scope Guard**

No hand-authored golden values. No engine tuning if the current output looks
wrong or surprising. If repeated current-engine output is unstable, stop and
escalate instead of changing engine behavior. Do not mutate or relocate the
committed M1 B fixture path during later milestones; M2 must create its v0.2.1
fixture as a separate copy that reuses the exact `.tif` bytes.

**Concrete Verification**

Offline:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_v01_oracle_capture synthetic_stability_check
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test parity_v01_oracle_capture
git status --short
```

The bless command is run once to create or refresh B goldens; the default command
must then pass as a read-only comparison.

**Dependencies**

Steps 1 and 2 green.

**Commit Obligation**

Patch bump, stage `Cargo.toml` and Step 3 files, conventional commit, tag
`v<version>`.

## Step 4 - Network-Gated Real-Data Goldens

**Goal**

Capture oracle A from pinned `grit/1.0.0` and oracle C from pinned
`merit-basins/0.1.0`, while keeping all default M1 gates offline-green.

**Scope**

Oracle A:

- Use exactly
  `https://basin-delineations-public.upstream.tech/grit/1.0.0/`.
- Capture at least `zurich` at `GeoCoord::new(8.5417, 47.3769)` and
  `repparfjord` at `GeoCoord::new(23.04, 69.97)`.
- Include `hammerfest` at `GeoCoord::new(23.6821, 70.6634)` only with an
  explicit larger search radius, recommended `5000 m`, or document why it is
  excluded.
- Build the engine with no raster source. Expected refinement outcome is
  `NoRastersAvailable`.
- Record pinned URL plus ETag/byte identity or byte length plus content hash for
  `manifest.json`, `catchments.parquet`, and `graph.arrow`.

Oracle C:

- Use exactly
  `https://basin-delineations-public.upstream.tech/merit-basins/0.1.0/`.
- Attach the Step 2 TIFF raster source; `bench_delineate` is not a sufficient
  template because it does not attach a raster source.
- Start from exactly two candidate refined outlets, then accept them only if live
  measurement proves they are small-window cases:
  `rhine_basel` at `GeoCoord::new(7.5890, 47.5596)` with `5000 m` search radius,
  and `mekong_phnom_penh` at `GeoCoord::new(104.9300, 11.5700)` with `5000 m`
  search radius.
- Before blessing either C outlet, measure and record its terminal bbox,
  localized flow-dir tile count/bytes, localized flow-acc tile count/bytes,
  total `Engine::http_stats().total_bytes_in`, and configured search radius.
  The measured values, not the outlet names, are the smallness justification.
  Replace any outlet whose measured values exceed the hard ceiling.
- Assert `RefinementOutcome::Applied { refined_outlet }` and strict
  containment-clamped carve metadata: `0 < refined_area < terminal_area` and
  refined bbox contained by terminal bbox.
- Assert a hard C windowing ceiling before blessing and during network capture:
  per case, total raster-localization bytes must be far below full-raster size.
  Recommended ceiling is `500 MB` total bytes-in per outlet unless Step 4 records
  a tighter measured ceiling after live capture. A run that exceeds the ceiling
  fails even if the final geometry is correct.
- Record pinned URL plus ETag/byte identity or byte length plus content hash for
  `manifest.json`, `catchments.parquet`, `graph.arrow`, and `snap.parquet`.
  For C rasters specifically, record remote multipart ETag verbatim plus
  `Content-Length` from HEAD only; do not compute content hashes for
  `flow_dir.tif` or `flow_acc.tif`.
- Record the MERIT raster interpretation contract: remote COG source, localized
  plain north-up EPSG:4326 GeoTIFF window, PixelIsArea, ESRI flow-direction
  encoding, `uint8` flow direction, `float32` accumulation, and nodata policy.
- Prove the localized C windows decode tile-identically through the Step 2 TIFF
  source and `GdalRasterSource` before blessing C. Record the proof result in the
  C fixture README and golden metadata. With this proof, C is scoped as
  "core TIFF-reader carve proven tile-identical to the GDAL production decode,"
  not an unproven substitute for today's production raster reader.

For A and C, the network bless path must run at least three single-thread
`delineate()` calls per case before writing goldens. The default test path must
compile and skip network-only execution unless the env gate is present.

Recommended network command:

```bash
SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-core --test parity_v01_oracle_capture -- --ignored --nocapture
```

**Out-Of-Scope Guard**

Never download or commit whole remote datasets or the large MERIT raster mosaics.
The engine must COG-window only terminal-bbox tiles for C. No work against
`grit/2.0.0`. No M1 use of `merit/0.2.0`; that is an M4 handoff target. No GDAL
fallback may enter the default compile graph. If the Step 2 TIFF adapter cannot
read localized C windows, escalate; do not hide GDAL behind only `#[ignore]` or an
env var, because ignored tests still compile. Do not content-hash the large C
raster mosaics.

**Concrete Verification**

Offline:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test parity_v01_oracle_capture
git status --short
```

Network-gated refresh/proof:

```bash
SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-core --test parity_v01_oracle_capture -- --ignored --nocapture
```

If the final implementation uses a different env/test-name shape, document the
exact command in the test and fixture README, while keeping the offline command
above green without network.

GDAL-isolated C decode proof, network and GDAL gated:

```bash
SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-gdal --test raster_decode_parity merit_c_windows_tiff_match_gdal -- --ignored --nocapture
```

This proof must not be implemented as an unconditional `shed-core` dev
dependency on `shed-gdal`.

**Dependencies**

Steps 1, 2, and 3 green.

**Commit Obligation**

Patch bump, stage `Cargo.toml` and Step 4 files, conventional commit, tag
`v<version>`.

## Step 5 - Oracle Hardening And Final Gates

**Goal**

Make the oracle difficult to misuse after M2 by enforcing required cases,
required metadata, loader independence, and command discipline.

**Scope**

- Harden `parity_golden_artifacts` so it fails if any required A, B, or C golden
  is missing.
- Verify all goldens have canonical WKB, `area_km2`, input outlet, resolved
  outlet, refined outlet when applied, terminal ID, sorted upstream IDs,
  resolution method, resolver configuration including search radius, refinement
  outcome, canonicalizer metadata, and scalar comparison metadata.
- Verify A records `NoRastersAvailable` and identity for `manifest.json`,
  `catchments.parquet`, and `graph.arrow`.
- Verify B records committed fixture hashes and byte-identical `flow_dir.tif` /
  `flow_acc.tif` metadata for later v0.2.1 fixture reuse.
- Re-check against disk only the M2-immutable B `flow_dir.tif` and
  `flow_acc.tif` bytes at their committed M1 fixture path. Treat B
  `manifest.json`, `catchments.parquet`, and `graph.arrow` hashes as inert
  provenance only; do not re-hash those v0.1 files in the must-survive
  `parity_golden_artifacts` test.
- Verify B and C record strict carve metadata.
- Verify C records identity for `manifest.json`, `catchments.parquet`,
  `graph.arrow`, `snap.parquet`, `flow_dir.tif`, and `flow_acc.tif`.
- Verify C raster identity uses multipart ETag plus `Content-Length`, not a
  content hash, for `flow_dir.tif` and `flow_acc.tif`.
- Verify C records terminal bbox, per-raster tile count, per-raster fetched
  bytes, total bytes-in, hard byte ceiling, configured search radius, and the
  GDAL-vs-TIFF tile-identity proof.
- Verify `parity_golden_artifacts` imports no `DatasetBuilder`, `DatasetSession`,
  `Engine`, `AtomId`, or v0.1 loader-only type.
- Document offline and network commands in the fixture README.
- Document the M4 handoff: synthetic B rasters are the deterministic
  byte-identical M1-to-M4 parity path; `merit/0.2.0` is real-data v0.2.1 D8
  parity coverage for M4, not an M1 input.
- Document that M2 must not mutate or move the M1 B fixture in place; it creates
  a separate v0.2.1 fixture copy and reuses the same `.tif` bytes.
- Document that M1 already proved TIFF-vs-GDAL tile identity for B and C; M4 may
  reuse that proof for the byte-identical B rasters or re-run it if the reader
  implementation changes.

**Out-Of-Scope Guard**

Do not regenerate goldens in hardening except through documented bless commands.
Do not add HFX v0.2 loading, staged APIs, strategy traits, atom-to-unit renames,
or engine behavior changes.

**Concrete Verification**

Offline final M1 gate:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_v01_oracle_capture
cargo test -p shed-core --test parity_golden_artifacts
git status --short
```

Optional network proof:

```bash
SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-core --test parity_v01_oracle_capture -- --ignored --nocapture
```

After completing meaningful work, log it:

```bash
clog log --title "Completed M1 durable v0.1 parity oracle" \
  --body "Captured canonical v0.1 parity goldens, offline artifact harness, synthetic D8 fixture, network-gated grit/1.0.0 non-refined oracle, and network-gated merit-basins/0.1.0 real D8-refined oracle." \
  --tags "shed,hfx-v0.2,parity"
```

**Dependencies**

Steps 1 through 4 green.

**Commit Obligation**

Patch bump, stage `Cargo.toml` and Step 5 files, conventional commit, tag
`v<version>`.

## Gate Mapping

| Gate / requirement | Step 1 | Step 2 | Step 3 | Step 4 | Step 5 |
|---|---|---|---|---|---|
| `cargo build -p shed-core` | Adds canonicalizer compile surface | Compiles TIFF test support under canonical no-feature command | Compiles B capture path | Compiles A/C network-gated paths offline | Final required gate |
| `cargo test -p shed-core --test parity_v01_oracle_capture` | May be introduced as skeleton | Runs synthetic smoke offline | Compares B offline | Compiles/skips A+C offline; network run captures A+C | Final required gate |
| `cargo test -p shed-core --test parity_golden_artifacts` | Creates loader-free artifact harness | Continues offline with fixture docs | Validates B goldens | Validates A+B+C committed goldens | Final required gate |
| Oracle A, real non-refined |  |  |  | Captures `grit/1.0.0` with `NoRastersAvailable` | Enforced in artifacts |
| Oracle B, synthetic refined offline |  | Creates committed fixture and TIFF source | Captures `Applied` strict-carve goldens |  | Enforced in artifacts |
| GDAL-vs-TIFF tile identity |  | Proves B fixture window outside `shed-core` gate | B records proof metadata | Proves C localized windows outside `shed-core` gate | Artifact metadata enforces proof record |
| Oracle C, real refined network-gated |  | Provides TIFF source for localized windows |  | Captures `merit-basins/0.1.0` with `Applied` strict carve and byte/tile ceiling | Enforced in artifacts |
| Canonical normalizer | Defines single implementation | Used by smoke if needed | Used by B | Used by A+C | Idempotence enforced |
| Remote identity pins | Defines schema |  |  | A pins manifest/catchments/graph; C pins manifest/catchments/graph/snap/rasters | Required fields enforced |
| Never-download guard for C |  |  |  | Measures terminal bbox, tile count, fetched bytes, and enforces ceiling | Required fields enforced |
| Post-M2 survival | Starts loader-free artifact test | Keeps artifact test independent | Reads B goldens from disk; v0.1 input hashes are inert provenance | Reads A/C goldens from disk only | Explicit no v0.1 loader/type imports |
| Version bump and tag | Required | Required | Required | Required | Required |

Network requirements:

- `cargo build -p shed-core` is offline and must survive M2.
- `cargo test -p shed-core --test parity_golden_artifacts` is offline and must
  survive M2.
- `cargo test -p shed-core --test parity_v01_oracle_capture` is offline by
  default in M1; its A and C capture cases are network-gated and may be retired
  after M2 when the v0.1 loader is deleted.
- `SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-core --test parity_v01_oracle_capture -- --ignored --nocapture`
  is network-gated and intentionally not part of normal network-less CI.
- `cargo test -p shed-gdal --test raster_decode_parity synthetic_b_tiff_matches_gdal -- --ignored --nocapture`
  is GDAL-gated and validates B reader fidelity outside the `shed-core` graph.
- `SHED_PARITY_R2_CAPTURE=1 cargo test -p shed-gdal --test raster_decode_parity merit_c_windows_tiff_match_gdal -- --ignored --nocapture`
  is network+GDAL-gated and validates C reader fidelity outside the `shed-core`
  graph.

## Choices To Review

- **Golden format:** Recommend JSON metadata with canonical WKB encoded as hex or
  base64. JSON keeps the artifact harness simple and reviewable; WKB stays the
  geometry truth.
- **Coordinate precision:** Recommend choosing a fixed decimal precision in Step
  1 and recording it as a canonicalizer version. The executor must justify the
  value with degenerate-ordering tests and either cross-architecture validation
  or a documented pre-rounding-divergence margin; changing it after capture
  invalidates the oracle.
- **Scalar comparison:** Recommend explicit coordinate absolute epsilon and
  `area_km2` absolute/relative epsilon tied to the WKB precision, unless Step 1
  proves exact equality is stable across repeated runs.
- **Hammerfest:** Recommend inclusion only with `5000 m` search radius. If the
  executor excludes it, the exclusion must be documented in A's fixture README.
- **Network gating:** Recommend env var plus `#[ignore]` for expensive A/C
  refresh tests, following the existing network-test idiom. Ignoring a test is
  only an execution gate, not a compile-dependency gate.
- **C outlet choice:** Recommend `rhine_basel` and `mekong_phnom_penh` with
  `5000 m` search radius as candidates only. They are accepted only after live
  measurement records terminal bbox, tile counts, fetched bytes, and compliance
  with the hard ceiling.
- **Raster source:** Recommend the Step 2 `tiff`/`cog.rs`-based test source for
  B and C only after tile-identical proof against `GdalRasterSource` in isolated
  `shed-gdal` tests. GDAL must not enter canonical M1 `shed-core` commands; any
  GDAL fallback requires explicit replanning because `#[ignore]` and env gates do
  not prevent compile or link requirements.
