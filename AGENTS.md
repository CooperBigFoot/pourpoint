# Project Instructions

A rule appears in this file only if (a) it encodes a project choice that cannot be inferred from the code, or (b) default model output violates it. Practices a model already follows unprompted, and anything rustfmt or clippy enforces mechanically (see `[workspace.lints]` in `Cargo.toml`), are deliberately absent.

## 0. Project Overview

`pourpoint` is intended to be the watershed extraction engine that consumes compiled HFX datasets from the sibling `../hfx` repository. `../hfx` defines the open HydroFabric Exchange contract, validator, and adapter-side normalization rules; this repository is the runtime engine that should load those HFX artifacts and perform outlet resolution, upstream traversal, terminal refinement, and final watershed geometry extraction.

Treat `../hfx/spec/HFX_SPEC.md` as the canonical contract for on-disk inputs. In practical terms, `pourpoint` should read `manifest.json`, `catchments.parquet`, `graph.arrow`, and optionally `snap.parquet`, `flow_dir.tif`, and `flow_acc.tif`, while keeping all source-fabric-specific logic out of the hot path. If a question comes up about file layout, schema, topology semantics, snapping rules, or raster refinement behavior, read `../hfx` first and align this repo with the spec rather than inventing a parallel contract here.

## 1. Workspace Layout and Tooling

- `src/` — the binary crate: CLI parsing and the composition root. Nothing else.
- `crates/*` — library crates: all domain logic lives here.

```bash
cargo fmt
cargo clippy --workspace --all-targets
cargo test --workspace
```

Clippy denies `.unwrap()`, `.expect()`, and `println!`/`eprintln!` in library crates (tests are exempt via `clippy.toml`; `main.rs` may use `.expect("reason")` for truly unrecoverable setup).

## 2. Design Doctrine

Four rules. They are one design stance seen four ways: a module means one thing, receives exactly what it needs, in types that cannot lie, and dies rather than guess.

### 2.1 Denotation line

Before implementing a module, state in one line in its `//!` doc what it computes as a mathematical object. Carriers must be named domain types, not placeholders.

```rust
//! snap : GeoCoord × FlowAccumulation → GridCoord   (pure, deterministic)
//! delineation = fold(grow, seed, upstream_cells)
```

If the line cannot be written, the design is not ready; say so instead of coding around it. In review, when the denotation line and the diff disagree, one of them is wrong.

### 2.2 Authority narrows

`src/main.rs` is the composition root: only it reads config files and environment variables, resolves paths, initializes `tracing`, and opens files or stores. Library crates receive everything as arguments — a function in `crates/*` that reads an env var, constructs a `Path` from a literal, or touches global state is a violation.

At every call, pass the narrowest argument that suffices: the one field, not the config struct; `&[Cell]`, not the whole raster, when only a slice is read.

### 2.3 Type-driven design

Encode domain invariants in the type system; invalid states must fail to compile.

- **Parse, don't validate (hard rule).** Raw input (CLI args, file contents, API payloads) is converted into domain types once, at the composition root. No raw primitive crosses into `crates/*` where a domain type exists: `fn delineate(comid: Comid, pour_point: GeoCoord)`, never `(comid: u64, lat: f64, lon: f64)`.
- **Newtypes** wherever two values of the same primitive type could be swapped: IDs, coordinates (grid vs. geographic), thresholds, distances, indices. Bare primitives are fine for unambiguous locals.
- **Enums over booleans.** Never `bool` for a domain state with two named possibilities: `enum TraceDirection { Upstream, Downstream }`, not `upstream: bool`. Applies to fields, parameters, and return values.
- **Typestate** (`Pipeline<Unfitted>` → `Pipeline<Fitted>` via `PhantomData`) for pipelines and resources with a lifecycle, where calling methods out of order is a logic bug. Do not force it on plain structs.

### 2.4 Fail loud

An error is either propagated with `?` or handled at one named per-item isolation point — never discarded to make code compile. `.unwrap_or_default()` on a required value, `let Ok(x) = … else { continue }` that silently skips a broken item, and `.ok()` that drops the error are all bugs.

The one exception: a batch loop over independent items (e.g. per-basin processing) may have exactly one isolation point that catches per-item failure, records which item failed and why, and continues. That point exists once per pipeline, not once per function.

## 3. Errors and Logging

- Library crates use `thiserror`; the binary uses `anyhow` with `.context()`.
- Every error variant gets a doc comment stating *when* it fires, and named fields, not tuples — the message should carry the values needed to act on it (`"no cell above threshold {threshold} within {radius} cells of ({x}, {y})"`).
- Diagnostics go through `tracing` with structured fields (`debug!(x = point.x, "snapping")`), not format strings. `#[instrument]` on public functions, with `skip` for large args. Levels: `error` = broken, `warn` = degraded, `info` = milestones, `debug` = internals, `trace` = hot loops.

## 4. Documentation

Documentation is for agents landing in the code, applied proportionally to complexity — not decoration.

- Simple module: the `//!` denotation line and a sentence suffice.
- Complex crate: `crates/foo/README.md` with purpose, a Mermaid architecture diagram (never ASCII art), a glossary of domain terms and math symbols, and the key entry-point types.
- Fallible public functions get an `# Errors` section; skip doc comments on obvious helpers and trivial getters.
- Math-style names (`dx`, `acc`, `phi`) are allowed in algorithm code if the module doc carries a glossary.

## 5. Style Residue

- Builder pattern (`with_*` returning `Self`) for config structs with more than 3 fields.
- No `use super::*`; explicit imports only.
