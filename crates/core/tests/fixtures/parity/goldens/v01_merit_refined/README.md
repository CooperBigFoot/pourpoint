# Oracle C - merit-basins/0.1.0 Refined Goldens

> **Archived evidence:** Oracle C is an immutable HFX v0.1 capture. Its public
> source was intentionally deleted because MERIT redistribution is prohibited.
> PR #72 and commit `60e4a55` record that decision; both historical manifest URLs
> now return 404. The old harness at commit `b25575f` is implementation history,
> not a supported network route. Do not refresh this file or claim an outage.

These goldens are captured from:

```text
https://basin-delineations-public.upstream.tech/merit-basins/0.1.0/
```

The capture path opens the remote dataset, attaches `LocalTiffRasterSource`, and
then runs the unmodified `Engine::delineate()`. During refinement, the engine
calls `DatasetSession::localize_raster_window()` for the terminal bbox; the COG
reader range-reads only intersecting remote `flow_dir.tif` and `flow_acc.tif`
tiles into `HFX_CACHE_DIR`, then `LocalTiffRasterSource` reads those local
windows for the actual carve.

Live Step 4 accepted oracle:

- `rhine_basel` was evaluated at `GeoCoord::new(7.5890, 47.5596)` with a
  `5000 m` search radius.

Real-data D8 parity is achieved through `rhine_basel`. The
`mekong_phnom_penh` candidate at `GeoCoord::new(104.9300, 11.5700)` with a
`5000 m` search radius remains deferred and is not a required or gating golden.
After the deterministic dissolve fix, it still showed residual run-to-run
canonical-WKB drift at continental scale. The suspected source is downstream of
pourpoint's dissolve path, likely floating-point nondeterminism in
`geo::BooleanOps::union`; this is tracked as follow-up work rather than part of
the Step 4b gate. The durable artifact test requires only C `rhine_basel`; it
must not treat Mekong as a missing required case.

The accepted C record asserts the public-result invariants available from
`DelineationResult`: `RefinementOutcome::Applied`, finite `refined_outlet`
inside the terminal-unit bbox, non-empty final watershed geometry, positive
finite geodesic `area_km2`, three-run canonical WKB/scalar stability, and
window byte counts under the 500 MB per-outlet ceiling. The golden records do
not store a refined-terminal sub-polygon metric because `DelineationResult`
exposes only the final assembled watershed geometry. Terminal-carve containment
was independently verified during the Step 4 investigation; see
`docs/hfx-v02-redesign/m1-step4-c-investigation.md`. Terminal carve behavior is
also covered by the `refine.rs` unit tests.

MERIT raster contract recorded in the JSON:

- Remote COG source, localized to plain north-up EPSG:4326 GeoTIFF windows
- PixelIsArea raster interpretation; refinement uses pixel centers
- ESRI D8 flow-direction encoding
- `uint8` flow direction with `255` nodata
- `float32` accumulation with source nodata decoded as `NaN`

`localize_raster_window()` is `pub(crate)`, so the GDAL proof cannot call it
directly from `pourpoint-gdal`. The proof first materializes windows by running the
core capture delineations, then reads the cached `.tif` files through both
`LocalTiffRasterSource` and `GdalRasterSource`. For the blessed `rhine_basel`
window, it verifies matching tile geotransforms, sample values, nodata handling,
and direct terminal-carve output. The C oracle is therefore scoped as: core TIFF
reader carve proven tile-identical to the GDAL production decode for the
localized C window.

This M1 oracle records inert v0.1 behavior and historical remote identity only;
offline comparison must not fetch or re-hash its remote artifacts. Current
real-data evidence is separate under
`crates/gdal/tests/fixtures/merit-current/` and comes from licensed local HFX.

M1 already proved TIFF-vs-GDAL tile identity for the accepted `rhine_basel`
windows. M4 may reuse the synthetic B proof for byte-identical B rasters, and
may re-run the C proof if the reader implementation changes.

Current local-HFX recapture command:

```bash
POURPOINT_MERIT_RECAPTURE_ROOT="$LICENSED_MERIT_HFX_ROOT" \
POURPOINT_MERIT_RECAPTURE_OUTPUT="$MERIT_EVIDENCE_OUTPUT" \
POURPOINT_MERIT_RECAPTURE_HFX_COMMIT=<full-hfx-builder-commit> \
POURPOINT_MERIT_RECAPTURE_BLESS=1 \
cargo test -p pourpoint-gdal --test merit_local_recapture -- --ignored --nocapture
```

The target rejects stale D8 v1 input and writes no absolute source root. It
records the manifest adapter version and caller-supplied full HFX builder commit.
It creates separate current evidence and never rewrites this archived oracle.

Decode proof command, after capture has populated `HFX_CACHE_DIR`:

```bash
# Archived only: the old network-backed MERIT C decode proof cannot be rerun.
```
