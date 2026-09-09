# Correct Basel TDX-Hydro watershed geometry

## Outcome

Correct pourpoint so default TDX-Hydro delineation at Basel (latitude 47.5596, longitude 7.5890) returns a valid watershed. The observed successful engine return contains a self-touching exterior ring. Rejecting that result alone is containment, not completion: the default Basel delineation must work.

Preserve the intended watershed footprint and existing hole policy. Necessary geometry changes are permitted when justified by the diagnosed defect; measure their boundary and area effects. Validity obtained merely by deleting small fragments or increasing smoothing is not sufficient evidence of correctness.

This is a standalone local vision draft. It has not been verified on the target branch. Before substantive implementation, `implement-vision` must publish and verify this new standalone draft on its target branch under that workflow's durability gate.

## Reproduction evidence

Original evidence directory:

`~/.local/share/hfx/operations/2026-09-09-tdx-delivery/basel-study/tdx-run/`

Retain its original artifacts, including `watershed.wkb`, `watershed.geojson`, `geometry-diagnostic.json`, `metadata.json`, `upstream-unit-ids.json`, and delivery receipts. The metadata records the wheel identity, source commit, dataset identity, defaults, and execution results. The observed source commit was `7a43834a870d0f5d46ed6d23926205ee58c8fb93`; do not assume the current checkout is identical.

Pinned compiled HFX dataset on Hetzner:

`s3://pourpoint-hfx/hfx/tdx-hydro-nga-20230126-global-62basin-hfx-0.3.0-af443be35774/`

The recorded call was `Engine(uri)` followed by `delineate(lat=47.5596, lon=7.5890)`, with consumer defaults. The result selected terminal unit 230303076 and 4,032 upstream units. Resolved outlet coordinates were approximately (longitude 7.588708449410564, latitude 47.56112089110772). D8 refinement was skipped because no D8 auxiliary was declared. Reported engine area was 35,571.41890155664 km²; this is an observation of invalid output, not an authoritative target area.

Both WKB and GeoJSON contain identical coordinate sequences. The diagnostic reports:

`Ring Self-intersection[9.47912155386593 47.9047317771196]`

Prior read-only investigation found eight nonadjacent repeated-vertex pairs in the main exterior and endpoint touches, rather than proper crossings. Five other polygon parts were individually valid. The reported point is about 147 km from the query, so an outlet-only investigation would miss the failing geometry.

Additional private reports beside `tdx-run/`:

- `geometry-investigation/geometry-findings.md`
- `geometry-investigation/geometry-report.json`
- `geometry-investigation/geometry-local-detail.json`
- `source-path-investigation/report.md`

These reports and retained artifacts provide evidence, not a proved root cause. They are local resources and are not guaranteed to exist for another machine or checkout. Do not copy private operational artifacts or credentials into the repository indiscriminately.

## Investigation and implementation requirements

Establish the first invalid stage using the real selected input geometries and intermediate results: stored input, dissolve, cleanup, or subsequent assembly. The retained cache lacks source catchment geometry and stage intermediates, so final-output inspection alone cannot resolve attribution.

Read-only access to the pinned compiled HFX dataset on Hetzner and controlled Basel reruns are authorized. Preserve remote artifacts and original local evidence. Use fresh output locations for new evidence. This authorization does not extend to modifying the source delivery or restarting the wider comparison study.

Before changing production code, add and demonstrate a failing regression that exercises the actual failing path. A mocked proxy or only an after-the-fact passing test does not meet this requirement. Correct the demonstrated cause, then make the same regression pass. Choose fixture size, instrumentation, and implementation mechanisms from evidence.

Prevent invalid final geometry from silently succeeding. A full geometry-validity check must cover the observed endpoint self-touch class, not merely proper segment crossings. If a result cannot be made valid under the intended geometry semantics, fail explicitly rather than silently discard features or guess. This safeguard does not replace successful correction of Basel.

## Relevant source findings to verify

The prior investigation inspected the pinned revision. Reconcile these findings with the implementation checkout before relying on line numbers or behavior:

- `crates/core/src/engine.rs` runs staged outlet resolution, traversal, pre-merge materialization, refinement, dissolve, and result composition.
- `crates/core/src/assembly.rs` assembles polygons through dissolve, cleanup or an explicit repairer, hole handling, winding normalization, and area computation. The inspected default path checked emptiness and area, not full topology validity.
- `crates/core/src/algo/dissolve.rs` uses spatial sorting and a deterministic pairwise union path.
- `crates/core/src/algo/clean_topology.rs` assumes CCW exteriors and CW holes when offsetting. The pinned overlay dependency emits the opposite winding, while assembly normalizes winding after cleaning. This is a concrete static mismatch, but its causal role in Basel remains unproved.
- Cleanup can retain only the largest fragment from an individual self-union. Inspect this behavior against the footprint-preservation requirement rather than assuming discarded parts are harmless.
- `crates/core/src/algo/watershed_geometry.rs` encodes processing order; its cleaned state was not proof of OGC validity.
- `crates/core/src/algo/self_intersection.rs` was unused in the default path and detected proper crossings only. It cannot establish validity for this endpoint-touch failure.
- Python default repair behavior uses the Rust cleaner. Explicit GDAL repair is a different path. Requiring the caller to select a workaround does not fix the default behavior.
- Normal export writes the result coordinates. Existing WKB/GeoJSON agreement does not support a serialization-only explanation.

Input invalidity, overlay precision, and cleanup behavior remain possible contributors. Do not label the delivered HFX source invalid based on the final watershed alone. Follow `../hfx/spec/HFX_SPEC.md` as the canonical input contract and keep source-fabric-specific logic out of the runtime hot path.

## Acceptance evidence

- A failing-before/passing-after regression through the demonstrated failing path.
- A controlled default Basel rerun against the pinned dataset producing a nonempty valid watershed without requiring a caller workaround.
- Independent validity verification of exported WKB and GeoJSON, including endpoint self-touches and the full MultiPolygon relationships rather than only crossing checks.
- Evidence locating the first failing stage and explaining why the fix addresses it.
- Measured boundary and area differences with an explanation of necessary changes. Do not use invalid prior output as unquestioned geometric truth; relate the corrected footprint to the selected inputs and intended processing semantics.
- Confirmation that outlet resolution and upstream selection were preserved, or a clearly evidenced explanation if the investigation reveals a relevant defect there.
- Relevant regression coverage and workspace validation using the repository's native commands: `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace`. Report any validation that cannot run and why.

## Boundaries

Do not modify the compiled HFX dataset, original evidence, or existing delivery receipts. Do not introduce TDX-specific runtime exceptions, change unrelated snapping or refinement policy, or silently relax validity criteria. Publication of datasets or study results and resumption of the broader comparison study are excluded. Repository delivery mechanics remain governed by the explicitly invoked implementation workflow.

Follow repository design, error-handling, and journaling instructions. No implementation or further workflow is authorized merely by creation of this draft.
