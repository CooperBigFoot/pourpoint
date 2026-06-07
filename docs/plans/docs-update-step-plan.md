# Documentation Correctness Audit And Step Plan

Date: 2026-06-07

Scope: plan only. Implementers must edit documentation, doc comments, and the PEP 561 stub only. No engine behavior changes, no binding behavior changes, no new features, no pyshed release, no broadening wheel platform claims, and no rewriting historical records.

Verified orientation:

- `CLAUDE.md` requires proportional docs: complex crates get a crate README with Mermaid architecture diagram, glossary, and key types; simple modules get only lean module docs; Mermaid is preferred over ASCII diagrams.
- `CLAUDE.md` version policy says normal workspace commits must run `./scripts/bump-version.sh patch`, stage `Cargo.toml` plus `Cargo.lock` if it changes, commit conventionally, and tag `v<version>`. The `crates/python/` pyshed exemption means pyshed-only doc/stub commits do not bump workspace or pyshed versions and do not create release tags.
- `../hfx/spec/HFX_SPEC.md` says current HFX is `0.2.1`; required root artifacts are `manifest.json`, `catchments.parquet`, and `graph.parquet`; snap and D8 rasters are manifest-declared auxiliaries.
- Local verification commands used for this audit:
  - `rg --files -g '*.md' -g '*.pyi' -g '*.rs' README.md CONTRIBUTING.md AGENTS.md SECURITY.md docs crates/core crates/gdal crates/python`
  - `rg -n "graph\\.arrow|grit/1\\.0\\.0|global/hfx|~/.cache/hfx|Platform support \\(v0\\.2\\.0\\)|LevelSelection\\.FINEST.*0\\.2\\.0|R3|Phase|M[0-9]|v1\\.0\\.0|v0\\.1|atom" README.md AGENTS.md crates/core/README.md crates/python/README.md crates/python/API.md docs/raster-cache.md docs/telemetry.md docs/benchmarks/delineate-harness.md docs/basin-geoparquet-export.md`
  - `rg -n "terminal_atom_id|terminal_unit_id|upstream_unit_ids|geometry_wkb|to_geojson|bench_trace|__all__|set_log_level|AreaOnlyResult|DelineationResult" crates/python/src crates/python/python/pyshed/__init__.py crates/python/python/pyshed/__init__.pyi`
  - `curl -sL https://pypi.org/pypi/pyshed/json | python3 -c 'import json,sys; data=json.load(sys.stdin); print(data["info"]["version"]); [print(f["filename"]) for f in data["releases"][data["info"]["version"]]]'`
  - `wc -l crates/gdal/src/*.rs`
  - `test -f crates/core/tests/fixtures/parity/v021_synthetic_refined/manifest.json && sed -n '1,80p' crates/core/tests/fixtures/parity/v021_synthetic_refined/manifest.json`
  - `python3 - <<'PY' ... compare __all__ in __init__.py to top-level names in __init__.pyi ... PY`
  - `nl -ba crates/python/API.md | sed -n '8,35p'` confirmed the API export list omits both `LevelSelection` and `bench_trace`.
  - `nl -ba crates/core/src/engine.rs | sed -n '730,748p'` confirmed `BestEffort` refinement with no D8 aux returns `best_effort_no_d8_aux_declared()` rather than erroring.

## 1. Doc-Surface Inventory

| Surface | Classification | Rationale |
|---|---|---|
| `README.md` | LIVING | Root user-facing docs and runnable examples must reflect current HFX v0.2.1, pyshed 0.2.x, API names, public datasets, and cache behavior. |
| `CONTRIBUTING.md` | LIVING | Build, platform, and version policy docs. Existing `0.1.0 -> 0.1.1` examples are version-bump examples, not dataset anchors; verify and leave unless misleading. |
| `AGENTS.md` | LIVING | Agent-facing project contract; must align with HFX spec and current artifact names. |
| `SECURITY.md` | LIVING | Current security contact. No correctness defects found in audit. |
| `crates/core/README.md` | LIVING | Complex crate README; already has Mermaid, glossary, and key types, but contains stale `graph.arrow` and milestone-era wording. |
| `crates/gdal/README.md` | LIVING target decision | No README exists. Audit found 7 source files and 1045 lines, with `raster_reader.rs` at 545 lines. This crosses the CLAUDE.md complex-crate bar because it bridges GDAL raster reads, config, conversion, WKB, errors, and GEOS repair. Add a README. |
| `crates/core/src/**/*.rs` doc comments | LIVING | Public/module docs must match HFX v0.2.1 and current API. Most `v0.1` references found are test/source-history comments; audit before changing. |
| `crates/gdal/src/**/*.rs` doc comments | LIVING | Public/module docs must match current GDAL bridge behavior. No stale dataset refs found in initial grep. |
| `crates/python/README.md` | LIVING | PyPI long description/user docs; contains stale platform anchor, placeholder hosted URL, `graph.arrow`, and Linux cache path. |
| `crates/python/API.md` | LIVING | Public Python API reference; must mirror runtime and `.pyi`. Export list is missing `LevelSelection` and `bench_trace`. |
| `crates/python/python/pyshed/__init__.pyi` | LIVING | PEP 561 public surface; missing `bench_trace`. |
| `crates/python/CHANGELOG.md` | HISTORICAL/POINT-IN-TIME | Dated release history. Verify that `grit/1.0.0` references are historical. Do not rewrite unless a current live usage example appears outside a dated entry. |
| `docs/raster-cache.md` | LIVING | Current living docs for raster cache behavior; contains R3/Phase 4 wording and should be reframed to current behavior. |
| `docs/telemetry.md` | LIVING | Current telemetry contract; contains Phase A/B wording but otherwise current stage names. Reword phase label if keeping as living docs. |
| `docs/benchmarks/delineate-harness.md` | LIVING | Current benchmark harness docs; contains `grit/1.0.0` and Linux cache fallback wording. |
| `docs/basin-geoparquet-export.md` | LIVING | Current export format docs. `M5` wording should be audited as milestone-era; schema appears current from `crates/core/src/export/schema.rs`. |
| `docs/investigations/*` | HISTORICAL/POINT-IN-TIME | Investigation records. Do not rewrite; stale dataset names are legitimate context. |
| `docs/hfx-v02-redesign/*` | HISTORICAL/POINT-IN-TIME | Milestone trail and critiques. Do not rewrite. |
| `docs/plans/*` existing files | HISTORICAL/POINT-IN-TIME | Existing PCE trail, milestone-plan, roadmap, and prior step plans. Do not rewrite. This new file is the only plan-file write for this audit. |
| Gitignored `docs/plans/pyshed-0.2.0-*` if present | HISTORICAL/POINT-IN-TIME | Explicitly leave byte-for-byte. `find docs/plans -maxdepth 1 -type f -name 'pyshed-0.2.0-*'` found tracked examples; same rule applies to ignored siblings. |
| Golden fixture READMEs under `crates/core/tests/fixtures/**` | HISTORICAL/POINT-IN-TIME | They document v0.1 oracles/parity fixtures and may legitimately name `grit/1.0.0`. Do not rewrite. |
| Parity/bench test source | HISTORICAL/POINT-IN-TIME for audit text | Test source may intentionally encode oracle names or benchmark aliases. Do not rewrite as documentation in this milestone unless a doc comment is user-facing and currently wrong. |

## 2. Fix Ledger

Every row below must be implemented with exact wording reviewed in context. "Corrected text" is the required substance; maintainers may make small grammar changes if the verification remains true.

| File:line(s) | Current text | Corrected text | Verified ground truth |
|---|---|---|---|
| `README.md:8-9` | `manifest.json`, `catchments.parquet`, `graph.arrow`, optional root `snap.parquet`, `flow_dir.tif`, `flow_acc.tif` | `manifest.json`, `catchments.parquet`, `graph.parquet`; optional snap and D8 raster artifacts are declared in `manifest.json` auxiliaries. | `../hfx/spec/HFX_SPEC.md` artifact summary; `crates/core/src/reader/manifest.rs:4-8` says snap/raster data is expressed through `auxiliary[]`. |
| `README.md:21` | `pip install pyshed   # macOS arm64 only in v0.1` | `pip install pyshed` plus nearby platform note: current PyPI wheels are Apple Silicon macOS only (`macosx_11_0_arm64`). | PyPI JSON query returned version `0.2.2` and file `pyshed-0.2.2-cp39-abi3-macosx_11_0_arm64.whl`; local `crates/python/pyproject.toml:7` is `0.2.2`. |
| `README.md:31` | `print(result.terminal_atom_id)` | `print(result.terminal_unit_id)` | `crates/python/src/result.rs:62-65`; `.pyi` has `DelineationResult.terminal_unit_id` at `crates/python/python/pyshed/__init__.pyi:48-50`; no `terminal_atom_id` in runtime grep. |
| `README.md:43-44` | Root must contain `graph.arrow` and optional root auxiliaries. | Root must contain HFX v0.2.1 core artifacts: `manifest.json`, `catchments.parquet`, `graph.parquet`; auxiliaries are manifest-declared. | HFX spec artifact summary; `crates/core/src/reader/manifest.rs:22-25` supports only `0.2.1` and `EPSG:4326`. |
| `README.md:54` | Public URL `.../grit/1.0.0/` | `https://basin-delineations-public.upstream.tech/grit/2.0.0/` | User-provided canonical dataset list reverified against HFX spec shape; benchmark code still has stale const at `crates/core/src/bin/bench_delineate.rs:17`, so docs must not rely on that alias until code is fixed elsewhere. |
| `README.md:56-59` | Caches `manifest.json` and `graph.arrow` under `~/.cache/hfx/...`; Parquet artifacts only range-read. | Say remote metadata/artifact validation cache root comes from `HFX_CACHE_DIR` or OS cache dir via `dirs::cache_dir()`; on macOS default is `~/Library/Caches/hfx`, on Linux typically `~/.cache/hfx`; Parquet row-group cache is enabled by default for remote Python engines and disabled for local paths. | `crates/core/src/cache.rs:95-101` uses `dirs::cache_dir().join("hfx")`; `crates/python/src/engine.rs:49-50` enables parquet cache for remote paths by default. |
| `README.md:66-91` | Canonical GRIT HFX v1.0.0 and examples using `grit/1.0.0`. | Canonical examples should use GRIT HFX v0.2.1 fabric `grit/2.0.0` with the default `refine=True`. State that GRIT has no D8 raster, so best-effort refinement safely skips with a `best_effort_no_d8_aux_declared` outcome. Reserve `refine=False` for MERIT `merit/0.2.0` examples affected by overlapping Pfaf D8 coverage. | HFX spec version `0.2.1`; local manifest fixture is `format_version":"0.2.1"` and declares one D8 aux; `crates/core/src/engine.rs:739-742` shows `BestEffort` with no D8 aux skips without error; `clog search "AmbiguousD8Coverage"` found the 2026-06-04/05 MERIT ambiguity records; `crates/core/README.md:164-169` documents MERIT ambiguity. |
| `README.md:new section` | No honest cold/warm performance section. | Add Performance and caching section: cold first open of global GRIT over R2 is about 2 min for about 42 GB global data and network round trips, but cite the measurement source and use the measured value if local re-runs differ; warm repeat open with validation sidecar is about 10 s; local large dataset open about 10 s; per-delineation about 80 ms single and about 10 ms batched; first open is the tax. Include `HFX_CACHE_DIR`, macOS default cache path, remote/local parquet-cache defaults, and no claim that R2 global first-open is fast. | User-supplied measured perf; critique notes prior cold measurements may be closer to ~178 s, so published docs should cite the run source; `crates/core/src/cache.rs:129-135` validation sidecar path; `crates/python/src/engine.rs:49-50` parquet cache defaults. |
| `AGENTS.md:7` | `graph.arrow` and optional root artifacts. | `graph.parquet`; optional snap and D8 raster artifacts are declared by `manifest.json` auxiliaries. | `../hfx/spec/HFX_SPEC.md`; `crates/core/src/reader/manifest.rs:4-8`. |
| `crates/core/README.md:11` | M3 step-planning phrasing. | Current contract phrasing: staged delineation is implemented and returns typed intermediates around the stable `Engine::delineate` surface. | `crates/python/src/engine.rs:227-346` exposes staged methods; `crates/core/src/engine.rs` has the corresponding methods. |
| `crates/core/README.md:67-73` | `compose_result` signature omits `units: &PreMergeDrainageUnits`. | Update signature to include `units: &PreMergeDrainageUnits` between upstream and refinement. | `crates/python/src/engine.rs:338-345` calls `compose_result(outlet, upstream, &units.inner, refinement, dissolved)`; `.pyi:316-323` includes `units`. |
| `crates/core/README.md:97` | `decode graph.arrow` | `decode graph.parquet` | HFX spec artifact summary; repo tests include `crates/core/tests/graph_parquet_reader.rs`. |
| `crates/core/README.md:141-190,220` | M4/R3 milestone labels. | Keep technical limitation but reword as current behavior: built-in D8 is the only terminal-refinement strategy; MERIT `merit/0.2.0` overlapping Pfaf D8 tiles can raise `AmbiguousD8Coverage`; use `refine=False` only for runnable MERIT examples affected by that ambiguity. | `crates/core/README.md:164-169`; `clog search "AmbiguousD8Coverage" --format short`; `crates/core/src/session.rs:717` raises `AmbiguousD8Coverage`. |
| `crates/core/src/cache.rs:95` | `Return the configured cache rooted at HFX_CACHE_DIR or ~/.cache/hfx.` | Reword to say the cache is rooted at `HFX_CACHE_DIR` or the platform cache directory from `dirs::cache_dir()` joined with `hfx`; do not imply `~/.cache/hfx` on macOS. | `crates/core/src/cache.rs:97-101` uses `dirs::cache_dir().map(|path| path.join("hfx"))`; macOS default is `~/Library/Caches/hfx`. |
| `crates/gdal/README.md:new file` | No README. | Add concise crate README with Purpose, Mermaid architecture diagram, Glossary, Key types. Diagram should show `shed_core::RasterSource` / `GeometryRepair` traits -> `GdalRasterSource` / `GdalGeometryRepair` -> GDAL dataset/window reads, config, conversion, WKB. Key types: `GdalRasterSource`, `GdalGeometryRepair`, `GdalConfig`, `GdalError`, `gdal_to_geo_transform`, `decode_wkb_multipolygon`. | `wc -l crates/gdal/src/*.rs` returned 1045 total across 7 files; source list verified with `find crates/gdal/src -type f -name '*.rs' -maxdepth 1 -print`. |
| `crates/python/README.md:5-6` | HFX v0.1 datasets no longer load. | Keep because it is current behavior, but make it explicit: only HFX v0.2.1 loads; v0.1 hard-errors as unsupported. | `crates/core/src/reader/manifest.rs:4-6,22-25`; `crates/core/src/error.rs` has `UnsupportedFormatVersion`. |
| `crates/python/README.md:16` | `Platform support (v0.2.0)` | Drop version anchor: `Platform support: Apple Silicon macOS only ...` | PyPI JSON query returned only `pyshed-0.2.2-cp39-abi3-macosx_11_0_arm64.whl`. |
| `crates/python/README.md:53-55,77,92-94,111` | Public URL `https://basin-delineations-public.upstream.tech/global/hfx` | Use `https://basin-delineations-public.upstream.tech/grit/2.0.0/` for runnable examples; use `merit/0.2.0/` only in D8-specific prose with `refine=False` note. | User-supplied canonical datasets; HFX spec v0.2.1; local grep found no code support proving `global/hfx`. |
| `crates/python/README.md:58-62` | `graph.arrow` and `~/.cache/hfx`. | `graph.parquet`; cache root is `HFX_CACHE_DIR` or OS cache dir (`~/Library/Caches/hfx` on macOS, usually `~/.cache/hfx` on Linux). Note persistent validation/cache sidecar vs in-memory Parquet row-group cache. | `../hfx/spec/HFX_SPEC.md`; `crates/core/src/cache.rs:95-101`; `crates/python/src/engine.rs:49-50`. |
| `crates/python/README.md:99-101` | In-memory Parquet cache is per-engine and not persisted to disk. | Keep for `parquet_cache`, but distinguish from persistent remote artifact/validation cache under `HFX_CACHE_DIR`. | `crates/python/src/engine.rs:196-200` creates per-engine row-group/footer caches; `crates/core/src/cache.rs` handles persistent remote artifact cache. |
| `crates/python/README.md:149-150` | `LevelSelection.FINEST` only level selection in `0.2.0`. | Drop stale anchor: only `LevelSelection.FINEST` is currently supported; multi-level selection is planned. | `crates/python/src/staged.rs:11-18` only defines `FINEST`; `.pyi:42-46`. |
| `crates/python/README.md:157-159` | `R3 note`. | Reword without milestone label: pre-merge units are whole source drainage units and differ from final merged/refined output. | `crates/python/src/staged.rs:197-201` class attr text; current API exposes `R3_NOTE`, but user-facing prose need not lead with milestone label. |
| `crates/python/API.md:10-30` | Public exports omit `LevelSelection` and `bench_trace`. | Add both `LevelSelection` and `bench_trace` to public exports. Add a short `bench_trace(path: os.PathLike[str] | str) -> Iterator[None]` section describing the context manager that writes Rust stage-span benchmark telemetry while active. | `crates/python/python/pyshed/__init__.py:116-128,131-151`; `nl -ba crates/python/API.md | sed -n '8,35p'` shows the omissions; AST export/stub check reported `missing_from_pyi: ['bench_trace']`. |
| `crates/python/API.md:76-77` | HFX v0.1 not accepted. | Keep, strengthen to hard-error unsupported version if needed. | `crates/core/src/reader/manifest.rs:4-6,22-25`. |
| `crates/python/API.md:211-212` | Only valid selection in `0.2.0`. | Drop stale version anchor. | `crates/python/src/staged.rs:11-18`. |
| `crates/python/API.md:214-278` | R3 labels in API docs. | Keep `R3_NOTE` property because it is a runtime attribute, but explanatory prose should not read like a milestone plan. | `.pyi:186-201`; `crates/python/src/staged.rs:197-201`. |
| `crates/python/python/pyshed/__init__.pyi:1-10` | No `bench_trace` declaration. | Import `Iterator` and `os.PathLike` typing as needed; add `def bench_trace(path: PathLike[str] | str) -> context manager/Iterator[None]` with a type that matches `@contextmanager` facade. | Runtime `bench_trace(path: os.PathLike[str] | str) -> Iterator[None]` at `crates/python/python/pyshed/__init__.py:116-128`; `__all__` includes it at lines `131-151`; AST check found this as the only missing export. |
| `docs/raster-cache.md:7,9,21` | R3 / Phase 4 wording. | Reword as current behavior. Add MERIT caveat: examples using real MERIT D8 refinement can hit `AmbiguousD8Coverage`; use `refine=False` for MERIT runnable examples. Do not imply GRIT or single-raster local fixtures require refinement to be disabled. | `crates/core/README.md:164-169`; `crates/core/src/engine.rs:739-742`; `clog search "AmbiguousD8Coverage" --format short`. |
| `docs/telemetry.md:3` | `Phase A/B telemetry`. | Reword as current benchmark telemetry contract. | Stage names verified in `crates/core/src/telemetry/mod.rs` via grep output; docs table matches known stage names. |
| `docs/benchmarks/delineate-harness.md:17` | Never falls back to user `~/.cache/hfx`. | Say harness uses a run-specific `HFX_CACHE_DIR` child and does not use the user's normal OS cache dir. Avoid Linux-only path as generic default. | `crates/core/src/bin/bench_delineate.rs:547-569` sets/restores `HFX_CACHE_DIR`; `crates/core/src/cache.rs:95-101` OS cache default. |
| `docs/benchmarks/delineate-harness.md:41-43` | `--dataset r2` expands to `grit/1.0.0`. | Update documentation to `grit/2.0.0` only after code constant is fixed in a separate code-change milestone, or explicitly mark current code alias stale and add a task. Because this milestone is docs-only, do not claim the harness alias expands to `grit/2.0.0` until `crates/core/src/bin/bench_delineate.rs:17` changes. | `crates/core/src/bin/bench_delineate.rs:17` currently proves docs and code are stale together. This is an open docs-only constraint issue. |
| `docs/basin-geoparquet-export.md:120-125` | Minimal example includes `merit/2024.1/d8-carved`. | Prefer current fabric examples using the actual default method semantics, such as `grit/2.0.0/d8-best-effort` when refinement remains enabled and best-effort skips for no D8 aux; use `grit/2.0.0/no-refine` only for an explicitly disabled-refinement example. If keeping MERIT, use `merit/0.2.0` and note D8 ambiguity for overlapping Pfaf tiles. | User-supplied canonical datasets; export method labels verified from `crates/core/src/export/identity.rs` and `crates/core/src/export/schema.rs`; `crates/core/src/engine.rs:739-742` confirms default best-effort skip for no D8 aux. |
| `docs/basin-geoparquet-export.md:146` | `M5 does not add...` | Reword as current CLI status without milestone label. | CLI status should be verified with `cargo run --bin shed -- delineate --help` or source inspection before edit. |
| `crates/python/CHANGELOG.md:58` | Dated `grit/1.0.0` in version `0.1.8`. | Leave byte-for-byte; classify as historical release note. | Line is under `## [0.1.8] - 2026-04-22`, verified by `rg -n`. |
| `CONTRIBUTING.md:67-87` | Pyshed version-bump examples use `0.1.0`. | Leave unless maintainers prefer neutral placeholders; these are script examples, not stale current-version anchors. | `CLAUDE.md` quick reference uses the same illustrative version style; not a dataset or current platform claim. |
| `SECURITY.md` | No stale correctness issue found. | No edit. | `nl -ba SECURITY.md` shows only current contact/response text. |

## 3. Step Sequence

### Step 1: Root And Agent Contract Docs

Scope:

- `README.md`
- `AGENTS.md`

Work:

- Replace `graph.arrow` with `graph.parquet` and describe auxiliaries as manifest-declared.
- Replace all root README `grit/1.0.0` / `global GRIT HFX v1.0.0` references with `grit/2.0.0`.
- Fix `terminal_atom_id` to `terminal_unit_id`.
- Remove the stale `v0.1` platform anchor.
- Fix cache-path prose to use OS cache dir and `HFX_CACHE_DIR`, with macOS default `~/Library/Caches/hfx`.
- Add honest Performance/caching section with the measured cold/warm/local/per-delineation numbers.
- Make root examples runnable against `grit/2.0.0` with the default `refine=True`; explain that GRIT has no D8 raster so best-effort refinement skips safely. Use `refine=False` only for MERIT examples that would otherwise hit overlapping-Pfaf D8 ambiguity.

Exact gates:

- `rg -n "graph\\.arrow|grit/1\\.0\\.0|v1\\.0\\.0|terminal_atom_id|\\batom\\b|macOS arm64 only in v0\\.1|~/.cache/hfx" README.md AGENTS.md` returns no user-facing stale hits, except illustrative version-bump examples if any.
- Run root Python examples with `pyshed` importable, either from `maturin develop` or an installed wheel, against one of:
  - fast local: `crates/core/tests/fixtures/parity/v021_synthetic_refined/manifest.json` parent directory with default refinement enabled; confirm the chosen coordinate resolves and adjust if needed, or
  - public remote: `https://basin-delineations-public.upstream.tech/grit/2.0.0/` with default refinement enabled.
- Capture actual output and timing for the examples in the implementation notes or PR description.

Verification command:

```bash
rg -n "graph\\.arrow|grit/1\\.0\\.0|v1\\.0\\.0|terminal_atom_id|\\batom\\b|macOS arm64 only in v0\\.1|~/.cache/hfx" README.md AGENTS.md
python3 - <<'PY'
import pyshed
engine = pyshed.Engine("crates/core/tests/fixtures/parity/v021_synthetic_refined")
result = engine.delineate(lat=-2.5, lon=2.5)
print(result.terminal_unit_id, round(result.area_km2, 6))
PY
```

Dataset choice:

- Prefer local synthetic v0.2.1 for fast runnable examples if coordinates are confirmed by an actual run. It has a single D8 aux declaration, so default refinement should exercise the feature rather than disable it.
- Use GRIT `grit/2.0.0` for public URL examples with the default `refine=True`; GRIT has no D8 raster, so best-effort refinement skips safely.

Version discipline:

- Workspace-doc commit. Run `./scripts/bump-version.sh patch`; stage `Cargo.toml` and `Cargo.lock` if changed; conventional commit; tag `v<version>`.

### Step 2: Core And GDAL Crate Documentation

Scope:

- `crates/core/README.md`
- `crates/core/src/**/*.rs` doc comments only where audit proves stale user/agent-facing docs, including `crates/core/src/cache.rs:95`
- `crates/gdal/README.md` new file
- `crates/gdal/src/**/*.rs` doc comments only if stale docs are found

Work:

- Update `crates/core/README.md` architecture diagram from `graph.arrow` to `graph.parquet`.
- Update staged API signature examples, especially `compose_result(..., units, refinement, dissolved)`.
- Remove milestone-era wording where it reads as current docs (`M3`, `M4`, `R3`) while preserving the actual technical concepts.
- Keep and clarify MERIT `AmbiguousD8Coverage` limitation; only MERIT examples that must run use `refine=False`.
- Fix the stale Linux-centric `crates/core/src/cache.rs:95` doc comment to describe `HFX_CACHE_DIR` or the platform cache directory from `dirs::cache_dir()`.
- Add `crates/gdal/README.md` because the crate is complex by CLAUDE.md's standard: 7 files, 1045 lines, and native GDAL/GEOS bridge responsibilities.
- The GDAL README must include Purpose, Mermaid architecture diagram, Glossary, and Key types.

Exact gates:

- `rg -n "graph\\.arrow|M3|M4|Offline M4|Release note: M4|R3 divergence|R3 disagreement" crates/core/README.md crates/gdal/README.md` has no stale milestone wording unless it is a runtime literal such as `R3_NOTE`.
- `cargo doc --workspace --no-deps` runs clean with no warnings.
- `cargo build --workspace --exclude pyshed` green.
- `cargo check -p pyshed` green.
- `cargo clippy --workspace -- -D warnings` green.

Verification command:

```bash
rg -n "graph\\.arrow|M3|M4|Offline M4|Release note: M4|R3 divergence|R3 disagreement" crates/core/README.md crates/gdal/README.md
cargo doc --workspace --no-deps
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo clippy --workspace -- -D warnings
```

Dataset choice:

- No runnable data examples should be added here unless necessary. If added, use local synthetic v0.2.1 with default refinement, GRIT with default best-effort skip, or MERIT with `refine=False` and explicit D8 ambiguity context.

Version discipline:

- Workspace-doc/doc-comment commit. Run `./scripts/bump-version.sh patch`; stage `Cargo.toml` and `Cargo.lock` if changed; conventional commit; tag `v<version>`.

### Step 3: Pyshed User Docs And API Stub

Scope:

- `crates/python/README.md`
- `crates/python/API.md`
- `crates/python/python/pyshed/__init__.pyi`
- `crates/python/CHANGELOG.md` only for verification/classification, not rewrite

Work:

- Remove stale platform version anchor while keeping Apple Silicon macOS-only reality.
- Replace `global/hfx` examples with `grit/2.0.0`.
- Replace `graph.arrow` with `graph.parquet`.
- Fix cache path prose to distinguish persistent `HFX_CACHE_DIR`/OS cache from per-engine in-memory Parquet cache.
- Add `LevelSelection` and `bench_trace` to API public exports; add `bench_trace` to `.pyi`.
- Keep HFX v0.1 hard-error statements.
- Drop stale `0.2.0` anchors for platform and `LevelSelection.FINEST`.
- Reword user prose around `R3` while preserving the runtime `R3_NOTE` attribute in API docs and stubs.
- Do not rewrite dated changelog entries; flag `crates/python/CHANGELOG.md:58` as historical.

Exact gates:

- `.pyi` vs runtime consistency check returns no missing public exports other than allowed private implementation details.
- `API.md` public export enumeration matches `pyshed.__all__` for every documented public export, including `LevelSelection` and `bench_trace`.
- `rg -n "graph\\.arrow|global/hfx|Platform support \\(v0\\.2\\.0\\)|LevelSelection\\.FINEST.*0\\.2\\.0|~/.cache/hfx" crates/python/README.md crates/python/API.md crates/python/python/pyshed/__init__.pyi` returns no stale hits.
- Runnable Python quickstart, batch, staged, `geometry=False`, and `bench_trace` examples execute with `pyshed` importable, either from `maturin develop` or an installed wheel, against local synthetic v0.2.1 or GRIT `grit/2.0.0`; use default refinement except for MERIT examples that need `refine=False`.
- `cargo check -p pyshed` green.

Verification command:

```bash
python3 - <<'PY'
import ast
from pathlib import Path
init = ast.parse(Path("crates/python/python/pyshed/__init__.py").read_text())
pyi = ast.parse(Path("crates/python/python/pyshed/__init__.pyi").read_text())
exports = []
for node in init.body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "__all__":
                exports = [elt.value for elt in node.value.elts]
pyi_names = {n.name for n in pyi.body if isinstance(n, (ast.ClassDef, ast.FunctionDef))}
pyi_names.add("__version__")
allowed_stub_only = {"ProgressCallback", "ProgressEvent", "_Outlet", "__version__"}
print("missing_from_pyi:", sorted(set(exports) - pyi_names))
print("extra_in_pyi:", sorted(pyi_names - set(exports) - allowed_stub_only))
assert not (set(exports) - pyi_names)
PY
python3 - <<'PY'
import ast
import re
from pathlib import Path
init = ast.parse(Path("crates/python/python/pyshed/__init__.py").read_text())
exports = []
for node in init.body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "__all__":
                exports = [elt.value for elt in node.value.elts]
api = Path("crates/python/API.md").read_text()
match = re.search(r"## Public Exports\n\n.*?\n\n((?:- `[^`]+`\n)+)", api, re.S)
assert match, "API.md public exports list not found"
api_exports = set(re.findall(r"- `([^`]+)`", match.group(1)))
missing = sorted((set(exports) | {"__version__"}) - api_exports)
print("missing_from_api_exports:", missing)
assert not missing
PY
rg -n "graph\\.arrow|global/hfx|Platform support \\(v0\\.2\\.0\\)|LevelSelection\\.FINEST.*0\\.2\\.0|~/.cache/hfx" crates/python/README.md crates/python/API.md crates/python/python/pyshed/__init__.pyi
cargo check -p pyshed
```

Dataset choice:

- Fast examples: `crates/core/tests/fixtures/parity/v021_synthetic_refined` with default refinement enabled, after confirming the chosen coordinate resolves.
- Public examples: `https://basin-delineations-public.upstream.tech/grit/2.0.0/` with default refinement enabled; best-effort skips because GRIT has no D8 aux.
- D8-specific MERIT examples: `https://basin-delineations-public.upstream.tech/merit/0.2.0/`, but document `AmbiguousD8Coverage` and use `refine=False` for runnable MERIT examples.

Version discipline:

- Pyshed-only doc/stub commit. No workspace bump. No pyshed version bump. No pyshed release. Conventional doc-only commit is okay, but no `v*` or `pyshed-v*` tag.

### Step 4: Living Docs Under `docs/`

Scope:

- `docs/raster-cache.md`
- `docs/telemetry.md`
- `docs/benchmarks/delineate-harness.md`
- `docs/basin-geoparquet-export.md`

Work:

- Reword raster-cache and telemetry docs away from phase labels into current behavior.
- Keep COG/raster-cache facts but add the MERIT overlapping-Pfaf D8 limitation wherever refinement examples appear.
- Fix Linux-only cache path language.
- Resolve benchmark harness `grit/1.0.0` contradiction:
  - Preferred: file a separate code-change task to update `crates/core/src/bin/bench_delineate.rs:17` to `grit/2.0.0`, then update docs in that later code milestone.
  - Docs-only alternative: say the current harness alias is stale and should not be used as a canonical docs example; provide direct dataset URL examples instead.
- Update Basin GeoParquet examples to current dataset labels and remove `M5` milestone wording.

Exact gates:

- `rg -n "grit/1\\.0\\.0|merit-basins/0\\.1|global/hfx|~/.cache/hfx|Phase [0-9A-Z/]+|\\bM[0-9]\\b|\\bR3\\b" docs/raster-cache.md docs/telemetry.md docs/benchmarks/delineate-harness.md docs/basin-geoparquet-export.md` returns no stale live-doc hits except runtime literal `R3_NOTE` if intentionally referenced.
- `cargo run -p shed-core --features test-fixtures --bin bench_delineate -- --mode hot --dataset local --outlet 0,0 --iterations 1 --out /tmp/shed-local-bench.jsonl` runs and writes JSONL.
- Reader smoke in `docs/basin-geoparquet-export.md` remains valid if dependencies are installed; if not installed, document that it was not run.

Verification command:

```bash
rg -n "grit/1\\.0\\.0|merit-basins/0\\.1|global/hfx|~/.cache/hfx|Phase [0-9A-Z/]+|\\bM[0-9]\\b|\\bR3\\b" docs/raster-cache.md docs/telemetry.md docs/benchmarks/delineate-harness.md docs/basin-geoparquet-export.md
cargo run -p shed-core --features test-fixtures --bin bench_delineate -- --mode hot --dataset local --outlet 0,0 --iterations 1 --out /tmp/shed-local-bench.jsonl
```

Dataset choice:

- Benchmark smoke uses local fixture via `--dataset local`.
- Avoid remote benchmark examples as mandatory gates because measured cold GRIT first-open is about 2 min and network-bound.

Version discipline:

- Workspace-doc commit. Run `./scripts/bump-version.sh patch`; stage `Cargo.toml` and `Cargo.lock` if changed; conventional commit; tag `v<version>`.

### Step 5: Final Sweep And Milestone Gates

Scope:

- All living surfaces from this inventory.
- Historical surfaces only for classification checks; do not edit them.

Work:

- Run broad grep gates over living docs.
- Run all runnable examples and capture outputs/timings.
- Run Rust doc/build/check/clippy gates.
- Re-run `.pyi` consistency check.
- Confirm changelog and fixture historical references were not rewritten.

Exact gates:

- Grep gate:

```bash
rg -n "grit/1\\.0\\.0|merit-basins/0\\.1|global/hfx|graph\\.arrow|terminal_atom_id|\\batom\\b|macOS arm64 only in v0\\.1|Platform support \\(v0\\.2\\.0\\)|~/.cache/hfx" README.md AGENTS.md CONTRIBUTING.md SECURITY.md crates/core/README.md crates/gdal/README.md crates/python/README.md crates/python/API.md crates/python/python/pyshed/__init__.pyi docs/raster-cache.md docs/telemetry.md docs/benchmarks/delineate-harness.md docs/basin-geoparquet-export.md
```

- Examples-run gate:
  - Every Python block intended to be runnable must execute with `pyshed` importable, either from `maturin develop` or an installed wheel.
  - Every Python and CLI block intended to be runnable must execute against local synthetic v0.2.1 or GRIT `grit/2.0.0`; local synthetic and GRIT examples should use default refinement unless the example is specifically demonstrating disabled refinement.
  - The implementer must confirm the chosen local synthetic coordinate resolves and adjust the coordinate if needed.
  - MERIT `merit/0.2.0` examples must either use `refine=False` or be non-runnable limitation examples documenting `AmbiguousD8Coverage`.
  - Capture actual output and timings.

- Build/doc gates:

```bash
cargo doc --workspace --no-deps
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo clippy --workspace -- -D warnings
```

- `.pyi` vs runtime consistency:

```bash
python3 - <<'PY'
import ast
from pathlib import Path
init = ast.parse(Path("crates/python/python/pyshed/__init__.py").read_text())
pyi = ast.parse(Path("crates/python/python/pyshed/__init__.pyi").read_text())
exports = []
for node in init.body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "__all__":
                exports = [elt.value for elt in node.value.elts]
pyi_names = {n.name for n in pyi.body if isinstance(n, (ast.ClassDef, ast.FunctionDef))}
pyi_names.add("__version__")
allowed_stub_only = {"ProgressCallback", "ProgressEvent", "_Outlet", "__version__"}
missing = sorted(set(exports) - pyi_names)
extra = sorted(pyi_names - set(exports) - allowed_stub_only)
print("missing_from_pyi:", missing)
print("extra_in_pyi:", extra)
assert not missing
assert not extra
PY
```

- `API.md` export enumeration consistency:

```bash
python3 - <<'PY'
import ast
import re
from pathlib import Path
init = ast.parse(Path("crates/python/python/pyshed/__init__.py").read_text())
exports = []
for node in init.body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "__all__":
                exports = [elt.value for elt in node.value.elts]
api = Path("crates/python/API.md").read_text()
match = re.search(r"## Public Exports\n\n.*?\n\n((?:- `[^`]+`\n)+)", api, re.S)
assert match, "API.md public exports list not found"
api_exports = set(re.findall(r"- `([^`]+)`", match.group(1)))
missing = sorted((set(exports) | {"__version__"}) - api_exports)
print("missing_from_api_exports:", missing)
assert not missing
PY
```

Version discipline:

- If this step only verifies and does not commit, no bump.
- If it fixes any workspace docs/doc comments, normal workspace bump/tag.
- If it fixes only `crates/python` docs/stub, pyshed exemption: no bump, no release, no tag.

## 4. Milestone Gates

The implementer should not call the milestone complete until all gates pass or a specific gate is recorded as blocked with evidence.

| Gate | Command / Requirement |
|---|---|
| Living-doc stale dataset refs | `rg -n "grit/1\\.0\\.0|merit-basins/0\\.1|global/hfx" <living-doc-list>` returns no hits. Historical surfaces are excluded. |
| HFX artifact names | `rg -n "graph\\.arrow" <living-doc-list>` returns no hits. |
| Atom vocabulary | `rg -n "\\batom\\b|terminal_atom_id" README.md crates/python/README.md crates/python/API.md docs/*.md docs/benchmarks/*.md` returns no user-facing hits. |
| Stale platform/version anchors | `rg -n "macOS arm64 only in v0\\.1|Platform support \\(v0\\.2\\.0\\)|LevelSelection\\.FINEST.*0\\.2\\.0" <living-doc-list>` returns no hits. |
| Cache path honesty | Docs say `HFX_CACHE_DIR` overrides the cache root; default is OS cache dir via `dirs::cache_dir()`, macOS `~/Library/Caches/hfx`, Linux usually `~/.cache/hfx`; remote parquet cache defaults on in Python, local off. |
| Cache doc comment | `crates/core/src/cache.rs:95` no longer says the fallback is always `~/.cache/hfx`; it names `HFX_CACHE_DIR` or the platform cache directory. |
| Examples run | Every runnable Python/CLI example executes against local synthetic v0.2.1 or GRIT `grit/2.0.0`; Python gates state that `pyshed` must be importable; outputs/timings captured. Local synthetic and GRIT examples use default refinement unless explicitly demonstrating disabled refinement. MERIT examples use `refine=False` or document `AmbiguousD8Coverage`. |
| Rust docs | `cargo doc --workspace --no-deps` clean. |
| Rust build | `cargo build --workspace --exclude pyshed` green. |
| Pyshed check | `cargo check -p pyshed` green. |
| Clippy | `cargo clippy --workspace -- -D warnings` green. |
| `.pyi` consistency | AST/export check returns no missing exports and no unexpected extras. |
| `API.md` exports consistency | API.md public export list contains every name in `pyshed.__all__`, including `LevelSelection` and `bench_trace`, plus documented `__version__`. |
| Historical preservation | `git diff -- docs/investigations docs/hfx-v02-redesign docs/plans crates/core/tests/fixtures` shows no rewrites except this new plan file. |

## 5. Version Discipline Per Step

| Step | Commit group | Version rule |
|---|---|---|
| Step 1 Root and agent docs | Workspace docs (`README.md`, `AGENTS.md`) | Normal workspace rule: `./scripts/bump-version.sh patch`, stage `Cargo.toml` plus `Cargo.lock` if changed, conventional commit, tag `v<version>`. |
| Step 2 Core and GDAL crate docs | Workspace docs/doc comments (`crates/core`, `crates/gdal`) | Normal workspace rule: patch bump, stage version files, conventional commit, tag `v<version>`. |
| Step 3 Pyshed user docs and API stub | Pyshed-only docs/stub (`crates/python/README.md`, `API.md`, `.pyi`, optional changelog classification only) | Pyshed exemption: no workspace bump, no pyshed version bump, no pyshed release, no tag. |
| Step 4 Living docs under `docs/` | Workspace docs (`docs/raster-cache.md`, `docs/telemetry.md`, benchmark/export docs) | Normal workspace rule: patch bump, stage version files, conventional commit, tag `v<version>`. |
| Step 5 Final sweep | Verification only unless fixes are needed | No bump for verification-only. If fixes touch workspace docs, normal workspace bump/tag. If fixes touch only pyshed docs/stub, pyshed exemption. |

Tooling must never create its own commit or tag. Version changes are folded into the real commit. Do not mix workspace docs and pyshed-only docs in one commit, because the version rules differ.

## Open Questions

1. `docs/benchmarks/delineate-harness.md` says `--dataset r2` expands to `grit/1.0.0`, and code confirms the same stale value at `crates/core/src/bin/bench_delineate.rs:17`. Because this milestone forbids code changes, should the implementer document the alias as currently stale and avoid it, or split out a small code-change milestone to update the alias to `grit/2.0.0`?
2. Should `docs/raster-cache.md` remain a living current behavior doc, as assumed here, or be moved/reclassified as a point-in-time R3/Phase 4 note? If it remains living, phase wording must be removed.
3. For public examples, should maintainers prefer runnable local synthetic examples in root docs, or public GRIT `grit/2.0.0` examples with an explicit cold-open warning? The plan allows both, but the final docs should pick one style per section. Either way, do not disable refinement for GRIT/local by default; reserve `refine=False` for MERIT ambiguity or examples specifically showing disabled refinement.

Summary: this plan sequences five steps: root/agent docs, core+GDAL crate docs, pyshed docs+stub, living docs under `docs/`, and a final gate sweep. The main open decisions are how to handle the stale benchmark harness alias under a docs-only constraint, whether raster-cache docs should stay living, and whether public examples should prioritize local synthetic speed or hosted GRIT realism while keeping default refinement behavior accurate.
