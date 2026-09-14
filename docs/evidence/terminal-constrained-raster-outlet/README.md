# Terminal-constrained raster outlet selection evidence

Vision: [2026-09-14 terminal-constrained raster outlet selection](../../../planning/visions/2026-09-14-terminal-constrained-raster-outlet-selection-after-vector-snapping.md).
Base: `78839dec936241d8df9b6f822dc299e3011e5f60`.

## Regression-first proof

Commit `84e7e20` adds real Engine-path regressions before production changes.
`cargo test -p pourpoint-core --test staged_delineation ranked_terminal -- --nocapture`
ran nine cases: seven failed and two controls passed. The [original output](engine-regressions-red.txt)
records incorrect containing-cell seeds or explicit vector guards, not fixture-loading failures.
The same cases now pass. Added cases cover disabled refinement and all-unusable D8.

Separate real-result reporting regressions proved missing GeoJSON selected-center
fields before the additive CLI/Python fixes: [CLI red](cli-reporting-red.txt),
[Python red](python-reporting-red.txt). New `refined_lon` and `refined_lat` fields
are null unless refinement applied. Original and resolved coordinates retain their meanings.

## Seed and geometry provenance

- A controlled 5×5 raster has independent south-flowing columns. The terminal
  includes rows and columns 1 through 3. The expected carve is exactly the selected
  column from row 1 through the selected seed, assembled with the unchanged whole
  upstream rectangle. Tests assert symmetric-difference geometry and direct/staged
  canonical WKB parity, not only outlet coordinates.
- The boundary reference `(2.0, -2.5)` now chooses center `(1.5, -2.5)` by higher
  accumulation instead of the containing center `(2.5, -2.5)`. A full four-cell tie
  at `(2.0, -2.0)` chooses `(1.5, -1.5)` by row-major order.
- An original input outside the selected terminal and its vector reference favor
  different cells. The vector reference wins. External high-accumulation cells,
  unusable flow cells, and non-finite accumulation are excluded. External finite
  references remain valid, without clamping or a raster radius.
- Missing upstream accumulation does not remove valid upstream flow cells from
  tracing. Candidate and trace masks are separate. GRASS sinks and signed coverage
  exits remain usable; header nodata remains excluded.
- All historical golden and evidence files remain byte-for-byte unchanged.
  Existing synthetic and projected-GRASS parity fixtures resolve by containment,
  so their ranked seeds, canonical WKB, area, and upstream sets remain unchanged.
- The generated donut fixture previously used undefined ESRI code `0` at its sole
  threshold-qualified seed `(row 0, column 0)`. It now uses code `64` (north), a
  valid coverage exit. This is an in-memory test fixture, not a historical raster.
  Its exact ring/hole corner assertions are unchanged and pass.

## Validation

- `cargo check --workspace`: passed.
- `cargo fmt --all -- --check`: passed ([receipt](format.txt)).
- `cargo test -p pourpoint-core --lib algo::`: 305 passed.
- Focused core library, staged, D8 auxiliary, and D8 parity tests: 623 library,
  28 staged, 30 auxiliary, and 6 parity tests passed; 4 library tests ignored.
- `cargo test --workspace --exclude pourpoint-python`: 856 passed, 15 ignored,
  including Rust doctests and CLI/GDAL coverage.
- `cargo clippy --workspace --all-targets`: passed; existing test-code warnings.
- `cargo doc --workspace --no-deps`: passed; eight existing invalid-HTML-tag
  warnings in unrelated module denotation lines.
- Installed native debug wheel: all Python tests, 51 passed and 8 skipped.
- New Python regression file: Ruff check and format check passed
  ([receipt](python-style.txt)).
- CLI real-result reporting regression: passed.
- Ignored GDAL `synthetic_b_tiff_matches_gdal` explicitly executed: passed.
- Strict MkDocs build: passed using an isolated uv cache.
- Public-docs claims/local links, five checker tests, and reader-floor audit: passed
  ([receipt](docs-audit.txt)).

Plain `cargo test --workspace` was attempted and failed at the macOS arm64
`pourpoint-python` lib-test linker with unresolved `_Py*` symbols under the
extension-module build configuration. No Rust assertion failed in that attempt.
The remaining workspace and installed-wheel tests above provide native validation;
no build configuration was changed to hide the host limitation.

## Real-data limits

The existing current-HFX MERIT Rhine/Basel metadata record remains historical.
Its vector point already coincides with the selected raster center, so it does
not discriminate the new misalignment behavior. Available standard MERIT caches
use obsolete HFX/D8 schemas; the available current GRIT cache has no D8 pair.
Existing cached real MERIT TIFF windows cannot drive the current Engine without
compatible catchment and snap inputs. The historical recapture command also opens
an obsolete remote dataset. No dataset was rewritten, no licensed data was
redistributed, and no downstream study was executed.

Controlled fixtures provide deterministic misalignment proof. Projected GRASS
parity exercises actual TIFF decoding and the Engine projection path but is a
synthetic conformance fixture, not a new field-data accuracy claim.

## Durable receipts

- [Focused green](focused-green.txt)
- [Native workspace green](workspace-native-green.txt)
- [Plain workspace linker limitation](workspace-host-linker-failure.txt)
- [Python green](python-green.txt), [CLI green](cli-green.txt)
- [GDAL synthetic parity green](gdal-synthetic-green.txt)
- [Strict docs build](docs-green.txt), [Clippy](clippy.txt), [Rustdoc](rustdoc.txt)
