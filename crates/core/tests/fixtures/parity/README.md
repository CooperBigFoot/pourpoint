# Parity Golden Artifact Contract

Milestone 1 parity goldens are loader-independent JSON records. Geometry truth is
the `canonical_wkb_hex` field: little-endian 2D WKB emitted by
`pourpoint-core::algo::canonical_wkb_multi_polygon`.

## Canonicalizer

- `canonicalizer_version`: `pourpoint-canonical-wkb-v1`
- Coordinate precision: 6 decimal places (`CANONICAL_WKB_DECIMAL_PRECISION = 6`)
- Coordinate absolute epsilon: `0.000001`
- Ring closure: explicit first vertex repeated as last
- Ring orientation: exterior rings are CCW; interior rings are CW
- Ring start vertex: lexicographically smallest rounded `(x, y)`; duplicate
  rounded coordinates are tied by the full adjacent cyclic vertex sequence
- Hole order: normalized ring bbox, signed area, full rounded vertex sequence
- Polygon/component order: normalized exterior bbox, polygon area, hole count,
  full rounded exterior sequence, then full rounded hole sequences
- Antimeridian-crossing geometries are out of scope for M1 because the selected
  A/B/C outlets are far from +/-180 degrees.

The 6-decimal precision is intentionally coarser than normal f64 operation
noise. M1 goldens require pre-rounding coordinate divergence to remain below
`1e-9` degrees, giving at least a 500x margin below the `5e-7` degree half-step
where a rounded coordinate could flip. Changing this precision changes the
canonicalizer version and invalidates captured goldens.

## Golden Record Fields

- `canonical_wkb_hex`: hex-encoded canonical final geometry WKB
- `area_km2`: scalar area compared with epsilon policy, not byte-exact equality
- `input_outlet`: original outlet coordinate
- `resolved_outlet`: resolved outlet coordinate
- `refined_outlet`: refined outlet coordinate, present only when refinement
  outcome is `Applied`
- `terminal_id`: version-neutral terminal identifier as `i64`
- `upstream_ids`: sorted version-neutral upstream identifier set as `Vec<i64>`
- `resolution_method`: outlet resolution method label
- `resolver_config`: resolver settings, including `search_radius_m`
- `refinement_outcome`: refinement status and optional reason
- `canonicalizer_version`: canonicalizer contract version
- `comparison_policy`: coordinate absolute epsilon plus `area_km2`
  absolute/relative epsilon tied to canonical WKB precision

## Commands

Offline M4 gate:

```bash
cargo build --workspace --exclude pourpoint-python
cargo check -p pourpoint-python
cargo test -p pourpoint-core --test d8_refinement_parity
cargo test -p pourpoint-core --test d8_aux_accessor
cargo test -p pourpoint-core --test parity_golden_artifacts
cargo test -p pourpoint-core --test staged_delineation
```

Network-gated capture and refresh:

```bash
POURPOINT_PARITY_R2_CAPTURE=1 cargo test -p pourpoint-core --test parity_v01_oracle_capture -- --ignored --nocapture
```

Golden refresh is intentionally explicit. Do not regenerate or re-bless M1
goldens during offline comparison work.

Network-gated M4 ambiguity-boundary proof:

```bash
POURPOINT_HFX_V02_REAL_D8_REFINEMENT=1 cargo test -p pourpoint-core --test d8_refinement_parity -- --ignored --nocapture
```

## Synthetic Refined Raster Fixture

`v01_synthetic_refined/` is oracle B's committed v0.1 input fixture. It mirrors
the existing `simple_convergent_5x5` refinement geometry with real TIFF bytes.

- Dimensions: 5 columns x 5 rows for both `flow_dir.tif` and `flow_acc.tif`
- CRS: EPSG:4326
- Transform: north-up GDAL transform `[0, 1, 0, 0, 0, -1]`
- Origin: upper-left PixelIsArea corner `(0, 0)`
- Pixel size: `1 x -1` degrees
- Extent: `x=[0, 5]`, `y=[-5, 0]`
- Pixel interpretation: GeoTIFF `GTRasterTypeGeoKey=PixelIsArea`; pourpoint uses
  pixel centers for raster refinement, so cell `(row=2, col=2)` is
  `(lon=2.5, lat=-2.5)`
- Flow direction samples: one-band unsigned 8-bit, ESRI D8 encoding, nodata
  tag `255`
- Flow accumulation samples: one-band 32-bit float, nodata tag `-1`, decoded
  by readers as `NaN`
- Carve contract: terminal catchment ID `1` is the rectangle
  `(0, -5, 5, 0)`, outlet `(2.5, -2.5)`, snap threshold `500`, and center
  accumulation `800`

M2 must not mutate or move this M1 B fixture in place. The v0.2.1 work creates a
separate `v021_synthetic_refined/` fixture copy and reuses the exact same
`flow_dir.tif` and `flow_acc.tif` bytes. The durable artifact test re-hashes the
committed M1 TIFFs and the converted v0.2.1 TIFF copy so accidental byte drift
in either path fails offline after M2.

The B TIFFs are the deterministic, byte-identical M1-to-M4 parity path. For M4
real-data D8 parity, use `merit/0.2.0`; `merit-basins/0.1.0` is the M1
real-data v0.1 oracle C input, not the M4 v0.2.1 target.

M4 ships exactly one blessed strategy: built-in D8 raster refinement. The pantry
is D8-only. Full aux-to-strategy binding, reverse-DNS aux parsing,
Python-authored strategies, and additional blessed strategies are deferred.

The real carve sequence is:

```text
rasterize -> mask flow-dir + accumulation -> snap -> masked trace -> polygonize
```

There is no clamp, intersection, or cleaning stage in the D8 carve. Final
watershed assembly is always merge-after: pristine pre-merge unit records remain
available for inspection, then final assembly excludes the whole terminal,
inserts the refined terminal geometry, and dissolves. The R3 disagreement is
therefore intentional: pre-merge terminal records can disagree with final
refined geometry and `area_km2`.

M4's real-data D8 proof is now an ambiguity-boundary proof, not a successful
carve assertion. It is both `#[ignore]`d and env-gated, so offline tests compile
it but do not open the network:

```bash
POURPOINT_HFX_V02_REAL_D8_REFINEMENT=1 cargo test -p pourpoint-core --test d8_refinement_parity -- --ignored --nocapture
```

That historical capture opens `https://basin-delineations-public.upstream.tech/merit/0.2.0/`,
expects format version `0.2.1`, 60 de-blessed `hfx.aux.d8_raster.v1` declarations under
`aux/d8/pfaf_NN/flow_{dir,acc}.tif`, and one snap declaration. It resolves the
`rhine_basel` terminal bbox, proves D8 selection uses bounded extent-header
reads rather than legacy root `flow_dir.tif`/`flow_acc.tif` downloads, and then
asserts that pourpoint selects the manifest-first covering declaration and carves
successfully for overlapping Pfaf declarations. D8 coverage uses inclusive
rectangle semantics, so exact bbox equality and edge-touching count. MERIT-Hydro
D8 rasters are per-Pfaf-02 basin windows; irregular basins have overlapping
rectangular extents, so a boundary terminal is fully covered by more than one
declaration. The historical v1 contract required overlapping entries to be windows
of a single coherent D8 fabric (identical values in the overlap), so the
manifest-first covering tile is selected deterministically and the carve never
reads outside the terminal bbox. The merit adapter is correct; selection is no
longer a consumer-side gap.

The committed offline fixture now declares `hfx.aux.d8_raster.v2` with required
`crs`, `flow_dir_encoding`, and `flow_acc_units` metadata. The reader accepts
`uint8` or `int8` direction samples and `float32` or `int32` accumulation
samples; signed layouts normalize to `u8` and `f32` before parity comparison.

Release note: real-data carve on overlapping-Pfaf terminals is now exercised by
the network proof, which asserts an applied contained carve rather than a typed
ambiguity boundary.

GDAL parity proof command:

```bash
cargo test -p pourpoint-gdal --test raster_decode_parity synthetic_b_tiff_matches_gdal -- --ignored --nocapture
```

M1 already proved TIFF-vs-GDAL tile identity for B and for the accepted C
`rhine_basel` windows. M4 may reuse the B proof for the byte-identical raster
bytes, or re-run the proof if the reader implementation changes.
