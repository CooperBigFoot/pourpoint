# M4 Step Plan — Adversarial Critique (Round 3, post-revision)

Target: `docs/hfx-v02-redesign/m4-step-plan.md` (revised twice). Re-reviewed
personally against the same sources (milestone contract, R1–R7 roadmap, fixtures
README, HFX D8 spec, live code: `engine.rs`, `staged.rs`, `session.rs`,
`cog.rs`, `algo/refine.rs`, `algo/traits.rs`, `reader/manifest.rs`). No delegation.

**Verdict: DISPATCH AS-IS.** The plan is sound. All four Round-1 material defects
were fixed in Round 2, and all four Round-2 minors are now folded in correctly.
Proceed to human sign-off, then executors.

---

## Round-2 minors — all resolved

1. **Mode-agnostic strategy boundary — RESOLVED.** Lines 88-92 state
   `TerminalRefinementInput` deliberately omits `RefinementMode`; the engine
   interprets `TerminalRefinementDecision::BestEffortSkipped` as a visible skip
   under `BestEffort` and escalates the same missing-data condition to a hard
   error under `RequireD8`. This matches the provenance model and the existing
   `RefinementMode::Disabled` short-circuit in `engine.rs:647`.

2. **`TerminalRefinementDecision` sketch — RESOLVED.** Lines 64-73 define
   `Applied { refined_outlet, geometry: ContainedTerminalPolygon, provenance }`
   and `BestEffortSkipped { provenance }`. Correctly omits a `Disabled` variant:
   the engine short-circuits `Disabled` before dispatch, so the strategy never
   needs to represent it. Consistent with Step 1's type list and the dissolve
   mapping.

3. **`tiff` test accessibility — RESOLVED.** Lines 328-331 note `tiff = "0.9"`
   is already a core dependency via `cog.rs` and instruct adding a narrow
   dev-dependency or exposing a `#[cfg(test)] pub(crate)` fixture writer, with an
   explicit "do not hand-author binary TIFF bytes." Verified in-repo: `cog.rs`
   already imports `tiff::encoder::TiffEncoder`.

4. **Oversized extent-header behavior — RESOLVED.** Lines 577-578 require the
   network-gated test to treat an "extent header too large" error as a real
   failure/escalation, not a silently skipped declaration — closing the one path
   where a too-small extent range could have masked a real coverage miss.

---

## Standing assessment (carried forward, still PASS)

- **Parity:** the D8 carve in `algo/refine.rs` is untouched; the strategy
  delegates to `refine_terminal_from_source`; the offline gate compares
  `v021_synthetic_refined` to the committed M1 `oracle_b_synthetic_refined.json`
  through the frozen `shed-canonical-wkb-v1` canonicalizer; no clamp/intersection/
  cleaning is added; `ContainedTerminalPolygon` is non-strict.
- **Seam shape:** terminal-only input, contained-geometry output, always-merge-
  after; pantry exposes both `session` and the engine `RasterSource`; trait
  generality is scoped honestly as D8-for-now.
- **Tile selection:** inclusive closed-rectangle coverage (handles the exact
  synthetic extent==bbox case); extent-only bounded header reads; typed
  ambiguity/no-cover/multi-tile errors; single-declaration M4 assumption stated
  and risk-acknowledged.
- **Oracle C vs merit/0.2.0:** readiness is the primary criterion; C-parity is a
  documented bonus; M1 golden never relaxed; divergence escalated.
- **R6 failure visibility:** explicit `RequireD8` hard-errors naming the missing
  aux; convenience `BestEffort` emits visible named skip provenance.
- **Gate integrity:** offline gates are network-free; real merit test is
  `#[ignore]` + env-gated; the multi-declaration accessor test exercises real
  selection offline.
- **Doctrine/non-scope:** per-commit patch bump + tag, `Cargo.lock` staged every
  commit, `cargo check -p pyshed` build hold, thiserror/named-fields/no-strict-
  containment, Mermaid README update; deferred work (general aux binding, Python
  strategies, extra strategies, reverse-DNS parser, level selection) stays out.
- **Step granularity:** Step 2 is additive, Step 3 swaps then retires legacy; no
  step leaves the workspace red; each is a single coherent commit.

No further defects found. Ready to dispatch.
