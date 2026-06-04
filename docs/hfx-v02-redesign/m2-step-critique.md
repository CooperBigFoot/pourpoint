# M2 Step Plan — Adversarial Critique

Target: `docs/hfx-v02-redesign/m2-step-plan.md` (the 10-step plan).
Contract: `docs/hfx-v02-redesign/milestone-plan.md` (M2 section).
Method: read every step, grep the rename surface and the two `hfx-core` APIs,
diff shed's actual calls against the v0.2.1 contract, check the specs. Evidence is
`file:line`, not vibes.

**Headline:** the plan's central architectural claim — "Step 1 is a mechanical
rename + thin pyshed bindings; v0.2.1 reader *behavior* is added later in Steps
2–6" — is false. The crates.io→local dependency flip removes whole API families
that have **no rename target**, so the readers and session rewrites of Steps
2/4/5/6 and the fixture rewrite of Step 7 are *compile-forced* into the same
boundary as Step 1. The per-step green gates for Steps 2–7 are therefore
unsatisfiable in the stated order. **NO-GO** until Step 1 / the ordering is
re-drawn.

---

## Per-Surface Verdict Table

| # | Surface | Verdict | One-line reason |
|---|---|---|---|
| 1 | Rename = one compile boundary? | **BROKEN** | Dep flip deletes `RasterAvailability`/`SnapAvailability`/`MainstemStatus` and changes `AdjacencyRow::new` arity — no rename target; Steps 2/4/5/6/7 are forced into Step 1 to even compile. |
| 2 | Can the M1 durable gate break? | **SOUND** | `parity_golden_artifacts.rs` is already loader/`hfx_core`/`AtomId`-free; goldens are version-neutral `i64`; canonicalizer untouched; B fixture protected. One NIT on the Step 8 grep. |
| 3 | Offline gate secretly networked? | **SOUND** | Real-data test is `#[ignore]` + `SHED_HFX_V02_REAL_R2_LOAD` env, mirroring M1's `SHED_PARITY_R2_CAPTURE`; offline tier compiles-and-skips. |
| 4 | Snap/bbox rejections actually tested? | **WEAK** | Snap referential-integrity + 4-value `StemRole` coverage is real and spec-backed; but the plan hard-rejects missing `graph.parquet` `bbox_*` while the spec table marks them `Required = No`. Justification mis-cited. |
| 5 | Deferred mechanism leak? | **SOUND** | Resolver distance tie-break explicitly preserved; M3/M4 work fenced off. One factual NIT in an Open Decision. |
| 6 | pyshed gate honesty | **SOUND** | Uses `--exclude pyshed` + `cargo check -p pyshed`; explicitly forbids bare `--workspace` and `.cargo/config.toml`. (The *acceptance-criterion* contradiction about pyshed method names is filed under Surface 1.) |

---

## Surface 1 — BROKEN: the dependency flip is not separately compilable from the readers

### 1a. Removed APIs have no rename target (library will not compile after Step 1's flip)

The plan, Step 1 (lines 119–121), enumerates the compile boundary as exactly:
`AtomId/AtomCount`, `CatchmentAtom`, terminal/upstream atom methods, `atom_count`,
`MainstemStatus`. That list is incomplete. The local v0.2.1 `hfx-core` does **not
export several types shed's library source calls**, and they are not renames:

Evidence — v0.2.1 `hfx-core` exports (`../hfx/crates/hfx-core/src/lib.rs:21–33`)
and a zero-hit grep across that crate:

```
RasterAvailability : 0
SnapAvailability   : 0
MainstemStatus     : 0
CatchmentAtom      : 0
AtomId             : 0
atom_count         : 0
```

But shed's **library** (not tests) calls exactly those:

- `crates/core/src/session.rs:10` imports `RasterAvailability, SnapAvailability`.
- `crates/core/src/session.rs:229,265,423` — `manifest.snap() == SnapAvailability::Present`.
- `crates/core/src/session.rs:239,277,502` — `matches!(manifest.rasters(), RasterAvailability::Present(_))`.

There is no `SnapAvailability`/`RasterAvailability` to rename to. In v0.2.1
presence is expressed through `manifest.auxiliary[]` (blessed snap / blessed D8) —
which is **Step 2** (manifest auxiliary parsing) and **Step 6** (session wiring),
and the raster side is arguably **M4**. So `session.rs` cannot compile against the
v0.2.1 crate by any mechanical rename; it must be re-architected onto `auxiliary[]`
inside Step 1.

### 1b. `AdjacencyRow::new` changed arity (graph reader forced into Step 1)

- shed today: `crates/core/src/reader/graph.rs:173` → `AdjacencyRow::new(atom_id, upstream)` (2 args).
- v0.2.1: `../hfx/crates/hfx-core/src/graph.rs:20` → `pub fn new(id: UnitId, level: Level, upstream_ids: Vec<UnitId>)` (3 args).

The new constructor requires a `Level` the current Arrow reader never reads. You
cannot rename your way past a missing argument; you need the `graph.parquet`
reader that supplies `level` — that is **Step 4**. The plan's own Step 4 (line
259) even writes the 3-arg signature, proving it knows the arity changed, yet
Step 1 claims graph code compiles after a rename.

### 1c. Manifest API shape changed (manifest reader forced into Step 1)

`session.rs:288,354,397,513,655` call `manifest.atom_count()`. The v0.2.1
`Manifest` has no `atom_count`/`terminal_sink_id`; the loader-facing surface is
`unit_count` + `auxiliary[]` (Step 2). `manifest.rs` (10 atom hits) and
`catchment_store.rs` (`CatchmentAtom` → `CatchmentUnit`, a *type swap with a
different field set*, not a token rename) are likewise compile-forced.

### 1d. The per-step full-test gate cannot pass for Steps 2–6 (Step 7 entanglement)

The CI-aligned gate that runs "at the end of every step after Step 1" (plan lines
63–73) includes **`cargo test -p shed-core`** — full execution. The existing
integration tests build datasets from `testutil.rs` and then load them:

- `crates/core/tests/object_store_integration.rs:71,127–128,182–184` builds a v0.1
  manifest (`terminal_sink_id`, `atom_count`) inline and asserts
  `session.manifest().atom_count()` and `result.terminal_atom_id()`.
- `testutil.rs:190–200` emits a v0.1 `manifest.json` (`terminal_sink_id`,
  `atom_count`, `has_snap`, `has_rasters`); `:234` writes `graph.arrow`; `:334–417`
  writes `snap.parquet` with an `is_mainstem` Boolean column.

The fixture builder produces **v0.1-format bytes**. Until Step 7 rewrites it to
emit v0.2.1 (`graph.parquet`, `hfx.aux.snap.v1`, `unit_count`, `auxiliary[]`),
every reader test that runs through the builder fails at runtime. So the full
`cargo test -p shed-core` in the Steps 2–6 gates is red until Step 7 — but Step 7
is sequenced last. The fixture rewrite and the reader rewrites are mutually
entangled; they cannot be independently green.

### 1e. The v0.1 capture test must die in Step 1, not Step 8

`crates/core/tests/parity_v01_oracle_capture.rs` imports `hfx_core::AtomId`
(`:10`), builds `hfx_core::CatchmentAtom` (`:1022`), and references
`PipTieBreak::LowestAtomId` (`:1252`), `terminal_atom_id`/`upstream_atom_ids`
(`:601,619,1025`) — and exercises the v0.1 `Engine` loader path. After the dep
flip it cannot compile. Step 1's own verification runs `cargo test --workspace
--exclude pyshed --no-run` (plan line 156), which **compiles** this test. Yet the
plan defers its retirement to Step 8 (lines 433–436) and says "Do not port it."
Internal contradiction: Step 1's `--no-run` gate is red unless this file is
deleted/neutralized *in Step 1*.

### 1f. Step-1 file list misses `__init__.pyi` and `API.md`; pyshed acceptance is self-contradictory

The rename leaks into files Step 1 does not list. Step 1 "Files touched" only
names `crates/python/src/*.rs` and `crates/python/tests/*.py`. Missing:

- `crates/python/python/pyshed/__init__.pyi:44,59,77,92` — `terminal_atom_id`, `upstream_atom_ids`.
- `crates/python/API.md:173,178,209,214` — same symbols.

Both sit under `crates/`, so Step 1's acceptance grep (`rg … crates src`, plan
line 153) returns them. Worse, there is a contradiction at the heart of Step 1's
acceptance criterion (lines 142–145): "no … `terminal_atom_id`,
`upstream_atom_ids` … remain in Rust/Python source," *and* "pyshed compiles by
thin binding updates only." But pyshed's **public Python methods are literally
named** `terminal_atom_id`/`upstream_atom_ids`
(`crates/python/src/result.rs:48,81,148,181`). You cannot satisfy the grep without
renaming those public methods — which is a pyshed **API change**, explicitly
out-of-scope ("no pyshed API redesign," Step 1 line 165; milestone "build hold
only"). If instead you keep the names and only swap the inner call (true "thin
binding"), the grep fails. Pick one — the plan currently asserts both.

### Minimal fixes for Surface 1 (any of these makes it sound)

1. **Re-scope Step 1 honestly.** Rename it to "v0.2.1 contract cutover" and make
   it own — in the one red→green commit — the compile-critical minimum of: session
   `auxiliary[]`-based snap/raster availability, the `graph.parquet` reader (to
   supply `Level`), the manifest `unit_count`/`auxiliary[]` parse, the
   `CatchmentUnit` swap, the `hfx.aux.snap.v1` reader (to drop `is_mainstem`/
   `MainstemStatus`), **and** the `testutil.rs` fixture port. Steps 2–7 then become
   "harden behavior + add typed errors + add tests," not "add the reader." This is
   the truthful shape; the plan already half-admits it ("no v0.2.1 reader behavior
   *beyond what is required to compile*" — but "what is required to compile" is most
   of Steps 2–6).
2. **Or** keep the granular steps but delete/`#[ignore]` the existing v0.1
   integration + capture tests *in Step 1*, and relax the Steps 2–6 gate to
   `--no-run` until the reader+fixture set lands together; only re-enable full
   `cargo test -p shed-core` once Step 7 is done.
3. For 1f: scope the no-atom acceptance grep to `crates/core/src` (+ the renamed
   core methods), and state explicitly that pyshed **retains**
   `terminal_atom_id`/`upstream_atom_ids` Python method names until M5 (consistent
   with the milestone giving M5 the public-name cleanup), updating only the inner
   `self.inner.*` calls. Then `__init__.pyi`/`API.md` legitimately need no edit —
   but the plan must say so instead of leaving them as silent grep hits.

---

## Surface 2 — SOUND: the M1 durable gate holds

- `crates/core/tests/parity_golden_artifacts.rs:1–18` already imports **no**
  `DatasetBuilder`/`DatasetSession`/`Engine`/`AtomId`/`hfx_core` — only
  `shed_core::algo` canonicalizer symbols. The "load-bearing" invariant is already
  in place; M2 does not need to *create* it.
- Goldens are version-neutral: `terminal_id: i64`, `upstream_ids: Vec<i64>`
  (`parity_golden_artifacts.rs:33–34`; contract `fixtures/parity/README.md:36–37`).
- Canonicalizer is untouched: `shed-canonical-wkb-v1`, 6-dp
  (`README.md:9–10`); Step 3/8 explicitly forbid touching
  `algo/canonical_wkb*` and the constants.
- B fixture immutability: `README.md:86–89` and Step 1/8 "Must not touch …
  `v01_synthetic_refined/`." The durable test re-hashes the **original** M1 TIFFs;
  the converted v0.2.1 copy reuses those bytes. Good.

**NIT (Surface 2).** Step 8's verification grep (plan line 424) is
`rg "AtomId|UnitId|hfx_core|Engine|…|atom|unit" parity_golden_artifacts.rs`. It
will always match the **doc comment** at `parity_golden_artifacts.rs:4` ("…
`Engine`, `AtomId`, `hfx_core` …"), whose entire purpose is to *name* the
forbidden imports and explain why they're absent. Editing that prose to satisfy a
grep degrades load-bearing documentation. Fix: assert absence of `use … hfx_core`
/ loader **imports**, not absence of the words in comments.

**NIT (Surface 2).** Step 8 pins integrity of the M1 originals via re-hash, but
the *converted v0.2.1 copy's* TIFF bytes are only asserted "byte-identical" at
authoring time — no durable hash guards the copy. Drift in the copy surfaces only
at M4's carve parity, not in M2. Acceptable (M2 runs no carve), but worth a one
line note.

---

## Surface 3 — SOUND: offline gate stays offline

- `crates/core/src/source.rs:18` defines `PUBLIC_R2_CUSTOM_DOMAIN =
  "basin-delineations-public.upstream.tech"` and routes it at `:149,219`. The plan's
  ignored test uses exactly this path.
- The pattern mirrors the proven M1 gating:
  `parity_v01_oracle_capture.rs:41` `SHED_PARITY_R2_CAPTURE`, `:127` `#[ignore]`.
  The plan specifies **both** `#[ignore]` and `SHED_HFX_V02_REAL_R2_LOAD`
  (plan lines 80–91, 451–459), so the offline `cargo test … --test hfx_v02_loader`
  compiles-and-skips and no gate-line test reaches R2 without the env switch.
- The oracle is concrete/falsifiable: `unit_count == 22_337_300`, `topology ==
  dag`, exactly two `hfx.aux.snap.v1`, no D8.

Caveat (not a defect, a build-time obligation): this is a *plan*; the executor
must actually attach `#[ignore]` **and** early-return when the env is unset. The
plan states both, so it is sound on paper.

---

## Surface 4 — WEAK: snap coverage is solid; graph-bbox rejection is mis-justified

**Snap side is sound and spec-backed.** `../hfx/spec/aux/snap/v1.md` confirms every
premise the plan tests:
- Four `stem_role` values: `mainstem`, `tributary`, `distributary`, `unknown`
  (`snap/v1.md:89,134–135`). Step 5 tests all four + an invalid value (plan
  lines 307–310). Good.
- `references_levels` non-empty, and "referenced unit levels … must be listed in
  `metadata.references_levels`" (`snap/v1.md:74,97,126,132`). Step 5's
  `SnapReferentialIntegrity` covers `unit_id`-missing and level-not-declared (plan
  lines 301–304). Good.
- `unit_id` referential integrity and Point/LineString WKB are covered (plan
  lines 307–311).

**Graph side — the WEAK finding.** Step 4 (plan line 270) adds
`GraphMissingBboxColumn` "fired for missing required graph bbox columns," and the
milestone gate (milestone-plan line 177) demands "rejection of missing
`graph.parquet` `bbox_*` columns." But the spec table marks those columns
`Required = No`:

- `../hfx/spec/HFX_SPEC.md:147–150` — `bbox_minx … bbox_maxy | float32 | No`.

The only spec basis for requiring the *columns to exist* is the separate clause
"Parquet row-group statistics on `bbox_*` **must** be written"
(`HFX_SPEC.md:163–164`) — which implies the columns exist, even though their values
are nullable. The plan conflates "column must be present (stats clause)" with
"column required (table)." As written, a reviewer can argue the hard rejection
contradicts the spec table.

*Failure scenario:* a conformant producer writes `graph.parquet` without `bbox_*`
columns (legal per the `Required = No` table if the stats-clause is read as
applying only when bbox is present) → shed rejects a spec-legal dataset, or
GRIT v2.0.0's real graph trips the check unexpectedly. *Minimal fix:* re-base the
rejection on `HFX_SPEC.md:163–164` (stats-must-be-written ⇒ columns must exist)
and say so in Step 4's rationale; and add a test asserting the GRIT/MERIT real
graphs actually carry the columns, so the requirement is empirically grounded, not
asserted.

(Also confirmed real and well-covered: `upstream_ids` is `list<int64>`
[`HFX_SPEC.md:146`], same-level edges are mandated [`HFX_SPEC.md:174`,
`:117`], so Step 4's same-level integrity test and list-decode test are
testing real spec requirements. Legacy `graph.arrow` rejection: spec drops Arrow;
current loader is `graph.rs:1,14,27` Arrow-only, so the rejection is meaningful.)

---

## Surface 5 — SOUND: no deferred mechanism leaks; resolver behavior preserved

- **Resolver behavior is explicitly held constant.** The current ranking is
  weight DESC → mainstem DESC → snap_id ASC *within a distance-tolerance band*
  (`crates/core/src/resolver.rs:416,419–470`), and `DistanceFirst` keeps distance
  as a first-class tie-breaker. Step 1 (lines 146–148), Step 5 (line 310), and the
  Open Decisions (lines 519–522) all forbid dropping the distance tie-breaker and
  route any such change to M3. This directly answers the "ranking behavior change
  under guise of a type swap" attack — the plan resists it. Sound.
- No M3/M4 smuggling: Step 2 forbids the reverse-DNS parser and aux→strategy
  binding (lines 207–208); Step 6 forbids mixed-level traversal and a
  topology-specific strategy (lines 359–360); no refinement trait appears (that's
  M4). `auxiliary[]` is *stored as raw resolved path + metadata only* (Step 2 line
  194) — handle, not parser. Good.

**NIT (Surface 5):** the Open Decision (plan line 521) states "the snap.v1 spec
include[s] distance before snap id." The snap spec cascade is weight → break ties
by `stem_role = mainstem` (`snap/v1.md:104–106`); it does **not** mention distance.
Distance is *shed's resolver* behavior, not the snap spec's. The M2 directive
(preserve shed's current behavior) is unaffected, but the rationale misattributes
distance to the spec — fix the sentence so a future reader doesn't "correct" the
resolver to match a spec clause that isn't there.

---

## Surface 6 — SOUND: pyshed gate is CI-aligned

- The gate is `cargo build --workspace --exclude pyshed` + `cargo check -p pyshed`
  (plan lines 64–65), with explicit guards: "Do not add `cargo build --workspace`
  without `--exclude pyshed`. Do not add a `.cargo/config.toml`." (lines 75–76).
  This matches the milestone (lines 163–164, 187–189) and rests on `cargo check`,
  not a cdylib link. No macOS `_Py*` link is assumed in the gate.

(The only pyshed problem is the *acceptance-criterion* contradiction about whether
the public method names get renamed — filed under Surface 1f, since it is a rename
question, not a gate-command question.)

---

## "Also check" sweep

- **Version bump per commit:** present on every code step (Steps 1–9 each carry
  "patch bump … tag"; Step 10 conditional). `pyshed-v*` release explicitly
  forbidden (Step 1 line 162). ✔
- **thiserror doc-comments (WHEN they fire):** the plan describes each new
  variant's fire condition in prose but never explicitly instructs "add a
  `///` doc comment per `CLAUDE.md`." Substance is present; the instruction is
  implicit. *NIT:* add the explicit doc-comment mandate to each step that creates
  errors (Steps 2–5), since the new variants — `UnsupportedFormatVersion`,
  `AuxiliaryPathEscape`, `GraphReferentialIntegrity`, `InvalidStemRole`, etc. — are
  exactly where the convention matters.
- **No `.unwrap()`/`.expect()` in library:** the plan introduces none, but also
  never restates the rule. Existing unwraps are in `#[cfg(test)]` helpers
  (`assembly.rs:767`, `id_index.rs:295`), which is allowed. *NIT:* the snap/graph
  readers parse Arrow/Parquet columns — exactly where a careless `.unwrap()` on a
  downcast creeps in. A one-line reminder in Steps 4/5 is cheap insurance.
- **Newtypes/enums per doctrine:** `StemRole` (enum), `UnitId`/`UnitCount`/`Level`
  (newtypes), `FixtureUnit`/`FixtureGraphRow`/… (Step 7 typed helpers avoiding raw
  primitive leakage). ✔
- **Dependency-pin form surfaced as explicit decision:** yes — both the
  Dependency-Graph Note (plan lines 52–56) and Open Decisions (lines 515–517) flag
  path-patch vs git-rev `478dfa6` as an owner decision, with the
  crate-version-vs-format-version collision called out. ✔
- **Milestone edit recorded the GRIT v2.0.0 divergence + CI-aligned gate:** yes —
  milestone-plan lines 143–150 record GRIT v2.0.0 as a first-class network-gated
  target (`unit_count = 22,337,300`, `topology = dag`, two snap aux, **no D8**),
  and lines 162–172 use `--exclude pyshed` + `cargo check -p pyshed`. The
  "no D8 on GRIT v2.0.0" divergence (vs MERIT for M4 D8 parity,
  `fixtures/parity/README.md:91–93`) is captured. ✔

---

## BLOCKERS (must fix before sign-off)

1. **Surface 1a/1b/1c — Step 1 cannot compile as a "rename."** The dep flip
   removes `RasterAvailability`/`SnapAvailability`/`MainstemStatus`/`CatchmentAtom`
   (0 occurrences in v0.2.1 `hfx-core`) and changes `AdjacencyRow::new` to 3 args.
   `session.rs`, `graph.rs`, `manifest.rs`, `snap_store.rs`, `catchment_store.rs`
   must be substantively rewritten (Steps 2/4/5/6 surface) inside the same boundary
   as Step 1. Re-scope Step 1 (fix #1) or restructure the gate (fix #2).
2. **Surface 1d — Steps 2–6 per-step gate is unsatisfiable.** The shared
   `testutil.rs` emits v0.1-format bytes; full `cargo test -p shed-core` (run after
   every step) stays red until the Step 7 fixture port lands. Fold the fixture port
   into the compile boundary, or relax the mid-sequence gate to `--no-run`.
3. **Surface 1e — `parity_v01_oracle_capture.rs` must be retired in Step 1, not
   Step 8.** It references removed types + the v0.1 loader and breaks Step 1's
   `--no-run` gate. Move its deletion/neutralization to Step 1.
4. **Surface 1f — pyshed acceptance criterion is self-contradictory.** Resolve
   "no `terminal_atom_id` in Python source" vs "thin binding only / no API
   redesign" (pyshed public methods are named `terminal_atom_id`). Scope the grep
   to core and pin pyshed names to M5.

## NITS (optional)

- Step 1 file list omits `crates/python/python/pyshed/__init__.pyi` and
  `crates/python/API.md` (both grep-hit under `crates/`). Either list them or, per
  fix 1f, state they are intentionally untouched until M5.
- Step 8 grep targets comment prose in `parity_golden_artifacts.rs:4`; assert
  absence of imports instead.
- Surface 4: re-base graph `bbox_*`-required on the row-group-stats clause
  (`HFX_SPEC.md:163–164`), not the `Required = No` table; add a real-data column
  presence assertion.
- Surface 5: Open Decision (plan line 521) misattributes the distance tie-break to
  the snap.v1 spec; it is shed resolver behavior. Fix the sentence.
- Steps 2–5: add the explicit `thiserror` doc-comment mandate and a
  no-`.unwrap()`-in-reader reminder.
- No durable hash on the converted v0.2.1 fixture's TIFF copy (only the M1
  original is re-hashed). Acceptable for M2; note it.

---

## Recommendation: **NO-GO**

The plan's verification structure (durable parity gate, offline/network split,
pyshed `cargo check` hold, resolver-behavior freeze, spec-backed snap coverage,
explicit dependency-pin decision) is genuinely strong — Surfaces 2, 3, 5, 6 hold,
and Surface 4 is one citation away from sound. But the spine of the plan — the
ordered step boundary — rests on a false premise: that the `hfx-core` flip is a
mechanical rename with reader behavior layered on afterward. The v0.2.1 contract
**deletes** the availability/snap-status API and **changes** the graph constructor,
so the readers, the session, and the fixture builder are all compile-forced into
the first commit, and the per-step green gates for Steps 2–7 cannot hold in order.

Fix Blockers 1–4 (most cleanly by re-scoping Step 1 into an honest "v0.2.1 contract
cutover" boundary that absorbs the compile-critical minimum of Steps 2/4/5/6/7,
with Steps 2–7 demoted to behavior-hardening + tests). Re-submit; the remaining
surfaces need only the listed NITs.
