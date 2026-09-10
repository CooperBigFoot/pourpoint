# Regional watershed area correction evidence

Vision: [regional watershed area correction](../../visions/2026-09-10-regional-watershed-area-correction.md).
Base: origin/main after publication PR #158. Implementation changes only the
shared area calculation, its documentation, tests and attributed data fixture.
No geometry, dataset, snap, traversal, refinement or hole-fill policy changes.

## Before production correction

`regression-before.txt` records an actual compiled failing regression through
`decode_wkb_multi_polygon`, `geodesic_area` and the captured coordinates.
Component 1 reported 510065621.72408843 km², signed -8.50953915687569e-6 m²,
unsigned 510065621724088.44 m². The test explicitly proves the unsigned
complement branch before checking the public result. Component 8 is also
retained with exact bytes. `behavior-before.txt` adds failing winding, hole and
antimeridian cases. `complete-before.txt` reproduces the complete original
GRIT scalar 1020167146.7558798 km². The production-file hash before editing is
retained in `production-before.sha256`.

Commands, in that order, all exit 101 on the unmodified production code:

```sh
cargo test -p pourpoint-core --test regional_watershed_area -- --nocapture
cargo test -p pourpoint-core --test regional_watershed_area -- --nocapture
cargo test -p pourpoint-core --test regional_watershed_area captured_complete_watersheds -- --ignored --nocapture
```

## Complete replay and tolerances

Set `POURPOINT_GRIT_WKB` and `POURPOINT_TDX_WKB` to the retained original
`grit-run/watershed.wkb` and `tdx-run/watershed.wkb` paths in the vision's
private operator archive, then run:

```sh
cargo test -p pourpoint-core --test regional_watershed_area -- --include-ignored --nocapture
```

`regression-after.txt` records all six passing tests, including full GRIT
35903.30770278959972 km² (9 parts, no holes) and TDX
35571.42020080587827 km² (7 parts, no holes). The complete replay is ignored
in ordinary CI and requires explicit paths, with no private-directory default.
No remote delineation was repeated: these are unchanged saved-output replays.
`input-integrity.json` matches the original investigation's saved hashes.
The test compares every decoded coordinate before and after area measurement.
The area functions receive immutable geometry and perform no geometry changes.

Full replay uses an absolute 1e-7 km² (0.1 m²) tolerance. This is over six times
the largest retained Rust/independent-pyproj difference (TDX: 0.01556 m²;
GRIT: about 0.004 m²), but negligible against regional area and completely
separates the demonstrated Earth-sized complements. Synthetic WGS84 references
were checked with pyproj 3.7.2: 12308.77836146945 km² for the equatorial shell,
9231.614224814872 km² after the centered hole, and 24619.443759277194 km² for
the short-edge antimeridian rectangle. They use the same 0.1 m² bound.
Captured tiny components require negative signed magnitudes below 0.001 m²,
unsigned area above 5e14 m², and corrected area below 0.001 m². Exact floating
point equality is not an area acceptance condition.

## Semantics and compatibility

Both public helpers use the absolute signed area of each polygon independently,
including hole subtraction. A shared private helper rejects nonfinite results
and a negative shell-minus-hole magnitude before absolute value. No caps,
filters, projected approximation or fabric-specific branches are introduced.
The extra shell measurement is only needed for polygons containing holes.

The helpers interpret rings as minor/regional interiors smaller than half
Earth. HFX does not guarantee that bound. Signed reduction cannot detect
intended major interiors. Short-edge antimeridian coverage applies only to
these area helpers, not planar assembly, validation or snapping.

Empty MultiPolygon remains an error. Single empty Polygon (also when contained
in a MultiPolygon) retains its existing zero result; assembly separately
rejects empty final geometry. NaN and both infinities in shells and holes remain
errors. Hole subtraction assertions now require the correct positive number.
`HoleAreaExceedsShell` is a new public error variant: callers with exhaustive
`WatershedAreaError` matches must add this arm (or propagate the error).
Existing function signatures, AreaKm2 carrier and prior variants are unchanged.
This guard detects area inconsistency only, not arbitrary invalid topology.

## Fixture and license

Only the 163-byte reduced captured WKB is committed, under separate
[CC BY-NC-4.0 attribution](../../../crates/core/tests/fixtures/regional-area/README.md),
not the engine MIT license. Complete GRIT and TDX geometries remain private.
The fixture is not runtime data. Source users must respect its NonCommercial
terms; the engine's MIT terms do not grant commercial rights to this data.

To reconstruct the fixture, parse the original little-endian two-dimensional
MultiPolygon WKB. Copy the complete 77-byte Polygon slices at zero-based
positions 1 and 8. Prepend `struct.pack("<BII", 1, 6, 2)`. No coordinate bytes
are modified. The README records source and reduced hashes, creators,
compilation identity, derivation and upstream license references.

## Naming and checkout scope

Production responsibility remains `watershed_area`; the new private helper is
`regional_polygon_area_m2`. No delivery-shaped production architecture is added.
Unrelated historical COG milestone/test labels are not renamed.
No additional Git checkout was created. A test-only Python venv is below
`.worktrees/visions/regional-watershed-area-correction/test-env/`; it is not
committed. Original private evidence was read, never overwritten.

## Repository checks

- `cargo fmt`: exit 0; `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets`: exit 0; existing unrelated test
  warnings remain (retained in `clippy.txt`).
- `cargo test --workspace`: exit 101 at the known macOS PyO3 extension-module
  executable link step, not a failing assertion (`workspace-plain.txt`).
- Assisted workspace test: exit 0, **855 passed, 15 ignored**, 33 result groups
  (`workspace-assisted.txt`). This includes all ordinary area regressions;
  complete operational replay is run explicitly and separately.

The assisted invocation uses a project test venv created with `uv venv --python
3.11`, with `pyyaml` installed for the reader-floor subprocess test. `pyproj`
was installed only for independent numeric-reference checks. The matching
uv-managed CPython base installation supplies the link library:

```sh
PATH="$VIRTUAL_ENV/bin:$PATH" \
RUSTFLAGS="-C link-arg=$PYTHONHOME/lib/libpython3.11.dylib" \
PYO3_PYTHON="$VIRTUAL_ENV/bin/python" PYTHONHOME="$PYTHONHOME" \
cargo test --workspace --features pyo3/auto-initialize
```

`VIRTUAL_ENV` is the test-env path above, `PYTHONHOME` is the matching CPython
3.11 base installation, not the venv. No production features or loader behavior
were changed for this host workaround. The retained plain-command failure
precedes this assisted validation.

Committed command logs normalize only trailing whitespace and blank EOF lines.
Unmodified raw logs remain in the local test workspace `raw-logs/` directory.
