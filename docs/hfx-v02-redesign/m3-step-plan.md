# M3 Step Plan - Finest-Level Staged Delineation Skeleton

Milestone 3 decomposes the current monolithic `Engine::delineate` path into a
fixed, typed, inspectable stage skeleton while preserving final geometry and
area behavior. The M3 stage order is:

```text
select level -> resolve outlet within that level -> traverse upstream same-level graph
-> produce pre-merge drainage-unit records -> refine terminal placeholder
-> dissolve/assemble -> compose result
```

This order intentionally supersedes the older roadmap diagram that showed
resolve before level selection. For default-finest behavior, level selection is
dataset-global: `max(level)` from the loaded session. It needs no outlet and
must constrain resolution so nested coarser parents cannot win by area.

## Confirmed Starting Facts

- shed root version is `0.1.122`.
- `cargo build --workspace --exclude pyshed` and `cargo check -p pyshed` are the
  build gates; `pyshed` is a build-hold only.
- M1 `parity_golden_artifacts` is loader-independent and uses
  `shed-canonical-wkb-v1`, 6-decimal canonical WKB with total-order
  ring/hole/component ordering.
- The M2 converted fixture
  `crates/core/tests/fixtures/parity/v021_synthetic_refined/` exists with
  byte-identical B rasters copied from M1.
- Current `Engine::delineate` is monolithic:
  resolve outlet, collect upstream units, try refinement, assemble watershed,
  compose `DelineationResult`.
- `DelineationOptions.refine` is currently a raw `bool`.
- `resolver.rs` has no selected-level input. PiP bbox candidates are tie-broken
  by upstream area, local area, then unit ID across all candidate levels. Snap
  candidates are read from the first snap declaration only and are queried by
  bbox, not by `references_levels`.
- `DatasetSession::validate_graph_catchments` builds a
  `HashMap<UnitId, Level>` but `DatasetSession` does not store it and exposes no
  `max_level()`, `levels()`, or `level_of()` accessor.
- `assemble_watershed` is the dissolve stage today: refine-on substitutes a
  terminal geometry override; refine-off dissolves all whole upstream units,
  including the terminal.
- `dissolve_spatial_reduce_strategy` sorts by `spatial_key` and uses the fixed
  `rayon::join` split tree in `dissolve_reduce_strategy`; M3 must preserve this
  path verbatim.
- HFX v0.2.1 says `catchments.parquet` carries `level`, `parent_id`,
  `area_km2`, `up_area_km2`, outlet coordinates, bbox, and geometry;
  `graph.parquet` edges are same-level; `hfx.aux.snap.v1` declarations have
  `references_levels`; snap rows derive their level from the referenced unit.
- GRIT `2.0.0` has exactly two `hfx.aux.snap.v1` declarations and no
  `hfx.aux.d8_raster.v1`.

## M3 Gate

Offline gate, green with no network:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test finest_level_resolution
cargo test -p shed-core --test snap_resolution_cascade
cargo test -p shed-core --test parity_golden_artifacts
```

Network-gated tier:

```bash
SHED_HFX_V02_REAL_R2_DELINEATION=1 cargo test -p shed-core --test finest_level_resolution -- --ignored --nocapture
SHED_HFX_V02_REAL_R2_DELINEATION=1 cargo test -p shed-core --test snap_resolution_cascade -- --ignored --nocapture
```

The ignored GRIT `2.0.0` coverage must follow the M1/M2 `#[ignore]` plus
env-switch pattern, but uses a bounded readiness-tier proof rather than full
`DatasetSession::open` over all GRIT units. It proves default level is the
finest present using row-group `level` statistics; a real outlet resolves to the
finest containing unit rather than a coarser L0 parent by bbox-bounded
resolution; and the real L0/L1 snap aux ranking follows weight, `mainstem`,
distance, then snap ID through bounded snap decode.

Every executor step is an independent commit boundary. Every commit must:

```bash
./scripts/bump-version.sh patch
git add Cargo.toml Cargo.lock <step files>
git commit -m "<conventional commit message>"
git tag v$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
```

Do not let tooling create its own commit or tag. Always stage `Cargo.lock` with
`Cargo.toml` even if the diff is empty after inspection. Use `tracing`, not
`log` or `println!`; library errors use `thiserror` with named fields and
doc-commented variants explaining when they fire; no `.unwrap()` or `.expect()`
in library code. Apply newtypes, enums-over-bools, parse-don't-validate, and
typestate only where it removes a real invalid state. Use Mermaid diagrams, not
ASCII diagrams.

## Step 1 - Phase 0 Type And Stage Contract Design

**Rationale**

The intermediate types are the interface. M3 must design the stage contracts
before moving code so that inspect-a-step and swap-refinement-later fall out
from the types instead of from comments.

**Exact Scope**

- Add or update stage-contract documentation in `crates/core/README.md` with a
  Mermaid pipeline diagram for the M3 fixed skeleton.
- Add the concrete Rust type vocabulary in the narrowest engine/stage module
  that fits the current layout, for example `crates/core/src/staged.rs` or a
  local `engine::staged` module if public re-export timing is still unsettled.
- Touch `crates/core/src/engine.rs` only to expose the contracts if needed; do
  not mechanically decompose behavior in this step.

**Type/Contract Changes**

Define the vocabulary on paper and in code:

- `LevelSelection`: enum with `Finest` only in M3. This is the typed level
  choice; no user strategy and no raw `Level` parameter at the public boundary.
- `SelectedLevel`: private-field newtype containing `Level`, constructible only
  from `DatasetSession` via `LevelSelection`. This enforces "cannot resolve at a
  level the dataset lacks".
- `RefinementMode`: enum replacing `DelineationOptions.refine: bool`, with
  `BestEffort` as the default current behavior and `Disabled` for refine-off.
  The existing `with_refine(bool)` may remain as a deprecated compatibility
  shim in M3, but it must map immediately to the enum at the boundary.
- `LevelResolvedOutlet`: resolved outlet plus `SelectedLevel`, with invariant
  `session.level_of(unit_id) == selected.level()`.
- `SameLevelUpstreamUnits`: terminal, selected level, and `UpstreamUnits`, with
  invariant that every contained unit is at the selected level.
- `PreMergeDrainageUnit`: pristine record `{ id, level, area, up_area, outlet,
  geometry }`.
- `PreMergeDrainageUnits`: terminal-first collection of
  `PreMergeDrainageUnit`, including the whole terminal polygon. This stage is
  intentionally not carved.
- `TerminalRefinement`: placeholder result shape only. In M3 it can represent
  `Disabled`, `NoRastersAvailable`, `NoRasterSourceProvided`, or `Applied`
  using today's internal D8 path if already present; do not introduce a D8 trait
  or strategy seam.
- `DissolvedWatershed`: final geometry plus `AreaKm2`, produced only by the
  dissolve/assemble stage.

Exact independently callable stage signatures:

```rust
pub fn select_level(&self, choice: LevelSelection) -> Result<SelectedLevel, EngineError>;

pub fn resolve_outlet_at_level(
    &self,
    outlet: GeoCoord,
    level: SelectedLevel,
    config: &ResolverConfig,
) -> Result<LevelResolvedOutlet, EngineError>;

pub fn traverse_upstream_at_level(
    &self,
    outlet: &LevelResolvedOutlet,
) -> Result<SameLevelUpstreamUnits, EngineError>;

pub fn produce_pre_merge_units(
    &self,
    upstream: &SameLevelUpstreamUnits,
) -> Result<PreMergeDrainageUnits, EngineError>;

pub fn refine_terminal_placeholder(
    &self,
    resolved: &LevelResolvedOutlet,
    units: &PreMergeDrainageUnits,
    options: &DelineationOptions,
) -> Result<TerminalRefinement, EngineError>;

pub fn dissolve_watershed(
    &self,
    units: &PreMergeDrainageUnits,
    refinement: &TerminalRefinement,
    options: &DelineationOptions,
) -> Result<DissolvedWatershed, EngineError>;

pub fn compose_result(
    &self,
    resolved: LevelResolvedOutlet,
    upstream: SameLevelUpstreamUnits,
    refinement: TerminalRefinement,
    dissolved: DissolvedWatershed,
) -> DelineationResult;
```

`Engine::delineate` must become a thin composition that literally calls these
stage methods in order, so `delineate() == explicit staged composition` by
construction.

Typestate decision: do not add a fluent typestate pipeline in M3 unless the
executor discovers the free functions cannot protect order clearly. The staged
function inputs already encode lifecycle constraints: traversal requires a
`LevelResolvedOutlet`; resolution requires a `SelectedLevel`; `SelectedLevel`
requires a real session level. Typestate is warranted only if a `DelineationRun`
object is introduced for ergonomics and compile-time ordering genuinely removes
duplicated runtime checks. Residual invariant: same-run consistency across
separately passed intermediates is a caller contract in M3, not type-enforced;
`DelineationRun` is the M4/M5 home if that invariant needs compile-time
ownership.

`TerminalRefinement::NoRasterSourceProvided` is M4-volatile because M4 will
re-key refinement availability around D8 aux declarations rather than today's
engine-level `RasterSource` attachment.

**Verification**

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test parity_golden_artifacts
```

Manual read-only checks: confirm the README Mermaid diagram uses the M3 order;
confirm the contracts expose stage outputs without adding strategy seams.

**Risk/Escape**

STOP and escalate if the contract cannot preserve current
`DelineationResult` fields or if introducing the types requires a pyshed
redesign instead of a minimal binding build-hold update.

## Step 2 - Persist Session Level Index And Select Default Finest

**Rationale**

Default-finest level selection is dataset-global and must precede resolution.
The session already computes the required `UnitId -> Level` map; M3 should store
and expose it rather than recomputing from Parquet.

**Exact Scope**

- `crates/core/src/session.rs`: store the `HashMap<UnitId, Level>` currently
  returned by `validate_graph_catchments`.
- Add `DatasetSession::level_of(UnitId) -> Option<Level>`,
  `DatasetSession::levels() -> impl Iterator<Item = Level>` or an owned small
  sorted collection, and `DatasetSession::max_level() -> Option<Level>`.
- For the ignored GRIT readiness-tier tests, add a bounded `max_level` /
  `level_of` path that uses row-group `level` statistics and bounded lookup
  rather than materializing the full stored map. This is distinct from the
  offline session path, which may reuse the stored
  `HashMap<UnitId, Level>` from `validate_graph_catchments`
  (`crates/core/src/session.rs:743`).
- Add `SelectedLevel` construction through `Engine::select_level`.
- Add focused unit coverage in `crates/core/tests/staged_delineation.rs` or
  `crates/core/tests/finest_level_resolution.rs`.

**Type/Contract Changes**

`SelectedLevel` is parse-don't-validate at the stage boundary: raw
`LevelSelection::Finest` is parsed into a `SelectedLevel` only after the session
proves that level exists. No downstream stage accepts a raw selected `Level`.

If no levels exist despite a non-empty manifest, introduce a documented
`EngineError` variant with named fields explaining that session level indexing
is empty for a loaded dataset. This should be unreachable after M2 validation;
if it fires in tests, treat it as an integrity error.

**Verification**

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

The staged test proves `LevelSelection::Finest` returns the max fixture level
and that `SelectedLevel` cannot be constructed from a missing level through the
public API.

**Risk/Escape**

STOP if the executor is tempted to scan `catchments.parquet` during every level
selection call. The stored validation index is the M3 path.

## Step 3 - Constrain PiP Resolution To The Selected Level

**Rationale**

On nested multi-level data, a point can be inside both a fine unit and its
coarser parent. The current area tie-break can choose the coarser parent. M3
must filter to the selected level before applying the existing PiP tie-break.

**Exact Scope**

- `crates/core/src/resolver.rs`: add a selected-level-aware resolver entry
  point, for example `resolve_outlet_at_level(session, outlet, level, config)`.
- Keep the existing resolver behavior available only as a compatibility wrapper
  if tests still need it; production `Engine::delineate` must call the
  level-aware path.
- `crates/core/tests/finest_level_resolution.rs`: add a two-level nested
  offline fixture test.

**Type/Contract Changes**

PiP path rules:

- Query by bbox as today.
- Decode candidates as today.
- Filter candidate catchments to `SelectedLevel` before strict
  contains/intersects and before upstream-area/local-area/unit-id tie-break.
- Report `candidates_considered` consistently. The plan preference is to expose
  the count after selected-level filtering because the stage is "resolution
  within this level"; if preserving existing provenance is required, add a
  separate internal count rather than changing the external meaning silently.
- Return `LevelResolvedOutlet`, not raw `ResolvedOutlet`, from the engine stage.

Behavior on existing single-level fixtures must not change because the level
filter is a no-op when all units have the same level.

**Verification**

```bash
cargo test -p shed-core --test finest_level_resolution
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

`finest_level_resolution` proves an outlet inside a nested L1 child and L0
parent resolves to the L1 child under default finest, even if the L0 parent has
larger `area_km2` and `up_area_km2`.

**Risk/Escape**

STOP if any proposed fix changes PiP geometry containment semantics or the
single-level tie-break cascade. M3 may filter by level; it may not redesign PiP.

## Step 4 - Constrain Snap Resolution To The Selected Level

**Rationale**

Snap files do not store a level column. HFX derives snap level from the target
unit and constrains declarations through `references_levels`. M3 must enforce
both constraints before ranking candidates.

**Exact Scope**

- `crates/core/src/session.rs`: expose snap declarations enough for resolver
  selection without adding a general aux-to-strategy binding mechanism. This
  must include opening the level-matching declaration's snap parquet, not only
  exposing metadata for the store currently returned by `session.snap()`.
- `crates/core/src/resolver.rs`: make snap resolution selected-level-aware.
- `crates/core/src/reader/snap_store.rs`: add only narrow query helpers if
  needed; do not redesign the store.
- `crates/core/tests/snap_resolution_cascade.rs`: add offline snap fixtures for
  level filtering and cascade ranking.

**Type/Contract Changes**

Snap declaration selection:

- Candidate declarations are `hfx.aux.snap.v1` entries whose
  `references_levels` contains `SelectedLevel`.
- `DatasetSession` must make the selected declaration's `SnapStore` available.
  Acceptable M3 implementations are either opening all snap declarations at
  session open as a small `Vec<(SnapDecl, SnapStore)>` keyed by declaration
  metadata, or lazily opening the chosen declaration's artifact at resolve time
  from the session root/remote store. The current `snaps.first()`-only store is
  insufficient because GRIT `2.0.0` has two snap declarations and the
  finest-level declaration may not be first.
- Narrow deterministic multiple-match rule for M3: sort matching declarations
  by `metadata.name` ascending, then artifact `snap` path ascending, and use the
  first declaration only. Record this in docs and tests. Before Step 4 starts,
  verify the real GRIT manifest: if its two declarations are per-level, this
  rule will not fire on GRIT, and the real-data proof should instead emphasize
  that the L1 declaration's store is opened rather than `snaps.first()`.
  Broader merge/priority policy is deferred.
- If no declaration matches the selected level, snap resolution falls back to
  PiP only if that is today's configured behavior; otherwise it returns the
  existing no-snap/no-candidate style error. Do not invent a new strategy.

Snap target filtering:

- Query the selected declaration's snap artifact by bbox.
- For every snap candidate, require
  `session.level_of(target.unit_id()) == SelectedLevel`.
- Then apply radius filtering.
- Then rank using M3's required `WeightFirst` cascade:
  `weight DESC -> stem_role == mainstem -> distance ASC -> snap_id ASC`.
- Existing `DistanceFirst` behavior remains opt-in and keeps its current
  tolerance-band semantics.

This deliberately preserves today's production `WeightFirst`
`weight -> mainstem -> distance -> snap_id` ordering from
`crates/core/src/resolver.rs:458`, so existing single-level fixtures stay green
by construction, not incidentally.

**Verification**

```bash
cargo test -p shed-core --test snap_resolution_cascade
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

`snap_resolution_cascade` proves:

- declarations not referencing the selected level are ignored;
- snap targets whose unit level differs from `SelectedLevel` are ignored even
  when the declaration matches;
- higher weight beats lower weight;
- `StemRole::Mainstem` beats non-mainstem on equal weight;
- after equal weight and stem role, the nearer candidate with lower distance
  wins;
- lower `snap_id` breaks only an exact weight, stem-role, and distance tie.

**Risk/Escape**

STOP if implementing declaration choice starts to look like aux-to-strategy
binding or user-authored resolution strategy selection. M3's rule is deliberately
narrow.

## Step 5 - Add Pre-Merge Drainage-Unit Stage

**Rationale**

Roadmap R3 requires two inspection outputs that may disagree: pristine upstream
unit records and final watershed geometry. M3 must make the pre-merge output
explicit before dissolve.

**Exact Scope**

- `crates/core/src/engine.rs` or the new staged module: implement
  `produce_pre_merge_units`.
- `crates/core/src/reader/catchment_store.rs`: add a narrow full-record query
  helper only if current `query_by_ids` cannot return geometry together with
  id, level, area, upstream area, and outlet.
- `crates/core/tests/staged_delineation.rs`: assert record contents.

**Type/Contract Changes**

`PreMergeDrainageUnit` is pristine:

```rust
pub struct PreMergeDrainageUnit {
    id: UnitId,
    level: Level,
    area: AreaKm2,
    up_area: Option<AreaKm2>,
    outlet: GeoCoord,
    geometry: MultiPolygon<f64>,
}
```

`PreMergeDrainageUnits` includes the whole terminal polygon. It never contains
a carved terminal. This intentional divergence must be documented in the type
docs and README:

- summing `PreMergeDrainageUnit.area` does not define final `area_km2`;
- unioning pre-merge geometries does not define final refined geometry;
- final geometry is produced only by the downstream dissolve stage.

The terminal must remain first or otherwise be available through a typed
accessor; non-terminal ordering is deterministic but should not become a public
semantic promise beyond what tests need. This terminal-first ordering cannot
affect final geometry because the downstream dissolve path must still call
`dissolve_spatial_reduce_strategy`, which re-sorts polygons by `spatial_key`
before the fixed reduce tree.

**Verification**

```bash
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

The staged test proves the v0.2.1 converted fixture's pre-merge list includes
the terminal as a whole polygon and includes each required field. It also proves
all records are at `SelectedLevel`.

**Risk/Escape**

STOP if this stage starts modifying geometries, dropping the terminal, carving
the terminal, or computing final area. Those belong downstream.

## Step 6 - Capture Offline Refine-Off Backstop Before Refactor

**Rationale**

A parity backstop must predate the refactor it polices. Capturing a golden after
the staged dissolve rewrite would freeze any Step 7 drift as "correct" and
provide a false green.

**Exact Scope**

- Before moving `try_refine`, `assemble_watershed`, or `delineate`, capture a
  small offline non-refined golden from the current M2 engine against
  `crates/core/tests/fixtures/parity/v021_synthetic_refined/`.
- If Step 6 runs after Step 1 has touched code, assert byte-equality of the
  capture against a pre-Step-1 baseline run before committing it, so the
  backstop is anchored to M2 behavior rather than post-refactor output.
- Store it under a committed fixture path such as
  `crates/core/tests/fixtures/parity/goldens/v021_synthetic_nonrefined/`.
- Record provenance in the fixture README: captured from the pre-M3 M2 engine,
  with `RefinementMode::Disabled` or today's `with_refine(false)`, before staged
  geometry movement, including the commit/version used for both the baseline
  and capture.
- Use the M1 canonicalizer contract (`shed-canonical-wkb-v1`) and include final
  canonical WKB, `area_km2`, terminal ID, upstream IDs, resolved outlet, and
  disabled refinement outcome.

**Type/Contract Changes**

No production type changes. This step creates inert evidence before behavior
moves.

**Verification**

```bash
cargo test -p shed-core --test parity_golden_artifacts
```

If a temporary capture harness is needed, remove or gate it before the commit so
the durable assertion remains loader-independent or fixture-local. Do not
regenerate the golden after Step 7.

**Risk/Escape**

STOP if no one can prove the bytes came from the pre-M3 engine. Do not bless
post-refactor output as the baseline.

## Step 7 - Refine Placeholder And Dissolve Stage Without Geometry Drift

**Rationale**

M3 needs the refinement result shape so the future M4 D8 trait has a typed slot,
but it must not introduce the D8 strategy seam yet. The dissolve stage must
preserve current geometry behavior.

**Exact Scope**

- `crates/core/src/engine.rs`: move `try_refine` logic behind
  `refine_terminal_placeholder` without changing outcomes, but reuse the
  terminal polygon already decoded into `PreMergeDrainageUnits`.
- `crates/core/src/assembly.rs`: expose or route through the existing
  geometry-only `assemble_from_geometries` path so `dissolve_watershed`
  consumes pre-merge geometries rather than re-querying catchments by ID.
- `crates/core/src/algo/dissolve.rs`: do not change the deterministic
  `spatial_key` sort or fixed `rayon::join` split tree.
- `crates/core/tests/staged_delineation.rs`: add refined-off and
  best-effort/no-raster cases.
- Re-home terminal-override guard coverage from the `assemble_watershed` ID
  query path to the new `dissolve_watershed` stage: empty refined terminal,
  terminal replacement, and "bad whole terminal WKB is bypassed when a refined
  override exists" must still be covered where behavior now lives.

**Type/Contract Changes**

`TerminalRefinement` is a placeholder output shape, not a trait:

- `Disabled`
- `NoRastersAvailable`
- `NoRasterSourceProvided`
- `Applied { refined_outlet, geometry }` if current behavior already applies
  through the existing `RasterSource`

`RefinementOutcome` in `DelineationResult` must be composed from
`TerminalRefinement` so the public result stays compatible.

`dissolve_watershed` consumes pristine `PreMergeDrainageUnits` plus
`TerminalRefinement`. Refine-off dissolves all whole units. Refine-on dissolves
whole upstream units minus terminal plus the refined terminal override, exactly
as `assemble_watershed` does today. It must build the geometry list from
`PreMergeDrainageUnits` and call `assemble_from_geometries`, preserving
`build_assembly_options` handling for geometry repair, hole-fill mode, and
clean epsilon. It must not call `assemble_watershed` after pre-merge has already
decoded geometry, because that would re-query and re-decode every unit.

`produce_pre_merge_units` must obtain terminal and unit geometry through the
instrumented `query_geometries_by_ids` path
(`crates/core/src/reader/catchment_store.rs:1112`), not by manually calling
`CatchmentUnit::geometry()` plus `decode_wkb_multi_polygon` as the uncounted
resolver path does (`crates/core/src/resolver.rs:537`).
`refine_terminal_placeholder` already receives `&PreMergeDrainageUnits`; it
must reuse the pre-merge terminal polygon from that collection rather than
calling `query_geometries_by_ids(&[terminal])`. Preserve the existing
decode-once invariant tested by `applied_refinement_decodes_terminal_geometry_once`;
the test's `== 1` assertion must stay meaningful and must not be weakened.

**Verification**

```bash
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

Add a determinism check in `staged_delineation`: run the dissolve stage over the
same pre-merge units more than once and compare canonical WKB bytes. This test
must exercise the existing `dissolve_spatial_reduce_strategy` path, not a new
union implementation. Prefer the existing permute-under-fixed-4-thread-pool
pattern from the dissolve tests over a plain run-twice check.

Add a focused offline Applied-path structural check even though full D8 refined
parity waits for M4: when `TerminalRefinement::Applied` supplies a refined
terminal geometry, the dissolve stage substitutes it and excludes the whole
terminal polygon from the final geometry list.

**Risk/Escape**

STOP if any step requires changing dissolve results, canonicalizer precision,
hole-fill behavior, topology cleaning, or area computation. M3 may move logic;
it may not change final geometry/area behavior.

## Step 8 - Make `delineate()` A Thin Staged Composition

**Rationale**

The one-call API remains important, but after M3 it should be a convenience
composition over inspectable stages, not a separate behavior path.

**Exact Scope**

- `crates/core/src/engine.rs`: rewrite `Engine::delineate` to call the stage
  methods in the fixed M3 order.
- Preserve `delineate_area_only`, batch methods, telemetry stage guards, and
  error provenance.
- Minimal `crates/python` binding updates only if a core signature used by
  pyshed changes enough to break `cargo check -p pyshed`.

**Type/Contract Changes**

`DelineationOptions` stores `RefinementMode`, not `bool`. Keep
`with_refine(bool)` as a boundary parser if needed:

- `true -> RefinementMode::BestEffort`
- `false -> RefinementMode::Disabled`

Add `with_refinement_mode(RefinementMode)` for typed callers.

`compose_result` must use the same final fields as today:
terminal unit ID, input outlet, resolved outlet, resolution method, upstream
unit IDs, refinement outcome, geometry, and `area_km2`.

**Verification**

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

`staged_delineation` proves `Engine::delineate(outlet, options)` equals the
explicit staged composition on terminal ID, resolved outlet, resolution method,
upstream ID set, refinement outcome, final canonical WKB, and `area_km2`.

**Risk/Escape**

STOP if `delineate()` and explicit composition can diverge because they call
different helpers. The implementation should literally compose the public stage
methods.

## Step 9 - Assert Offline Parity Backstop For Refine-Off Geometry

**Rationale**

M3 restructures geometry-producing logic, but offline refined parity against M1
cannot be re-proven until M4 because the D8 carve trait is not in scope. The
non-refined real Oracle A path is network-gated. A silent M3 drift in dissolve
or refine-off assembly would otherwise surface only at M4. Step 6 captured the
baseline before the refactor; this step wires the durable assertion against it.

**Exact Scope**

- Add the assertion in `crates/core/tests/staged_delineation.rs` or a focused
  parity helper used by that test.
- The assertion must read the pre-M3 golden from Step 6 and must not regenerate
  or re-bless it.

**Type/Contract Changes**

No production type changes. This is a behavior-preservation guard.

Chosen backstop: option (a), offline refine-off golden from the M2 converted
v0.2.1 fixture, captured before the M3 refactor in Step 6. It is preferred for
M3 because it is network-free, exercises the staged dissolve/refine-off path,
and uses the same canonical WKB normalizer as M1. Option (b), network-gated
Oracle A re-parity, remains useful but is not the primary M3 close criterion
because offline must catch drift without network.

**Verification**

```bash
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test parity_golden_artifacts
```

The staged test fails if canonical final WKB, terminal ID, upstream IDs, or
`area_km2` drift from the captured non-refined v0.2.1 golden.

**Risk/Escape**

STOP if the executor cannot explain whether a red parity backstop is caused by
intentional behavior change, fixture conversion, or canonicalization. Do not
re-bless the golden to make M3 pass.

## Step 10 - Wire Final Offline And Network Gates

**Rationale**

M3 closes only when the full gate proves the skeleton, level-aware resolution,
snap cascade, M1 parity artifacts, pyshed build hold, and real GRIT
multi-level/snap behavior.

**Exact Scope**

- `crates/core/tests/staged_delineation.rs`
- `crates/core/tests/finest_level_resolution.rs`
- `crates/core/tests/snap_resolution_cascade.rs`
- Existing `crates/core/tests/parity_golden_artifacts.rs` remains green and is
  not rewritten except for any required fixture-path documentation.
- Add ignored GRIT `2.0.0` tests following the M1/M2 env-switch pattern.

**Type/Contract Changes**

No new production types unless test compilation exposes a missing accessor that
belongs to earlier steps.

Test target mapping:

- `staged_delineation`: verifies each stage output independently on the
  converted v0.2.1 fixture; proves `delineate()` equals explicit staged
  composition; proves pre-merge includes the whole terminal; proves final
  geometry comes from dissolve; proves the offline refine-off parity backstop;
  includes the dissolve determinism canonical-WKB check.
- `finest_level_resolution`: uses a two-level nested fixture to prove default
  finest resolves an outlet to the finest containing unit, not a coarser parent.
  Its ignored GRIT test proves default level is the finest present using
  row-group `level` statistics and a real outlet resolves to the finest
  containing unit through bounded resolution.
- `snap_resolution_cascade`: uses an offline snap.v1 fixture to prove selected
  level filtering and `weight -> mainstem -> distance -> snap_id`. Its ignored
  GRIT test proves the same cascade on the real L0/L1 snap aux through bounded
  snap decode.
- `parity_golden_artifacts`: proves M1 loader-independent artifacts stay green
  and canonicalizer contract is unchanged.

**Verification**

Run the full M3 offline gate:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test finest_level_resolution
cargo test -p shed-core --test snap_resolution_cascade
cargo test -p shed-core --test parity_golden_artifacts
```

Run the network tier only when explicitly enabled:

```bash
SHED_HFX_V02_REAL_R2_DELINEATION=1 cargo test -p shed-core --test finest_level_resolution -- --ignored --nocapture
SHED_HFX_V02_REAL_R2_DELINEATION=1 cargo test -p shed-core --test snap_resolution_cascade -- --ignored --nocapture
```

**Risk/Escape**

STOP if offline tests require network, if GRIT tests run without the env switch
or require full `DatasetSession::open`, if `cargo check -p pyshed` requires a
public Python redesign, or if any final geometry/area drift appears outside the
level-filtering surface.

## Explicit Out Of Scope

- No level-selection strategy beyond default finest.
- No user-authored resolve, traverse, dissolve, or level-selection strategy
  seam.
- No D8 refinement trait; M4 owns the trait. M3 only provides a placeholder
  refinement result shape and may move today's `try_refine` behavior.
- No mixed-level or cross-level traversal.
- No pyshed redesign and no CLI JSON-key rename. `pyshed` remains build-hold
  only through `cargo check -p pyshed`; if a core signature breaks bindings,
  make the minimal mechanical binding update.
- No general aux-to-strategy binding mechanism.
- No dissolve algorithm replacement and no canonicalizer precision/version
  change.

## Decisions For The Human

No open decisions remain.

- RESOLVED: GRIT network-proof scope is bounded readiness-tier, mirroring M2
  with row-group `level` statistics, bbox-bounded resolution, and bounded snap
  decode rather than full `DatasetSession::open`.
- RESOLVED: production `WeightFirst` retains distance as a sub-tie-break:
  `weight -> mainstem -> distance -> snap_id`, matching
  `crates/core/src/resolver.rs:458`.

No human decision is needed for the offline refine-off parity backstop or the
narrow deterministic snap declaration rule; this plan chooses those so the
sub-orchestrator does not invent broader policy.
