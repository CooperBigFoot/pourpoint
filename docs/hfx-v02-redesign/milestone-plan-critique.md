# Adversarial Critique — shed HFX v0.2 Redesign Milestone Plan

Target: `docs/hfx-v02-redesign/milestone-plan.md`
Reviewer stance: adversarial. Verified against the actual code, not the plan's
self-description.

## Ground truth established before judging (read-only verification)

- `crates/core/Cargo.toml` pins `hfx-core` via `hfx-core.workspace = true`, and
  the workspace root `Cargo.toml:5` pins `hfx-core = "=0.2.0"` **from crates.io**.
  That release still models the v0.1 contract: shed's `reader/manifest.rs`
  parses `format_version "0.1"`, builds `hfx_core::Manifest` via a
  `ManifestBuilder::new(.., terminal_sink_id, .., atom_count, ..)`, and uses
  `AtomCount`. `cargo build -p shed-core` succeeds today against these v0.1 types.
- The **local** `../hfx/crates/hfx-core` has *already been rewritten to v0.2.1*:
  `UnitId`, `UnitCount`, `CatchmentUnit`, `DrainageGraph`/`AdjacencyRow`,
  `Manifest::auxiliary()`, `FormatVersion::V0_2_1`. A grep for `Atom` in that
  crate returns **nothing** — `AtomId`/`AtomCount` do not exist there. Its
  `ManifestBuilder::new` signature dropped `terminal_sink_id` and `atom_count`.
- shed depends on `hfx_core::AtomId` across **16 source files** (engine,
  resolver, assembly, upstream, catchment_store, graph, snap_store, error,
  telemetry, parquet_cache, source_telemetry, id_index, testutil, refine, +
  perf tests).
- `crates/python/src/result.rs` exposes `terminal_atom_id()` /
  `upstream_atom_ids()` straight from the core result type as the Python API.
- `crates/core/src/resolver.rs` has **no notion of `level`**. Resolution is
  point-in-polygon with area-based tie-breaking ("highest upstream area", then
  "highest local area", then "lowest atom ID"); or snap-file nearest-geometry.
- `crates/core/src/testutil.rs` `DatasetBuilder` emits **v0.1 fixtures only**:
  `format_version "0.1"`, `terminal_sink_id`, `atom_count`, and writes
  `graph.arrow`.
- `crates/core/src/algo/refine.rs` confirms the carve claim: it masks **both**
  flow-dir and accumulation tiles so the trace is strictly contained — a
  sub-polygon of the coarse terminal. "Containment-clamped" is accurate.
- `algo/upstream.rs` already maintains a visited set for tree **and** DAG.
- Crate package name is `shed-core`, so the gate commands (`-p shed-core`) are
  runnable; the named test files don't exist yet (fine — they are to-be-created).

The plan is judged against this reality.

---

## Objections

### 1. [BLOCKER] — M2 silently entails a crates.io→local hfx-core dependency swap that the plan never names

M2's scope is written as in-shed loader work ("Load `catchments.parquet` as
drainage units", "Delete v0.1 root assumptions"). But every domain type M2
needs — `UnitId`, `UnitCount`, `CatchmentUnit`, `DrainageGraph`,
`Manifest::auxiliary()`, `FormatVersion::V0_2_1` — lives **only** in the
unreleased local `../hfx/crates/hfx-core`. shed currently builds against
`hfx-core "=0.2.0"` from crates.io, which has none of them. M2 is therefore
gated on an **external dependency migration** (re-point to a path/git/new
published hfx-core) that is nowhere in the plan, the ordering section, or the
cross-milestone dependency list.

**Fix demanded:** add an explicit M2 precondition (or a dedicated M1.5
milestone): "Re-point `hfx-core` to the v0.2.1 crate, pin the exact
revision/version, and record the upgrade in the dependency graph." Until that
revision is published or wired as a path/git dep, M2 cannot start, and the plan
must say so.

### 2. [BLOCKER] — Terminology migration is forced *atomically at M2*, not deferrable to M3/M5

The plan spreads atom→drainage-unit across M3 ("introduce drainage-unit
vocabulary") and M5 ("remove remaining public atom naming"), and the roadmap
files it as deferred TODO #6. This is impossible. The moment M2 swaps to the
v0.2.1 hfx-core, **`AtomId` and `AtomCount` cease to exist as symbols**. Every
one of the 16 files that names `AtomId` stops compiling *simultaneously*. You
cannot land an M2 that "loads drainage units" while M3/M5 still hold `AtomId` —
the crate will not build between M2 and M5.

The rename is not a cosmetic late-stage cleanup; it is a hard, repo-wide,
single-commit prerequisite of M2's own compile. M5's gate ("no public method
exposes `atom`") describes the *end* of a migration that M2 is forced to perform
in full.

**Fix demanded:** fold the complete `AtomId`→`UnitId` /
`terminal_atom_id`→`terminal_unit_id` / `atom_count`→`unit_count` migration into
M2 (or a migration milestone immediately before M2) as named scope with its own
gate (`cargo build` workspace-wide is green; no `Atom` identifier remains in
core). Delete the pretense that naming "settles last" in M5.

### 3. [BLOCKER] — The M1 parity gate dies at M2; the only *live* parity check is M4, so parity is not actually "captured before restructuring"

M1's gate is `cargo test -p shed-core --test parity_v01_oracle`, and M1 builds
its fixtures from the v0.1 `DatasetBuilder`. M2 deletes the v0.1 loaders and
(per objection 1/2) breaks `DatasetBuilder` and the v0.1 `Manifest` types
outright. So the executable M1 test **cannot survive M2** — it references types
and a builder that M2 removes. After M2, the only parity comparison is M4's, run
against a *converted* fixture. That means the plan's central claim — "freeze
parity before restructuring" — degrades to "freeze some bytes, then re-verify
much later against a converted target," which is exactly the moving-target
failure the role brief warns about.

**Fix demanded:** M1 must produce **committed golden artifacts that are inert
to later refactors**: serialized WKB + `area_km2` + resolved/refined outlet +
terminal id + upstream id set written to disk as fixture files, *plus* a
checked-in copy of the v0.1 input dataset (not a builder call). The parity
*comparison harness* must be designed in M1 to read those committed bytes and be
runnable on the v0.2.1 path, so M4 re-points inputs without re-deriving the
golden. State explicitly that the golden bytes never regenerate from a builder
M2 will mutate.

### 4. [MAJOR] — M3 stage ordering (resolve → select level) is backwards for multi-level data; resolution is level-blind today and will tie-break toward the *coarsest* unit

`resolver.rs` resolves a point by point-in-polygon with area tie-breaking
("highest upstream area / highest local area" wins). HFX v0.2.1 guarantees
*perfect nesting*: a coordinate inside a finest unit is also inside its parent
at every coarser level. So PiP over a multi-level `catchments.parquet` returns a
containing unit at **every** level, and the area tie-break selects the **largest
= coarsest** — the exact opposite of M3's "default finest = max(level)".

M3's fixed stage order puts "resolve outlet" *before* "select level", but
resolution cannot disambiguate nested hits without already knowing the target
level. This is a hidden circular dependency between two stages the plan declares
linear. The roadmap's own open question ("how do multiple snap aux entries
interact with the default finest-level choice during outlet resolution?") is
unresolved and M3 inherits it with no gate.

**Fix demanded:** M3 must define level selection as an input that *constrains*
resolution (filter candidates to the selected level before PiP/tie-break), or
re-order the skeleton to select-level-then-resolve. Add a gate asserting that on
a 2-level nested fixture, an outlet resolves to the **finest** containing unit,
not the coarsest. Without this, M3's "default finest" is unverified and
contradicted by current code.

### 5. [MAJOR] — M4's D8 strategy needs aux→path resolution, which is the deferred aux-binding mechanism leaking in; the plan never says who turns `hfx.aux.d8_raster.v1` into flow_dir/flow_acc paths

Today `engine.rs::try_refine` gates on `session.raster_paths()` (v0.1 root
raster paths) and calls `session.localize_raster_window(RasterKind::FlowDir,..)`.
In v0.2.1 those paths come from `manifest.auxiliary[]`. M4 says "implement the
built-in D8 strategy using `hfx.aux.d8_raster.v1`" while M4 *and* M5 explicitly
defer "full aux-to-strategy binding." But the D8 strategy is physically unable
to run without *some* resolution from the parsed `hfx.aux.d8_raster.v1` decl
(M2's output) to the two raster artifact paths the session localizes. That
minimal resolver is undefined: M2 only "parses entries", M4 "uses" them, and
nobody is assigned the binding glue.

**Fix demanded:** M4 must explicitly scope a *minimal, hardcoded blessed-D8
accessor* (decl → `flow_dir`/`flow_acc` relative paths → session localize),
clearly distinct from the general deferred binding mechanism, and the gate must
exercise it (load a v0.2.1 dataset whose D8 aux declares the rasters, and prove
the carve runs off the manifest-declared paths). Also rewrite the refinement
"no rasters" outcome: today's `RefinementOutcome::NoRastersAvailable` keys off
v0.1 `raster_paths()`; in v0.2.1 it must key off absence of the D8 aux decl.

### 6. [MAJOR] — "Defer pyshed" does not mean pyshed keeps compiling; the workspace will not build through M2–M4

`crates/python/src/result.rs` calls `terminal_atom_id()` / `upstream_atom_ids()`
on the core result and re-exports them as the Python API surface. M2's type swap
(objection 1/2) and any method rename break pyshed's compile immediately. The
plan treats pyshed as out-of-scope ("deferred to a separate campaign"), and M5's
gate `cargo test -p shed-core` conveniently skips it — but a deferred *redesign*
is not the same as a crate that no longer builds. If pyshed is in the workspace,
`cargo build`/`cargo test` workspace-wide is red from M2 onward, and CI breaks.

**Fix demanded:** the plan must state the pyshed holding strategy across M2–M5
explicitly — either (a) update pyshed's thin bindings to the renamed core
methods as mechanical in-scope work in M2 (keeping it compiling without
redesigning it), or (b) formally exclude pyshed from the workspace default
members for the duration and say so. Pick one; "deferred" as written leaves the
build broken.

### 7. [MAJOR] — M4's parity gate can fail for conversion reasons, not behavior reasons, yet M4 is allowed to close on it

M4 compares the *converted v0.2.1 fixture* output against the M1 oracle via
"canonical geometry WKB plus area tolerance." But the carve depends on the exact
raster window localized, which now flows through the new aux path resolution and
the fixture conversion. The plan's own Risk #1 and #2 admit conversion
inequivalence and WKB-byte fragility. As written, a red M4 gate is ambiguous:
behavior regression vs. lossy conversion vs. nondeterministic ring ordering. A
parity gate that cannot distinguish "the engine changed" from "the fixture
converter changed" is not a parity gate.

**Fix demanded:** M4 must (a) pin a *byte-identical raster aux* between the v0.1
oracle capture and the v0.2.1 fixture (same `flow_dir.tif`/`flow_acc.tif`, only
the manifest declaration differs), so the carve input is provably unchanged; and
(b) define the canonical-WKB normalization (ring orientation, start vertex,
coordinate precision) as an M1 deliverable, used identically on both sides.
Tighten Risk #1/#2 into acceptance criteria, not residual risks.

### 8. [MAJOR] — No milestone owns the testutil/fixture-builder rewrite, yet M3 and M4 depend on it

Every staged-delineation test (M3) and D8 parity test (M4) needs a **v0.2.1**
fixture builder. The only builder today (`testutil.rs::DatasetBuilder`) writes
`graph.arrow`, `terminal_sink_id`, `atom_count`, `format_version "0.1"` — all
deleted by M2. M2's scope mentions "convert the M1 fixture bundle" but not
rebuilding the *programmatic* `DatasetBuilder` that ~all existing engine/unit
tests rely on. Without a v0.2.1 `DatasetBuilder`, M3/M4 have nothing to test
against and the existing in-crate tests (engine.rs, catchment_store, etc.) go
dark.

**Fix demanded:** give the v0.2.1 test-fixture infrastructure explicit scope
(in M2 or a dedicated slice): a `DatasetBuilder` that emits `graph.parquet` with
`bbox_*` columns, multi-level `catchments.parquet`, `manifest.auxiliary[]`, and
the new manifest fields. Gate: the existing engine/unit test suite is ported and
green on v0.2.1 fixtures. Quantify how many existing tests depend on the v0.1
builder so the cost is visible.

### 9. [MAJOR] — `graph.parquet` is a brand-new reader; the plan treats `graph.arrow`→`graph.parquet` as a deletion, not a build

M2 lists "Delete v0.1 ... `graph.arrow`" and "Load `graph.parquet` with ...
`upstream_ids` and bbox columns" in one breath. But shed's current `reader/graph.rs`
reads Arrow; reading `graph.parquet` (list<int64> `upstream_ids`, four `bbox_*`
float32 columns, row-group/Hilbert sort assumptions per the spec) is a new
parquet reader with its own schema-validation and referential-integrity surface.
That is substantial net-new work hidden inside a "delete" bullet, with no
dedicated gate for graph schema/columns.

**Fix demanded:** call out the `graph.parquet` reader as explicit M2 scope with
its own gate: positive load, rejection of missing `bbox_*` columns, rejection of
`graph.arrow`, and referential integrity (every catchment has exactly one graph
row; `upstream_ids` reference existing same-level units).

### 10. [MINOR] — Topology (tree vs dag) and `stem_role` have no milestone or gate

The spec admits `topology: "dag"` with distributaries and a `stem_role` enum
(`mainstem|tributary|distributary|unknown`); the roadmap files tree-vs-dag as
deferred TODO #7. `upstream.rs` already dedups for DAG, so traversal is safe —
but M2's loader contract never mentions reading `topology` or `stem_role`, and
no gate asserts DAG behavior end to end. The plan implicitly assumes tree.

**Fix demanded:** either state "tree-only is in scope; DAG datasets are rejected
at load with a clear diagnostic" as explicit M2 scope+gate, or include a minimal
DAG traversal parity test. Don't leave topology unhandled and unstated.

### 11. [MINOR] — `FormatVersion` gate is real but under-specified; v0.1 rejection must be a *typed* unsupported-version error, not a generic parse failure

The new hfx-core has `FormatVersion::V0_2_1` and a `FromStr` that errors on
anything else (`UnsupportedFormatVersion`). M2's gate says "rejection of wrong
`format_version`", which is good, but the spec (HFX_SPEC §Manifest) requires a
v0.2 reader to "reject v0.1 datasets with a clear unsupported-version diagnostic
rather than attempting dual-reader compatibility." A v0.1 manifest also carries
removed fields; the rejection must be attributable to the version, not to a
downstream missing-field error.

**Fix demanded:** M2 gate must assert that loading a v0.1 manifest yields a
*specific* unsupported-version error (naming `0.1` / required `0.2.1`), distinct
from missing-field errors, and that `graph.arrow` presence is rejected at file
presence, not deep in parsing.

### 12. [MINOR] — M5's gate (`cargo test -p shed-core`) cannot verify the deferred-scope claims it asserts

M5 promises "migration docs state the hard cut + deferred pyshed scope" and "no
public engine result method exposes `atom`." `cargo test -p shed-core` proves
neither. The atom-naming claim needs a mechanical check (grep/`cargo doc` lint /
a compile-time test that the public API has no `atom` identifier); the docs
claim is unverifiable by the stated command.

**Fix demanded:** replace the hand-wave with a concrete check — e.g. a test or
CI grep asserting zero `atom` identifiers in the core public surface, and a
docs-presence check. Otherwise M5 closes on assertion, not verification.

---

## Axis-by-axis summary

1. **Parity integrity** — *Fails.* The carve claim itself is verifiable
   (refine.rs is genuinely containment-clamped), but the M1 gate cannot survive
   the M2 cut (obj. 3), the live check is deferred to M4 against a converted,
   possibly-inequivalent fixture (obj. 7), and the golden is built from a
   builder M2 mutates (obj. 8). Parity is asserted across the cut, not preserved.
2. **Ordering & hidden dependencies** — *Fails.* The crates.io→local hfx-core
   swap (obj. 1) and the forced atomic rename (obj. 2) are invisible
   prerequisites of M2. resolve↔select-level is circular (obj. 4).
3. **Verifiability in isolation** — *Partially fails.* M2/M3/M4 gates are real
   commands, but M1's gate is non-durable (obj. 3), M4's is ambiguous (obj. 7),
   and M5's does not test its claims (obj. 12).
4. **Scope creep / deferred-mechanism leakage** — *Fails.* Aux→path binding
   leaks into M4 with no owner (obj. 5); pyshed "deferral" leaves the build red
   (obj. 6).
5. **Missing gates / milestones** — *Fails.* No milestone owns: the dependency
   upgrade (obj. 1), the terminology migration as a unit (obj. 2), the v0.2.1
   `DatasetBuilder` rewrite (obj. 8), the `graph.parquet` reader as net-new
   (obj. 9), topology/`stem_role` handling (obj. 10).
6. **"Types are the interface" bet** — *Undermined.* The plan front-loads
   *shed-authored* intermediate types in M3 but ignores that the foundational
   types are **imported from hfx-core** and already fixed upstream. The real
   keystone (adopt the v0.2.1 hfx-core type vocabulary) is implicit inside M2,
   not front-loaded as the roadmap's Phase 0 demands. The bet is made on types
   the plan doesn't acknowledge it doesn't own.

---

## VERDICT: NEEDS REVISION

Must-fix before this converges:

- **[BLOCKER 1]** Name the `hfx-core` v0.2.1 dependency migration as an explicit
  precondition/milestone of M2.
- **[BLOCKER 2]** Fold the complete `AtomId`→`UnitId` repo-wide rename into M2
  (or an immediately-preceding migration milestone); stop deferring naming to M5.
- **[BLOCKER 3]** Redefine M1 to emit inert committed golden artifacts + a
  comparison harness that survives the M2 cut; do not regenerate goldens from a
  builder M2 mutates.
- **[MAJOR 4]** Make level selection constrain resolution (or reorder the
  skeleton); add a finest-unit-resolution gate.
- **[MAJOR 5]** Scope a minimal blessed-D8 aux accessor in M4, distinct from the
  deferred binding; re-key the "no rasters" outcome off the D8 aux decl.
- **[MAJOR 6]** State the pyshed hold strategy (port thin bindings in M2, or
  exclude from default workspace members); don't leave the build red.
- **[MAJOR 7]** Pin byte-identical raster aux across oracle/fixture and define
  canonical-WKB normalization in M1, so M4's gate is unambiguous.
- **[MAJOR 8]** Give the v0.2.1 `DatasetBuilder`/test-fixture rewrite explicit
  scope and a "existing suite ported & green" gate.
- **[MAJOR 9]** Treat `graph.parquet` as a net-new reader with its own gate.

Resolve the four BLOCKER/MAJOR clusters around the unacknowledged hfx-core
upgrade, the forced rename, the non-durable parity gate, and the level/resolve
ordering, and re-submit. The minors (10–12) should be folded in at the same time
since they cost little once the structure is fixed.

---

# Re-review — Round 2 (revised plan)

Re-verified the revised `milestone-plan.md` against the same ground truth, plus
two new spot-checks: the `hfx.aux.snap.v1` schema and the resolver's snap
ranking, and exactly what each M2 gate command compiles.

## Round-1 objections — disposition

- **[BLOCKER 1] hfx-core dep cutover** — *Resolved.* M2 scope now owns
  re-pointing `hfx-core` to the v0.2.1 source and recording it in the dependency
  graph; ordering and Risk #1 reinforce it.
- **[BLOCKER 2] forced atom→unit rename** — *Resolved.* M2 scope performs the
  repo-wide `AtomId`→`UnitId` / `AtomCount`→`UnitCount` / method rename; M5
  ordering explicitly states it no longer carries the migration.
- **[BLOCKER 3] non-durable parity** — *Resolved.* M1 now commits inert golden
  artifacts + a checked-in v0.1 input copy + byte-identical rasters + a
  canonicalization spec + a harness that survives the cut; capture test may
  retire while goldens/harness persist.
- **[MAJOR 4] resolve↔level circularity** — *Resolved.* M3 reorders to
  select-level-then-resolve, constrains resolution to the level, and adds a
  `finest_level_resolution` gate on a two-level nested fixture.
- **[MAJOR 5] D8 aux binding leakage** — *Resolved.* M4 scopes a minimal
  blessed-D8 accessor distinct from the deferred binding, re-keys availability
  off the D8 decl, and adds a `d8_aux_accessor` gate.
- **[MAJOR 6] pyshed build hold** — *Resolved* in intent (M2 keeps pyshed
  compiling mechanically; gate uses `cargo build --workspace`), but see new
  objection 13 — the gate does not actually compile pyshed's or shed-core's test
  code.
- **[MAJOR 7] ambiguous parity gate** — *Resolved.* M4 reuses the exact M1
  raster bytes and applies M1 canonicalization, so a red gate isolates behavior
  or path resolution from conversion.
- **[MAJOR 8] fixture-builder rewrite** — *Mostly resolved.* M2 owns the v0.2.1
  fixture infrastructure with an `hfx_v02_test_fixtures` gate; residual gating
  gap folded into objection 13.
- **[MAJOR 9] graph.parquet net-new reader** — *Resolved.* M2 scopes it with a
  `graph_parquet_reader` gate and referential-integrity checks.
- **[MINOR 10] topology** — *Resolved.* M2 loads `topology`, keeps traversal
  dedup-safe, and the loader gate includes a DAG fixture.
- **[MINOR 11] typed version rejection** — *Resolved.* M2 requires a typed
  unsupported-version diagnostic before missing-field parsing; gate asserts it.
- **[MINOR 12] M5 verifiability** — *Resolved.* M5 gate adds an `rg` atom-naming
  check with an allowlist plus a docs-presence requirement.

## Remaining objections

### 13. [MAJOR] — The M2 gate does not compile the test code that holds most of the renamed symbols, so the "compiling workspace" boundary is not actually verified until M5

M2's headline is that the repo crosses the v0.2.1 boundary "as a compiling
workspace," and its central fixes (objections 2, 6, 8) all assert *completeness*
of the rename and fixture port. But the gate cannot see it:

- `cargo build --workspace` compiles lib/bin/example targets only. It does
  **not** compile `#[cfg(test)]` modules or any integration test.
- `cargo test -p shed-core --test hfx_v02_loader` (and the other two `--test`
  targets) compile the library in normal cfg plus *that one* integration file.
  They do **not** compile the in-lib unit-test modules, nor the existing
  integration tests (`outlet_resolution.rs`, `session_open.rs`, etc.), nor
  pyshed's tests.

The bulk of `AtomId` / `DatasetBuilder` / `terminal_atom_id()` consumers live in
exactly that uncompiled test code: `engine.rs` has ~10 `#[cfg(test)]` tests built
on `AtomId::new` and `DatasetBuilder`; `crates/core/tests/*` use the v0.1
builder; `crates/python` has its own tests. A half-finished rename or a
not-yet-ported test fixture passes every M2 gate command and only surfaces at
M5's bare `cargo test -p shed-core`. That means the "one clean compile boundary"
claim — the justification for forcing the whole migration into M2 — is unproven
by M2's own gate.

**Fix demanded:** add `cargo test -p shed-core` (runs lib unit tests + all
integration tests) to the M2 gate, and `cargo test --workspace --no-run` (or
`cargo build --workspace --tests`) so pyshed's and every crate's test code is
compiled at the boundary. Without compiling the test code, M2 cannot claim the
migration and fixture port are complete.

### 14. [MAJOR] — The snap-feature reader rewrite and the resolver's `MainstemStatus`→`StemRole` migration are unnamed, though they are the exact analog of the graph.parquet/AtomId work that *was* scoped

The v0.2.1 cutover forces a second reader rewrite and a second type migration
that the plan addresses only obliquely:

- shed's resolver ranks snap candidates by `target.mainstem_status()`
  (`resolver.rs:443-448`), returning a v0.1 `MainstemStatus`. The new hfx-core
  `SnapTarget` has **no** `mainstem_status()`; it exposes
  `stem_role() -> Option<StemRole>` (`hfx-core/src/snap.rs:116`). So the resolver
  will not compile after the M2 dep cut — a forced migration of the same class as
  `AtomId`→`UnitId`, but never named. (It *is* compiler-caught by
  `cargo build --workspace`, so it won't pass silently — but it is unscoped
  effort and unspecified behavior: the mainstem tie-break must be redefined in
  terms of the four-valued `StemRole`, and the ranking cascade in the snap aux
  schema is now weight → `stem_role = mainstem` → id.)
- The snap *feature* reader is net-new: v0.1 read a root `snap.parquet`; v0.2.1
  reads an `hfx.aux.snap.v1` parquet with a new column set (`id`, `unit_id`,
  `weight`, `stem_role`, optional `bbox_*`, `geometry`) selected via
  `references_levels`. That is as much new reader surface as `graph.parquet`,
  which the plan correctly elevated to its own scope bullet and gate — the snap
  reader got neither.

M3 mentions snap *level filtering* (`references_levels`) but not the column/type
migration the resolver ranking depends on, and M2's snap coverage is limited to
"parse `manifest.auxiliary[]` into stored declarations" plus an optional fixture.

**Fix demanded:** name the `hfx.aux.snap.v1` snap-feature reader as explicit M2
scope (parallel to `graph.parquet`), and name the resolver
`MainstemStatus`→`StemRole` ranking migration as part of M2's forced type
migration (parallel to `AtomId`). Add a snap-resolution gate in M3 that
exercises the weight → mainstem(`stem_role`) → id cascade against an
`hfx.aux.snap.v1` fixture.

### 15. [MINOR] — M1's committed oracle may be synthetic-only; carve parity then proves little about real datasets

No HFX dataset is checked into the repo today (no `manifest.json` /
`catchments.parquet` / `graph.arrow` outside `target`/`.venv`). M1's "checked-in
v0.1 input dataset" will therefore be materialized from the synthetic
`DatasetBuilder` (3-atom linear, synthetic rasters) unless a real sample is
introduced. A synthetic 3-cell carve will not exercise the containment-clamp
edge cases (multi-ring terminals, snap-to-high-accumulation across a real
window) that the parity gate exists to protect.

**Fix demanded (low priority):** M1 should commit at least one realistic
multi-unit, real-raster carve case alongside the synthetic one, or explicitly
acknowledge in M1 that parity coverage is synthetic and name the follow-up that
adds a real-data parity case.

## Axis recheck

1. **Parity integrity** — Now sound in structure (durable goldens, pinned
   rasters, canonicalization). Residual: representativeness (obj. 15).
2. **Ordering & hidden dependencies** — hfx-core cutover, forced rename, and
   resolve/level are all fixed. One hidden migration remains unnamed (obj. 14),
   though compiler-caught.
3. **Verifiability in isolation** — Improved everywhere except the M2 gate, which
   does not compile the test code it implicitly depends on (obj. 13).
4. **Deferred-mechanism leakage** — Clean: D8 accessor and pyshed hold are
   correctly bounded.
5. **Missing gates/milestones** — graph.parquet, fixtures, topology, version
   rejection now owned. Snap reader is the remaining unscoped analog (obj. 14).
6. **Types-are-the-interface** — Now front-loads the imported-type cutover (M2)
   ahead of the consuming slices. Sound.

## VERDICT: NEEDS REVISION

Down from 3 BLOCKER + 6 MAJOR to **0 BLOCKER + 2 MAJOR**. The structural problems
are fixed; what remains is gate completeness and one unnamed forced migration.

Must-fix to converge:

- **[MAJOR 13]** Add `cargo test -p shed-core` and a workspace test-compile
  (`cargo test --workspace --no-run`) to the M2 gate, so the rename/fixture-port
  completeness M2 claims is actually verified at the boundary, not deferred to M5.
- **[MAJOR 14]** Name the `hfx.aux.snap.v1` snap-feature reader (M2 scope+gate,
  parallel to `graph.parquet`) and the resolver `MainstemStatus`→`StemRole`
  migration (M2 forced-migration scope), with an M3 snap-resolution cascade gate.

Fold in **[MINOR 15]** (real-data parity case) at the same time. These are
additive edits, not structural rework — one more pass should converge.

---

# Re-review — Round 3 (final)

Verified the latest `milestone-plan.md` against the round-2 must-fix list.

- **[MAJOR 13] M2 gate compiles test code** — *Resolved.* M2's gate now runs
  `cargo test -p shed-core` (lib unit tests + integration) and
  `cargo test --workspace --no-run` (compiles every crate's test targets,
  including pyshed, without requiring a Python runtime). The gate note states
  this proves the rename and fixture port are complete at the M2 boundary rather
  than deferred to M5. The migration-completeness claim is now actually gated.
- **[MAJOR 14] snap reader + StemRole migration** — *Resolved.* M2 scope now
  names the net-new `hfx.aux.snap.v1` snap-feature reader (id, unit_id, weight,
  optional stem_role, optional bbox, WKB geometry) and its referential-integrity
  checks, parallel to `graph.parquet`, plus the forced
  `MainstemStatus`→`StemRole` resolver-ranking migration as part of the type
  cutover. Gate adds `snap_aux_reader`. M3 scopes the v0.2.1 ranking cascade
  (weight → `stem_role = mainstem` → snap ID) with a `snap_resolution_cascade`
  gate, consistent with the snap aux v1 spec and the existing resolver order.
- **[MINOR 15] real-data parity** — *Resolved.* M1 requires a realistic
  real-raster carve case when a sample can be checked in at acceptable size, and
  otherwise mandates an explicit synthetic-only statement plus a recorded
  follow-up task.

No new objections. The gates are runnable commands tied to specific behaviors,
each milestone is verifiable without later milestones, deferred mechanisms
(full aux binding, pyshed redesign, Python-authored strategies, non-finest level
selection) remain clean campaign boundaries, and the imported-type cutover is
correctly front-loaded ahead of the slices that consume it.

## VERDICT: CONVERGED

0 BLOCKER, 0 MAJOR. From the round-1 baseline of 3 BLOCKER + 6 MAJOR, all
structural defects (unacknowledged hfx-core upgrade, forced rename smeared across
milestones, non-durable parity gate, resolve/level circularity, leaked aux
binding, red pyshed build, under-gated boundary, unnamed snap migration) are
fixed with explicit scope and gates. The plan is sound to execute.
