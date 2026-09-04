# Outlet authority: resolution chooses, refinement quantizes

Refactor pourpoint around one invariant: outlet resolution chooses a single authoritative hydrological outlet, and terminal refinement only computes the geometry draining to that outlet. Refinement never relocates the outlet.

## Why this matters

Today the engine resolves an outlet twice, and the second resolution can silently overturn the first.

Vector resolution (`crates/core/src/resolver.rs`) ranks the snap features within the search radius using the configured strategy, weight-first by default, picks a winner, and records the nearest point on that winner's geometry as the resolved outlet. This is the hydrological decision, and the weight-first default exists precisely so a request sitting on a tiny tributary stub resolves to the mainstem (see the root changelog's 0.1.56 entry and the `default_strategy_picks_mainstem_over_coincident_tiny_stub` test).

Terminal refinement (`crates/core/src/algo/refine.rs`, called through `D8RasterRefinementStrategy` in `crates/core/src/refinement.rs`) then rasterizes the terminal polygon, masks the flow-direction and accumulation tiles to it, and calls `snap_pour_point`, which scans every masked cell at or above the accumulation threshold and takes the one nearest the resolved point, breaking ties by accumulation. Nothing constrains that search to the reach the vector stage chose. A high-accumulation cell on a neighbouring branch inside the same terminal unit can win, and the traced basin then belongs to a different stream than the one the ranking selected. The ranking strategy is undone, the terminal unit is still correct, and no provenance says anything happened.

Both resolution paths (snap and point-in-polygon) feed the same untyped coordinate into refinement, so the refinement stage cannot tell whether it holds a network-resolved point or merely the request point that identified a unit by containment.

## The intended model

The mathematical behaviour is described in `../../pourpoint-joss/planning/pourpoint-algorithm-draft.tex` (in the repository that is a sibling of this repository's parent directory). The companion `../../pourpoint-joss/planning/pourpoint-outlet-authority-refactor-prompt.md` records the typed design that motivated this work. These are reasoning aids rather than repository contracts. This vision settles two pourpoint-specific differences explicitly: the vector-cell threshold guard below and the preserved public outlet semantics under containment.

In plain terms:

1. **Resolution produces a typed authority.** Either a vector point (terminal unit, the winning snap feature's provenance, and the nearest point on its geometry) or a unit-only containment result (terminal unit chosen by point-in-polygon with its tie-break provenance). Dispatch stays dataset-driven exactly as today: a snap declaration for the selected level means vector resolution, otherwise containment. Vector candidate generation (radius filter) and ranking (weight-first or distance-first cascade) are unchanged in behaviour.

2. **Refinement maps the authority onto a three-way seed decision** and never chooses among branches when a vector point exists:
   - **Vector point present.** When the projected vector point maps into the selected localized raster window, the seed is its unique containing cell. This is grid quantization, not a snap. The cell is usable only if it lies inside the rasterized terminal mask, has a valid flow direction, has a defined accumulation value, and meets the threshold. Projection or grid mapping can instead fail, including when valid HFX snap geometry places the authoritative point outside the terminal-bbox raster window. A mapping failure or unusable cell is a visible best-effort skip (whole terminal polygon retained, so the result is the coarse watershed) or an error under require-D8. The engine does not widen raster selection or localization, search for a substitute cell, fall back to the raster ranker, or apply neighbourhood tolerance.
   - **Unit only and D8 usable for the terminal.** Candidate cells are every masked terminal cell at or above the threshold. A ranker selects the seed. The only shipped ranker is today's rule: nearest cell to the request point, higher accumulation breaking equal-distance ties.
   - **Unit only and no usable D8.** The coarse watershed. This is the existing best-effort skip behaviour made explicit in the type, not new behaviour.

3. **For either raster seed**, trace every masked cell that drains to the seed, polygonize, and dissolve the refined terminal polygon with the unchanged upstream unit polygons. The terminal unit and the upstream unit set are never altered by refinement. This part of the carve is unchanged.

### Vector path guard, stated precisely

Grid mapping is fallible:

$$\operatorname{cellAt}_R(\operatorname{project}_R(p_{\mathrm{vec}}))\to\operatorname{Result}(\operatorname{GridCell}_R,\operatorname{GridMappingError}).$$

A mapping error produces no cell and no accumulation measurement. For a successfully mapped $c_{\mathrm{out}}$, the seed is accepted only if

$$\operatorname{usable}(c)\iff c\in\mathcal G_{\mathrm{term}}\ \wedge\ F_R(c)\ \text{defined}\ \wedge\ A(c)\ \text{defined}\ \wedge\ A(c)\ge\tau .$$

The algorithm draft currently says vector quantization does not apply an accumulation threshold. This vision amends that statement by adding the accumulation guard. The threshold $\tau$ is the existing snap-threshold option. It plays two different roles that must not be conflated in code or docs: in the raster fallback path it defines the candidate set $\mathcal N_\tau$; in the vector path it is a yes/no predicate on one already-chosen cell. Its purpose in the vector path is to make a misaligned centerline fail visibly instead of seeding a trace on a hillslope cell and returning a sliver basin.

### Why the resolution type is two-way and the seed decision is three-way

The companion design prompt names `OutletResolution` as a three-way sum with a `RasterResolved` variant. In this repository the resolution stage cannot honestly claim "raster resolved": whether a D8 declaration covers the terminal, whether the raster source is attached, and whether the window localizes are only known at refinement time, after the terminal geometry is fetched (`select_d8_raster_for_terminal`, `localize_d8_raster_window`). So the resolution stage returns the two-way authority and the refinement stage returns the three-way seed decision. The invariant is identical; only the stage that names the third case moves. Updating the external reasoning aids is welcome but not required for this work.

## Settled decisions

- **Unmappable or unusable vector point: strict guard, visible skip, no fallback.** Chosen over widening the localized window, falling back to the raster ranker (which reintroduces branch hopping), and a same-channel one-cell tolerance (more machinery; may be revisited once skip rates are measured). The skip reason must record the failure kind, threshold, optional mapped cell, and optional measured accumulation. Mapping failure and undefined accumulation use absence, never a fabricated numeric value. This makes skip rates on MERIT and GRIT measurable later.
- **No caller override of authority.** Dispatch remains dataset-driven. A future feature may add an option to force containment-plus-raster resolution when a snap index exists; do not add it now.
- **Ranker seam is Rust-only.** Keep candidate generation and ranking as separate steps behind a trait seam on both the vector side (radius set, then weight-first or distance-first cascade) and the raster side (threshold set, then nearest-with-accumulation-tie), shaped so a user-defined ranker can be added later without redefining the candidate domain. No Python-authored, CLI-selectable, or additional built-in rankers ship now. The core README already states Python-authored strategies are outside the runtime surface; keep that statement true.
- **Public field meanings are preserved.** `resolved_outlet` stays the authoritative vector point or the request point under containment, including the containment-without-D8 coarse case. The latter intentionally preserves current repository behaviour rather than adopting the algorithm draft's terminal-unit declared outlet. `refined_outlet` becomes the centre of the quantized or ranked seed cell. CLI and Python field names do not change. Provenance strings and the Rust provenance enums gain the seed kind and the new skip reason. Python `TerminalRefinement.status` keeps its three values.
- **The snap-threshold option keeps its name and default.** It now means the raster candidate-generation threshold plus the vector-cell usability guard.

## Scope boundaries

In scope: the resolver's typed result, the staged types in `crates/core/src/staged.rs` that carry it, the refinement strategy seam and carve entry so the vector path quantizes instead of snapping, the ranker seam, provenance and skip-reason enums, the Python and CLI provenance renderings, golden recapture, and every document that currently describes refinement as "snapping to the nearest raster cell".

Out of scope: caller-selectable authority, user-defined rankers on any surface, changes to the HFX specification (it already says raster refinement is an engine behaviour), changes to upstream traversal or dissolve, and any change to how the raster fallback path ranks cells.

## Observable evidence of success

- A regression fixture in which the vector stage selects a mainstem reach while a higher-accumulation cell on a different branch inside the same terminal unit is nearer to the resolved point. Before this work the traced basin follows the other branch; after it, the seed is the cell containing the vector point and the basin follows the selected reach. The fixture must be shown to fail on current `main` before the change lands (a check that never failed proves nothing).
- A fixture where the vector point's mapped cell is below threshold, has undefined accumulation, or lies outside the mask yields a best-effort skip whose reason names the failed conjunct, preserves the threshold, and carries the measured accumulation only when one exists; the same case yields an error under `RefinementMode::RequireD8`.
- A valid-HFX fixture whose winning snap geometry places the vector point outside the selected localized raster window yields a distinct grid-mapping best-effort skip with no cell and no accumulation, and an error under `RefinementMode::RequireD8`. No numeric accumulation is invented.
- A dataset with no snap declaration and a D8 pair still resolves by containment and ranks raster cells exactly as today; existing parity goldens for that path are unchanged.
- A generic fixture with neither a snap declaration nor a D8 pair produces the coarse watershed with the explicit coarse seed decision in provenance.
- `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace` pass, including the D8 gates listed in `crates/core/README.md`.
- The core README refinement section, `docs/how-it-works.md`, the `//!` denotation lines of the touched modules, and the `# Errors` tables describe the new behaviour. Today they say the terminal is replaced "at the snapped raster cell"; that sentence must go.

## Repository facts the implementing agent needs

- Doctrine is in `AGENTS.md`: denotation line per module, parse at the boundary, newtypes and enums for domain states, fail loud with one isolation point per batch loop, `thiserror` in library crates, structured `tracing`.
- `RefinementMode` (BestEffort, RequireD8, Disabled) is defined in `crates/core/src/staged.rs`. `BestEffortSkipReason` is defined in `crates/core/src/refinement.rs`; its general Availability, MisDeclaration, and DataGeometryIntegrity forms carry a typed source and diagnostic, while its three specific declaration/source-absence forms retain their existing shapes. Extend this taxonomy rather than adding a parallel mechanism.
- `snap_pour_point` in `crates/core/src/algo/snap.rs` is the current unconstrained search and also performs the out-of-tile bounds check. The vector path needs a quantization function with the usability predicate; the raster fallback keeps the search.
- The carve is CRS-agnostic grid arithmetic behind the projection seam; the vector point is forward-projected into the declared raster CRS (`forward` in `crates/core/src/algo/projection.rs`) before quantization, and only the refined outlet and carved rings are inverse-projected.
- Public accessors: `DelineationResult::resolved_outlet`, `refinement()` with `RefinementOutcome::Applied { refined_outlet, provenance }` in `crates/core/src/engine.rs`; Python bindings in `crates/python/src/result.rs` and `crates/python/API.md`; CLI JSON properties `resolved_lat`, `resolved_lon`, and the refined-outlet rendering in `src/main.rs`.
- Data alignment reality: MERIT snap geometries and their MERIT Hydro D8 cells can be misaligned; the adapter preserves source river WKB rather than proving cell alignment. GRIT provides segment-stem and reach-stem snap indices alongside a planetary EPSG:8857 D8 entry, with the default finest-level path using reach stems. These vector geometries can sit a cell or more off the raster channel, so vector-path skips are expected on real data. Snap-enabled HydroBASINS builds may carry optional HydroRIVERS features and no D8; HydroBASINS is not evidence for the no-snap coarse path.

## Goldens that will move

`crates/core/tests/fixtures/parity/goldens/v01_merit_refined` (`rhine_basel`, asserted through `parity_golden_artifacts.rs`) runs the vector path and pins `refined_outlet` and canonical WKB, so it is expected to move. Its documented refresh command is stale because the named test target no longer exists; implementation must provide and document a working recapture command before updating the network-backed golden, then record recapture provenance in its README. The projected-GRASS fixture asserted through `d8_refinement_parity.rs` has no snap declaration, resolves by point-in-polygon, and must remain byte-for-byte unchanged as containment-plus-D8 parity evidence. Do not weaken either assertion to make a golden pass.

## Remaining risk

Skip frequency on real data is unmeasured. If the strict guard skips refinement on a large share of GRIT or MERIT vector resolutions, that is a signal to design a same-channel tolerance in a later vision, not a reason to relax the guard here. Make the skip reason rich enough that the measurement can be made from logs and result provenance alone.
