# Milestone 4 Step Plan - D8 Refinement Trait As The Default Strategy

Revision notes: this plan incorporates the adversarial critique in
`docs/hfx-v02-redesign/m4-step-critique.md`. The corrected plan removes the
phantom "containment clamp", avoids a strict containment gate that can reject
today's raster-mask carve, gives the strategy access to the engine's
`RasterSource`, and keeps legacy raster APIs alive until the engine is swapped
to the strategy.

Starting facts re-confirmed on 2026-06-04:

- Workspace version is `0.1.133`; branch is `main`; unrelated untracked files
  exist and must not be touched.
- M4's contract is the `docs/hfx-v02-redesign/milestone-plan.md` M4 section.
  The build gate is `cargo build --workspace --exclude pyshed`; `pyshed` remains
  a `cargo check -p pyshed` build hold.
- Current refinement is `Engine::refine_terminal_placeholder` in
  `crates/core/src/engine.rs`. It directly localizes rasters, then calls
  `refine_terminal_from_source`.
- The D8 carve in `crates/core/src/algo/refine.rs` is:
  rasterize terminal -> mask flow direction and accumulation -> snap on masked
  accumulation -> masked upstream trace -> polygonize the trace mask. There is
  no output containment clamp, no intersection step, and no output cleaning
  tolerance. Do not add one.
- `D8RasterDecl` has only `flow_dir`, `flow_acc`, and `flow_dir_encoding`.
  The blessed-D8 spec says per-entry coverage is queried by inspecting raster
  headers at read time.
- `crates/core/tests/fixtures/parity/README.md` confirms M4's real-data D8
  target is `merit/0.2.0`, not M1's Oracle C input `merit-basins/0.1.0`.

## Type Design

The type seam is the first implementation step. Do not start by moving engine
code.

Place the M4 orchestration contract in `crates/core/src/refinement.rs` and
re-export the public pieces from `crates/core/src/lib.rs`. Keep
`crates/core/src/algo/refine.rs` as the algorithm module and preserve its carve
unchanged.

Trait sketch:

```rust
pub trait TerminalRefinementStrategy: Send + Sync {
    fn refine_terminal(
        &self,
        input: TerminalRefinementInput<'_>,
        pantry: &D8RefinementPantry<'_>,
    ) -> Result<TerminalRefinementDecision, TerminalRefinementError>;
}

pub struct TerminalRefinementInput<'a> {
    pub terminal_unit: UnitId,
    pub terminal_geometry: &'a MultiPolygon<f64>,
    pub resolved_outlet: GeoCoord,
    pub snap_threshold: SnapThreshold,
}

pub struct D8RefinementPantry<'a> {
    pub session: &'a DatasetSession,
    pub raster_source: Option<&'a (dyn RasterSource + Send + Sync)>,
}

pub enum TerminalRefinementDecision {
    Applied {
        refined_outlet: GeoCoord,
        geometry: ContainedTerminalPolygon,
        provenance: RefinementProvenance,
    },
    BestEffortSkipped {
        provenance: RefinementProvenance,
    },
}
```

Reasoning:

- Input is terminal-only and includes the pre-merge terminal geometry by
  reference, preserving M3's decode-once invariant.
- The pantry is deliberately D8-specific for M4. It is not the deferred general
  aux pantry or aux-to-strategy binding mechanism.
- `raster_source` must be in the pantry because `refine_terminal_from_source`
  requires `&dyn RasterSource`, and today the source lives on `Engine`, not on
  `DatasetSession`.
- The trait should be object-safe. The engine can hold
  `Box<dyn TerminalRefinementStrategy + Send + Sync>`, defaulting to the one
  shipped strategy: `D8RasterRefinementStrategy`.
- The strategy is mode-agnostic: `TerminalRefinementInput` deliberately does not
  carry `RefinementMode`. The engine interprets
  `TerminalRefinementDecision::BestEffortSkipped` as a visible skip under
  `BestEffort`, and escalates the same missing-data condition to a hard error
  before or after strategy dispatch under `RequireD8`.
- Scope the public claim honestly: M4 exposes the Rust refinement seam and ships
  the D8 strategy behind it, but meaningful custom-aux strategies remain blocked
  on the deferred aux-binding design. Do not market this as full custom aux
  authoring support.

Contained output wrapper:

```rust
pub struct ContainedTerminalPolygon {
    polygon: MultiPolygon<f64>,
}
```

This wrapper documents the terminal-shrink contract at the type boundary, but
M4 must not enforce strict vector containment. The unchanged carve emits a
polygonized raster mask; on real irregular terminal polygons it can extend a
fraction of a raster cell beyond the vector terminal even though it came from
terminal-masked flow inputs. Therefore:

- `ContainedTerminalPolygon::new_unchecked_from_d8_carve` or similarly named
  constructor may verify only non-emptiness for the built-in D8 result.
- If executors add an optional containment diagnostic, it must be non-fatal or
  tolerant to at least one selected raster cell in each direction. It must never
  hard-error unchanged D8 output.
- Do not add a clamp/intersection/cleaning step to make containment pass.
  Final overlap-free assembly is still provided by
  `dissolve(whole upstream - whole terminal + carved terminal)`.

Provenance shape:

```rust
pub enum RefinementProvenance {
    Disabled,
    Applied {
        strategy: RefinementStrategyName,
        why: AppliedRefinementReason,
    },
    BestEffortSkipped {
        strategy: RefinementStrategyName,
        why: BestEffortSkipReason,
    },
}

pub enum RefinementStrategyName {
    BuiltInD8,
    BestEffortD8IfPresent,
}

pub enum AppliedRefinementReason {
    D8AuxMatchedTerminalBbox { declaration_index: usize },
}

pub enum BestEffortSkipReason {
    NoD8AuxDeclared,
    NoRasterSourceProvided,
}
```

Map this onto M3 shapes with the smallest break:

- Keep `TerminalRefinement::Disabled`.
- Change `TerminalRefinement::Applied` to carry provenance.
- Replace `NoRastersAvailable` / `NoRasterSourceProvided` with a visible
  skipped shape, or keep compatibility constructors that map to
  `BestEffortSkipped` while result serialization is updated deliberately.
- Update every match site in `engine.rs`, especially dissolve and
  `refinement_outcome_from_terminal`.
- Before renaming result labels, inspect committed goldens. Current refined B is
  `Applied` and nonrefined backstop is `Disabled`, so no committed golden should
  require the old no-raster labels, but executors must confirm with `rg`.

`RefinementMode` remains an enum. Do not reintroduce booleans. If explicit D8
must be represented, add a named variant such as `RequireD8`; keep the default
as the named best-effort behavior.

Parse-don't-validate boundary:

- `reader::manifest::D8RasterDecl` is parsed manifest metadata.
- The M4 accessor converts a declaration into a typed `D8RasterHandle` only
  after resolving paths inside the dataset root and proving header coverage for
  the terminal bbox.
- Internal refinement code takes `D8RasterHandle`, never raw manifest strings.

## Oracle C vs merit/0.2.0 Resolution

Read-only investigation found no committed `merit/0.2.0` real-data golden or
cached v0.2.1 carve evidence. The committed Oracle C record is for
`merit-basins/0.1.0`, and the parity README explicitly says M4's target is
`merit/0.2.0`.

Therefore the network-gated `merit/0.2.0` test primarily proves readiness:
the accessor selects among 60 D8 declarations, range-reads only the selected
terminal-bbox windows, and produces an applied D8 carve that runs through final
dissolve. A match against M1 Oracle C is a documented bonus, not the expected
acceptance criterion.

Network-test decision:

- If `rhine_basel` from `merit/0.2.0` matches Oracle C under frozen
  `shed-canonical-wkb-v1` 6-dp canonicalization and area tolerance, document
  that bonus C-parity assertion in the ignored test.
- If it diverges, keep Oracle C strict, document that the v0.1 golden does not
  transfer to the v0.2.1 recompile, and assert readiness/containment-diagnostic
  invariants instead.
- Escalate if divergence is material and unexplained, or if the remote dataset
  cannot establish whether `rhine_basel` is covered by a single D8 declaration.

## Step List

### 1. Define the D8-for-now Refinement Contract and Provenance Types

Goal: front-load the seam and provenance types while keeping the old placeholder
behavior buildable.

Files touched:

- `crates/core/src/refinement.rs` (new)
- `crates/core/src/lib.rs`
- `crates/core/src/staged.rs`
- `crates/core/src/engine.rs` for result/provenance mapping only
- `crates/core/README.md`

Type/code shape:

- Add `TerminalRefinementStrategy`, `TerminalRefinementInput`,
  `D8RefinementPantry`, `ContainedTerminalPolygon`,
  `TerminalRefinementDecision`, `RefinementProvenance`,
  `RefinementStrategyName`, `AppliedRefinementReason`, and
  `BestEffortSkipReason`.
- Add `TerminalRefinementError` with `thiserror`, named fields, and doc comments.
  Include non-empty validation and algorithm/source wrapping errors. Do not add
  a strict "not contained" hard error for D8 output.
- Update `TerminalRefinement` and `RefinementOutcome` to carry provenance, while
  keeping dissolve semantics unchanged.
- Update `crates/core/README.md` Mermaid diagram from `refine terminal
  placeholder` to a terminal-refinement strategy seam.
- State in docs that M4's pantry is D8-only and full custom aux binding is
  deferred.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test staged_delineation
```

Tests added/greened:

- Extend `staged_delineation` assertions for disabled and visible best-effort
  skipped provenance.
- Keep `applied_refinement_decodes_terminal_geometry_once` meaningful.

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, README, `Cargo.toml`, and `Cargo.lock`.
- Commit `feat(core): define d8 refinement strategy contract`.
- Tag `v$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')`.

### 2. Add the Blessed-D8 Accessor Alongside Legacy Raster APIs

Goal: add typed D8 selection/localization without breaking the current
placeholder, which still calls `raster_paths()` and `localize_raster_window()`.

Files touched:

- `crates/core/src/session.rs`
- `crates/core/src/cog.rs`
- `crates/core/src/refinement.rs`
- `crates/core/src/error.rs`
- `crates/core/tests/d8_aux_accessor.rs` (new)

Type/code shape:

- Add new APIs; do not remove legacy APIs in this step:
  - `has_d8_aux() -> bool`
  - `select_d8_raster_for_bbox(bbox: Rect<f64>) -> Result<D8RasterHandle, SessionError>`
  - `localize_d8_raster_window(handle: &D8RasterHandle, kind: RasterKind, bbox: Rect<f64>) -> Result<LocalizedRasterWindow, SessionError>`
- Keep existing `raster_paths()` and `localize_raster_window()` untouched enough
  for `Engine::refine_terminal_placeholder` to compile and run until Step 3.
- `D8RasterHandle` contains declaration index, resolved flow-dir URI/path,
  resolved flow-acc URI/path, selected object-store paths for remote sessions,
  and `FlowDirEncoding`.
- Local resolution uses `root.join(&decl.flow_dir)` and `root.join(&decl.flow_acc)`
  after existing path-escape validation.
- Remote localization must use the selected declaration path, not
  `RasterKind::artifact()`. Reconcile both remote URI styles:
  - URI strings for the `RasterSource` should be full artifact URLs derived from
    the dataset URL plus the selected declaration path.
  - Object-store localization should use `remote_artifact_path(remote_root,
    &decl.flow_dir)` / `remote_artifact_path(remote_root, &decl.flow_acc)`.
- Add an extent-only COG header reader:
  - It reads only the IFD data needed for dimensions, model pixel scale, and
    model tiepoint.
  - Do not reuse the 16 MiB window-localization `HEADER_RANGE_BYTES` as the
    planned extent range. Start with a small constant such as 64 KiB or 256 KiB,
    then fail with a typed "extent header too large" error rather than silently
    downloading 16 MiB per declaration.
  - Worst case for 60 declarations should be stated in code/test docs as
    roughly 60 `head` requests plus 60 small range reads, bounded by the new
    extent range constant, before selected-window tile reads.
  - The extent reader must tolerate non-tiled GeoTIFFs for header-only tests;
    tile offsets are required later only for actual COG window localization.
- Cache extents by declaration index and raster kind if the implementation can
  do so surgically. If not, document that the first terminal may inspect all
  declarations but only via the small extent range.

Tile-selection semantics:

- Use inclusive closed-rectangle coverage: exact equality and edge-touching
  count as containing/intersecting. This is mandatory because the synthetic
  fixture terminal bbox equals raster extent.
- Iterate declarations in manifest order.
- Read flow-dir extent; skip if it does not intersect the terminal bbox.
- For candidates whose flow-dir intersects, read flow-acc extent.
- Accept exactly one candidate whose flow-dir and flow-acc extents inclusively
  contain the terminal bbox.
- If no candidate contains but multiple intersect, return a typed
  `TerminalSpansD8Tiles` / `NoSingleCoveringD8Tile` hard error. M4 does not
  implement mosaicking.
- If more than one candidate contains the bbox, return `AmbiguousD8Coverage`.
- Do not fall back from selected-declaration failures to another declaration
  unless selection itself failed before a unique candidate was chosen.

Offline multi-declaration fixture:

- In `d8_aux_accessor`, build a temp HFX dataset by copying
  `v021_synthetic_refined`.
- Add a generated non-intersecting first D8 declaration with tiny local
  GeoTIFFs whose extent is far away, e.g. `[100,105] x [100,105]`.
- Generate those two non-selected TIFFs in the test with the `tiff` crate and
  GeoTIFF tags needed by the extent reader. They do not need valid D8 samples or
  tiling because they must never be localized; the extent reader must not
  require tile tags.
- `tiff = "0.9"` is already a core dependency via `cog.rs`. If the integration
  test crate cannot access it directly, add a narrow dev-dependency or expose a
  `#[cfg(test)] pub(crate)` fixture writer from core; do not hand-author binary
  TIFF bytes.
- Keep the second declaration pointing at the committed B `flow_dir.tif` and
  `flow_acc.tif`, then assert selection picks declaration index 1.
- For ambiguity, duplicate the committed B declaration twice in a temp manifest;
  no extra raster bytes are needed.

Typed errors:

- Add `thiserror` variants with doc-commented named fields for missing required
  D8 aux, no covering D8 tile, ambiguous D8 coverage, terminal spans multiple
  D8 tiles, and COG extent header read failure.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_aux_accessor
cargo test -p shed-core --test staged_delineation
```

Tests added/greened:

- Accessor runs from manifest-declared D8 paths in `v021_synthetic_refined`.
- Offline multi-declaration selection skips non-intersecting declaration 0 and
  picks declaration 1.
- Inclusive containment handles exact terminal-bbox == raster-extent equality.
- Ambiguous coverage hard-errors.
- Explicit missing-D8 selection hard-errors.
- Refine-off still dissolves whole terminal through legacy engine behavior.

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, tests, fixtures if any, `Cargo.toml`, and `Cargo.lock`.
- Commit `feat(core): add declared d8 raster accessor`.
- Tag the new version.

### 3. Implement Built-In D8 Strategy and Swap the Engine

Goal: move the existing D8 carve behind the trait without changing results, then
retire the legacy raster path only after the engine no longer depends on it.

Files touched:

- `crates/core/src/refinement.rs`
- `crates/core/src/engine.rs`
- `crates/core/src/session.rs`
- `crates/core/tests/d8_refinement_parity.rs` (new)
- `crates/core/tests/staged_delineation.rs`

Type/code shape:

- Add `D8RasterRefinementStrategy`.
- Its implementation:
  - obtains a unique `D8RasterHandle` from the pantry/session,
  - uses `pantry.raster_source` when present and returns a typed best-effort
    skip decision when the source is absent; the engine owns the later decision
    to accept that skip under `BestEffort` or hard-error under `RequireD8`,
  - localizes the selected declaration's flow-dir and flow-acc windows for the
    terminal bbox,
  - records the same telemetry as the placeholder,
  - calls `refine_terminal_from_source(raster_source, flow_dir_uri, flow_acc_uri,
    terminal_geometry, resolved_outlet, snap_threshold)`,
  - wraps the returned polygon as the D8 carve output without clamp,
    intersection, or cleaning.
- Engine builder wires `D8RasterRefinementStrategy` as the default boxed
  strategy.
- `Engine::refine_terminal_placeholder` may remain as a compatibility-named
  method, but its body now delegates to the strategy.
- Only after this swap, remove or deprecate legacy `raster_paths` availability
  and legacy root-path localization if no other call sites remain.
- Preserve final geometry:
  `dissolve(whole upstream - whole terminal + carved terminal)`.
- Preserve decode-once: use `units.terminal_unit().geometry()` from
  `PreMergeDrainageUnits`; no second catchment geometry query.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test d8_aux_accessor
cargo test -p shed-core --test staged_delineation
```

Tests added/greened:

- `d8_refinement_parity` loads `v021_synthetic_refined`, runs D8 through the
  strategy, and compares final output to M1
  `goldens/v01_synthetic_refined/oracle_b_synthetic_refined.json` via frozen
  `shed-canonical-wkb-v1`.
- Assert `Applied` provenance uses `BuiltInD8` with the selected declaration
  index.
- Keep `applied_refinement_decodes_terminal_geometry_once` green.

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, tests, `Cargo.toml`, and `Cargo.lock`.
- Commit `feat(core): run d8 refinement through strategy seam`.
- Tag the new version.

### 4. Lock R6 Failure Semantics

Goal: make explicit required D8 a hard-error path and convenience delineation a
visible named best-effort path.

Files touched:

- `crates/core/src/staged.rs`
- `crates/core/src/engine.rs`
- `crates/core/src/refinement.rs`
- `crates/core/src/error.rs`
- `crates/core/tests/d8_aux_accessor.rs`
- `crates/core/tests/staged_delineation.rs`

Type/code shape:

- Extend `RefinementMode` with a named explicit variant such as `RequireD8`.
  Keep default as `BestEffort`; keep `Disabled`.
- Explicit `RequireD8`:
  - missing `hfx.aux.d8_raster.v1` hard-errors,
  - no selected covering declaration hard-errors,
  - missing `RasterSource` hard-errors,
  - declared-but-unreadable rasters hard-error,
  - algorithm errors hard-error.
- Convenience `BestEffort`:
  - no D8 aux declared -> visible
    `BestEffortSkipped { strategy: BestEffortD8IfPresent, why: NoD8AuxDeclared }`
    and whole-terminal dissolve,
  - no raster source -> visible best-effort skip only if no source is attached
    and the behavior matches current convenience expectations,
  - declared selected D8 that fails to read/refine -> hard error, never hidden
    whole-terminal downgrade.
- `Disabled` always returns disabled provenance and whole-terminal dissolve.
- Keep `RefinementMode` out of `TerminalRefinementStrategy`; the engine is the
  policy layer that turns strategy decisions into either visible skips or hard
  errors.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_aux_accessor
cargo test -p shed-core --test staged_delineation
```

Tests added/greened:

- `RequireD8` with no D8 aux hard-errors and names `hfx.aux.d8_raster.v1`.
- `BestEffort` with no D8 aux visibly skips and dissolves whole terminal.
- `Disabled` dissolves whole terminal with disabled provenance.
- Declared-but-unreadable selected D8 hard-errors in both `RequireD8` and
  `BestEffort`.

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, tests, `Cargo.toml`, and `Cargo.lock`.
- Commit `fix(core): make d8 refinement skips explicit`.
- Tag the new version.

### 5. Prove Offline Parity and Artifact Durability

Goal: make the offline M4 gate green and keep M1/M3 invariants intact.

Files touched:

- `crates/core/tests/d8_refinement_parity.rs`
- `crates/core/tests/parity_golden_artifacts.rs` only if adding M4 durability
  checks for the v0.2.1 copied B TIFFs
- `crates/core/tests/staged_delineation.rs`
- `crates/core/tests/fixtures/parity/README.md`

Type/code shape:

- Compare `v021_synthetic_refined` final output to M1
  `v01_synthetic_refined` golden through the frozen canonicalizer.
- Do not change the canonicalizer.
- Do not modify `algo/refine.rs` to satisfy tests.
- Assert the final geometry path still excludes the whole terminal when a D8
  carve is applied, then inserts the carved terminal geometry.
- Document that pre-merge unit records remain pristine and may disagree with
  final refined area/geometry by design.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test staged_delineation
```

Tests added/greened:

- Offline synthetic B parity through v0.2.1 declared D8 aux.
- M1 durability stays green.
- M3 staged skeleton stays green.

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, tests/docs, `Cargo.toml`, and `Cargo.lock`.
- Commit `test(core): prove v021 d8 refinement parity`.
- Tag the new version.

### 6. Add Network-Gated merit/0.2.0 D8 Readiness

Goal: prove the 60-declaration real-data path without making offline tests
networked.

Files touched:

- `crates/core/tests/d8_refinement_parity.rs`
- `crates/core/tests/fixtures/parity/README.md`
- Optional new `crates/core/tests/fixtures/parity/goldens/v021_merit_refined/README.md`
  if a fresh v0.2.1 readiness/golden artifact is captured

Type/code shape:

- Add an ignored test guarded by:

```bash
SHED_HFX_V02_REAL_D8_REFINEMENT=1 cargo test -p shed-core --test d8_refinement_parity -- --ignored --nocapture
```

- The test early-returns unless the env var is `1`.
- It opens `https://basin-delineations-public.upstream.tech/merit/0.2.0/` or the
  documented M4 real-data source.
- It runs `rhine_basel` at `GeoCoord::new(7.5890, 47.5596)` with `5000 m`
  search radius unless the v0.2.1 dataset docs require an updated outlet.
- It proves:
  - manifest declares the expected 60 D8 entries,
  - header selection picks one declaration whose extents inclusively contain the
    terminal bbox,
  - if `rhine_basel` spans declarations, the test fails with the typed
    multi-tile error and records an escalation,
  - selected declaration paths, not root `flow_dir.tif` / `flow_acc.tif`, drive
    localization,
  - only extent-header ranges are read before selection and only selected
    terminal-bbox COG tile ranges are read after selection,
  - an "extent header too large" error is treated as a real failure/escalation,
    not as a silently skipped declaration,
  - refinement result is applied with D8 provenance,
  - the D8 carve is non-empty; any containment check is diagnostic/tolerant, not
    a strict hard gate against the unchanged carve.
- Compare against M1 Oracle C only as a bonus check. If it diverges, do not
  relax Oracle C; document readiness-only status.

Verification:

Offline:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_refinement_parity
```

Network-gated:

```bash
SHED_HFX_V02_REAL_D8_REFINEMENT=1 cargo test -p shed-core --test d8_refinement_parity -- --ignored --nocapture
```

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage code, tests/docs, `Cargo.toml`, and `Cargo.lock`.
- Commit `test(core): add merit v02 d8 refinement readiness proof`.
- Tag the new version.

### 7. Final Documentation and Gate Cleanup

Goal: leave M4 coherent for M5 and future aux-binding work.

Files touched:

- `crates/core/README.md`
- `crates/core/tests/fixtures/parity/README.md`
- `docs/hfx-v02-redesign/milestone-plan.md` only if status notes are maintained
  there

Type/code shape:

- Document that M4 ships exactly one blessed strategy: built-in D8 raster
  refinement.
- Document that the current pantry is D8-only; full aux binding,
  reverse-DNS aux parsing, Python-authored strategies, and additional blessed
  strategies are deferred.
- Document the real carve sequence exactly:
  rasterize -> mask flow-dir and accumulation -> snap -> masked trace ->
  polygonize.
- Document always-merge-after and the intentional disagreement between pristine
  pre-merge unit records and final refined geometry/area.
- Document inclusive D8 tile coverage semantics and the M4 single-declaration
  assumption/risk.

Verification:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test d8_aux_accessor
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test staged_delineation
```

Commit:

- Run `./scripts/bump-version.sh patch`.
- Stage docs, code if any, `Cargo.toml`, and `Cargo.lock`.
- Commit `docs(core): document d8 refinement strategy seam`.
- Tag the new version.

## Gate

Offline green gate:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test d8_aux_accessor
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test staged_delineation
```

Network-gated real-data proof:

```bash
SHED_HFX_V02_REAL_D8_REFINEMENT=1 cargo test -p shed-core --test d8_refinement_parity -- --ignored --nocapture
```

If the network test is not run, M4 may pass the offline gate, but release notes
must say real-data `merit/0.2.0` readiness was not verified in that environment.

## Open Questions / Escalations

- Does `merit/0.2.0` `rhine_basel` fit wholly within one D8 declaration? The
  ignored test must prove this. If it spans declarations, stop and escalate;
  multi-declaration mosaicking is outside M4 unless the milestone owner expands
  scope.
- Does `merit/0.2.0` reproduce M1 Oracle C within frozen canonicalizer
  tolerance? Unknown offline and not required. The ignored test documents the
  result without relaxing Oracle C.
- If multiple D8 declarations contain the same real terminal bbox, escalate
  instead of inventing a Pfaf/level-selection policy. Level-selection strategy
  is non-scope.
- If a strict vector containment check would reject unchanged D8 carve output,
  remove or downgrade that check. Do not change the carve and do not add a
  clamp/intersection step.

## Audit Journaling

After each meaningful step, log:

```bash
clog log --title "M4 step N: short result" \
  --body "Decisions made, tests run, parity/accessor result, any risk or escalation" \
  --tags "shed,hfx-v02,m4,d8-refinement"
```

For real-data divergence or tile-selection surprises, also record a problem:

```bash
clog problem --title "M4 real-data D8 refinement issue" \
  --solution "What was proven or why escalation is needed" \
  --severity medium
```
