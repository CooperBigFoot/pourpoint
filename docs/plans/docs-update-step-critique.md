# Adversarial Critique — Documentation Correctness Step Plan

Date: 2026-06-07
Critic: independent re-verification against live code (read + read-only shell). Trusts nothing the plan asserts.
Target: `docs/plans/docs-update-step-plan.md`

## Verdict

**DISPATCH WITH MINORS.**

The plan is strong: every doc surface is inventoried, the high-risk fixes are verified
against real code (not asserted), the examples-run gate is genuinely enforced, the
performance section is honest, and the per-step version discipline is correct — including
the workspace-vs-pyshed split that is a common failure mode. It is not DISPATCH-AS-IS
because two substantive accuracy gaps and one minor gap survive into the gates. None rises
to SEND BACK: no entire surface is missed, no perf claim is aspirational, no version rule
is wrong, and the examples gate does not allow inspection-only passes. The blockers below
are bounded and precisely fixable.

---

## What I independently re-verified (and the plan got right)

| Claim | Verified ground truth | Status |
|---|---|---|
| `terminal_unit_id`, not `terminal_atom_id` | `crates/python/src/result.rs:64`, `crates/core/src/engine.rs:123,213`; `.pyi:50,105,140,156,190`; no `terminal_atom_id` anywhere | ✅ correct |
| README `terminal_atom_id` bug | `README.md:31` literally `print(result.terminal_atom_id)` | ✅ real defect |
| macOS cache default | `crates/core/src/cache.rs:99` uses `dirs::cache_dir().join("hfx")` → macOS `~/Library/Caches/hfx`, not `~/.cache/hfx` | ✅ correct |
| `compose_result` omits `units` in README | core `compose_result` has `units: &PreMergeDrainageUnits` at `crates/core/src/engine.rs:856`; `crates/core/README.md:67-73` omits it | ✅ real defect |
| `graph.arrow` → `graph.parquet` | cache writes `graph.parquet` (`cache.rs:186`); stale `graph.arrow` confirmed at `README.md:8,43,56`, `AGENTS.md:7`, `crates/core/README.md:97`, `crates/python/README.md:58` | ✅ correct |
| gdal crate has no README, 7 files / 1045 lines | `ls crates/gdal` (only `Cargo.toml src tests`); `wc -l crates/gdal/src/*.rs` = 1045, `raster_reader.rs` 545 | ✅ correct; README justified |
| Canonical datasets `grit/2.0.0` (0.2.1, no D8) / `merit/0.2.0` (60 Pfaf D8 tiles) | memory `r2-canonical-hfx-datasets`; matches HFX 0.2.1 shape | ✅ correct |
| `bench_trace` absent from `.pyi` | AST check: `missing_from_pyi: ['bench_trace']`; runtime defines it `__init__.py:117`, in `__all__:150` | ✅ caught |
| benchmark alias is stale in code | `crates/core/src/bin/bench_delineate.rs:17` `R2_DATASET = ".../grit/1.0.0/"` | ✅ correct; open question legit |
| bench sets/restores `HFX_CACHE_DIR` | `bench_delineate.rs:547,557,569`; cache root defaults to `env::temp_dir().join("shed-bench-cache")` (`:52-55`) | ✅ correct |
| `--dataset local` requires `--features test-fixtures` | `bench_delineate.rs:351,367` | ✅ gate is runnable |
| CHANGELOG / investigations / fixtures = historical, do-not-rewrite | inventory + ledger row `CHANGELOG.md:58` "leave byte-for-byte" | ✅ correct |
| Version discipline split | Steps 1/2/4 workspace bump+tag; Step 3 pyshed exemption no bump/no tag; "do not mix surfaces" | ✅ correct |
| clippy gate scope | CI runs `cargo clippy --workspace -- -D warnings` (`.github/workflows/ci.yml:58`); build/test use `--exclude pyshed` + `cargo check -p pyshed` (`:87,90`). Plan's gate set matches CI exactly. | ✅ correct (no scoping bug) |
| Performance honesty | cold≈2min/warm≈10s/local≈10s/80ms·10ms, cache on-remote off-local, `HFX_CACHE_DIR`, macOS path, "first open is the tax", explicit "no claim R2 global first-open is fast" | ✅ honest, no "fast"/"instant" |

The `refine` default is `True` → `RefinementMode::BestEffort` (`crates/python/src/engine.rs:164`,
`crates/core/src/staged.rs` `From<bool>`); `BestEffort` with no D8 aux returns
`Ok(TerminalRefinement::best_effort_no_d8_aux_declared())` (`engine.rs:741-742`) — **no
error**. `AmbiguousD8Coverage` is raised only when `select_d8_raster_for_bbox` finds >1
overlapping tile (`crates/core/src/session.rs:717`). This is the crux of MUST-FIX 2 below.

---

## MUST-FIX-BEFORE-RUN

### 1. API.md exports list also omits `LevelSelection`, not just `bench_trace` — and no gate catches it

The plan's ledger row for `crates/python/API.md:10-30` says "Public exports omit `bench_trace`
… add `bench_trace`." But the enumerated "exports these names" bullet list (`API.md:10-30`)
is missing **two** names relative to `__all__`:

- `bench_trace` (plan caught this)
- `LevelSelection` (**plan missed this**)

Evidence: `__all__` = `[..., 'LevelSelection', ..., 'bench_trace', 'set_log_level']`
(AST dump). API.md's bullet list ends `… set_log_level / __version__` with **no
`LevelSelection`** entry, even though `LevelSelection.FINEST` is used in API.md examples at
`:183,203,211`. So the top-of-file export enumeration is inaccurate today and would remain
inaccurate after the plan's fix.

Worse, the gates only diff `.pyi` against `__all__` (Step 3 / Step 5 AST check). **No gate
diffs API.md's enumerated exports against `__all__`,** so this drift is invisible to the
plan's own verification.

Required:
- Add `LevelSelection` (and `bench_trace`) to the `API.md:10-30` exports list.
- Add a gate that asserts the API.md exports enumeration ⊇ `__all__` (a simple
  `rg`-per-name check, or extend the AST script to parse the API.md bullet list).

### 2. The plan over-prescribes `refine=False`; this would teach users to disable the headline feature and rests on a misread of the engine

The plan repeatedly frames `refine=False` as the workaround needed so examples run — for
GRIT and for the local synthetic fixture, not just MERIT (e.g. ledger row `README.md:66-91`
"should pass `refine=False` or use GRIT because GRIT has no D8 raster"; Step 1 verification
command uses `refine=False` on the synthetic fixture; Step 3 dataset-choice "Public
examples: grit/2.0.0 with `refine=False`"; milestone "Examples run" gate).

Verified reality:
- **GRIT `grit/2.0.0` (no D8 aux):** default `refine=True` → `BestEffort` → returns
  `best_effort_no_d8_aux_declared()` with no error (`engine.rs:741-742`). `refine=False` is
  **not** required; the result is identical (refinement skipped).
- **Local synthetic fixture `v021_synthetic_refined`:** its `manifest.json` declares exactly
  **one** `hfx.aux.d8_raster.v1` (`flow_dir.tif`/`flow_acc.tif`). `has_d8_aux()` is true and
  selection is unambiguous (single tile), so default `refine=True` **actually refines** —
  there is no ambiguity to avoid. The plan's Step 1 command at `:123` disables refinement on
  the one fixture whose name and purpose is to exercise refinement.
- **MERIT `merit/0.2.0` only:** 60 overlapping Pfaf D8 tiles → `select_d8_raster_for_bbox`
  returns >1 → `AmbiguousD8Coverage` (`session.rs:717`). **This is the only place
  `refine=False` is genuinely required** (clog #63).

Consequence if implemented literally: the published quickstart/examples show
`pyshed.Engine(url, refine=False)` as canonical, actively teaching users to turn off the
default terminal refinement — a doc-correctness regression introduced by the plan, premised
on "GRIT has no D8 raster" (true, but that means default refine is *safe*, not that it must
be disabled).

Required:
- Reserve `refine=False` strictly for `merit/0.2.0` examples (documenting
  `AmbiguousD8Coverage` / clog #63).
- Root README quickstart (`README.md:24-33`) and GRIT examples should use the **default**
  (`refine=True`) — they run as-is (refinement skipped on GRIT, performed on the
  single-raster local fixture). State plainly that GRIT carries no D8 raster so refinement
  is a no-op there.
- Keep the per-example dataset naming the plan already requires, but correct the rationale
  so the docs do not imply refinement must be disabled.

---

## MINOR (fix or explicitly waive)

### 3. `crates/core/src/cache.rs:95` doc comment is itself Linux-centric/stale

The doc comment reads "Return the configured cache rooted at `HFX_CACHE_DIR` or
`~/.cache/hfx`." The plan classifies `crates/core/src/**/*.rs` doc comments as LIVING and
corrects this exact `~/.cache/hfx` Linux-ism in every user-facing doc, but the fix ledger
has **no row** for the source doc comment that is the origin of the inaccuracy. Either add a
ledger row (Step 2) to reword it to match `dirs::cache_dir()` reality (macOS
`~/Library/Caches/hfx`), or explicitly note it is intentionally left.

### 4. Step 1 example coordinates are asserted, not yet confirmed

The Step 1 verification command uses `lat=-2.5, lon=2.5` against `v021_synthetic_refined`.
The fixture bbox is `[0.0, -5.0, 5.0, 0.0]`, so the point is inside — plausible — but the
plan asserts a successful delineation without having run it. This is acceptable because the
examples-run gate forces execution with captured output; just flag that the implementer
must confirm the coordinate resolves (not `NoOutletFound`) and adjust if needed.

### 5. Prerequisite not stated: pyshed must be importable

Every Python example gate (`python3 - <<'PY' import pyshed`) requires a built/installed
`pyshed` (`maturin develop` or the published wheel). The plan never states this prerequisite
for the examples-run gate. Add a one-line setup note so the gate is reproducible.

---

## Axis-by-axis

1. **Completeness** — PASS with gaps. All required surfaces inventoried and classified
   correctly: root `README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `SECURITY.md`,
   `crates/core/README.md`, the gdal-README decision (justified against the complex-crate
   bar: 1045 lines / 7 files / native GDAL+GEOS bridge), core/gdal doc comments,
   `crates/python/README.md`, `API.md`, `.pyi` (bench_trace gap caught), `CHANGELOG.md`,
   `docs/raster-cache.md`, `docs/telemetry.md`, `docs/benchmarks/delineate-harness.md`,
   `docs/basin-geoparquet-export.md`. Historical surfaces (investigations, hfx-v02-redesign,
   plans, golden-fixture READMEs, test source) correctly marked do-not-rewrite, and the plan
   does not propose rewriting CHANGELOG history. **Gap:** API.md `LevelSelection` omission
   (MUST-FIX 1).

2. **Verified vs asserted** — PASS. Spot-checked the high-risk fixes myself: real
   `compose_result`/`delineate` signatures and `terminal_unit_id` against
   `crates/python/src` + `.pyi`; canonical dataset paths against memory + HFX shape; macOS
   cache default via `dirs::cache_dir()`; `terminal_unit_id` (not `terminal_atom_id`). All
   correct. The one fix that would itself introduce an error is the blanket `refine=False`
   prescription (MUST-FIX 2).

3. **Examples actually run** — PASS. The plan requires every runnable example to execute
   against a named dataset (local synthetic v0.2.1 or `grit/2.0.0`) with captured
   output/timing (Step 1 gate, Step 5 examples-run gate, milestone "Examples run"), and it
   handles clog #63 with `refine=False` for MERIT. It does **not** allow inspection-only
   passes. (The `refine=False` *over-application* is a correctness issue per MUST-FIX 2, but
   the run gate itself is properly enforced.)

4. **Honest performance** — PASS. Uses cold≈2min (R2 grit/2.0.0 ~42GB) / warm≈10s /
   local≈10s / 80ms single / ~10ms batched; states cache ON-remote / OFF-local, names
   `HFX_CACHE_DIR` and the macOS default path, says "first open is the tax," and explicitly
   forbids claiming R2 global first-open is fast. No "fast"/"instant" language. Note: memory
   `r2-open-reuse-complete` records a measured cold of ~178s (~3min) post-optimization; the
   "~2min" anchor (supplied in the task framing) is slightly optimistic against that figure —
   the implementer should cite the measurement source in the published section rather than
   leave it as a bare "about 2 min."

5. **Version discipline** — PASS. Workspace-doc steps (1, 2, 4) require
   `./scripts/bump-version.sh patch` + stage `Cargo.toml`/`Cargo.lock` + conventional commit
   + `v<version>` tag; the pyshed-only step (3) correctly takes the exemption (no workspace
   bump, no pyshed bump, no release, no tag); the plan forbids mixing the two surfaces in one
   commit. Correct.

6. **Sequencing & verifiability** — PASS. Gates are concrete (greps + `cargo
   doc/build/check/clippy` + AST consistency), read-only where they should be, and ordered so
   example-running is feasible (root → core/gdal → python → docs → sweep). Build scoping
   matches CI: `cargo build --workspace --exclude pyshed`, `cargo check -p pyshed`, `cargo
   clippy --workspace`. The only verifiability gap is the missing API.md↔`__all__` gate
   (folded into MUST-FIX 1).

---

## Summary

Dispatch with the two MUST-FIX items resolved first:
1. Add `LevelSelection` (and `bench_trace`) to the API.md exports list **and** add a gate
   diffing API.md's enumeration against `__all__`.
2. Stop prescribing `refine=False` for GRIT and local examples — reserve it for
   `merit/0.2.0`; use the default (`refine=True`) elsewhere and correct the rationale.

Then address the three MINORs (cache.rs doc comment, Step 1 coordinate confirmation, pyshed
import prerequisite). The rest of the plan is sound and verified.
