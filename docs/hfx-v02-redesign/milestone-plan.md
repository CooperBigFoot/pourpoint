# shed HFX v0.2 Redesign Milestone Plan

This plan breaks the `shed` HFX v0.2 redesign into independently verifiable
vertical slices. It incorporates the critique in
`docs/hfx-v02-redesign/milestone-plan-critique.md`: the hfx-core v0.2.1 type
cutover, repo-wide atom-to-unit migration, durable parity oracle, v0.2 fixture
infrastructure, `graph.parquet` and `hfx.aux.snap.v1` readers,
level-constrained resolution, minimal D8 aux accessor, and pyshed build hold are
first-class work.

The campaign remains Rust-core-first. pyshed redesign, Python-authored
strategies, full aux-to-strategy binding, and level-selection strategies beyond
default finest are outside this milestone set.

## 1. Milestone List

### M1 - Durable v0.1 Parity Oracle

Goal: capture today's working v0.1 behavior as inert committed artifacts before
any dependency, loader, or pipeline restructuring.

Scope:

- Capture parity from the current `Engine::delineate` on checked-in v0.1 input
  datasets, not from a builder that later milestones will mutate.
- Commit the v0.1 input dataset copy and the golden outputs as fixture files.
- Golden outputs include canonical final geometry WKB, `area_km2`, resolved
  outlet, refined outlet when applied, terminal ID, upstream ID set, and
  refinement outcome.
- Include at least one D8-refined case using byte-identical
  `flow_dir.tif`/`flow_acc.tif` that will be reused by the converted v0.2.1
  fixture.
- Include at least one realistic multi-unit, real-raster carve case if a
  representative sample can be checked in at acceptable size. If M1 must ship
  with synthetic fixtures only, the milestone must say so explicitly and record
  a follow-up task for adding real-data parity coverage.
- Define the canonical geometry normalization used by all later parity checks:
  ring orientation, start vertex, component ordering, and coordinate precision.
- Add a parity comparison harness that reads committed golden artifacts from
  disk and can be reused after the v0.2.1 cutover without regenerating goldens.

Explicit non-scope:

- No HFX v0.2 loader work.
- No hfx-core dependency migration.
- No staged API.
- No strategy traits.
- No behavior changes.

Gate:

```bash
cargo build -p shed-core
cargo test -p shed-core --test parity_v01_oracle_capture
cargo test -p shed-core --test parity_golden_artifacts
```

The capture test proves the committed goldens match the current v0.1 engine.
The artifact test proves the goldens are readable, canonicalized, and independent
of `DatasetBuilder`. After M2 deletes v0.1 loaders, the capture test may be
retired, but the committed golden files and artifact comparison harness must
remain.

Why it is a vertical slice:

This proves the full current engine path end to end before it is disturbed:
outlet resolution, upstream traversal, containment-clamped D8 refinement,
dissolve, and result composition. Parity survives the hard cut because the
oracle is serialized output plus byte-identical raster inputs, not executable
v0.1 loader compatibility.

### M2 - HFX Core Cutover And v0.2.1 Loader

Goal: make `shed` compile and load datasets against the v0.2.1 HFX type
contract.

> **Version terminology — do not conflate the two `0.2.x` numbers.** The
> **HFX format version** (`manifest.json::format_version`) and the **`hfx-core`
> crate version** are independent, and their numbers collide misleadingly:
>
> | Thing | Current value | Contract it models |
> |---|---|---|
> | HFX on-disk format (the spec) | `0.2.1` | `UnitId`, `auxiliary[]`, `unit_count` |
> | `hfx-core` crate shed is locked to (crates.io) | `0.2.0` | the **v0.1** format: `AtomId`, `AtomCount`, `terminal_sink_id`, `FormatVersion::V0_1` |
> | `hfx-core` crate in local `../hfx` | `0.2.64` | the **v0.2.1** format: the target of this cutover |
>
> shed is **not** already on the v0.2.1 contract: `Cargo.lock` resolves
> `hfx-core` to registry `0.2.0` and shed's source still uses `AtomId`
> throughout. The crate that "is already at v0.2.1" is the unreleased local
> `../hfx/crates/hfx-core`. A crate version like `hfx-core = "0.2.64"` gives no
> hint that it implements HFX format `0.2.1`, so the dependency edit below must
> carry a comment mapping crate version to HFX format version.

Scope:

- Re-point `hfx-core` from crates.io `=0.2.0` to the v0.2.1-contract source.
  Because the v0.2.1-contract `hfx-core` is unpublished, the default route is
  the existing local path patch:
  `[patch.crates-io] hfx-core = { path = "../hfx/crates/hfx-core" }`. Record
  local hfx rev `478dfa6` and crate version `0.2.64` in the dependency graph
  and annotate `Cargo.toml` with the crate-version-to-HFX-format-version mapping
  so the misleading `0.2.x` collision cannot mislead a future reader. A path
  patch is not a reproducible pin; if hard reproducibility is required, escalate
  to a git dependency pinned to rev `478dfa6`.
- Treat the dependency cut, atom-to-unit rename, manifest `unit_count` and
  `auxiliary[]` parsing, auxiliary-based snap/D8 availability, v0.2.1
  catchment-unit loading, `graph.parquet` loading, `hfx.aux.snap.v1` loading,
  session assembly, v0.2.1 fixture-builder rewrite, and retirement of
  v0.1-loader-only tests as one red-to-green compile boundary. The local
  v0.2.1 `hfx-core` crate removes APIs such as `RasterAvailability`,
  `SnapAvailability`, `CatchmentAtom`, `MainstemStatus`, `AtomId`, and
  `AtomCount`, and `AdjacencyRow::new` now requires a `Level`; these surfaces
  cannot be split into a separately compiling mechanical rename followed by
  later reader work.
- Perform the forced repo-wide type migration caused by that dependency cut:
  `AtomId` to `UnitId`, `AtomCount` to `UnitCount`, terminal/upstream atom
  methods to terminal/upstream unit methods, and `atom_count` to `unit_count`.
- Perform the forced snap-domain migration caused by that dependency cut:
  v0.1 `MainstemStatus` to v0.2.1 `StemRole`, with resolver ranking redefined
  in terms of `stem_role = mainstem`.
- Keep pyshed compiling by mechanically updating its thin bindings to the
  renamed core result methods. This is a build hold only, not the pyshed
  redesign.
- Accept only `manifest.json::format_version == "0.2.1"` and CRS `EPSG:4326`.
  A v0.1 manifest must fail with a typed unsupported-version diagnostic before
  downstream missing-field parsing.
- Load `catchments.parquet` as HFX v0.2.1 drainage units: `id`, `level`,
  `parent_id`, `area_km2`, `up_area_km2`, outlet coordinates, bbox columns, and
  geometry.
- Build the net-new `graph.parquet` reader for `id`, `level`, `upstream_ids`,
  and `bbox_*` columns.
- Build the net-new `hfx.aux.snap.v1` snap-feature reader selected from
  `manifest.auxiliary[]`, reading `id`, `unit_id`, `weight`, optional
  `stem_role`, optional `bbox_*`, and WKB point/linestring geometry.
- Validate graph referential integrity needed by the engine: every catchment has
  exactly one graph row, graph IDs exist, upstream IDs exist, and graph edges are
  same-level.
- Validate snap referential integrity needed by the engine: every snap
  `unit_id` exists, referenced unit levels are included in the declaration's
  `references_levels`, and `stem_role` values are one of the HFX v0.2.1 enum
  values when present.
- Parse `manifest.auxiliary[]` into stored declarations for blessed D8, blessed
  snap, and generic reverse-DNS aux entries.
- Define topology handling for this campaign: load `topology`, preserve it in
  the session, and keep traversal dedup-safe for `tree` and `dag`. No
  topology-specific strategy is introduced.
- Rewrite the test fixture infrastructure to emit HFX v0.2.1:
  `manifest.json`, `catchments.parquet`, `graph.parquet` with `bbox_*`,
  optional `hfx.aux.snap.v1`, optional `hfx.aux.d8_raster.v1`, and multi-level
  nested fixtures.
- Convert the M1 parity fixture to HFX v0.2.1 while reusing byte-identical D8
  rasters.
- Treat real GRIT v2.0.0 as a first-class M2 loader target. The loader gate must
  include a separate network-gated ignored bounded readiness proof that opens
  `https://basin-delineations-public.upstream.tech/grit/2.0.0/` through
  `source.rs`'s public R2 custom-domain path and verifies the real
  `format_version = "0.2.1"` dataset without materializing all units. The
  proof checks manifest facts (`unit_count = 22,337,300`, `crs = EPSG:4326`,
  `topology = "dag"`, two `hfx.aux.snap.v1` declarations, and no
  `hfx.aux.d8_raster.v1`), graph `bbox_*` schema/footer facts, L0+L1 presence
  from graph row-group `level` statistics, one bounded graph row-group decode,
  and one bounded snap aux row-group WKB decode. This test is opt-in via an
  env var plus `#[ignore]`; the offline gate must stay green with no network.

Explicit non-scope:

- No full aux-to-strategy binding mechanism.
- No custom aux parsing beyond raw resolved path plus metadata handles.
- No mixed-level traversal.
- No v0.1 compatibility flag.
- No pyshed API redesign.
- No full-scale validated `DatasetSession::open` over all 22,337,300 GRIT
  units as an M2 gate. Current debug full opens take 30+ minutes and are
  memory-heavy; full-scale validation, likely including streaming/lazy
  referential validation for planetary-scale datasets, is deferred to a future
  performance/scale milestone.

Gate:

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

The loader gate must verify positive load of the converted parity fixture,
typed rejection of v0.1 and wrong `format_version`, rejection of legacy
`graph.arrow`, rejection of missing `graph.parquet` `bbox_*` columns based on
the spec's row-group-statistics requirement for `bbox_*`, same-level graph
referential-integrity failures, successful load of a minimal DAG fixture,
successful parsing of D8 and snap aux declarations, and compile-and-skip the
ignored network-gated GRIT v2.0.0 bounded readiness proof. Run that real-data
tier explicitly with:

```bash
SHED_HFX_V02_REAL_R2_LOAD=1 cargo test -p shed-core --test hfx_v02_loader -- --ignored --nocapture
```

The snap reader gate must verify the v0.2.1 snap schema, `StemRole` parsing,
level references, and snap `unit_id` referential integrity. `cargo check -p
pyshed` is the pyshed build hold because plain `cargo build --workspace` links
the PyO3 extension module on macOS and fails on pre-existing `_Py*` symbols.
`cargo test -p shed-core` and `cargo test --workspace --exclude pyshed --no-run`
compile the in-lib unit tests, integration tests, and workspace test targets so
the forced rename and fixture port are complete at the M2 boundary rather than
deferred to M5.

Why it is a vertical slice:

This proves the repository has crossed the HFX v0.2.1 boundary as a compiling
workspace: the dependency vocabulary, loader, graph reader, snap reader,
fixtures, resolver ranking types, and thin Python bindings all agree on drainage
units.

### M3 - Finest-Level Staged Delineation Skeleton

Goal: decompose monolithic delineation into typed, inspectable stages with
default-finest behavior that is correct on multi-level data.

Scope:

- Implement the fixed stage order as select level, resolve outlet within that
  level, traverse upstream same-level graph, produce pre-merge drainage-unit
  records, refine terminal placeholder, dissolve/assemble, compose result.
- Default level selection is `max(level)` present in the dataset.
- Resolution is constrained by the selected level before point-in-polygon or
  snap tie-breaking. Nested parent units must not win over finest units by area.
- Snap resolution chooses from `hfx.aux.snap.v1` entries whose
  `references_levels` includes the selected level. Multiple matching entries
  require a deterministic narrow rule in this milestone; broader selection
  strategies are deferred.
- Snap candidate ranking follows the v0.2.1 cascade for the selected level:
  weight first, `stem_role = mainstem` as the named tie-break, then deterministic
  snap ID ordering.
- Pre-merge output returns pristine drainage-unit records including id, level,
  area, upstream area, outlet, and geometry.
- One-call `delineate()` becomes a thin composition over the inspectable stages.
- Refine-off or no-refinement path dissolves whole upstream units.

Explicit non-scope:

- No level-selection strategy beyond default finest.
- No user-authored resolution, traversal, or dissolve strategy.
- No D8 refinement trait yet except a placeholder result shape if needed.
- No pyshed redesign.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test finest_level_resolution
cargo test -p shed-core --test snap_resolution_cascade
```

The staged delineation test must verify each stage output independently on the
HFX v0.2.1 converted fixture and verify `delineate()` equals explicit staged
composition. The finest-level test must use a two-level nested fixture and prove
an outlet resolves to the finest containing unit, not the coarser parent. The
snap cascade test must use an `hfx.aux.snap.v1` fixture and prove ranking by
weight, then `stem_role = mainstem`, then snap ID. The pre-merge unit list must
include the whole terminal, while final geometry is produced by downstream
dissolve.

Why it is a vertical slice:

This proves a complete v0.2 delineation can run through the new inspectable
architecture before adding pluggable refinement, and it closes the
resolve-versus-level circularity explicitly.

### M4 - D8 Refinement Trait As The Default Strategy

Goal: move today's containment-clamped D8 carve behind the only user-authored
seam in this campaign and prove parity on v0.2 inputs.

Scope:

- Define the Rust-native terminal-refinement trait.
- Implement the built-in D8 raster refinement strategy using
  `hfx.aux.d8_raster.v1`.
- Add a minimal blessed-D8 aux accessor, distinct from the deferred general
  binding mechanism: choose the applicable D8 declaration, resolve `flow_dir`
  and `flow_acc` relative paths inside the dataset root, and feed session
  raster localization.
- Re-key refinement availability from v0.1 root raster paths to presence or
  absence of a D8 aux declaration.
- Preserve today's carve exactly: containment-clamped terminal-only shrink,
  masked flow direction, masked accumulation, snap, masked trace, polygonize.
- Final geometry is dissolve of whole upstream units minus whole terminal plus
  carved terminal.
- Expose refinement provenance in the result.
- Missing explicitly required D8 aux hard-errors for explicit D8 invocation.
- Convenience `delineate()` may use a named best-effort default, but the
  outcome must be visible.

Explicit non-scope:

- No full aux-to-strategy binding design.
- No Python-authored strategies.
- No additional blessed strategies.
- No custom reverse-DNS aux parser.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test d8_aux_accessor
```

The D8 parity test must compare the HFX v0.2.1 converted fixture output against
the M1 oracle using the M1 canonicalization rules. The v0.2.1 fixture must reuse
the exact `flow_dir.tif` and `flow_acc.tif` bytes captured in M1, so a red gate
isolates engine behavior or declared-path resolution rather than raster
conversion. The accessor test must prove the strategy runs from
manifest-declared `hfx.aux.d8_raster.v1` paths, hard-errors on missing explicit
D8 aux, and supports refine-off whole-terminal dissolve.

Why it is a vertical slice:

This proves the first complete recommended slice: v0.2 loading, staged
inspection, D8-as-default behind a trait, manifest-declared raster access, and
parity-green final geometry.

### M5 - Rust Core API Cutover And Campaign Boundary

Goal: make the Rust core redesign coherent as the new public surface and record
what is intentionally deferred.

Scope:

- Stabilize result types for terminal unit, resolved outlet, upstream
  drainage-unit records, refinement provenance, final geometry, and area.
- Document intentional divergence: pristine pre-refinement unit records may not
  union or sum to final refined geometry/area.
- Ensure telemetry and error names match the staged pipeline.
- Update migration notes: v0.1 input unsupported, HFX v0.2.1 required.
- Document that pyshed redesign is deferred even though thin bindings were kept
  compiling in M2.
- Ensure every implementation commit includes a patch bump and tag per
  `CLAUDE.md`.

Explicit non-scope:

- pyshed redesign is deferred to a separate campaign/design session.
- Full aux-to-strategy binding is deferred.
- Python-authored strategies are deferred.
- Level-selection strategies beyond default finest are deferred.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test -p shed-core
rg "atom|Atom" crates/core/src crates/core/README.md docs/hfx-v02-redesign
```

The grep gate must have an explicit allowlist for historical fixture names or
quoted migration notes only; no core public engine result method may expose
`atom` terminology for drainage units. A docs-presence check must verify that
the migration docs state the hard cut to HFX v0.2.1 and list deferred pyshed,
aux-binding, Python-authored strategy, and non-finest level-selection work.

Why it is a vertical slice:

This proves the Rust core redesign is usable, inspectable, named consistently,
and ready to hand off to separate pyshed and aux-binding design work without
leaving a red workspace.

## 2. Ordering Justification

M1 must come first because parity cannot be reconstructed after the v0.1 loader
and v0.1 hfx-core vocabulary are removed. The oracle is committed output and
byte-identical raster input, so later milestones do not need executable v0.1
compatibility.

M2 must come before all v0.2 execution work because the local HFX v0.2.1
contract removes `AtomId`, `AtomCount`, and v0.1 snap `MainstemStatus`
semantics. The dependency cutover, repo-wide `UnitId` migration, `StemRole`
resolver migration, `graph.parquet` reader, `hfx.aux.snap.v1` reader, and v0.2
fixture builder are one compile boundary. Splitting naming or snap cleanup into
later milestones would leave the crate or its test suite unbuildable.

M3 comes after M2 because staged delineation depends on the v0.2.1 session,
multi-level unit records, graph reader, snap reader, and fixture builder. It
also must fix level selection before resolution; otherwise nested multi-level
data resolves to coarser parents by area.

M4 comes after M3 because refinement needs typed stage outputs: selected level,
resolved terminal unit, upstream unit records, terminal geometry, and dissolve
semantics. It also depends on M2's parsed aux declarations and M1's durable
goldens.

M5 comes last because public API polish and migration documentation should
settle around the proven Rust core behavior. It no longer carries the forced
atom-to-unit migration; that happens in M2 where the dependency cut makes it
unavoidable.

Reordering consequences:

- Starting M2 before M1 loses the only trustworthy v0.1 parity oracle.
- Starting M3 before M2 designs stages around obsolete v0.1 symbols and
  fixtures, and around a snap ranking model that no longer exists.
- Starting M4 before M3 turns the trait into an unanchored abstraction without
  stable stage inputs.
- Deferring pyshed compile handling or test-code compilation past M2 leaves the
  workspace apparently green while stale `AtomId`, `DatasetBuilder`, or
  `MainstemStatus` references remain.

## 3. Cross-Milestone Dependencies

- M2 depends on M1 for committed golden outputs, byte-identical raster inputs,
  canonicalization rules, and converted-fixture requirements.
- M3 depends on M2 for the hfx-core v0.2.1 dependency, `UnitId` vocabulary,
  `StemRole` resolver vocabulary, HFX v0.2.1 loader, graph reader, snap reader,
  and v0.2 fixture builder.
- M4 depends on M1 for parity goldens, M2 for D8 aux declarations and raster
  path resolution inputs, and M3 for staged refinement inputs and dissolve
  outputs.
- M5 depends on M2 for completed terminology migration and pyshed build hold,
  and on M3/M4 for final stage/result names.

Deferred design-session boundaries:

- Full aux-to-strategy binding blocks richer custom refinement support, but not
  the minimal built-in D8 accessor in M4.
- pyshed redesign depends on M5 and belongs outside this campaign.
- Python-authored strategies depend on a separate FFI/callback design.
- Level-selection strategies beyond default finest depend on a separate
  level-selection design session.

## 4. Risks

- The v0.2.1-contract hfx-core appears unpublished: crates.io only carries the
  v0.1-contract crate `0.2.0`, while the v0.2.1 contract lives at crate `0.2.64`
  in local `../hfx`. M2 must therefore expect to pin an exact git revision or
  path dependency, not a crates.io bump, unless `../hfx` publishes the
  v0.2.1-contract crate first. Note the crate-version vs HFX-format-version
  collision (`hfx-core 0.2.x` does not imply HFX format `0.2.x`) and annotate
  the dependency accordingly.
- The M2 compile boundary is large because `AtomId` disappears from the
  dependency and `MainstemStatus` is replaced by `StemRole`. The gate is
  intentionally workspace-wide and test-compiling to catch partial renames,
  stale fixture-builder users, snap resolver breakage, and pyshed breakage
  early.
- Canonical geometry comparison can still expose real nondeterminism in dissolve
  or WKB encoding. M1 turns this into an acceptance criterion by defining the
  normalizer before the cutover.
- A realistic real-raster parity fixture may be too large to check in. M1 must
  either include one or explicitly mark parity coverage as synthetic and create
  a follow-up task for representative real-data coverage.
- Multiple same-level snap aux declarations may need a deterministic temporary
  rule in M3. Broader snap selection policy is not allowed to grow into a full
  strategy design in this campaign.
- DAG loading is included only to preserve topology and prove dedup-safe
  traversal. Any DAG-specific hydrologic policy remains outside this campaign.
