# M2 Step Plan - HFX Core Cutover And v0.2.1 Loader

This plan is for Milestone 2 only. It defines ordered implementation steps for
the sub-orchestrator; it does not write production code. The critique in
`docs/hfx-v02-redesign/m2-step-critique.md` is accepted: the dependency flip is
not a mechanical rename. The local v0.2.1 `hfx-core` crate removes API families
with no rename target (`RasterAvailability`, `SnapAvailability`,
`CatchmentAtom`, `MainstemStatus`, `AtomId`, `AtomCount`) and changes
`AdjacencyRow::new` from `(id, upstream_ids)` to `(id, level, upstream_ids)`.
Therefore manifest parsing, auxiliary-based availability, `graph.parquet`,
`catchments.parquet`, `hfx.aux.snap.v1`, session assembly, fixture emission, and
retirement of v0.1-only tests are part of the same red-to-green contract
boundary as the dependency cut.

## Confirmed Starting Facts

- shed is on `main`, root version `0.1.115`.
- `cargo test -p shed-core --test parity_golden_artifacts` is green: 5 passed.
- `cargo build --workspace --exclude pyshed` is green.
- `cargo check -p pyshed` is green.
- Root `Cargo.toml` pins `hfx-core = "=0.2.0"` from crates.io and has a
  commented local `[patch.crates-io]` for `../hfx/crates/hfx-core`.
- Local `../hfx` is at `478dfa6` (`fix(adapter-merit-v2): filter dangling
  upstream COMIDs in graph stage`). Its workspace version is `0.2.64`, and
  `../hfx/crates/hfx-core` exports the v0.2.1 contract: `UnitId`,
  `UnitCount`, `Level`, `CatchmentUnit`, `AdjacencyRow`, `DrainageGraph`,
  `FormatVersion::V0_2_1`, and `StemRole`.
- HFX format `0.2.1` requires `manifest.json`, `catchments.parquet`, and
  `graph.parquet`; removes `atom_count`, `terminal_sink_id`, `has_rasters`,
  `has_snap`, `fabric_level`, and `flow_dir_encoding`; adds `unit_count` and
  `auxiliary[]`; and rejects legacy `graph.arrow`.
- `hfx.aux.snap.v1` has four stem roles: `mainstem`, `tributary`,
  `distributary`, and `unknown`. Its default snap cascade is weight, mainstem,
  distance, then snap id.

## Dependency-Graph Note

Default M2 dependency cut:

```toml
[workspace.dependencies]
hfx-core = "=0.2.0"

# HFX format 0.2.1 is implemented by the unpublished local hfx-core crate
# version 0.2.64 at ../hfx rev 478dfa6.
[patch.crates-io]
hfx-core = { path = "../hfx/crates/hfx-core" }
```

The important mapping is: `hfx-core` crate version `0.2.64` at rev `478dfa6`
implements HFX on-disk format version `0.2.1`. The crate version is not the
format version. The Cargo.toml edit and this plan must both preserve that
mapping so a future reader does not mistake crates.io `hfx-core = "=0.2.0"` for
the v0.2.1 contract.

Open dependency decision: the local path patch is not a reproducible pin. It is
the default because the published plan expects the unpublished local crate. If
the owner wants hard reproducibility, replace the path patch with a git
dependency pinned to rev `478dfa6` and record the exact remote URL before Step 1
starts.

## CI-Aligned M2 Gate

Run this exact gate after Step 1 and after every later step:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
cargo test --workspace --exclude pyshed --no-run
cargo test -p shed-core --test hfx_v02_loader
cargo test -p shed-core --test graph_parquet_reader
cargo test -p shed-core --test snap_aux_reader
cargo test -p shed-core --test hfx_v02_test_fixtures
cargo test -p shed-core --test parity_golden_artifacts
```

Do not add `cargo build --workspace` without `--exclude pyshed`. Do not add a
`.cargo/config.toml`. Plain `cargo check -p pyshed` is the pyshed build hold.

Network-gated real-data tier:

```bash
SHED_HFX_V02_REAL_R2_LOAD=1 cargo test -p shed-core --test hfx_v02_loader -- --ignored --nocapture
```

The ignored test is named
`grit_v200_public_r2_loads_real_v021_multilevel_dag`. It opens
`https://basin-delineations-public.upstream.tech/grit/2.0.0/` through
`source.rs`'s `PUBLIC_R2_CUSTOM_DOMAIN` path and performs a bounded readiness
proof against real GRIT v2.0.0 bytes: manifest facts
(`format_version == "0.2.1"`, `unit_count == 22_337_300`, `topology == dag`,
exactly two `hfx.aux.snap.v1` declarations, and no `hfx.aux.d8_raster.v1`),
graph `bbox_*` footer/schema facts, L0+L1 presence from graph row-group level
statistics, and one bounded graph plus snap aux decode. It must not call a full
`DatasetSession::open` over all units. This tier is separate from the offline
gate; offline M2 must stay green with no network.

## New Integration Test Files

- `crates/core/tests/hfx_v02_loader.rs`: positive load of converted v0.2.1
  fixtures, typed v0.1 and wrong-version rejection before missing-field
  parsing, CRS rejection, legacy `graph.arrow` rejection, manifest
  `auxiliary[]` parsing, topology preservation, and ignored bounded GRIT
  v2.0.0 public R2 readiness proof.
- `crates/core/tests/graph_parquet_reader.rs`: `graph.parquet` schema,
  list<int64> upstream decoding, bbox column presence justified by the
  row-group-stats clause in `HFX_SPEC.md`, same-level referential integrity, one
  graph row per catchment, missing/extra graph row errors, and legacy Arrow
  rejection.
- `crates/core/tests/snap_aux_reader.rs`: manifest-selected
  `hfx.aux.snap.v1` artifact loading, `id`, `unit_id`, `weight`, optional
  `stem_role`, optional bbox, WKB Point/LineString geometry, valid four-value
  `StemRole`, invalid stem-role rejection, `unit_id` referential integrity, and
  `references_levels` checks.
- `crates/core/tests/hfx_v02_test_fixtures.rs`: fixture builder emits v0.2.1
  `manifest.json`, `catchments.parquet`, `graph.parquet` with bbox columns,
  optional snap aux, optional D8 aux, multi-level nested fixtures, and a
  converted parity fixture that reuses byte-identical M1 B TIFFs.

## Step 1 - Atomic v0.2.1 Contract Cutover

Intent: Land one red-to-green commit that flips `hfx-core` to the local v0.2.1
contract and rewrites every compile-forced surface that depends on removed API
families. This step owns the minimal working v0.2.1 loader: manifest
`unit_count`/`auxiliary[]`, auxiliary-based snap/D8 availability, v0.2.1
catchment units, `graph.parquet` with `level`, `hfx.aux.snap.v1`, session
assembly, DAG-safe upstream traversal, v0.2.1 test fixtures, retirement or
neutralization of v0.1-only tests, and thin pyshed build-hold updates.

Files touched: root `Cargo.toml`, `Cargo.lock`, `crates/core/src/error.rs`,
`crates/core/src/reader/manifest.rs`,
`crates/core/src/reader/catchment_store.rs`,
`crates/core/src/reader/id_index.rs`, `crates/core/src/reader/graph.rs`,
`crates/core/src/reader/snap_store.rs`, `crates/core/src/session.rs`,
`crates/core/src/resolver.rs`, `crates/core/src/algo/upstream.rs`,
`crates/core/src/assembly.rs`, `crates/core/src/engine.rs`,
`crates/core/src/testutil.rs`, `crates/core/README.md`, `src/main.rs`,
`crates/gdal/tests/raster_decode_parity.rs`, `crates/core/tests/*.rs`,
`crates/python/src/*.rs`, and `crates/python/tests/*.py` where required to keep
the crates compiling. `crates/python/python/pyshed/__init__.pyi` and
`crates/python/API.md` are deliberately not renamed in M2 if pyshed keeps its
public atom-named API. Must not touch
`crates/core/tests/fixtures/parity/v01_synthetic_refined/`,
`crates/core/src/algo/canonical_wkb*`, or canonicalizer constants.

New types/errors introduced:

- `UnsupportedFormatVersion { found, expected }`: fires when
  `manifest.format_version` is not `"0.2.1"`, including `"0.1"`, before any
  missing-field parsing.
- `UnsupportedCrs { found, expected }`: fires when manifest CRS is not
  `EPSG:4326`.
- `UnitCountMismatch { manifest_count, actual_count }`: fires when
  `unit_count` differs from `catchments.parquet` row count.
- `AuxiliaryDeclParse { schema, reason }`: fires when an aux entry lacks
  required structural fields or known-schema metadata.
- `AuxiliaryPathEscape { schema, artifact, path }`: fires when a declared
  artifact path is absolute or escapes the dataset root.
- `AuxiliaryArtifactMissing { schema, artifact, path }`: fires when a declared
  aux artifact is absent.
- `MissingBboxColumn { artifact, column }`: fires when `catchments.parquet` is
  missing required catchment bbox columns.
- `GraphMissingBboxColumn { column }`: fires when `graph.parquet` lacks a bbox
  column needed to satisfy the spec's row-group-statistics requirement on
  `bbox_*`.
- `LegacyGraphArrowRejected { path }`: fires when a v0.2.1 dataset contains or
  falls back to legacy `graph.arrow`.
- `GraphReferentialIntegrity { reason }`: fires when graph IDs do not exactly
  match catchment IDs, upstream IDs are missing, row level differs from the
  catchment level, or an edge crosses levels.
- `SnapAuxMetadataInvalid { name, reason }`: fires for missing or invalid snap
  aux `name`, `description`, `references_levels`, or `weight_semantics`.
- `InvalidStemRole { row, value }`: fires when snap `stem_role` is not
  `mainstem`, `tributary`, `distributary`, or `unknown`.
- `SnapReferentialIntegrity { snap_id, unit_id, reason }`: fires when snap
  `unit_id` is absent or the referenced unit level is not included in
  `references_levels`.
- `SnapGeometryInvalid { row, reason }`: fires for non-Point/LineString snap
  WKB.

Acceptance criteria: `cargo build --workspace --exclude pyshed`,
`cargo check -p pyshed`, and `cargo test --workspace --exclude pyshed --no-run`
are green in the same commit as the dependency cut. `cargo test -p shed-core` is
green because v0.1 fixture-builder users are ported to v0.2.1 or removed. The
v0.1 executable capture test `parity_v01_oracle_capture.rs` is retired,
removed, or otherwise excluded from compilation in this step; the durable
artifact gate remains. `DatasetSession` loads v0.2.1 fixtures through
manifest/catchments/graph/snap readers. `AdjacencyRow::new` is called with a
`Level`. `manifest.snap()` and `manifest.rasters()` assumptions are gone from
core and replaced by stored aux declarations. `WeightFirst` resolver behavior is
preserved mechanically: weight DESC, `StemRole::Mainstem` preference, existing
distance tie-breaker, then snap id. pyshed compiles by updating internal calls
from core's renamed methods; its public Python names
`terminal_atom_id`/`upstream_atom_ids` may remain until the M5 public API
cleanup.

Verification commands:

```bash
rg -n "AtomId|AtomCount|CatchmentAtom|MainstemStatus|RasterAvailability|SnapAvailability|atom_count|terminal_sink_id" crates/core/src src
rg -n "terminal_atom_id|upstream_atom_ids" crates/core/src src
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test --workspace --exclude pyshed --no-run
cargo test -p shed-core
cargo test -p shed-core --test parity_golden_artifacts
```

Commit/version doctrine: run `./scripts/bump-version.sh patch`, stage root
`Cargo.toml` with the dependency and code changes, commit with a conventional
message, and tag `v$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')`.
Do not cut a `pyshed-v*` release.

Non-scope: no engine delineation behavior change; no staged pipeline; no
level-selection strategy; no mixed-level traversal; no custom reverse-DNS aux
parsing; no full aux-to-strategy binding; no pyshed public API redesign.

## Step 2 - Manifest and Auxiliary Hardening

Intent: Add focused coverage and typed diagnostics around the v0.2.1 manifest
boundary already introduced in Step 1. This step hardens behavior; it does not
defer manifest work needed for compilation.

Files touched: `crates/core/src/reader/manifest.rs`,
`crates/core/src/session.rs`, `crates/core/src/error.rs`,
`crates/core/tests/hfx_v02_loader.rs`, and fixture helpers only as needed. Must
not touch v0.1 parity fixture files.

New types/errors introduced: only refinements to Step 1 errors if coverage
shows a missing typed case. Any new library error variant must be `thiserror`,
use named fields, and document when it fires.

Acceptance criteria: v0.1 manifests fail as `UnsupportedFormatVersion` before
`unit_count` or `auxiliary[]` errors; wrong CRS is typed; D8 aux metadata and
artifact path escape are typed; snap aux metadata and artifact path escape are
typed; generic reverse-DNS aux entries are stored as raw resolved path plus
metadata only.

Verification commands:

```bash
cargo test -p shed-core --test hfx_v02_loader manifest_
cargo test -p shed-core --test hfx_v02_loader auxiliary_
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
```

Commit/version doctrine: patch bump, stage root `Cargo.toml`, conventional
commit, and tag.

Non-scope: no reverse-DNS aux parser; no aux-to-strategy binding; no v0.1
compatibility flag.

## Step 3 - graph.parquet Reader Hardening

Intent: Harden the v0.2.1 graph reader introduced in Step 1. The emphasis is on
schema tests, bbox-column rationale, and referential-integrity failure modes.

Files touched: `crates/core/src/reader/graph.rs`,
`crates/core/src/session.rs`, `crates/core/src/error.rs`,
`crates/core/tests/graph_parquet_reader.rs`, and fixture helpers. Must not keep
any production fallback to `graph.arrow`.

New types/errors introduced: no new required types beyond Step 1 unless tests
reveal an untyped case. `GraphMissingBboxColumn` must cite the
`HFX_SPEC.md` row-group-statistics clause requiring stats on `bbox_*`; do not
claim the spec table marks bbox columns non-nullable.

Acceptance criteria: positive graph load from parquet; `upstream_ids` decodes
as `list<int64>`; every catchment has exactly one graph row; every graph row ID
exists; upstream IDs exist; graph row level matches catchment level; edges are
same-level; missing bbox columns are rejected based on the stats requirement;
legacy Arrow is rejected. Add an assertion in the real-data GRIT tier, or a
documented fixture equivalent if network is unavailable, that real graphs carry
the bbox columns.

Verification commands:

```bash
cargo test -p shed-core --test graph_parquet_reader
cargo test -p shed-core --test hfx_v02_loader legacy_graph_arrow
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
```

Commit/version doctrine: patch bump, stage root `Cargo.toml`, conventional
commit, and tag.

Non-scope: no topology-specific traversal strategy; no mixed-level traversal.

## Step 4 - hfx.aux.snap.v1 Reader and Resolver Ranking Hardening

Intent: Harden snap aux loading and prove that the forced `StemRole` type swap
does not change current resolver behavior.

Files touched: `crates/core/src/reader/snap_store.rs`,
`crates/core/src/reader/manifest.rs`, `crates/core/src/session.rs`,
`crates/core/src/resolver.rs`, `crates/core/src/error.rs`,
`crates/core/tests/snap_aux_reader.rs`, and fixture helpers.

New types/errors introduced: no new required types beyond Step 1 unless tests
reveal an untyped case.

Acceptance criteria: reader loads `id`, `unit_id`, `weight`, optional
`stem_role`, optional bbox, and WKB Point/LineString geometry.
`StemRole::Mainstem`, `Tributary`, `Distributary`, and `Unknown` are accepted;
invalid stem roles are rejected. `unit_id` referential integrity and
`references_levels` are checked. Resolver ranking uses `StemRole::Mainstem`
while preserving the existing distance tie-breaker before snap id. Any proposal
to drop distance is out of scope and must be escalated to M3.

Verification commands:

```bash
cargo test -p shed-core --test snap_aux_reader
cargo test -p shed-core --test hfx_v02_loader auxiliary_snap
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
```

Commit/version doctrine: patch bump, stage root `Cargo.toml`, conventional
commit, and tag.

Non-scope: no snap aux selection strategy beyond manifest loading; no snap
cascade semantic change; no M3 finest-level behavior.

## Step 5 - Fixture and Parity Durability Hardening

Intent: Finish fixture-specific proof that all active tests run on v0.2.1
fixtures while the M1 durable parity artifacts remain immutable and
loader-independent.

Files touched: `crates/core/src/testutil.rs`,
`crates/core/tests/hfx_v02_test_fixtures.rs`,
`crates/core/tests/fixtures/parity/README.md`, new v0.2.1 fixture directories,
and active tests that still need fixture updates. Must not touch or move
`crates/core/tests/fixtures/parity/v01_synthetic_refined/manifest.json`,
`flow_dir.tif`, or `flow_acc.tif`.

New types/errors introduced: builder-only typed helpers such as `FixtureUnit`,
`FixtureGraphRow`, `FixtureSnapDecl`, and `FixtureD8Decl` are acceptable if they
avoid raw primitive leakage past fixture boundaries. No production errors unless
the builder reuses production loader validation.

Acceptance criteria: fixture builder emits `format_version: "0.2.1"`,
`unit_count`, `auxiliary[]`, `catchments.parquet`, `graph.parquet` with bbox
columns, optional snap aux, optional D8 aux, single-level, multi-level nested,
and DAG fixtures. The converted parity fixture is separate from the M1 v0.1
fixture and reuses byte-identical B TIFF bytes. The durable artifact test still
imports no loader, `Engine`, `hfx_core`, `UnitId`, or `AtomId`; comments may
name forbidden imports as documentation. If the converted v0.2.1 TIFF copy is
not hash-guarded in M2, document that its drift is caught by later M4 carve
parity rather than by the M2 durable artifact gate.

Verification commands:

```bash
rg -n "^use .*hfx_core|^use .*DatasetSession|^use .*DatasetBuilder|^use .*Engine|UnitId::|AtomId::" crates/core/tests/parity_golden_artifacts.rs
cargo test -p shed-core --test hfx_v02_test_fixtures
cargo test -p shed-core --test parity_golden_artifacts
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
```

Commit/version doctrine: patch bump, stage root `Cargo.toml`, conventional
commit, and tag.

Non-scope: no parity re-blessing; no M1 fixture mutation; no engine behavior
change. `parity_v01_oracle_capture.rs` stays retired in M2; porting capture to
v0.2.1 is out of scope because behavioral parity against the M1 oracle is M4.

## Step 6 - Remote GRIT v2.0.0 Bounded Readiness Proof

Intent: Add the first-class real-data M2 loader proof for GRIT v2.0.0 without
putting network access into the offline gate and without materializing or
validating all 22,337,300 units.

Files touched: `crates/core/tests/hfx_v02_loader.rs`,
`docs/hfx-v02-redesign/m2-step-plan.md`, and
`docs/hfx-v02-redesign/milestone-plan.md`. No production code unless the test
exposes a bug in URL-root artifact resolution.

New types/errors introduced: no new errors unless bounded remote reads expose a
typed case already listed above.

Acceptance criteria: ignored test
`grit_v200_public_r2_loads_real_v021_multilevel_dag` is compiled by offline
`cargo test -p shed-core --test hfx_v02_loader` but runs only when
`SHED_HFX_V02_REAL_R2_LOAD=1` is set. It opens
`https://basin-delineations-public.upstream.tech/grit/2.0.0/` through
`PUBLIC_R2_CUSTOM_DOMAIN`, checks `format_version == "0.2.1"`,
`unit_count == 22_337_300`, `crs == EPSG:4326`, `topology == dag`, two snap aux
declarations, and no D8 aux by reading `manifest.json` only. It proves graph
`bbox_*` columns from the real `graph.parquet` schema/footer, proves L0+L1 from
graph row-group `level` statistics, reads one bounded graph row group to decode
real `id`, `level`, and `list<int64>` upstream data, and reads one bounded
snap aux row group selected by bbox statistics to decode real snap IDs,
unit IDs, weights, bbox columns, and Point/LineString WKB. The test must not
call full `DatasetSession::open`, must not assert `session.graph().len() ==
unit_count`, and should complete in seconds. Offline gate must not fetch
network.

Deferred follow-up: a full-scale validated `DatasetSession::open` over all
22,337,300 GRIT units is not an M2 gate because current debug runs take 30+
minutes and are memory-heavy. Full-scale validation, likely including
streaming/lazy referential validation for planetary-scale datasets, is deferred
to a future performance/scale milestone.

Verification commands:

```bash
cargo test -p shed-core --test hfx_v02_loader
SHED_HFX_V02_REAL_R2_LOAD=1 cargo test -p shed-core --test hfx_v02_loader -- --ignored --nocapture
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
```

Commit/version doctrine: patch bump, stage root `Cargo.toml`, conventional
commit, and tag.

Non-scope: no real-data delineation parity, no D8 refinement, no remote capture
or golden re-blessing, no full 22M-unit validated open as a gate, and no
streaming/lazy validation redesign.

## Step 7 - Final M2 Gate and Drift Audit

Intent: Run the full CI-aligned M2 gate and audit for explicit non-scope drift,
version/tag discipline, and immutable M1 parity survival before declaring M2
complete.

Files touched: only documentation or test expectation comments if the audit
finds stale text. Production changes discovered here require returning to the
relevant earlier step.

New types/errors introduced: none.

Acceptance criteria: full CI-aligned gate passes; network-gated GRIT test
passes when explicitly enabled; `pyshed` is checked but not linked as cdylib; no
`.cargo/config.toml` was added; M1 B fixture path and TIFF hashes are unchanged;
canonicalizer version and precision are unchanged; no v0.1 compatibility flag
or mixed-level traversal was added. pyshed public atom-named methods may remain
only as the explicit M2 build-hold compromise; M5 owns public API cleanup.

Verification commands:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
cargo test --workspace --exclude pyshed --no-run
cargo test -p shed-core --test hfx_v02_loader
cargo test -p shed-core --test graph_parquet_reader
cargo test -p shed-core --test snap_aux_reader
cargo test -p shed-core --test hfx_v02_test_fixtures
cargo test -p shed-core --test parity_golden_artifacts
SHED_HFX_V02_REAL_R2_LOAD=1 cargo test -p shed-core --test hfx_v02_loader -- --ignored --nocapture
```

Commit/version doctrine: if any audit fix is committed, patch bump and tag it.
After meaningful completion, log the work with `clog log`.

Non-scope: no M3 staged pipeline, no M4 refinement trait, no M5 pyshed redesign,
and no behavior parity claim beyond loader readiness.

## Open Decisions / Escalations

- Reproducibility: path patch to `../hfx/crates/hfx-core` is the default but is
  not a hard pin. Owner decision needed if M2 must use a git dependency pinned
  to `478dfa6`.
- Snap cascade discrepancy: published M2 text says weight DESC -> mainstem DESC
  -> snap id ASC, while current `WeightFirst` and the snap.v1 spec include
  distance before snap id. M2 must preserve current behavior mechanically. Any
  proposal to drop distance or otherwise change resolver behavior is out of
  scope and must be escalated to M3.
- pyshed public naming: M2 keeps pyshed compiling. Public Python
  `terminal_atom_id`/`upstream_atom_ids` names may remain until M5; renaming
  them in M2 is a pyshed API redesign and should be escalated.
- `parity_v01_oracle_capture.rs`: retire or neutralize it in Step 1 because it
  references removed v0.1 loader types. Do not port it to v0.2.1 capture in M2.
- Loader complexity: if manifest/catchment/graph/snap reader interactions grow
  hard to navigate, add `crates/core/README.md` updates with a Mermaid module
  diagram. Do not add ASCII diagrams.
- Any step that appears to require engine delineation behavior changes,
  mixed-level traversal, aux-to-strategy binding, reverse-DNS aux parsing, or
  pyshed API redesign is not an M2 step; stop and escalate.
