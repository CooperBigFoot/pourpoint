# M3 Step Plan — Adversarial Critique

Target: `docs/hfx-v02-redesign/m3-step-plan.md`
Contract: `docs/hfx-v02-redesign/milestone-plan.md` §M3
Source read: `engine.rs`, `resolver.rs`, `session.rs`, `assembly.rs`,
`algo/dissolve.rs`, `reader/catchment_store.rs`, `reader/snap_store.rs`.

Bottom line: the plan's level-before-resolve correction and PiP filter are
**sound and well-evidenced**. But the plan contains one **internal contradiction
that, as written, regresses behavior** (pre-merge geometry vs. re-querying
dissolve → double-decode, breaks an existing invariant test), one **architecture
gap that makes the snap-level constraint unimplementable as scoped** (the session
opens only the first snap declaration), one **backstop that can pass while
behavior drifts** (golden frozen after the refactor it is meant to police), and a
**network-tier proof that silently requires the full GRIT open M2 deferred**.
These block sign-off.

---

## Front 1 — Intermediate type design (keystone)

**VERDICT: weak (mostly holds; one real invalid state left representable).**

What holds, with evidence:

- The newtypes are **not** cosmetic. `SelectedLevel` (private `Level`,
  constructible only from `DatasetSession` via `LevelSelection`) genuinely makes
  "resolve at a level the dataset lacks" unrepresentable. The free-function stage
  signatures (plan L144-185) encode ordering: `traverse_upstream_at_level` needs
  a `LevelResolvedOutlet`, which needs a `SelectedLevel`, which needs a real
  session level. Traverse-before-select-level is a compile error. Good.
- Enums-over-bools and parse-don't-validate are respected at the boundary:
  `LevelSelection` not raw `Level`; `RefinementMode` replaces
  `DelineationOptions.refine: bool` (`engine.rs:190`, `engine.rs:231`);
  `with_refine(bool)` kept only as a boundary parser. No raw primitive leaks past
  the stage edge — `outlet` is already `GeoCoord`.
- The placeholder `TerminalRefinement::Applied { refined_outlet, geometry }`
  **does** survive contact with M4. The roadmap R2 output is "geometry contained
  in the terminal" + a refined outlet, which is exactly this shape. M4 can evolve
  the *internal* `TerminalRefinement` while `RefinementOutcome` (the public result
  at `engine.rs:30-42`) stays frozen, because `compose_result` maps one to the
  other. Note `RefinementOutcome::Applied` carries only `refined_outlet` today;
  the plan correctly routes `geometry` to dissolve, not into the public outcome.

What is weak:

- **Cross-run argument mixing is representable.** `compose_result(resolved,
  upstream, refinement, dissolved)` (plan L178-184) and
  `refine_terminal_placeholder(resolved, units, options)` (L164-169) take several
  independently-constructed intermediates with **no type link** proving they came
  from the same delineation run. Nothing stops a caller passing a
  `LevelResolvedOutlet` from run A with a `SameLevelUpstreamUnits` from run B. The
  plan's keystone is "invalid states unrepresentable," and this is an invalid
  state left representable. The plan itself raises `DelineationRun` typestate and
  then defers it (L191-197). That deferral is defensible for a skeleton, but the
  plan should **state the residual invariant explicitly** ("same-run consistency
  of stage inputs is a caller contract, not type-enforced in M3") rather than
  imply the types make everything fall out for free.
- `NoRasterSourceProvided` (plan L138, mirrors `engine.rs:38`) is tied to the
  deprecated `RasterSource` engine seam. M4 re-keys availability to D8-aux
  presence (milestone-plan §M4), so this variant is likely to be re-typed. Minor;
  acceptable as a placeholder, but call it out as M4-volatile so nobody treats it
  as stable surface.

**Fix:** add one sentence to Step 1 acknowledging the un-type-enforced same-run
invariant and naming `DelineationRun` as the M4/M5 home for it; mark
`NoRasterSourceProvided` as M4-volatile.

---

## Front 2 — Resolve-vs-level circularity

**VERDICT: holds for the offline path; the GRIT network proof is broken (see
Front 8).**

- **Order is reconciled, not silently wrong.** The plan explicitly supersedes the
  roadmap's Resolve→Level diagram with Level→Resolve (plan L13-16) and matches the
  contract (milestone-plan L234-236). Good — it neither follows the wrong diagram
  nor hides the change.
- **PiP truly precedes and constrains.** Step 3 filters candidates to
  `SelectedLevel` *before* `contains/intersects` and *before* the
  upstream-area/area/id tie-break at `resolver.rs:609-628`. This is the correct
  interception point: in perfect nesting an outlet inside an L1 child is also
  inside the L0 parent (the parent geometry is the union of children), so the
  bbox query at `resolver.rs:521` returns both, both pass `contains`, and the
  tie-break at `resolver.rs:613-619` picks the L0 parent by larger
  `upstream_area`. Filtering to finest drops L0 first. The fix is real and
  targets the right line. Note the resolver already has `session` in scope and
  `CatchmentUnit` exposes `level()` (used at `session.rs:760`), so the PiP filter
  needs **no** `level_of` map lookup — it can compare `unit.level()` directly.
- **Default level is session-global.** `max_level()` from the stored map, no
  outlet input (Step 2). Correct per decision #2.
- **The level accessor reuses the discarded map — and that map already exists on
  both open paths.** `validate_graph_catchments` returns
  `HashMap<UnitId, Level>` at `session.rs:743/843`; it is built and dropped at
  `session.rs:246-249` (local) and `session.rs:457-490` (remote). Crucially, even
  the remote *validation-cache-hit* branch still materializes it
  (`session.rs:451-456`), so storing the field works uniformly. Step 2's "session
  already computes it" claim is accurate. Good.

**Concern (not blocking offline):** retaining the full map is fine for fixtures
and for the current (un-scaled) open, but it is O(units) resident memory (~350 MB
for GRIT's 22.3M units). That is acceptable *because M2 deferred full-scale open*
— but it means `max_level()` via the stored map is unusable at planetary scale,
which collides with the GRIT network proof. See Front 8.

---

## Front 3 — Snap cascade & multi-declaration rule

**VERDICT: broken as scoped — the session cannot select the level-matching snap
declaration.**

The ranking cascade itself is fine:

- The plan's `weight DESC → stem_role==Mainstem → snap_id ASC` (Step 4, plan
  L354) is the existing `WeightFirst` cascade at `resolver.rs:458-475` **minus the
  distance sub-tie-break** at `resolver.rs:473`. The plan correctly identifies
  and owns this drop (L361-366) and adds the right escalation clause.
- **The distance drop is safe on the existing fixture *by coincidence*, and the
  plan should say which test.** `weight_first_preserves_mainstem_then_distance_tie_breakers`
  (`resolver.rs:1066-1151`) is *exactly* the "equal weight, equal mainstem,
  different distance" case the plan says to stop-and-escalate on: snap 33 at
  `(0.201,0.2)` vs 44 at `(0.205,0.2)`, both weight 40, both mainstem. Today
  distance picks 33. Under `weight→mainstem→snap_id`, snap_id ASC also picks 33
  (33 < 44). It stays green **only because lower id happens to coincide with
  nearer here.** The plan must name this test so the executor confirms the
  coincidence rather than panicking or re-blessing. NICE-TO-HAVE but cheap
  insurance.

The structural break:

- **The session opens exactly one snap store — `snaps.first()` — regardless of
  level.** `session.rs:251-255` (local) and the remote equivalent build
  `snap: Option<SnapStore>` from `aux_declarations.snaps.first()`. `session.snap()`
  (`session.rs:566`) returns that single store. GRIT 2.0.0 has **two**
  `hfx.aux.snap.v1` declarations (plan L50). Step 4 says "candidate declarations
  are entries whose `references_levels` contains `SelectedLevel` … use the first
  by `metadata.name`/path." But if the finest level (L1) is referenced only by
  the *second* declaration, the loaded store is the wrong one and the entire
  level constraint silently resolves against L0 snap data. The plan's "expose snap
  declarations enough for resolver selection" (L327) and "add narrow query helpers"
  (L331) **do not address that the matching declaration's parquet is never
  opened.** This is the real work and it is unscoped.
- **Sub-question the plan must answer:** are GRIT's two declarations *per-level*
  (one references {0}, one references {1}) or *both multi-level*? If per-level,
  then for finest=L1 there is exactly **one** matching declaration and the
  "deterministic narrow multiple-match rule" (L339-342) **never fires on GRIT** —
  making the headline rule dead weight on the only real dataset, while the actual
  requirement (open the L1 declaration's store, not `snaps.first()`) goes
  unwritten. Check `../hfx/spec/HFX_SPEC.md` `references_levels` semantics and the
  real manifest before committing to the multiple-match framing.

**Fix:** Step 4 must specify how the level-matching declaration's `SnapStore` is
constructed. Options: (a) at open time, build a `Vec<(SnapDecl, SnapStore)>` keyed
by `references_levels`; or (b) resolve-time lazy open of the selected
declaration's artifact. Either is fine; "narrow query helpers on the existing
single store" is not, because the existing store is the wrong declaration. Also
re-justify or drop the multiple-match rule against the real per-level layout.

The snap-target level filter (`level_of(target.unit_id()) == SelectedLevel`,
L351-353) is correct and complements the declaration filter — keep it; note it
*does* need the `level_of` map because `SnapTarget` carries no level
(`snap_store.rs` returns `SnapTarget`, level derived from the referenced unit per
`session.rs:846-872`).

---

## Front 4 — `delineate()` == staged composition

**VERDICT: broken — Step 6 as written double-fetches/double-decodes geometry and
breaks an existing invariant test.**

The composition-by-construction intent is right (delineate literally calls the
stage methods, plan L187-189), and option threading is consistent *if* delineate
passes one `options` object into each stage. But Step 6's mechanism contradicts
Step 5's output:

- **Step 5** makes `produce_pre_merge_units` return `PreMergeDrainageUnit { …,
  geometry: MultiPolygon }` for **every** upstream unit including the whole
  terminal (plan L411-435). Producing those records decodes every upstream
  geometry via `query_geometries_by_ids` (`catchment_store.rs:608`), the path that
  increments the decode counter (`catchment_store.rs:1114`).
- **Step 6** then says `dissolve_watershed` should "keep calling
  `assemble_watershed` or add a thin wrapper that delegates to the same assembly
  code path" (plan L463-466). But `assemble_watershed` (`assembly.rs:152-213`)
  **re-queries the catchment store by id and re-decodes every geometry** — it
  ignores the geometries the pre-merge stage already decoded. And
  `refine_terminal_placeholder`, if it ports `try_refine` verbatim
  (`engine.rs:506-635`), **re-fetches and re-decodes the terminal a third time**
  (`engine.rs:526-543`).
- **Concrete regression:** `applied_refinement_decodes_terminal_geometry_once`
  (`engine.rs:855-900`) asserts the terminal geometry is decoded **exactly once**.
  Today that holds because refine decodes the terminal once and refine-on assembly
  *excludes* the terminal (`assembly.rs:159-167`). Under the staged plan as
  written: pre-merge decode (#1) + refine re-fetch (#2) + dissolve re-fetch of
  whole units (#3 for non-terminal, and terminal too on refine-off). The test
  **fails**, and the broader effect is 2× decode of *every* unit on large
  watersheds — a silent perf regression. The plan's Step 6 signature even says
  `dissolve_watershed` "consumes pristine `PreMergeDrainageUnits`" (L485) while
  the implementation note says call `assemble_watershed` (which consumes ids, not
  geometries). That is an internal contradiction.

**Fix (it is already enabled by the existing code):**

- `dissolve_watershed` must consume the pre-merge **geometries**, not re-query.
  `assemble_from_geometries(geometries, options)` (`assembly.rs:242-269`) is the
  existing geometry-only dissolve path that still calls `dissolve(polygons)` →
  `dissolve_spatial_reduce_strategy`. Build the polygon list from
  `PreMergeDrainageUnits`, swapping the whole terminal for the refined override
  when `TerminalRefinement::Applied`, then call `assemble_from_geometries`. This
  preserves the deterministic path *and* avoids re-decode.
- `refine_terminal_placeholder` already receives `units: &PreMergeDrainageUnits`
  (plan L164-169). It must pull the terminal polygon from the pre-merge terminal
  record, **not** re-query the store — preserving decode-once.
- Caveat the plan must note: moving production dissolve to
  `assemble_from_geometries` means the terminal-swap guards in
  `assemble_watershed` (`EmptyRefinedTerminalGeometry`, `MissingCatchments`,
  `DuplicateCatchment`) no longer cover the production path, and the assembly
  tests `terminal_override_replaces_coarse_terminal_geometry` /
  `terminal_override_bypasses_bad_terminal_wkb` (`assembly.rs:370-440`) would then
  exercise **dead code**. Re-home equivalent coverage at the `dissolve_watershed`
  level, or keep `assemble_watershed` as the single dissolve entry and instead fix
  decode-once by *not* decoding geometry in pre-merge — but that contradicts
  decision #5/R3 (pre-merge must carry polygons). The geometry-only path is the
  correct resolution.

---

## Front 5 — Parity-backstop gap

**VERDICT: weak — the chosen backstop can pass while real behavior drifts (false
green), because of ordering and refine-on blindness.**

- **Self-confirming capture risk.** Step 8 is sequenced **last**, after Step 6
  moves the dissolve/refine logic and Step 7 rewrites `delineate`. If the
  refine-off golden is generated by running the *already-refactored* M3 engine,
  it freezes whatever Step 6 produced as "correct" and can never detect a Step 6
  drift. A backstop that polices a refactor must be captured **before** the
  refactor. The plan does not state the provenance of the golden bytes.
  **Fix:** capture the refine-off golden from the **pre-M3 (M2) engine** output at
  the *start* of M3 (or reuse an M1 refine-off golden if one exists — the refined
  M1 goldens were captured at `crates/core/tests/fixtures/parity/`; check whether
  a `refine=false` variant already exists, since refine-off geometry is pure
  whole-unit dissolve and is loader/raster-independent). Commit the bytes early;
  add the *assertion* in Step 8, but the bytes must predate Step 6.
- **Right artifact / right normalizer: yes.** Capturing from
  `v021_synthetic_refined` (the M2-converted fixture, plan L26-28) compared with
  frozen `shed-canonical-wkb-v1` (plan L22-24) is correct and network-free.
- **Refine-on is unbackstopped offline, and the plan under-warns.** Step 6
  refactors *both* paths, but the only offline geometry guard is refine-off +
  determinism. A refine-on drift (e.g. wrong terminal excluded during the
  override swap) passes every offline gate and is not caught until M4's
  `d8_refinement_parity`. The plan acknowledges refined parity defers to M4 but
  does not flag that **Step 6's Applied-path rewrite is therefore unguarded** and
  must be treated as high-risk. Combined with the dead-code concern in Front 4,
  the Applied override mechanism could silently break. **Fix:** add an explicit
  Step 6 requirement to add a focused offline test that the Applied path
  substitutes the refined terminal and drops the whole terminal (a structural
  check on the dissolved geometry vs. the pre-merge terminal), even though full
  refined parity waits for M4.
- The stop-and-escalate clause (L598-601) is good and correctly forbids
  re-blessing the golden.

---

## Front 6 — Dissolve determinism preservation

**VERDICT: holds.**

- Step 6 forbids touching `spatial_key`/`rayon::join` (plan L467-468), which is
  the M1 nondeterminism fix at `dissolve.rs:56-86` (sort by `spatial_key`, fixed
  `rayon::join` at `len/2`). Routing the new stage through
  `assemble_from_geometries` → `dissolve()` (per the Front 4 fix) reuses this path
  verbatim.
- **Input order is irrelevant by construction:** `dissolve_spatial_reduce_strategy`
  re-sorts by `spatial_key` (`dissolve.rs:57`), so the "terminal-first" pre-merge
  ordering cannot perturb dissolve output. Good — the plan should note this, since
  it removes any worry that pre-merge ordering leaks into geometry.
- The plan's determinism assertion (run twice, compare canonical WKB, L497-500) is
  fine but **weaker** than the existing `overlapping_dissolve_is_byte_identical_across_parallel_runs`
  (`dissolve.rs:325-349`), which permutes 20× under a forced 4-thread pool.
  NICE-TO-HAVE: have the staged determinism test reuse the permute-under-pool
  pattern so it actually stresses the parallel reduce.

---

## Front 7 — Deferred-mechanism leakage

**VERDICT: holds — no M4/M5 scope smuggled in.**

Scanned every step:

- No D8 trait: `TerminalRefinement` is explicitly an enum result shape, not a
  trait (plan L137-138, L474-483). Good.
- No aux→strategy binding: Step 4 bounds itself to "without adding a general
  aux-to-strategy binding mechanism" (L327) and the narrow declaration rule is
  flagged as not a strategy (L386-390). The only risk is that implementing
  declaration selection *grows* into binding — but as scoped (pick the
  level-matching declaration deterministically) it is not binding. Watch this in
  execution per the Front 3 fix.
- No user-authored resolve/traverse/dissolve seam; no mixed/cross-level traversal
  (same-level traversal falls out of M2's same-level-edge validation at
  `session.rs:812-822`); no level-selection strategy beyond `LevelSelection::Finest`;
  no pyshed redesign (kept as `cargo check` hold); no CLI/Python JSON-key rename.
  The Explicit Out Of Scope section (L669-682) is accurate.

No leak to flag.

---

## Front 8 — Gates that silently need network

**VERDICT: weak — offline gate is clean, but the GRIT network proof silently
requires the full session open M2 deferred.**

- **Offline gate stays green with no network / no env vars.** The four test files
  use local fixtures; `parity_golden_artifacts` is M1 loader-independent. No
  offline test reaches R2. The ignored GRIT tier is gated by
  `SHED_HFX_V02_REAL_R2_DELINEATION=1` + `#[ignore]` (plan L66-76), matching the
  M1/M2 pattern. Good — confirm the tests also early-return when the env var is
  unset (so `--ignored` without the env is a no-op), as M2 did.
- **The GRIT proof forces the deferred full open.** The ignored proof must show
  "default level is the finest present" and "a real outlet resolves to the finest
  containing unit" (plan L74-76). `max_level()` is sourced from the stored
  `HashMap<UnitId, Level>` (Step 2), which is populated only by a full
  `DatasetSession::open` — and that open reads **all** catchment ids+levels even
  on the validation-cache-hit branch (`session.rs:451-456`). M2 **explicitly
  deferred** full validated open over all 22,337,300 units: "Current debug full
  opens take 30+ minutes and are memory-heavy" (milestone-plan L172-176). So the
  M3 GRIT delineation proof, as specified, is the very full open M2 said is out of
  scope — a 30-min, ~350 MB-resident test. M2's own GRIT readiness proof avoided
  `DatasetSession::open` entirely and used **bounded graph row-group `level`
  statistics** (milestone-plan L159-163) to prove L0+L1 presence. The M3 plan
  reverts to full open without acknowledging the cost or the M2 precedent.
  **This is a scope/feasibility question the planner must resolve**, and it may be
  a human decision (see Decisions): either (a) accept and document a long-running
  manual-tier proof, or (b) prove "finest level present" via bounded row-group
  level statistics (M2's technique) and prove resolution on a **bbox-bounded**
  open that does not materialize all 22M levels — which would require `max_level`
  to have a bounded implementation distinct from the full stored map.

---

## Front 9 — Execution hygiene

**VERDICT: holds.**

- Version-bump doctrine is correct and explicitly bakes in the M2 lesson: bump
  patch, stage **`Cargo.toml` AND `Cargo.lock`** even on empty diff, conventional
  message, tag, no tooling-created commits (plan L78-93). Good.
- pyshed kept as `cargo check -p pyshed` build-hold (plan L20-21, L521-522).
  `delineate(outlet, options)` keeps its signature, and `RefinementOutcome` is
  unchanged, so the binding break surface is limited to `with_refine`/options —
  the plan keeps `with_refine(bool)` as a shim, so pyshed should not break.
  Verify pyshed does not construct `DelineationOptions` via a field literal.
- thiserror/tracing/no-unwrap/Mermaid discipline is restated (L88-93). Step 2's
  new `EngineError` "empty level index" variant is required to be doc-commented
  with when-it-fires (L240-244) — good.
- Steps are independently committable with a verification block each. Good.

Minor: Step 6 must preserve `build_assembly_options` (`engine.rs:639-645`), which
folds the engine-level `geometry_repair` backend into options — `dissolve_watershed`
delegating to `assemble_from_geometries` must still pass repair/hole-fill/epsilon
through, or refine-off geometry will drift on repair-enabled engines. Call this
out so it isn't dropped during the move.

---

## (a) MUST-FIX before sign-off

1. **Resolve the pre-merge-geometry vs. re-query contradiction (Front 4).** Step 6
   must mandate `dissolve_watershed` consume pre-merge geometries via
   `assemble_from_geometries` (`assembly.rs:242`), and
   `refine_terminal_placeholder` must reuse the pre-merge terminal polygon — not
   re-`query_geometries_by_ids`. As written the plan breaks
   `applied_refinement_decodes_terminal_geometry_once` (`engine.rs:855`) and 2×-decodes
   every unit. Re-home the terminal-override guards/tests off the now-unused
   `assemble_watershed` path.
2. **Specify how the level-matching snap declaration is opened (Front 3).** The
   session opens only `snaps.first()` (`session.rs:251-255`); the level constraint
   is unimplementable without changing snap-store construction. Also verify against
   the real GRIT manifest whether the two declarations are per-level (in which case
   the multiple-match rule never fires and the real requirement is "open the L1
   declaration") and adjust the framing.
3. **Fix the backstop ordering (Front 5).** Freeze the refine-off golden from the
   **pre-M3 (M2) engine** (or an existing M1 refine-off golden) *before* Step 6,
   not from post-refactor output. State the golden's provenance. Add a focused
   offline structural check for the refine-**on** Applied override path, which is
   otherwise unguarded until M4.
4. **Reconcile the GRIT network proof with M2's deferred full open (Front 8).**
   Decide whether the ignored proof does a full (30-min, ~350 MB) open or uses
   bounded row-group level statistics + bbox-bounded resolution. If bounded,
   `max_level` needs a bounded path distinct from the full stored map.

## (b) NICE-TO-HAVE

- Name `resolver.rs:1066` in Step 4 and document that it stays green only because
  lower `snap_id` coincides with nearer distance there.
- Note in Step 5/6 that `dissolve_spatial_reduce_strategy` re-sorts inputs, so the
  terminal-first pre-merge order cannot affect dissolve output.
- Have the staged determinism test reuse the permute-under-4-thread-pool pattern
  from `dissolve.rs:325-349` instead of a plain run-twice.
- State the residual same-run consistency invariant on the stage intermediates
  (Front 1) and mark `NoRasterSourceProvided` as M4-volatile.
- Confirm `build_assembly_options` (`engine.rs:639`) is preserved through
  `dissolve_watershed` (geometry repair / hole-fill / epsilon).

## RE-REVIEW (plan revised after first critique)

The revised `m3-step-plan.md` resolves all four MUST-FIX blockers at the
mechanism level, not by hand-waving:

1. **Pre-merge vs. re-query (Front 4) — RESOLVED.** Step 7 (L533-573) now mandates
   `dissolve_watershed` build the geometry list from `PreMergeDrainageUnits` and
   call `assemble_from_geometries`, explicitly forbids calling `assemble_watershed`
   after pre-merge decode, requires `refine_terminal_placeholder` to reuse the
   pre-merge terminal polygon instead of `query_geometries_by_ids(&[terminal])`,
   preserves `build_assembly_options`, and re-homes the terminal-override guard
   coverage off the now-bypassed `assemble_watershed` path.
2. **Snap declaration opening (Front 3) — RESOLVED.** Step 4 (L336-364) now
   requires the session to open the level-matching declaration's `SnapStore`
   (`Vec<(SnapDecl, SnapStore)>` or lazy open), states `snaps.first()` is
   insufficient, and adds the GRIT per-level manifest check.
3. **Backstop ordering (Front 5) — RESOLVED.** Split into Step 6 (capture from the
   pre-M3 engine before any geometry movement, provenance recorded) and Step 9
   (assert, no regeneration). Step 7 (L588-591) adds the Applied-path structural
   check.
4. **GRIT proof scope (Front 8) — RESOLVED (correctly escalated).** Now an explicit
   human decision (L779-783) with the gate gated behind it (L77-79).

All five NICE-TO-HAVE items were folded in (named the `resolver.rs:1066` test,
the dissolve re-sort note, the permute-under-pool determinism pattern, the
residual same-run invariant, the `NoRasterSourceProvided` M4-volatility, and
`build_assembly_options` preservation). Both human decisions are escalated, not
pre-decided.

**Two residual executor-level notes (not blockers; the plan's direction is
correct):**

- **Decode-once counter mechanism.** Step 7 instructs the executor to "preserve
  the decode-once invariant tested by
  `applied_refinement_decodes_terminal_geometry_once`," but the counter is
  recorded **only** inside the `query_geometries_by_ids` path
  (`catchment_store.rs:1112-1115`). The resolver-style manual decode
  (`CatchmentUnit::geometry()` → `decode_wkb_multi_polygon`, as at
  `resolver.rs:537`) is **uncounted**. If `produce_pre_merge_units` (Step 5,
  L430-432, "narrow full-record helper… if `query_by_ids` cannot return geometry")
  obtains geometry via `query_by_ids` + manual decode, the terminal decode count
  becomes 0, and the `== 1` assertion fails (or gets silently weakened). **Pin
  the pre-merge geometry decode to the instrumented path** (route through
  `query_geometries_by_ids`, or make the new helper record the decode) so the
  invariant stays count-meaningful. One sentence in Step 5/7 fixes this.
- **Step 6 capture point.** Step 6 captures "from the current M2 engine," but it
  is sequenced after Steps 3-4 have already swapped `delineate`'s resolve call to
  the level-aware path. Those steps are no-ops on the single-level parity fixture
  (each step re-runs `parity_golden_artifacts`), so the bytes equal M2 — but the
  capture would be strictly safest taken before Step 1, or the plan should assert
  the Step 6 bytes are byte-equal to a pre-Step-1 run. Low risk because the
  geometry-producing path is untouched until Step 7.

**Verdict: sign-off-ready.** The two notes above are clarifications the executor
should apply in-flight; neither changes the plan's structure or blocks starting.

## (c) DECISIONS for the human (escalate, don't decide)

- **GRIT network-proof scope (Front 8).** Is a 30-minute, ~350 MB full
  `DatasetSession::open` acceptable for an opt-in, env-gated, `#[ignore]` proof,
  or must the M3 real-data proof stay bounded like M2's readiness proof? This
  changes whether `max_level`/`level_of` need a bounded (row-group-statistics)
  implementation now versus deferring planetary-scale level indexing to the same
  future performance milestone M2 named.
- **Distance sub-tie-break removal from `WeightFirst` (the plan already flags this
  at L688-691).** Dropping distance from the production cascade is a real behavior
  change on any dataset with equal-weight, equal-mainstem candidates at different
  distances. The plan chooses to drop it; confirm the owner accepts this as the
  M3-wide resolver contract before Step 4 starts.
