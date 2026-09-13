# Source build evidence

Source checkout: `c172e04a71bdca3a8eb09cd7227f90b1e38fe1f3` from origin/main.
Command: `uv pip install --python .venv-study/bin/python ./crates/python shapely fiona pyproj matplotlib cartopy boto3 python-dotenv pytest requests`.
Result: exit 0. uv reports `Built pourpoint @ file://.../crates/python`.
Build used the repository Cargo target cache; cached artifacts are not run evidence.
The package embeds version 0.3.0. The actual source revision and compiled extension
SHA256 in provenance identify the tested main-only engine, including geodesic
area and geometry fixes. `DelineationResult.refinement_skip_reason` is present.

Initial probe: one engine cold-open 77.983492 seconds; station 11001 delineation
12.435688 seconds, area 55894.018688873206 km², 5034 upstream units, valid geometry.
Remote schema diagnostics: no unreadable auxiliaries.
Main batch reused one engine after the timing probe; opening from metadata cache
was 4.768120 seconds. Probe result was reused, not delineated again.
