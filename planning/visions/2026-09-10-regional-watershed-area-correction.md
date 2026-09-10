# Regional watershed area correction

## Outcome and authority

Correct [pourpoint issue #157](https://github.com/CooperBigFoot/pourpoint/issues/157): reported area must represent the returned regional watershed on the WGS84 ellipsoid, without Earth-sized complement contributions from tiny components.

The user explicitly chose to keep the fix regional after a read-only investigation. Preserve returned geometry, including tiny components, and preserve snapping, traversal, refinement, hole-fill policy, and source datasets. Apply the correction consistently to shared single-polygon and multipolygon area calculations, not as a GRIT-specific workaround.

This is a local standalone vision draft. It has not been published or verified on the target branch. Before substantive implementation, `implement-vision` must publish and verify this new standalone draft under its target-branch durability gate. This document does not itself authorize automatic implementation.

## Proven failure and investigation

The reported run used commit `f268c70b6f94bb2a404360cb395686a7dc865f89`, geo 0.33.1, and geographiclib-rs 0.2.7. The dataset was `https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/`. Python `Engine(uri).delineate(47.5596, 7.5890)` used unchanged defaults. It returned 4,825 upstream units, terminal 13883878, and resolved longitude/latitude 7.588848216303331, 47.560422729739486.

The saved result is a valid, nonempty nine-part MultiPolygon with no holes. All shells are planar CCW. WKB and GeoJSON agree. The original reported area is **1,020,167,146.7558798 km²**; independent WGS84 measurement is **35,903.30770278559 km²**.

An external Rust diagnostic project replayed the unchanged saved WKB through the production decoder and `geodesic_area_multi`. It reproduced the original scalar exactly. Zero-based components 1 and 8 had these measurements:

| Component | Signed area, m² | Unsigned area, m² |
|---|---:|---:|
| 1 | -8.50953915687569e-6 | 510065621724088.44 |
| 8 | -1.6738187241571723e-5 | 510065621724088.44 |

Each tiny negative signed result becomes one Earth surface in the unsigned branch. This establishes exact Rust component attribution, not just an inference from Python. Independent pyproj had different near-zero signs; do not use it to infer the Rust component identities.

A diagnostic sum of absolute signed polygon areas gives **35,903.3077027896 km²**, approximately 0.004 m² from the independent WGS84 measurement. This was not installed as a production fix. The same-build TDX control reproduced **35,571.42020080588 km²**, unchanged under the diagnostic expression.

At investigation time, `watershed_area.rs`, `wkb.rs`, and `Cargo.lock` had no diff against the reported commit. The probe dependency lock introduced no new package identity except the probe itself. No production source or tests were changed and no full remote delineation was rerun.

## Relevant code and semantics

- `crates/core/src/algo/watershed_area.rs` calls `geo::GeodesicArea::geodesic_area_unsigned` for polygons and multipolygons. It rejects empty multipolygons and non-finite results.
- `crates/core/src/assembly.rs` dissolves, cleans or repairs, fills holes, canonicalizes, validates, then measures the returned geometry with `geodesic_area_multi`.
- `crates/core/src/algo/watershed_geometry.rs` normalizes planar winding. Planar winding and validity do not guarantee positive computed geodesic signed areas for near-degenerate rings.
- `crates/core/src/algo/wkb.rs` provides the production decoder, accepting `hfx::WkbGeometry` rather than raw bytes.
- geo's unsigned operation is a complement operation, not an absolute value. Its signed polygon operation already subtracts absolute hole areas with the shell's sign.
- geo's multipolygon operation sums constituent results. Taking `abs()` after summing signed polygon areas can cancel components with opposite winding. Regional measurement must aggregate polygon magnitudes independently while retaining hole subtraction.
- A diagnostic CCW-hole example produced an Earth-scale negative unsigned result. The existing `polygon_with_hole` test only asserts that area is less than the full shell, so that incorrect negative result can pass. Numeric positive-area assertions are required.

Use the narrow regional interpretation supported by the investigation: absolute signed area per polygon, including hole subtraction, then sum. Leave helper structure and other reversible implementation choices to the implementing agent. Do not replace the computation with area caps, component deletion, geometry repair, a projected approximation, or source-fabric branching.

## Regional boundary

Document minor/regional interior semantics: rings are interpreted as regional interiors smaller than half Earth, rather than intended major interiors covering most of the globe. The canonical HFX contract remains `../hfx/spec/HFX_SPEC.md`; it does not currently guarantee a hemisphere-size bound. Do not claim otherwise or change the upstream contract as part of this fix.

Signed magnitudes alone cannot detect whether a caller intended a major interior because the dependency already reduces signed areas to a half-Earth interval. Do not invent a threshold check that claims to prove that intent. General major-interior semantics and complete engine antimeridian support are excluded. A short-edge antimeridian area-helper regression does not certify planar assembly, validation, or snapping across the dateline.

The cause of the tiny geometry fragments and the separately reported skipped refinement (`vector_outlet_guard_failed`, `outside_terminal_mask`) were not established by this investigation. Neither needs to change to correct the demonstrated scalar. Their investigation is outside this vision.

## Acceptance evidence

Before changing production code, add and run a failing regression using captured geometry through the actual Rust decoder and public area path. Preserve the failing result as evidence. A reduced fixture is acceptable if it retains the exact failing coordinates and proves the actual complement path; do not replace the bug with a mocked proxy. Arrange a reproducible fixture with appropriate source attribution rather than depending silently on an operator's private home directory.

After the correction:

- The retained complete GRIT geometry measures approximately 35,903.30770279 km², with a justified numerical tolerance rather than platform-sensitive exact equality.
- No components or coordinates are removed, rounded, repaired, or otherwise changed to obtain that scalar.
- The captured TDX control remains consistent with 35,571.42020080588 km².
- Both public area functions have consistent regional semantics.
- Tests cover tiny negative components, reversed shells, mixed-winding disjoint polygons, hole subtraction with both winding directions, and short-edge or split antimeridian regional cases at the area-helper level.
- Hole assertions require the correct positive magnitude, not merely an upper bound. Keep empty and non-finite error behavior covered. Do not silently turn invalid computations into zero or hide shell/hole inconsistencies.
- Run the repository checks: `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo test --workspace`. Follow local AGENTS.md requirements, especially regression proof before fix and structured errors.

## Retained investigation evidence

Private operator archive, available during discovery:

`~/.local/share/hfx/operations/2026-09-09-tdx-delivery/basel-study-fixed/`

- `grit-run/watershed.wkb`, `watershed.geojson`, and `metadata.json`: unchanged original GRIT output.
- `tdx-run/`: unchanged same-build control.
- `actual-output-audit/README.md`, `geometry-audit.json`, and `analyze.py`: independent measurements and provenance qualifications.
- `area-rust-investigation-157/README.md`, `probe.rs`, `Cargo.toml`, `Cargo.lock`, `command.txt`, `run.txt`, `results.json`, and `input-sha256.json`: exact Rust replay, synthetic diagnostic cases, and input hashes.

The probe ran outside the repository. To reconstruct it, put `probe.rs` at `src/main.rs` in a separate Cargo project and adjust the retained manifest's absolute path dependency and input paths. The original probe temporary directory is not a durable dependency. If the private archive is unavailable, report the missing regression evidence rather than claiming the exact captured case has been verified.
