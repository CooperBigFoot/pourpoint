# M5 Integrated Step Plan - Rust API Cutover And Basin GeoParquet Export

M5 closes the v0.2 redesign campaign by stabilizing the Rust core public
surface, mechanically removing the remaining public `atom` terminology, landing
the already-converged basin GeoParquet export plan, and recording what remains
deferred for later pyshed and strategy-binding sessions.

This plan integrates W1 API cutover work with the W2 export sub-plan in
`docs/hfx-v02-redesign/m5-export-plan.md`. Treat export steps E1-E9 as given:
do not re-plan, re-number, or re-litigate them.

## Orientation Findings

- Repository state: `git branch --show-current` returned `main`; `grep
  '^version' Cargo.toml | head -1` returned `version = "0.1.140"`.
- M5 contract summary: `docs/hfx-v02-redesign/milestone-plan.md:331-425`
  defines M5 as Rust core public surface stabilization, folded basin
  GeoParquet export, migration notes, deferred pyshed redesign, and a hardened
  atom-terminology gate.
- Export sub-plan status: `docs/hfx-v02-redesign/m5-export-plan.md:1-12`
  explicitly folds basin GeoParquet export into M5 and records that the plan
  revision resolved the `fabric_version()` vs `adapter_version()` and Hilbert
  parameter critiques.
- Export E1-E9 gates are defined in
  `docs/hfx-v02-redesign/m5-export-plan.md:193-304`; E3 requires
  hand-computed Hilbert indices (`:219-229`), E6 proves footer metadata and row
  groups (`:255-265`), E7 adds a small golden fixture (`:267-277`), E8 is an
  optional thin CLI wrapper (`:279-289`), and E9 is M5 closure (`:291-304`).
- `CLAUDE.md:33-48` requires every workspace commit to run
  `./scripts/bump-version.sh patch`, stage `Cargo.toml` with the real change,
  use a conventional commit, and tag `v<version>`. `CLAUDE.md:50-61` says
  pyshed uses separate `pyshed-v*` releases; M5 mechanical pyshed renames ride
  in workspace commits and must not cut a pyshed release.
- Core result methods are already unit-named:
  `crates/core/src/engine.rs:60-128` exposes
  `terminal_unit_id()`, `resolved_outlet()`, `resolution_method()`,
  `upstream_unit_ids()`, `refinement()`, `geometry()`, `area_km2()`, and
  `geometry_wkb()`.
- Area-only result is also unit-named:
  `crates/core/src/engine.rs:133-185` exposes
  `terminal_unit_id()`, `upstream_unit_ids()`, and `refinement()`.
- R3 divergence is already documented in the staged record itself:
  `crates/core/src/staged.rs:203-209` says pre-merge drainage-unit records are
  pristine source records and do not define final `area_km2` or final refined
  geometry. `crates/core/README.md:69-74` and `:144-149` repeat the same
  boundary.
- Telemetry stages are staged-pipeline named:
  `crates/core/src/telemetry/mod.rs:27-93` defines names such as
  `outlet_resolve`, `upstream_traversal`, `watershed_assembly`, and
  `result_compose`; `crates/core/src/engine.rs:839-864` enters those stages.
- The exact W1 public leak surface remains outside core result accessors:
  `src/main.rs:356-362`, `:462-474` emit `terminal_atom_id` and
  `upstream_atom_count`; `crates/python/src/result.rs:44-87`, `:134-202`
  expose `terminal_atom_id`, `upstream_atom_ids`, and repr strings;
  `crates/python/src/geojson.rs:16-39` emits `terminal_atom_id` and
  `upstream_atom_count`.
- Core docs/comments still carry stale domain-atom prose:
  `crates/core/README.md:186-191`, `:229-230`;
  `crates/core/src/algo/refine.rs:1`, `:18`, `:39`, `:122`;
  `crates/core/src/error.rs:349-350`.
- A naive `rg "atom|Atom"` gate is not runnable because it catches std
  atomic internals such as `crates/core/src/source_telemetry.rs:9-120`,
  `crates/core/src/parquet_cache.rs:29-208`, and test counters in
  `session.rs`, `snap_store.rs`, and `catchment_store_perf_tests.rs`.
- A hard grep over all `docs/hfx-v02-redesign` is also not runnable because the
  planning and critique docs must describe the atom-to-unit migration. The hard
  no-leak gate must therefore cover live/public surfaces, while docs get
  targeted presence checks.
- HFX identity accessors are distinct: `../hfx/crates/hfx-core/src/manifest.rs:208-215`
  exposes `fabric_name()` and `fabric_version()`, while `:256-258` exposes
  `adapter_version()`. W2 `delineation` labels must use `fabric_version()`;
  `adapter_version()` is provenance only.
- HFX v0.2.1 is the hard input cut: `../hfx/spec/HFX_SPEC.md:257-268`
  defines manifest fields including `format_version`, `fabric_version`,
  `unit_count`, `adapter_version`, and `auxiliary`; `:414-424` records the
  v0.2.1 migration requirements.
- No production export module exists yet: `find crates/core/src -maxdepth 2
  -type d -name export -print` returned no paths.
- Existing durability gates are present as real test targets:
  `crates/core/tests/parity_golden_artifacts.rs`,
  `staged_delineation.rs`, `d8_refinement_parity.rs`, and
  `d8_aux_accessor.rs` were listed by `find crates/core/tests -maxdepth 1`.

## Phase A - W1 Core Surface Stabilization

### Step M5.W1 - Audit And Lock Core Result Surface

Goal: prove the Rust core result API is already unit-named and freeze the
public shape W2 export will consume.

Files touched:

- `crates/core/src/engine.rs` only if doc comments need clarification.
- `crates/core/README.md` to add an API-stability note if absent.
- No export code, no CLI JSON, no pyshed API in this step.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test -p shed-core --test staged_delineation
rg -n "terminal_unit_id|upstream_unit_ids|resolved_outlet|resolution_method|geometry_wkb" crates/core/src/engine.rs crates/core/README.md
! rg -n "terminal_atom_id|upstream_atom_ids" crates/core/src/engine.rs
```

Byte-freeze boundary: do not touch the M1 canonicalizer or committed goldens,
the M3 staged contracts except documentation clarifying the existing contract,
or the M4 carve algorithm/output.

Version reminder: if this lands as a commit, run `./scripts/bump-version.sh
patch`, stage `Cargo.toml` and any real lockfile change, use a conventional
message such as `docs: lock m5 result surface`, and tag `v<version>`.

### Step M5.W2 - Document R3 Divergence And Export Dependency

Goal: make the intentional R3 divergence explicit at the public docs boundary:
pre-refinement drainage-unit records are pristine inspection records and may not
union or sum to final refined geometry/area. Also add the README stage diagram
export persistence step without implying that export is part of delineation.

Files touched:

- `crates/core/README.md`.
- Optionally doc comments in `crates/core/src/staged.rs` if the existing wording
  needs a small cross-reference.

Gate:

```bash
cargo test -p shed-core --test staged_delineation
rg -n "pre-merge.*does not define final|R3|export persistence|GeoParquet" crates/core/README.md crates/core/src/staged.rs
```

Byte-freeze boundary: no changes to staged method signatures, `PreMergeDrainageUnit`
fields, assembly behavior, canonicalizer output, or M4 carve behavior. This is
documentation only unless a missing public doc line must be added.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

### Step M5.W3 - Audit Telemetry And Error Names

Goal: remove remaining domain-atom vocabulary from core telemetry/error public
prose while preserving staged-pipeline names and typed errors.

Files touched:

- `crates/core/src/error.rs` for the stale integrity comment at `:349-350`.
- `crates/core/src/telemetry/` only if a real `atom` domain leak is found.
- No `source_telemetry.rs`, `parquet_cache.rs`, or counter-only `Atomic*`
  cleanup unless the hardened gate shows a real domain leak.

Gate:

```bash
cargo test -p shed-core
rg -n "outlet_resolve|upstream_traversal|watershed_assembly|result_compose" crates/core/src/telemetry crates/core/src/engine.rs
! rg -n '\b[Aa]tom' crates/core/src/error.rs crates/core/src/telemetry crates/core/src/engine.rs crates/core/src/staged.rs
```

Byte-freeze boundary: do not change error variants in ways that affect existing
test assertions unless the assertion is stale terminology only. Do not touch
delineation behavior, canonicalizer, staged contracts, or carve output.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

## Phase B - W1 Mechanical Atom-To-Unit Rename

### Step M5.W4 - Rename CLI JSON Output Contract

Goal: mechanically rename CLI JSON/GeoJSON output keys from atom vocabulary to
unit vocabulary and record this as an output-contract change.

Files touched:

- `src/main.rs`.
- CLI tests or snapshots, if any exist for these keys.
- Migration notes or changelog docs introduced by W1.W6/W1.W7 may be stubbed
  here only if needed to keep the contract change discoverable.

Required target keys:

- `terminal_atom_id` -> `terminal_unit_id`.
- `upstream_atom_count` -> `upstream_unit_count`.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo test --workspace --exclude pyshed --no-run
! rg -n "terminal_atom_id|upstream_atom_count" src/main.rs
rg -n "terminal_unit_id|upstream_unit_count" src/main.rs
```

Byte-freeze boundary: this is an output-key rename only. Do not change
delineation behavior, output geometry bytes, canonicalizer/goldens, staged
contracts, or carve logic.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

### Step M5.W5 - Mechanical pyshed Thin-Binding Rename

Goal: rename only the thin pyshed public accessors, repr strings, and GeoJSON
property keys that still say atom. This is not the full pyshed redesign.

Files touched:

- `crates/python/src/result.rs`.
- `crates/python/src/geojson.rs`.
- `crates/python/python/pyshed/__init__.pyi`.
- `crates/python/API.md` and `crates/python/README.md` where they document the
  renamed public attributes or GeoJSON keys.
- pyshed tests that directly assert the renamed public attributes, repr strings,
  or GeoJSON keys.
- Any `__init__.py` re-export or helper asserting these names.

Required target names:

- `terminal_atom_id` -> `terminal_unit_id`.
- `upstream_atom_ids` -> `upstream_unit_ids`.
- `terminal_atom_id` GeoJSON property -> `terminal_unit_id`.
- `upstream_atom_count` GeoJSON property -> `upstream_unit_count`.
- repr strings use `terminal_unit_id=`.

Gate:

```bash
cargo check -p pyshed
! rg -n "terminal_atom_id|upstream_atom_ids|upstream_atom_count" crates/python
rg -n "terminal_unit_id|upstream_unit_ids|upstream_unit_count" crates/python/src crates/python/python/pyshed/__init__.pyi crates/python/API.md crates/python/README.md crates/python/tests
```

Byte-freeze boundary: no inspectable stages, strategy menu, Python-authored
strategies, Python export API, or pyshed release packaging. This is a
workspace build-hold rename.

Version reminder: workspace patch bump and `v<version>` tag only. Do not run
`bump-pyshed-version.sh` and do not create a `pyshed-v*` tag.

### Step M5.W6 - Rename Core Docs/Comments, Including refine.rs

Goal: remove stale atom vocabulary from core docs/comments in the hardened grep
scope.

Files touched:

- `crates/core/README.md` glossary/key-types rows.
- `crates/core/src/algo/refine.rs` doc comments only.
- `crates/core/src/error.rs` comment if not already handled by W3.

refine.rs decision: rename only the doc comments. The M4 freeze guards carve
behavior and output bytes, not comment immutability. Do not change imports,
types, control flow, raster masking, snap, trace, polygonization, or tests in
`algo/refine.rs`.

Gate:

```bash
cargo test -p shed-core --test d8_refinement_parity
cargo test -p shed-core --test parity_golden_artifacts
! rg -n '\b[Aa]tom' crates/core/src/algo/refine.rs crates/core/src/error.rs crates/core/README.md
```

How to prove carve byte identity: the executor must run both
`d8_refinement_parity` and `parity_golden_artifacts` after the comment-only
edit. A green pair means the frozen M4 carve output and M1 golden comparison are
unchanged.

Byte-freeze boundary: comments only in `refine.rs`; no canonicalizer/golden
updates; no staged contract changes; no carve algorithm or output changes.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

### Step M5.W7 - Split Atom Gates And Migration Notes Stub

Goal: define runnable live-surface no-domain-atom gates that fail on real leaks
but do not scan planning/critique prose. Create or update migration-note
scaffolding so W4/W5 output-contract changes and HFX v0.2.1 hard cut have a
home before export begins.

Files touched:

- `docs/hfx-v02-redesign/` migration note file or the existing campaign docs.
- Optional `scripts/` helper only if the executor wants a reusable grep wrapper;
  otherwise keep the gate inline in docs/CI.

Hard live-surface gates:

```bash
test -z "$(rg -n '\b[Aa]tom' crates/core/src crates/core/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n '\b[Aa]tom|terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python/src crates/python/python/pyshed/__init__.pyi crates/python/API.md crates/python/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n 'terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python)"
```

Acceptance rule: both filtered commands must print no lines. The hard gate
scope is live/public surface only: core source and README, CLI source, pyshed
Rust bindings, pyshed type stubs, and pyshed public docs. It intentionally does
not scan `docs/hfx-v02-redesign/*-plan.md` or `*-critique.md`, because those
files must describe the migration. Allowed filtered hits are limited to std
`Atomic*`/`atomic` identifiers. Do not add broad prose allowlist tokens such as
`migration`, `critique`, `historical`, or `v0.1`; those mask real leaks on the
same line.

Add a planted-leak negative proof to this step's notes or CI helper:

```bash
test -n "$(printf 'crates/core/README.md:1:terminal atom leak\n' | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -n "$(printf 'src/main.rs:1:terminal_atom_id\n' | rg 'terminal_atom_id|upstream_atom_ids|upstream_atom_count')"
```

Docs check: keep `docs/hfx-v02-redesign` out of the hard no-leak gate. Instead,
verify the migration-note deliverable exists and intentionally records the old
to new key mapping plus the HFX v0.2.1 hard cut.

Gate:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
rg -n "HFX v0\\.2\\.1|required|v0\\.1.*unsupported|terminal_unit_id|upstream_unit_count" docs/hfx-v02-redesign
test -z "$(rg -n '\b[Aa]tom' crates/core/src crates/core/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n '\b[Aa]tom|terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python/src crates/python/python/pyshed/__init__.pyi crates/python/API.md crates/python/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n 'terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python)"
test -n "$(printf 'crates/core/README.md:1:terminal atom leak\n' | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
```

Byte-freeze boundary: documentation/gate only. Do not touch delineation
behavior, canonicalizer/goldens, staged contracts, or carve output.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

## Phase C - W2 Basin GeoParquet Export

The following steps are the converged W2 export plan from
`docs/hfx-v02-redesign/m5-export-plan.md`. Keep IDs E1-E9 and their order. Do
not duplicate their full bodies in executor tickets; reference that file as the
source of truth. All Rust implementation steps still follow `CLAUDE.md`:
`tracing` instead of `log`/`println!`, `thiserror` with documented named-field
variants in library code, no `unwrap`/`expect` in library code, and type-driven
boundary parsing for new IDs/labels/options.

### Step M5.E1 - Document Export Contract

Source: `docs/hfx-v02-redesign/m5-export-plan.md:197-205`.

Gate:

```bash
rg "fabric_version|adapter_version|not a versioned spec|GeoParquet footer|Hilbert" docs/basin-geoparquet-export.md
```

Byte-freeze boundary: documentation only; do not touch delineation behavior,
canonicalizer/goldens, staged contracts, or carve output.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

### Step M5.E2 - Add Export Identity Types

Source: `docs/hfx-v02-redesign/m5-export-plan.md:207-217`.

Gate:

```bash
cargo test -p shed-core export_identity
```

Byte-freeze boundary: additive `crates/core/src/export/` identity types only.
No engine behavior, canonicalizer/goldens, staged contracts, or carve changes.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

### Step M5.E3 - Add Spatial Utility

Source: `docs/hfx-v02-redesign/m5-export-plan.md:219-229`.

Execution hygiene: Hilbert `xy2d`, order 16, global extent
`[-180.0, -90.0, 180.0, 90.0]` is net-new and load-bearing. The expected
indices in `export_spatial` tests must be independently hand-derived from the
documented algorithm, not copied from the implementation's own output.

Gate:

```bash
cargo test -p shed-core export_spatial
```

Byte-freeze boundary: additive export spatial helpers only. No engine behavior,
canonicalizer/goldens, staged contracts, or carve changes.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

### Step M5.E4 - Add Schema And Footer Metadata Builder

Source: `docs/hfx-v02-redesign/m5-export-plan.md:231-241`.

Gate:

```bash
cargo test -p shed-core export_schema
```

Byte-freeze boundary: additive export schema/metadata helpers only. No engine
behavior, canonicalizer/goldens, staged contracts, or carve changes.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

### Step M5.E5 - Add Row-Group Planner

Source: `docs/hfx-v02-redesign/m5-export-plan.md:243-253`.

Gate:

```bash
cargo test -p shed-core export_row_groups
```

Byte-freeze boundary: additive export row-group planner only. No engine
behavior, canonicalizer/goldens, staged contracts, or carve changes.

Version reminder: patch bump, stage `Cargo.toml`, conventional commit, tag.

### Step M5.E6 - Add Batch GeoParquet Writer

Source: `docs/hfx-v02-redesign/m5-export-plan.md:255-265`.

Execution hygiene: E6 proves Parquet-level GeoParquet metadata by reopening the
written file and reading footer `key_value_metadata`. If a cheap real-reader
path is already available when E6 lands, add one smoke load that verifies a
standard reader can see the `geo` footer. If no cheap path is available, record
that reader-acceptance gap in docs and do not block E6.

Gate:

```bash
cargo test -p shed-core export_writer
```

Byte-freeze boundary: additive export writer only. No engine behavior,
canonicalizer/goldens, staged contracts, or carve changes.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

### Step M5.E7 - Add Small Golden Export Fixture

Source: `docs/hfx-v02-redesign/m5-export-plan.md:267-277`.

Execution hygiene: E7 proves the committed export fixture through the standard
Parquet/Arrow path. If a cheap pyogrio/geopandas/pyshed real-reader smoke path
exists by this step, add one `geo` footer reader smoke load; if not, document
the gap and keep the gate Parquet-level.

Gate:

```bash
cargo test -p shed-core export_golden
```

Byte-freeze boundary: commit only a new tiny export fixture. Do not touch M1
goldens, M3 fixtures/contracts, or M4 parity fixtures/carve output.

Version reminder: patch bump, stage `Cargo.toml` and the new fixture,
conventional commit, tag.

### Step M5.E8 - Optional CLI Emit Command

Source: `docs/hfx-v02-redesign/m5-export-plan.md:279-289`.

Rule: implement only if it is a thin wrapper over the green core writer and an
input catalog shape is already settled. Skip rather than inventing a new catalog
format.

Gate if implemented:

```bash
cargo test --workspace --exclude pyshed cli_export
```

Byte-freeze boundary: CLI wrapper only. Do not change core delineation behavior,
canonicalizer/goldens, staged contracts, or carve output.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

## Phase D - Closure

### Step M5.W8/E9 - Migration, Deferral Ledger, Export Closure, Full Gate

Goal: close W1 and W2 together. Update migration docs to state the hard cut to
HFX v0.2.1, record CLI/pyshed mechanical rename output-contract changes, record
the additive export surface, and list deferred work. Then run one final
workspace-wide M5 gate.

This step is E9 from the export plan plus W1 closure. Keep the E9 export
deferrals: no versioned export spec, no conformance suite, no pyshed export API,
and no Python-authored strategies.

Files touched:

- `docs/hfx-v02-redesign/` migration/closure docs.
- `docs/basin-geoparquet-export.md` only for closure notes if needed.
- `docs/hfx-v02-redesign/milestone-plan.md` if the milestone summary still
  lacks the folded W2 scope.

Required deferral ledger:

- Full pyshed redesign: inspectable stages, strategy menu, Python-authored
  strategy callbacks, and Python export API.
- Full aux-to-strategy binding.
- Python-authored strategies.
- Level-selection strategies beyond default finest.
- Additional blessed refinement strategies.
- Versioned spec machinery or export conformance suite.

Full M5 closure gate:

```bash
cargo build --workspace --exclude pyshed
cargo check -p pyshed
cargo test -p shed-core
cargo test --workspace --exclude pyshed --no-run
cargo test -p shed-core --test parity_golden_artifacts
cargo test -p shed-core --test staged_delineation
cargo test -p shed-core --test d8_refinement_parity
test -z "$(rg -n '\b[Aa]tom' crates/core/src crates/core/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n '\b[Aa]tom|terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python/src crates/python/python/pyshed/__init__.pyi crates/python/API.md crates/python/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n 'terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python)"
test -n "$(printf 'crates/core/README.md:1:terminal atom leak\n' | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
rg -n "HFX v0\\.2\\.1|required|v0\\.1.*unsupported" docs/hfx-v02-redesign
rg -n "pyshed.*deferred|aux.*binding.*deferred|Python-authored.*deferred|level-selection.*deferred" docs/hfx-v02-redesign
test -f docs/basin-geoparquet-export.md
rg -n "basin GeoParquet|not a versioned spec|GeoParquet footer|fabric_version|Hilbert" docs/basin-geoparquet-export.md
cargo test -p shed-core export_identity
cargo test -p shed-core export_spatial
cargo test -p shed-core export_schema
cargo test -p shed-core export_row_groups
cargo test -p shed-core export_writer
cargo test -p shed-core export_golden
```

If E8 was implemented, add:

```bash
cargo test --workspace --exclude pyshed cli_export
```

Byte-freeze boundary: final closure must not regenerate or edit M1 goldens,
must not alter canonicalization behavior, must not change M3 staged contracts,
and must not alter M4 carve logic/output. The explicit durability tests above
are mandatory.

Version reminder: patch bump, stage `Cargo.toml` and `Cargo.lock` if changed,
conventional commit, tag.

## Sequencing Justification

Phase A comes first because W2 export depends directly on stable
`DelineationResult` accessors, refinement provenance, final geometry WKB, final
area, and manifest identity accessors. Export should consume those names, not
force a rename mid-implementation. Phase B follows because the mechanical
CLI/pyshed/core-doc atom-to-unit rename is an output-contract change; doing it
before export prevents new export docs or tests from copying stale vocabulary.
Phase C then lands E1-E9 in their converged order, from documentation through
identity, spatial utilities, schema, row groups, writer, fixture, optional CLI,
and closure. Phase D closes W1 and W2 together so the final workspace proves
core API coherence, export validity, migration notes, deferrals, and all prior
M1-M4 durability gates in one green run.

## Frozen-Artifact Boundary

Byte-frozen surfaces for every M5 step:

- M1 parity golden artifacts and canonicalizer behavior.
- M3 staged contract behavior and test expectations.
- M4 containment-clamped D8 carve algorithm and output bytes.
- Existing committed golden bytes.

`crates/core/src/algo/refine.rs` contains stale `atom` doc comments but is also
the frozen M4 carve file. M5 chooses doc-comment rename, not allowlisting the
file. The executor may change only those comments. The proof is running
`cargo test -p shed-core --test d8_refinement_parity` and
`cargo test -p shed-core --test parity_golden_artifacts` after the edit; both
must remain green, and no golden bytes may be updated.

## Self-Check

- `cargo build --workspace --exclude pyshed`: W1, W4, W7, and final W8/E9.
- `cargo check -p pyshed`: W5 and final W8/E9.
- `cargo test -p shed-core`: W3 and final W8/E9.
- `cargo test --workspace --exclude pyshed --no-run`: W4 and final W8/E9.
- `cargo test -p shed-core --test parity_golden_artifacts`: W6 and final W8/E9.
- `cargo test -p shed-core --test staged_delineation`: W1/W2 and final W8/E9.
- `cargo test -p shed-core --test d8_refinement_parity`: W6 and final W8/E9.
- Split live-surface atom gates and planted-leak proof: W7 and final W8/E9.
- Migration docs hard-cut and deferral checks: W7 and final W8/E9.
- `docs/basin-geoparquet-export.md` exists: E1 and final W8/E9.
- `export_identity`: E2 and final W8/E9.
- `export_spatial`: E3 and final W8/E9, with hand-derived Hilbert expectations.
- `export_schema`: E4 and final W8/E9.
- `export_row_groups`: E5 and final W8/E9.
- `export_writer`: E6 and final W8/E9, including footer-level `geo` metadata.
- `export_golden`: E7 and final W8/E9.
- `cli_export`: E8 only if implemented, then included in final W8/E9.
