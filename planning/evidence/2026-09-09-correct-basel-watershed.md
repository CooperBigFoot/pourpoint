# Basel watershed geometry correction evidence

Vision: [Correct Basel TDX-Hydro watershed geometry](../visions/2026-09-09-correct-basel-tdx-hydro-watershed-geometry.md).

## Result

The default Python call `Engine(uri).delineate(lat=47.5596, lon=7.5890)` now
returns a nonempty, valid watershed against the vision's pinned compiled HFX
source. No GDAL repair option, source-specific branch, smoothing increase, or
fragment-area cutoff is required.

- Terminal: **230303076**, unchanged.
- Upstream IDs: **4,032**, identical sequence to the retained original run.
- Resolved outlet: **(7.588708449410564, 47.56112089110772)**, exactly unchanged.
- Refinement: best-effort skipped because no D8 auxiliary was declared, unchanged.
- Engine area: **35,571.42020080588 km²**.
- Result: **7 polygon parts, 0 holes**. Both WKB and GeoJSON independently pass
  GEOS 3.13.1 full validity and have identical coordinate sequences.
- Output WKB SHA256:
  `32ff5570875e79459b0102b815b4b02f2af98ee12106457a490d6bbaaaa95fc6`.

## First invalid stage and failing-before proof

The 4,032 original selected geometries were captured by conditional read-only
Parquet range requests. Every stored geometry is individually nonempty and
GEOS-valid. Only five geometry row groups were fetched, not the 108 GB object.
The manifest hash was verified before and after retrieval and controlled runs:
`af443be357742550ea76ef774b83a1f86e683ea828bb796c9ca893425777be85`.
Remote artifacts, original local evidence and delivery receipts were not changed.

A test-only replay reads those exact WKB inputs in unit-ID order and calls the
production `assemble_from_geometries` path. It also captures production dissolve,
cleanup and hole-fill intermediates. Before production correction:

| Stage | GEOS validity | Parts | Holes |
|---|---|---:|---:|
| Stored selected inputs | All 4,032 valid individually | Per input | Per input |
| Dissolve | Ring self-intersection at (9.49722428914692, 47.8892873419403) | 7 | 53,877 |
| Cleanup | Same reported self-intersection | 6 | 29,597 |
| Hole fill / final | Ring self-intersection at (9.47912155386593, 47.9047317771196) | 6 | 0 |

**Dissolve is the first invalid stage**, not serialization or outlet refinement.
The reproduced final WKB is byte-identical to the original retained invalid
watershed, SHA256 `ad972004033b6119cdea04c58368b125b23b8be6fcb5198f243ae29c7effbd03`.
This is stronger than matching only the reported validity reason or area.

```text
cargo test -p pourpoint-core --lib captured_catchments_produce_valid_watershed -- --ignored --nocapture
before: exit 101, independent GEOS assertion fails at the observed Basel self-touch
 after: exit 0, identical test and selected-input set
```

The explicit replay requires `POURPOINT_GEOMETRY_INPUTS` (directory of `<id>.wkb`),
`POURPOINT_GEOMETRY_OUTPUT` (a nonexistent, fresh directory), and
`POURPOINT_VALIDATION_PYTHON` (an interpreter with Shapely). It is ignored in
ordinary CI because operational geometry is not committed. The portable
`point-tangent-hole.wkb` fixture is synthetic and independently GEOS-valid.
Its default assembly regression also failed before correction: its point-tangent
hole became an exterior pinch, leaving area 14 rather than the hole-filled 16.

Additional actual-path red/green regressions cover winding-independent closing,
a neck splitting into multiple retained lobes, filled-hole/island union, and an
explicit repair returning `MULTIPOLYGON(EMPTY)` that previously succeeded with
area zero. Separate failing logs preceded each correction. The final guard also
has endpoint-touch, shared-edge, overlap and disconnected-interior coverage.

Sequencing record: an independent worker wrote an unwired validity module before
the captured Basel replay ran. It was isolated, and its Cargo/module changes were
restored from `b87a92e` before the qualifying replay. That first compilation
failure is not counted as regression evidence. The preserved baseline patch has
only test additions and the synthetic fixture; original production behavior is
also confirmed by byte-identical output. Pending source and failure logs remain
in the private evidence directory.

## Correction and causal experiments

1. `geo` 0.29.3 / `i_overlay` 1.9.4 can encode a tangent hole as a pinched exterior.
   `geo` 0.33.1 / `i_overlay` 4.5.2 BooleanOps enables OGC contour reconstruction.
   The spatial sort and deterministic pairwise dissolve tree are preserved.
   Updating only this dependency made the real dissolved geometry GEOS-valid.
2. A valid final result from the dependency-only experiment was **not accepted**.
   The old cleaner's largest-fragment choice lost five real source parts totaling
   about 645.64 m². Relative to that rejected output, the final correction adds
   645.766570 m² and loses only 0.019301 m², isolating the restored fragments.
   The attempted manual-offset correction also left four inward
   slits totaling about 221.38 m². These intermediate results are retained.
3. Replace the hand-written offset and largest-fragment selection with maintained
   polygon buffering. Keep the same epsilon and equivalent mitre limit five
   (`2 asin(1/5)` minimum corner angle). Normalize winding on both passes and
   retain every noncollapsed offset component. No round joins are introduced.
4. Buffer mesh output can itself have endpoint pinches. OGC reconstruction runs
   after **each** offset, before another offset or hole handling. A real Basel
   run without this reconstruction was rejected by the new final guard.
5. Keep the hole policy unchanged. After filling holes, pairwise union is used
   when needed so covered island polygons do not overlap their filled shell.
6. Check nonemptiness and full MultiPolygon validity after hole handling and
   winding normalization, before area calculation. The same guard applies to
   explicitly supplied repair backends. A finite positive area is not proof of
   topology validity.

The final captured stages are all GEOS-valid. Dissolve has 7 parts and 53,885
holes; closing has 7 parts and 8 holes; the default hole policy yields 7 parts
and no holes. The many near-zero overlay holes are precision artifacts, not
53,885 authoritative source holes: the independent GEOS union has only 8 holes.
Closing restores that source-scale structure without deleting source parts.

## Footprint measurements

Reference geometry is an independent GEOS set union of the exact selected
inputs. It is valid, has 7 parts and 8 holes. For the default policy, remove all
reference holes and set-union the filled parts. This reference has 7 parts and
no holes. The invalid original is **not** used as authoritative footprint truth.

Areas below use **EPSG:3035** projected geometry:

| Comparison: final minus reference | Added m² | Lost m² | Net m² |
|---|---:|---:|---:|
| Selected-input union, holes retained | 1,410.467702 | 143.716421 | 1,266.751281 |
| Selected-input union, existing default hole policy applied | 111.082513 | 143.716422 | -32.633908 |
| Original output represented separately by GEOS make_valid(linework) | 1,299.414286 | 0.164849 | 1,299.249437 |

The independently computed WGS84 geodesic area is 35,571.42020079032 km².
Its tiny difference from the engine area is numerical geodesic accumulation,
not a different geometry. The default-policy reference geodesic area is
35,571.42023337309 km².

All six small source components remain present. Their combined lost area is
**0.019894 m²**, from finite-precision boundary movement, rather than removal.
The corrected result retains all seven source polygon regions. The 24.7 m inward
slit observed in the rejected manual-offset experiment is absent.

Against the default-policy reference, the densified discrete boundary Hausdorff
measurement is **0.003063592 m**. Boundaries are projected to EPSG:3035 and divided
into segments no longer than 1 m in the final supplemental measurement (about
2.04 million samples per boundary). Each sample is measured against the exact
opposing projected segments, in both directions. This is a sampled measurement;
the conservative continuous upper bound is **0.503063592 m**. The original 10 m
sampling measurement and its wider bound are retained separately. Against
the hole-preserved reference, the 32.59 m boundary difference includes intentional
hole removal. Geometry accuracy below the overlay's extent-dependent integer
grid is not guaranteed by validity alone.

## Compatibility and validation

All direct workspace geo dependencies move together. A narrow parallel overlay
adapter could not resolve with the old dependency graph: same-major `i_float`
and `i_shape` dependencies have incompatible tilde ranges. Avoiding the upgrade
would require maintaining a source-isolated three-crate fork. Public
Polygon/MultiPolygon carriers remain geo-types 0.7. The one required call-site
API adaptation is `Geodesic::distance` to `Geodesic.distance`; ranking policy is
unchanged. The geo dependency MSRV is now Rust 1.88; validation here used
Rust/Cargo 1.98. No repository toolchain or lower MSRV promise was declared.

The original projected-GRASS golden remains unchanged. A separate
`projected_grass_refined_ogc.json` records the corrected assembly. Exact-byte
runtime comparisons and existing tolerances remain enforced. The same input
fixture, terminal, upstream IDs and 374 derived carved cells are preserved,
including twenty-process determinism and GDAL/local raster-source parity.
Independent GEOS comparison confirms that **both historical and new canonical
MultiPolygons are valid**, have 2 parts and no holes. This migration is justified
by the corrected assembly, not by claiming the old canonical golden is invalid.
The canonical symmetric difference is about 0.862746 m² in EPSG:8857, with
boundary Hausdorff about 0.0001232 m after geographic-edge densification. The
engine's unrounded area metadata changes by -3.38260 m²; this is a different
quantity from the area of six-decimal canonical WKB (which changes by about
+0.861624 m² under independent WGS84 geodesic calculation). A separate artifact
regression retains both valid geometries and bounds canonical symmetric
difference to 1e-10 degrees² and densified point-to-segment boundary distance to
1e-8 degrees. It does not weaken the runtime exact-WKB assertions.
The historical synthetic D8 golden still passes unchanged. One exact projected
bounding-box assertion now uses a 1e-12 degree tolerance for a 2e-16 degree
arithmetic change. The unreadable-auxiliary Python test retains its original
1e-9 km² tolerance and has a new exact expected area, differing by 0.031738 m².

- `cargo fmt --all`: pass.
- `cargo clippy --workspace --all-targets`: pass; existing test warnings only.
- Workspace tests: **849 passed, 14 ignored**, 31 test-result groups.
- Python offline tests: **48 passed, 1 skipped, 7 network tests deselected**.
- Predicate-specific tests: 15 passed, including a 100,000-vertex ring.
- Independent GEOS adversarial corpus: 14 topology cases agree, including valid
  multi-ring point contacts and invalid disconnected interiors.
- Controlled default Basel consumer runs: valid WKB and GeoJSON; exact original
  upstream sequence, terminal, resolved outlet and refinement decision retained.

Plain `cargo test --workspace` on this macOS host cannot link its PyO3
extension-module test executable. Matching Python linkage, initialization and
subprocess interpreter selection are needed for the complete workspace run:

```sh
PATH="$VIRTUAL_ENV/bin:$PATH" RUSTFLAGS="-C link-arg=$PYTHONHOME/lib/libpython3.11.dylib" PYO3_PYTHON="$VIRTUAL_ENV/bin/python" PYTHONHOME="$PYTHONHOME" cargo test --workspace --features pyo3/auto-initialize
```

`VIRTUAL_ENV` is the fresh project test environment. `PYTHONHOME` is the matching
uv-managed CPython 3.11 installation, not the venv. PyYAML is present for the
repository's Python reader-floor probe. These are test invocation settings;
production features and loader behavior were not changed to accommodate them.
The plain-command failure and all successful assisted runs are retained.

## Evidence retention and scope

Private artifacts remain under the original operation's fresh sibling directories:

- `basel-geometry-correction-inputs/`: conditional source retrieval, per-input
  GEOS results, independent unions and all measured intermediate comparisons.
- `basel-geometry-correction/`: before/after replay intermediates, red/green logs,
  final default consumer exports and metadata, validity corpus, build identities
  and native validation logs.

The original `basel-study/tdx-run/` and source delivery receipts were not modified.
No source credentials or operational geometries are committed. The only new
checkout is `.worktrees/visions/correct-basel-watershed`; native tests create
short-lived repository probes below that checkout's target directory. Build
outputs are not included in the delivery.

Production names describe geometry responsibilities. The touched assembly and
its directly dependent comments no longer use delivery-component names. Existing
COG milestone labels remain only as unrelated historical regression traceability;
no broad naming cleanup or PCE artifact rename is included.
