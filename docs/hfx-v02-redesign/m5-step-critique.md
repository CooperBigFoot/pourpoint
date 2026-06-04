# Adversarial Critique — M5 Integrated Step Plan

Target: `docs/hfx-v02-redesign/m5-step-plan.md` + the M5 section edit in
`docs/hfx-v02-redesign/milestone-plan.md`.
Reviewer stance: attack the plan. Every objection below is grounded in a
read-only command I ran or a file:line I read, not in the plan's own assertions.
The converged E1-E9 design (label source, Hilbert ownership, bbox rounding,
row-group balancing, BasinId allowlist) is SETTLED and not re-litigated; I only
checked whether the integrated plan regressed it.

## Ground truth established before judging

- Repo state: `git branch --show-current` = `main`; `grep '^version' Cargo.toml`
  = `0.1.140`. Matches the plan's orientation.
- Core result API is already unit-named: `crates/core/src/engine.rs:75`
  `terminal_unit_id()`, `:95` `upstream_unit_ids()`, `:126` `geometry_wkb()`;
  the area-only struct mirrors it (`:136`,`:140`,`:149`). So Phase C export can
  consume stable names regardless of the Phase B rename — **the A→C dependency
  is real and the B-before-C ordering is a hygiene choice, not a compile
  dependency**. No E-step compiles against a name the rename later changes.
  Sequencing is SOUND.
- Manifest identity accessors exist and are distinct:
  `../hfx/crates/hfx-core/src/manifest.rs:209` `fabric_name()`, `:214`
  `fabric_version() -> Option<&str>`, `:257` `adapter_version() -> &str`. The
  step plan sources `delineation` off `fabric_version()` (orientation l.67) —
  the E-contract is NOT regressed here.
- Live W1 leak surface confirmed exactly as the plan states: `src/main.rs:361`,
  `:464` (`terminal_atom_id`), `:472` (`upstream_atom_count`);
  `crates/python/src/result.rs:48,81,136,148,181,197`;
  `crates/python/src/geojson.rs:17,37`; `crates/core/README.md:186-191,229-230`;
  `crates/core/src/algo/refine.rs:1,18,39,122`; `crates/core/src/error.rs:350`.
- The refine.rs leaks are doc comments only; carve output cannot change from a
  comment edit. The W6 "rename comments, re-run `d8_refinement_parity` +
  `parity_golden_artifacts`" decision is SOUND. Frozen-artifact integrity holds.
- Deferral ledger (Phase D) covers full pyshed redesign, aux→strategy binding,
  Python-authored strategies, non-finest level selection, additional refinement
  strategies, versioned spec / conformance suite / pyshed export API. Complete.
- Per-step version-bump doctrine present; W5 correctly forbids `bump-pyshed`
  and a `pyshed-v*` tag.

These surfaces are clear. The plan fails on two independently reproduced
defects below.

---

## BLOCKERS

### B1. [BLOCKER] The hardened atom gate can never go green — its scope scans the very planning docs that describe the atom→unit rename

The W7 inline gate (`m5-step-plan.md:280-282`), the W7 acceptance command
(`:297`), the full M5 closure gate (`:498`), and the milestone gate
(`milestone-plan.md:396`) all use:

```bash
test -z "$(rg -n "\b[Aa]tom" crates/core/src crates/core/README.md docs/hfx-v02-redesign \
  | rg -v "Atomic|atomic|atom-to-unit|AtomId|AtomCount|terminal_atom_id|upstream_atom_ids|atom_count|historical|migration|critique|v0\.1")"
```

I ran that exact command on the current tree:

```
exit=1
```

Even assuming W4/W5/W6 perfectly scrub all code/README/refine/error leaks, the
scope `docs/hfx-v02-redesign` still leaves **22 surviving lines**, distributed:

```
  16 docs/hfx-v02-redesign/m5-step-plan.md
   2 docs/hfx-v02-redesign/milestone-plan.md
   2 docs/hfx-v02-redesign/m5-export-plan.md
   2 docs/hfx-v02-redesign/m2-step-plan.md
```

These are not leaks — they are the committed planning prose that **describes**
the rename and therefore must contain the word "atom":

- `m5-step-plan.md:162` `## Phase B - W1 Mechanical Atom-To-Unit Rename`
- `m5-step-plan.md:4` "mechanically removing the remaining public `atom` terminology"
- `milestone-plan.md:350` "mechanically rename the remaining public atom vocabulary"
- `m5-export-plan.md:300` the old E9 gate line `rg "atom|Atom" ...`

The allowlist tokens (`historical|migration|critique|v0\.1`) do **not** match
these lines, so they survive the filter, `test -z` is false, and the gate
**FAILS by construction**. The plan even self-certifies the opposite:
`m5-step-plan.md:564` lists "Hardened atom grep gate: W7 and final W8/E9" as a
passing self-check. Per the dispatch's explicit rule — *"If the gate as written
can never go green … that is a BLOCKER"* — this lands.

Note the integrated plan made an already-broken inherited gate strictly worse:
it added 16 new atom-bearing lines (its own `m5-step-plan.md`) into the scanned
`docs/hfx-v02-redesign` directory. `milestone-plan.md:396` carries the same
defect and must be fixed in the same pass.

**Fix demanded:** split the gate. (a) Run the hard `test -z` no-leak gate only
over the **live code surface** — `crates/core/src` + `crates/core/README.md`
(+ `src` and `crates/python/src` for the CLI/pyshed renames). (b) For
`docs/hfx-v02-redesign`, either exclude the planning/critique/step docs by path
(`-g '!*-plan.md' -g '!*-critique.md'`) and scan only the single migration-notes
deliverable, or drop the docs directory from the hard gate entirely and keep a
separate, advisory doc check. Re-run the corrected command to prove `exit=0`
before dispatch, and propagate the identical fix to `milestone-plan.md:396`.

---

## MAJORS

### M1. [MAJOR] The pyshed `.pyi` type stub is a public-API leak that no W5/closure gate scans

`crates/python/python/pyshed/__init__.pyi:44,59,77,92` declare the public
Python surface:

```
44:    def terminal_atom_id(self) -> int: ...
59:    def upstream_atom_ids(self) -> list[int]: ...
77:    def terminal_atom_id(self) -> int: ...
92:    def upstream_atom_ids(self) -> list[int]: ...
```

W5 "Files touched" (`m5-step-plan.md:203-205`) lists only
`crates/python/src/result.rs` and `crates/python/src/geojson.rs`, and the W5
gate (`:219-220`) greps **`crates/python/src` only**. The `.pyi` lives under
`crates/python/python/` and is a type stub, so `cargo check -p pyshed` never
type-checks it. The closure atom gate scope (`crates/core/src`, README, docs)
does not include `crates/python` at all. Result: after W5 renames the real PyO3
methods, the stub still advertises `terminal_atom_id`/`upstream_atom_ids` —
methods that **no longer exist** — and every gate stays green. The public
contract the plan is trying to clean is left factually wrong and undetected.
(Confirmed: 4 stub hits, 0 caught by any plan gate.)

**Fix demanded:** add `crates/python/python/pyshed/__init__.pyi` (and any
`__init__.py` re-exports asserting these names) to W5's files-touched, rename
the four declarations, and extend the W5 gate to
`! rg -n "terminal_atom_id|upstream_atom_ids|upstream_atom_count" crates/python`.

### M2. [MAJOR] The allowlist masks real leaks on any line containing migration/critique/historical/v0.1 prose

The filter `rg -v "…|historical|migration|critique|v0\.1"` is line-level, so a
line is dropped if it contains one of those words **anywhere**, even alongside a
genuine domain-atom leak. Reproduced:

```
$ printf 'docs/x.md:1:the migration still says terminal atom everywhere\n' \
  | rg -v "Atomic|atomic|atom-to-unit|AtomId|AtomCount|terminal_atom_id|upstream_atom_ids|atom_count|historical|migration|critique|v0\.1"
(no output — the planted "terminal atom" leak was masked)
```

Because the scanned docs are saturated with "migration"/"critique"/"v0.1", a
future regression that reintroduces `terminal atom` or a new `*_atom_*` name on
such a line passes silently. The dispatch asks specifically whether the
allowlist is "so broad it masks real leaks" — it is.

**Fix demanded:** anchor the allowlist to **identifiers**, not prose: keep
`\bAtomic`, `\batomic`, `AtomId`, `AtomCount`, `terminal_atom_id`,
`upstream_atom_ids`, `atom_count`; drop the broad `historical|migration|
critique|v0\.1` prose tokens and instead handle planning-doc prose via path
exclusion (same fix as B1). Add a planted-leak negative test to W7 proving the
gate still fails on `terminal atom` / `upstream atoms` / a new `*_atom_*` name.

---

## MINORS

### m1. [MINOR] Allowlist is case-sensitive and already misses its own intended token

`rg -v "atom-to-unit"` does not match `Atom-To-Unit` (the heading at
`m5-step-plan.md:162`), so even the token the planner added to whitelist
migration prose fails on the capitalized form. This is subsumed by B1's fix but
shows the allowlist is brittle; the corrected gate must not depend on prose
tokens at all.

### m2. [MINOR] Convention enforcement not restated for the integrated export steps

Phase C steps E2-E6 add net-new Rust (identity types, writer, errors) but the
integrated tickets do not restate the `thiserror`/`tracing`/no-`unwrap`/
type-driven requirements; they rely on the reader following the source export
plan. `m5-export-plan.md:120` does specify the `thiserror` `ExportError` enum,
so this is not a gap in intent — add a one-line "conventions per CLAUDE.md
apply" reminder to Phase C to keep it discoverable for executors.

---

## Surfaces explicitly cleared

- **W1→W2 sequencing**: SOUND. Core API already unit-named (`engine.rs:75,95,
  126`); no E-step depends on the rename landing first. A→B→C→D is the minimal
  correct order; no false or missed dependency.
- **Frozen-artifact integrity**: SOUND. refine.rs is comment-only with
  `d8_refinement_parity` + `parity_golden_artifacts` reruns proving byte
  identity; no canonicalizer/golden/M3-contract/M4-carve change in any step.
- **Export contract not regressed**: SOUND. `fabric_version()` label source,
  global Hilbert extent `[-180,-90,180,90]` (E3 hygiene l.347-350), outward f32
  rounding and no `covering.bbox` preserved by reference, E6/E7 real-reader
  smoke-or-document-gap notes survive (l.399-403, 421-424).
- **Deferral-doc completeness**: SOUND. Hard cut to HFX v0.2.1 + full deferral
  ledger present and gate-checked.
- **Doctrine**: SOUND. Per-step version bump; W5 forbids `pyshed-v*`.
- **Rename over-reach**: NONE. W5 is scoped to thin bindings only, not the
  deferred pyshed redesign. (It under-reaches — see M1.)

---

## VERDICT: SEND BACK

One BLOCKER (the atom gate cannot go green — its `docs/hfx-v02-redesign` scope
scans the planning docs that necessarily contain "atom"; reproduced `exit=1`
with 22 survivors, 16 in `m5-step-plan.md` itself; `milestone-plan.md:396`
inherits it) plus two MAJORs (the pyshed `.pyi` public-stub leak that no gate
scans; the prose-level allowlist that masks real leaks) mean executors cannot
close M5 against the gate as written, and a half-done rename can pass. The
architecture — sequencing, frozen-artifact handling, export-contract fidelity,
deferral ledger, and doctrine — is sound; the defects are gate validity and
rename completeness, all mechanically fixable. Resubmit with: (1) gate split so
the hard no-leak check runs over live code only and the docs check excludes
planning/critique prose (propagated to `milestone-plan.md:396`), proven
`exit=0`; (2) `.pyi`/`__init__` added to W5 scope and gate; (3) identifier-anchored
allowlist plus a planted-leak negative test.

---

# Re-review — Round 2 (revised plan)

Re-verified the revised `m5-step-plan.md` and the propagated `milestone-plan.md`
M5 gate against the same ground truth, re-running every gate command.

## Round-1 dispositions

- **[BLOCKER B1] never-green gate** — *Resolved.* W7 (`m5-step-plan.md:286-324`)
  now splits the gate: the hard `test -z` no-leak checks scope to **live
  surfaces only** — `crates/core/src` + README, `src`, and `crates/python`
  (Rust + `.pyi` + `API.md`/`README.md`) — and explicitly exclude
  `docs/hfx-v02-redesign/*-plan.md`/`*-critique.md`, which get a presence check
  instead (`:310-312`, `:319`). Simulated post-rename: after the W3/W6 target
  lines (`error.rs:350`, `refine.rs:1/18/39/122`, `README.md:186/187/191/230`)
  are cleaned, the gate-1 filter returns **empty** — the gate goes green. The
  identical split is propagated to `milestone-plan.md` (verified the M5 gate
  block and explanatory note). B1 is fixed.

- **[MAJOR M1] `.pyi` public-stub leak** — *Resolved.* W5 files-touched
  (`:205-214`) now includes `crates/python/python/pyshed/__init__.pyi`,
  `crates/python/API.md`, `crates/python/README.md`, the asserting pyshed tests,
  and any `__init__.py`. W5's gate (`:228`) greps all of `crates/python`, and the
  catch-all closure gate (`:529`) `rg 'terminal_atom_id|upstream_atom_ids|
  upstream_atom_count' src crates/python` now catches the four `.pyi`
  declarations **and** the runtime tests (`test_behavioral.py:42-43,138,141`,
  `test_phase_d.py:160-281`) — all of which are in W5 scope. The stub can no
  longer go stale undetected.

- **[MAJOR M2] prose-token masking** — *Resolved.* The allowlist is now
  identifier-anchored (`\bAtomic[A-Za-z0-9_]*|\batomic\b`); the broad
  `migration|critique|historical|v0.1` tokens are gone, and W7 explicitly forbids
  re-adding them (`:299-301`). Masking regression reproduced fixed: the planted
  line `the migration still says terminal atom` now **survives** the filter
  (previously masked). A planted-leak negative proof is baked into W7 (`:306-307`,
  `:323`) and I confirmed `terminal atom leak` survives the filter → gate fails on
  a real leak as intended.

- **[MINOR m1] case-sensitivity** — *Moot.* Prose tokens removed.
- **[MINOR m2] conventions reminder** — *Resolved.* Phase C now states the
  `tracing`/`thiserror`/no-`unwrap`/type-driven `CLAUDE.md` requirements
  (`:336-339`).

## New finding

### n1. [MINOR] Double-backslash `\\b` in the fenced gate commands neuters the regex if copy-pasted verbatim

The gate commands render the word boundary as `'\\b[Aa]tom'`. Under single
quotes that is the literal two-char sequence `\b`, which `rg` reads as an escaped
backslash + literal `b`, matching nothing:

```
$ rg -n '\\b[Aa]tom' crates/core/src/algo/refine.rs   # → empty (no match)
$ rg -n '\b[Aa]tom'  crates/core/src/algo/refine.rs   # → matches the atom comments
```

A literal copy-paste therefore makes the no-leak `test -z` trivially pass. This
is *not* introduced by the revision — it is the established campaign convention
(`\\b[Aa]tom` appears 8× in `m5-step-plan.md`, 2× in `milestone-plan.md`), and
M1-M4 executed against the same convention without a silent-green failure, so
executors evidently normalize `\\b`→`\b` in practice. Low risk, but recommend
the executor run the single-backslash form (or `rg -e '\b[Aa]tom'`) and, ideally,
normalize the docs to single-backslash so the gate cannot silently no-op.

## Axis recheck

All seven attack surfaces now clear: sequencing (sound, unchanged), rename
completeness (`.pyi`/docs/tests covered), grep-gate soundness (live-scope,
identifier-anchored, planted-leak-proven, goes green post-rename), frozen
artifacts (refine.rs comment-only + parity reruns), export contract not
regressed, deferral-ledger complete, doctrine + Phase-C conventions enforced.

## VERDICT: DISPATCH WITH MINORS

Down from 1 BLOCKER + 2 MAJOR to **0 BLOCKER + 0 MAJOR**. The gate is now
demonstrably runnable, goes green only after a complete rename, fails on a real
leak, and no longer masks leaks or scans its own planning prose. The sole
remaining item (n1) is a copy-paste hazard in a long-standing doc convention,
fixable by the executor at run time. Executors may proceed; no re-review needed.

---

# Re-review — Round 3 (final)

Verified the n1 fix in both plan files.

- **[MINOR n1] doubled `\\b`** — *Resolved.* All grep gates in `m5-step-plan.md`
  (W3/W6 local gates, W7 live-surface gates `:289-291`, planted-leak proof
  `:306-307`, full closure gate `:320-323`) and the propagated
  `milestone-plan.md` M5 gate now use copy-paste-safe single-backslash `\b`.
  Confirmed: zero doubled `\\b` remain in either file's regexes (the only
  remaining `\\.` are correct literal-dot escapes in the docs presence check);
  the verbatim gate-1 is live (9 pre-rename hits → fails until W3/W6 clean them);
  the planted-leak proof still passes; and an `AtomicU64` line is still correctly
  filtered (no false positive). Both files carry the identical regex form.

No regressions from the round-2 state — the only change was the backslash
normalization, and every previously-cleared surface remains clear.

## VERDICT: DISPATCH AS-IS

0 BLOCKER, 0 MAJOR, 0 MINOR. From the round-1 baseline of 1 BLOCKER + 2 MAJOR,
all defects are fixed with verified gates: the atom gate is live-scoped,
identifier-anchored, goes green only after a complete rename, fails on real
leaks, masks nothing, and never scans its own planning prose; the `.pyi`/docs/
tests are in W5 scope and gate-covered; sequencing, frozen-artifact handling,
export-contract fidelity, deferral ledger, and doctrine are sound. The plan is
sound to execute.
