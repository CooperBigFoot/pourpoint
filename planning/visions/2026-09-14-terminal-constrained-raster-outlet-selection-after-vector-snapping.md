# Terminal-constrained raster outlet selection after vector snapping

Vector snapping precedes raster refinement. It chooses a snap feature, a point on that feature, and the terminal drainage unit. When raster refinement runs, that terminal unit remains binding, but the cell containing the vector snap point is not the mandatory raster outlet. Refinement chooses a threshold-qualified raster cell inside the terminal unit, using the vector snap point as its proximity reference.

## Outcome and motivation

PR #154, “Preserve vector outlet authority through D8 refinement,” implemented the vision in `planning/visions/2026-09-04-outlet-authority-resolution-chooses-refinement-quantizes.md`. It treats a vector snap point as an authoritative hydrological outlet, quantizes it to its containing raster cell, and skips or errors if that cell fails usability guards. It forbids searching for another cell. This is unsuitable when vector snap features and raster channels are misaligned: selecting the correct terminal unit does not prove that the containing cell is the correct raster outlet.

This vision supersedes that earlier vision's vector-point authority and no-relocation requirements. It is not a request to add a fallback after failed vector quantization. The normal refinement path must select a raster outlet within the chosen terminal unit, whether or not the cell containing the vector point would pass the old guards.

The user places no time or resource ceiling on reaching a coherent design. All additions from PR #154 may be removed or redesigned where needed. A minimal diff, preservation of its abstractions, or preservation of its quantization algorithm is not a requirement. This permits a focused algorithm and API rethink, not unrelated repository-wide changes.

## Settled behavior

1. Preserve existing vector candidate generation and ranking. First snap the input to the winning vector feature and obtain its snapped point and terminal unit.
2. With raster refinement disabled, the vector snap point remains the resolved outlet. This work does not redefine vector snapping for non-refined watersheds.
3. With raster refinement enabled and usable D8 inputs available, search threshold-qualified raster cells within the selected terminal unit. Rank proximity to the **vector snap point**, not the original input coordinate. Choose the nearest cell center under the existing raster proximity convention; break equal-distance ties by higher accumulation, then stable row-major grid order. Preserve the existing accumulation-threshold option, default, and declared-unit conversion.
4. Search the terminal candidate domain, not only the containing cell or a neighborhood around it. Do not impose a new raster search-radius restriction. A vector point on an unusable or below-threshold cell is not a reason to skip a viable candidate elsewhere in the unit.
5. The reference coordinate guides distance ranking; it is not a required raster cell. Valid HFX snap geometry can place that point outside the terminal mask or localized terminal raster window. Such placement alone must not prevent ranking candidates inside the unit. Projection errors and actual unavailable or unusable raster data remain explicit failures; do not clamp or fabricate a seed.
6. The binding constraint is the terminal unit, not the vector reach or river branch. A nearer qualifying cell on another branch inside that unit may win. Do not add same-reach matching or branch-locking rules.
7. Where the selected level has no snap features and outlet resolution uses containment, retain containment resolution and rank raster candidates against the original input coordinate. There is no vector point in this case.
8. Keep the terminal unit and upstream-unit set unchanged by refinement. Continue tracing within the terminal raster mask, producing a terminal-contained carve, then assembling it with the unchanged whole upstream units. Outward correction of the coarse terminal boundary is not included.
9. Preserve `BestEffort`, `RequireD8`, and `Disabled` policy meanings. If refinement cannot produce a usable result, best effort retains the whole coarse terminal with a visible reason; required D8 returns an error. A successful coarse fallback must not be reported as applied refinement.

The original input can lie outside the unit selected by vector snapping. It is deliberately **not** the raster proximity reference on the vector path. Snapping must influence both the selected unit and the location from which refinement measures proximity, without dictating the exact seed cell.

## Design and public results

Reconsider the resolution/refinement boundary, carriers, candidate selection, and provenance together. Internal types must express the new responsibility rather than retain “authoritative vector point” semantics that the algorithm no longer honors. Reuse useful existing mechanisms where justified; remove obsolete guards and abstractions where justified. Do not preserve a vector-first quantization attempt as hidden behavior.

Retain a clear distinction between the original input coordinate, vector snap location, and selected raster outlet. Existing `resolved_outlet` meanings can remain: the vector snap point on the vector path and the request point under containment. Applied `refined_outlet` identifies the selected raster cell center. CLI, Python, Rust, GeoJSON, diagnostics, and documentation must agree; no ranked result may claim `VectorOutletQuantized` provenance. Preserve practical public compatibility where it does not misrepresent behavior. If redesign requires public API changes, provide an explicit migration rather than silently repurpose fields or retain misleading authority claims.

Reversible implementation choices, including trait seams and type layout, belong to the implementing agent. Follow the repository doctrine: named domain carriers, narrow authority, explicit errors, and module denotation lines that describe the actual computation. HFX remains the canonical on-disk contract; refinement policy belongs to the engine.

## Observable evidence of success

Add failing regressions against the real current Engine path before changing production behavior. Then demonstrate the same tests pass under the new design. Test at least:

- A vector point lies on a below-threshold or otherwise unusable containing cell, while a qualifying cell exists inside the terminal. Refinement now applies from the ranked candidate rather than skipping or erroring solely because of that containing cell.
- The vector point lies exactly on a cell boundary, its containing cell is usable, and an equally near qualifying cell has higher accumulation. Selection follows the ranking tie-break, not a successful-quantization shortcut.
- Original input and vector snap point favor different qualifying cells. The selected seed is nearest the vector point. Include input outside the selected terminal unit.
- A cell outside the terminal is closer or has higher accumulation. It remains ineligible.
- A valid vector snap reference lies outside the terminal mask or localized window while usable terminal candidates exist. Its location alone does not abort selection.
- A winning candidate can lie on another branch within the terminal, without changing the selected terminal or upstream-unit set.
- Threshold boundaries, accumulation units, distance ties, and full ties preserve the specified deterministic behavior.
- No usable candidate or unusable D8 yields the correct visible coarse fallback or required-D8 error, never invented successful refinement. Preserve valid HFX terminal flow semantics, including supported GRASS sinks and coverage exits.
- Containment-only refinement and refinement-disabled behavior retain their established semantics. Direct and staged APIs agree, and user-visible outlet fields and provenance identify what happened.

Use controlled fixtures for deterministic discrimination. Reuse available real-data evidence to check that vector/raster misalignment produces the intended result, without treating private or licensed data as redistributable. Preserve historical evidence and golden files as historical records. Any new expected geometry must have explained seed-selection provenance; do not weaken geometry or parity assertions merely to accept changed outputs.

Run relevant focused tests, documentation checks, formatting, Clippy, and the documented workspace validation. Record any host limitations explicitly. No performance target is introduced, but preserve terminal-localized raster reads rather than expanding to whole-dataset raster processing.

## Repository starting points and scope

- `crates/core/src/resolver.rs`: vector and containment resolution, winning feature and coordinate provenance.
- `crates/core/src/engine.rs` and `crates/core/src/staged.rs`: original input, resolved outlet, staged refinement inputs, policies, and result assembly.
- `crates/core/src/refinement.rs`: strategy boundary, projection, decisions, skip reasons, and provenance.
- `crates/core/src/algo/refine.rs`: `RasterOutlet::VectorPoint` versus `UnitOnly`, vector guards, masking, seed selection, tracing, and polygonization.
- `crates/core/src/algo/snap.rs`: threshold candidate generation and nearest-center ranking. Its current `snap_pour_point` rejects reference points outside the raster window before ranking. That guard cannot remain a prerequisite for the new vector-reference search.
- `crates/core/src/algo/snap_threshold.rs`: current threshold default is 1,000 upstream cells. Effective threshold conversion follows the declared raster accumulation units.
- `crates/core/tests/staged_delineation.rs`, `d8_aux_accessor.rs`, `d8_refinement_parity.rs`, and relevant Engine unit tests: regression and parity starting points.
- `crates/python/src/result.rs`, `crates/python/src/staged.rs`, `crates/python/src/geojson.rs`, and `src/main.rs`: user-visible refinement outcomes and outlet reporting.
- `../hfx/spec/HFX_SPEC.md`: canonical input contract; runtime traversal and refinement policies are engine parameters.

Update active public documentation, API documentation, module descriptions, and changelog/migration notes that describe fixed vector-cell authority. The earlier vision and its evidence remain historical; this document explicitly supersedes its conflicting decisions.

Excluded: changing HFX schemas or hosted datasets, changing vector ranking defaults, introducing caller-selectable rankers, same-reach constraints, changing upstream traversal or dissolve, outward terminal-boundary correction, release/version work, and automatic execution of downstream studies. This is a standalone vision with no Program or Effort provenance.
