# Tile-count-independent planetary COG reads: evidence register

This document records evidence for the tile-count-independent planetary
COG-read vision through its 0.2.1 release-preparation commit. It remains an
evidence register rather than a release packet; tag creation, tag pushing, and
GitHub Release publication remain human-only actions.

## Measured staged-object evidence

The values in this section, and only this section, are measurements from two
unauthenticated bounded-range probe rounds run on 2026-07-24 against the public
prefix
`https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/`.
The live `manifest.json` returned HTTP 200 but carried no
`hfx.aux.d8_raster.v2` entry, so the probes addressed
`aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif` directly. Curl range requests
returned HTTP 206 without credentials. The probes performed no writes, left
the manifest untouched, and read under 8 KiB per raster.

Both rasters measured as:

- little-endian (`II`) BigTIFF with magic 43, offset size 8, reserved field 0,
  and the first IFD at byte 200 with 19 entries;
- 1,070,000 × 500,000 pixels with 512 × 512 tiles;
- a 2,090 × 977 tile grid containing 2,041,930 tiles;
- Compression 8 (DEFLATE);
- a present Predictor tag with value 1;
- `TileOffsets` with TIFF type 16 (`LONG8`), 8-byte elements, and byte extent
  `[3,998, 16,339,438)`;
- `TileByteCounts` with TIFF type 4 (`LONG`), 4-byte elements, and byte extent
  `[16,339,438, 24,507,158)`; and
- byte-identical index geometry between the two rasters. Index values and
  payloads were not asserted to be identical.

The per-raster measurements were:

| Object | Object size | BitsPerSample | SampleFormat |
|---|---:|---:|---:|
| `aux/d8/flow_dir.tif` | 50,686,516,478 bytes | 8 | 1 |
| `aux/d8/flow_acc.tif` | 205,069,870,081 bytes | 32 | 3 |

The second probe was run because review found that the first probe's evidence
file recorded scalar values without their TIFF types and did not record the
nodata tag at all. It made two bounded reads per raster: a 16-byte header read
and the 19-entry IFD at byte 200. It used no credentials, made no writes, and
remained under 8 KiB per raster. The second probe measured that `Compression`,
`Predictor`, `BitsPerSample`, `SampleFormat`, `TileWidth`, and `TileLength` are
all TIFF type 3 (`SHORT`); only `ImageWidth` and `ImageLength` are type 4
(`LONG`). This retroactively confirmed a blocking parser issue: an
element-width table that omitted `SHORT` would fail on six tags.

The second probe also measured `GDAL_NODATA` tag 42113 in both rasters as TIFF
type 2 (`ASCII`) with count 4. Its exact contents were `[50,53,53,0]`, or
`255\0`, for `flow_dir`, and lowercase `[110,97,110,0]`, or `nan\0`, for
`flow_acc`.

Three IFD tags not named by earlier planning artifacts were also measured:
`GeoKeyDirectory` (34735, `SHORT`, count 28), `GeoAsciiParams` (34737,
`ASCII`, count 32), and `GDAL_METADATA` (42112, `ASCII`, count 82). Their
measured value-region layout, together with the already relevant GeoTIFF
values, was:

| Value | Byte extent |
|---|---:|
| `GDAL_METADATA` | `[596, 678)` |
| `ModelPixelScale` | `[678, 702)` |
| `ModelTiepoint` | `[702, 750)` |
| `GeoKeyDirectory` | `[750, 806)` |
| `GeoAsciiParams` | `[806, 838)` |

There is then a gap to `TileOffsets` at byte 3,998.

The endpoint returned HTTP 403 to Python urllib's default User-Agent but HTTP
206 to curl. This is User-Agent filtering, not authentication: the prefix is
public and requires no credentials. A future HTTP 403 must not be interpreted
as evidence that the prefix is private.

## Baseline failure mechanisms

The two legacy failures are distinct:

1. The 262,144-byte extent bound is exceeded outright by eager index
   materialization.
2. The 16,777,216-byte window prefix fully contains `TileOffsets`, which ends
   at byte 16,339,438, but truncates `TileByteCounts`. It is exactly 7,729,942
   bytes short because 24,507,158 − 16,777,216 = 7,729,942.

The index end at byte 24,507,158 is 23.37 MiB. The “24.5 MiB” wording in the
bounded-read ADR is the rounded decimal-megabyte figure, 24.5 MB; it must not
be treated as a binary-MiB measurement.

## What M1 proved

The committed
[bounded-read ADR](../decisions/2026-07-24-bounded-reads-are-tile-count-independent.md)
defines boundedness structurally: byte and request cost must not grow with a
raster's tile count. A numeric ceiling is only a coarse backstop.

The committed
[parser-ownership ADR](../decisions/2026-07-24-remote-cog-parser-ownership.md)
assigns both the classic-TIFF/BigTIFF IFD and lazy-index walk and the
chunk-decode seam to `crates/core`. The decode seam includes DEFLATE expansion
and predictor application. The ADR rejected `async-tiff` 0.3.0 on source
inspection, not by prototype: its metadata construction reads all tags,
follows out-of-line multi-value tags into lists, and converts both tile indexes
to vectors. It would therefore materialize at least 24,503,160 planetary index
bytes before returning the IFD.

The owned walker began in M1 as a test-only prototype in
`crates/core/src/cog.rs` reached dimensions and
GeoTIFF scale/tiepoint with four requests and 274 bytes for classic TIFF, and
four requests and 476 bytes for BigTIFF. Both tile indexes remained typed
descriptors. The BigTIFF total of 476 bytes is below the first index offset of
3,998, substantiating that the prototype did not materialize either index. M2
promoted it into the production remote extent path.

The decode seam began in M1 as a test-only prototype in the same file with the
recorded known-value result: a zlib-framed `Compression=8` payload under
predictor 1 expanded to `[1,2,3,4,5,6,7,8]`, with no horizontal differencing.
M3 shipped owned bounded remote chunk decode.

M1 shipped two assertions explicitly labelled `TRANSITIONAL`. M2 converted the
extent assertion to green success, and its `TRANSITIONAL` label survives at
`cog.rs:3945`; exactly one such label exists in the module today. M3 converted
the window assertion to green success by replacing the failure-locking
assertion and removed its `TRANSITIONAL` label. That conversion remains
auditable through the historical names
`planetary_window_locks_truncated_tile_byte_counts_failure` and
`planetary_cache_window_locks_truncated_tile_byte_counts_failure`. Their
current success-oriented names are
`planetary_window_resolves_covered_tile_indexes_with_bounded_reads`
(`cog.rs:4631`) and
`planetary_cache_window_materializes_with_bounded_reads` (`cog.rs:4932`);
this cleanup renamed them without changing their converted-success assertions.

It also contains one `DURABLE` generated-fixture invariant that must survive M2
and M3. The invariant derives the index end from the fixture's own
`TileByteCounts` IFD entry and asserts that the result exceeds both legacy
bounds. Its M1-S2a falsification check set the fixture tile count to 1,000 and
made the invariant fail; it passes only with the measured count of 2,041,930.

`flate2 = "=1.1.9"` was initially committed under `[dev-dependencies]` for
M1's test-only decode prototype. M3 promoted it to production `[dependencies]`
when owned decode shipped.

## Network-gated layout guard and its limitation

The
`crates/core/tests/staged_cog_layout_probe.rs`
layout guard is `#[ignore]`d and additionally requires
`POURPOINT_STAGED_R2_COG_PROBE=1`. Libtest captures stderr, so this command is
not evidence:

```bash
cargo test -- --ignored
```

Without the variable it reports `1 passed` after the guard returns early, while
making no network request and verifying nothing. The mitigations present in M1
are that the `#[ignore]` reason names the variable and is visible in normal test
output, the crate-level documentation gives the exact command, and an explicit
`SKIPPED` message is visible with `--nocapture`.

The only invocation that verifies the staged layout is:

```bash
POURPOINT_STAGED_R2_COG_PROBE=1 cargo test -p pourpoint-core --test staged_cog_layout_probe -- --ignored
```

That full command was run on 2026-07-24 against the live staged objects. Its
recorded result was 1 passed, 0 failed, in 3.17 seconds.

## Scope boundary

M1 proves the staged layout and the two baseline failure mechanics only. No
production read-path behavior changed in M1.

- M2 owns the green extent path and must delete
  `EXTENT_HEADER_RANGE_BYTES` rather than raise or repurpose it.
- M3 owns window index descriptors, owned bounded chunk decode, predictor-1
  support, and the `docs/raster-cache.md` correction.
- M4 owned the live carve proof and extended this same register with its
  live-carve evidence.
- M5 owns release preparation.

## Witnessed public staged carve

### Run authority and immutable capture

The orchestrator ran the following opt-in command against the real public
staged objects at `2026-07-25T23:36:19Z` UTC:

```bash
POURPOINT_STAGED_R2_CARVE=1 cargo test -p pourpoint-core --test staged_r2_carve -- --ignored --nocapture
```

The process exited with status `0`; libtest reported `307.14s` and the exact
summary `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 307.14s`.
A per-test `test <name> ... ok` line is not reliably present in either direction
under `--nocapture`: an earlier 3.71-second `staged_cog_layout_probe` run
printed only a bare progress dot, while this 307.14-second run printed both the
over-60-seconds notice and the per-test success line. Merge verification
therefore keys on the serialized evidence line plus the substring
`test result: ok. 1 passed; 0 failed`, never on a per-test-name needle.

M4 has no M1-style silent skip. Running its ignored test without
`POURPOINT_STAGED_R2_CARVE` panics at `staged_r2_carve.rs:791` and reports
`1 failed`.

The following line is the immutable stdout evidence transported from that run:

STAGED_R2_CARVE_EVIDENCE:{"input_coord":[8.5417,47.3769],"resolved_coord":[8.538953815312432,47.38249957156025],"resolved_terminal_id":13882943,"snap":{"method":"Snap","strategy":"WeightFirst","snap_id":12115939,"weight":2234.2527,"mainstem_status":"mainstem","distance_m":652.6736170606541,"candidates_considered":5,"declaration_name":"reach-stems","declaration_artifact":"aux/snap_reaches.parquet","references_levels":[1],"weight_semantics":"drainage_area_km2_partitioned","declaration_status":"RECORDED_MEASUREMENT","bounds_status":"RECORDED_MEASUREMENTS_NOT_INDEPENDENT_PROOFS"},"upstream_count":275,"refinement":"Applied","route":{"public_custom_domain":"basin-delineations-public.upstream.tech","object_store_builder":"AmazonS3Builder::new","skip_signature":true,"bogus_aws_credentials_installed":true,"ambient_aws_credentials_consulted":false},"areas_km2":{"unrefined_terminal_geodesic":1.5045751705518662,"refined_terminal_geodesic":0.013500000000685453,"resolved_terminal_hfx_local":1.504575,"status":"DESCRIPTIVE_ONLY"},"store":{"initial_carve":{"flow_dir":{"key":"aux/d8/flow_dir.tif","get_opts_calls":8,"head_calls":2,"full_get_calls":0,"ranged_get_calls":6,"get_opts_range_bytes":808,"get_ranges_calls":4,"get_ranges_range_count":7,"get_ranges_range_bytes":75905,"max_get_ranges_range_bytes":75749,"payload_ranges_beyond_24507158":1},"flow_acc":{"key":"aux/d8/flow_acc.tif","get_opts_calls":8,"head_calls":2,"full_get_calls":0,"ranged_get_calls":6,"get_opts_range_bytes":808,"get_ranges_calls":4,"get_ranges_range_count":7,"get_ranges_range_bytes":267131,"max_get_ranges_range_bytes":266975,"payload_ranges_beyond_24507158":1}},"retained_session_delta":{"flow_dir":{"get_opts_calls":8,"head_calls":2,"full_get_calls":0,"ranged_get_calls":6,"get_opts_range_bytes":808,"get_ranges_calls":3,"get_ranges_range_count":6,"get_ranges_range_bytes":156,"max_get_ranges_range_bytes":48,"payload_ranges_beyond_24507158":0},"flow_acc":{"get_opts_calls":8,"head_calls":2,"full_get_calls":0,"ranged_get_calls":6,"get_opts_range_bytes":808,"get_ranges_calls":3,"get_ranges_range_count":6,"get_ranges_range_bytes":156,"max_get_ranges_range_bytes":48,"payload_ranges_beyond_24507158":0}},"observation_unit":"ObjectStore_API_calls_not_HTTP_requests"},"telemetry":{"event_count":1,"flow_dir":{"header_bytes":488,"tile_bytes":75749,"tile_count":1,"window_pixels":4108,"internal_path":"/var/folders/9m/29m0bx0j0rsdyqb0b6yns28w0000gn/T/.tmpr9zjEL/grit/grit-global-2.0.0/raster-windows/flow-dir.2252917690144854538.x522953-y89360-w79-h52.tif","direct_cached_path":"/var/folders/9m/29m0bx0j0rsdyqb0b6yns28w0000gn/T/.tmpr9zjEL/grit/grit-global-2.0.0/raster-windows/flow-dir.2252917690144854538.x522953-y89360-w79-h52.tif"},"flow_acc":{"header_bytes":488,"tile_bytes":266975,"tile_count":1,"window_pixels":4108,"internal_path":"/var/folders/9m/29m0bx0j0rsdyqb0b6yns28w0000gn/T/.tmpr9zjEL/grit/grit-global-2.0.0/raster-windows/flow-acc.17537180551063497292.x522953-y89360-w79-h52.tif","direct_cached_path":"/var/folders/9m/29m0bx0j0rsdyqb0b6yns28w0000gn/T/.tmpr9zjEL/grit/grit-global-2.0.0/raster-windows/flow-acc.17537180551063497292.x522953-y89360-w79-h52.tif"}},"ceilings":{"status":"RECORDED_MEASUREMENTS_NOT_INDEPENDENT_PROOFS","flow_dir":{"MAX_PLANNED_TILE_COUNT":{"observed":1,"ceiling":65536,"margin":65535},"MAX_COMPRESSED_CHUNK_BYTES":{"observed":75749,"ceiling":16777216,"margin":16701467},"MAX_COVERED_CHUNK_BYTES":{"observed":75749,"ceiling":1073741824,"margin":1073666075},"MAX_DECODED_CHUNK_BYTES":{"observed":262144,"ceiling":1048576,"margin":786432},"MAX_WINDOW_ALLOCATION_BYTES":{"observed":4108,"ceiling":1073741824,"margin":1073737716}},"flow_acc":{"MAX_PLANNED_TILE_COUNT":{"observed":1,"ceiling":65536,"margin":65535},"MAX_COMPRESSED_CHUNK_BYTES":{"observed":266975,"ceiling":16777216,"margin":16510241},"MAX_COVERED_CHUNK_BYTES":{"observed":266975,"ceiling":1073741824,"margin":1073474849},"MAX_DECODED_CHUNK_BYTES":{"observed":1048576,"ceiling":1048576,"margin":0},"MAX_WINDOW_ALLOCATION_BYTES":{"observed":16432,"ceiling":1073741824,"margin":1073725392}},"f32_decoded_chunk_statement":"512x512x4=1048576 equals MAX_DECODED_CHUNK_BYTES; ZERO MARGIN"},"decoded":{"flow_dir":{"sample_type":"U8","width":79,"height":52,"distinct_values":[1,2,3,4,5,6,7,8],"nodata_255_count":0,"nodata_255_fraction":0.0,"legal_grass_non_nodata_count":4108,"legal_grass_non_nodata_fraction":1.0,"distinct_cap":18,"distinct_cap_headroom_over_legal_plus_nodata":1,"minimum_legal_fraction":0.01},"flow_acc":{"sample_type":"F32","width":79,"height":52,"nan_count":0,"nan_fraction":0.0,"non_nan_count":4108,"non_nan_fraction":1.0,"non_nan_min":0.0009,"non_nan_max":2175.6726,"magnitude_ceiling_km2":1000000000.0,"minimum_non_nan_fraction":0.01},"claim":"value-domain bounds falsify broad differenced or grossly mis-assembled decoding but do not provide bit-exact staged-object ground truth; U8 zero-filled unwritten regions are not discriminated"},"live_manifest":{"byte_equal":true,"d8_declaration_present":false},"mutation_attempt_count":0}

### End-to-end result and route provenance

Both planetary predictor-1 rasters were opened over the public custom-domain
route while deliberately bogus AWS credentials were installed in the process.
Both initial-carve observations contain one real tile-payload range beyond byte
`24,507,158`; both retained-session deltas contain zero, demonstrating the
localized-window cache path. The carve resolved terminal `13,882,943`,
traversed `275` upstream units, and completed required-D8 refinement as
`Applied`.

The witnessed test builds its engine at `staged_r2_carve.rs:928-930` with the
TEST-ONLY
`LocalTiffRasterSource::with_encoding(hfx::FlowDirEncoding::Grass)`. The
shipped Python engine injects the GDAL-backed `GdalRasterSource` at
`crates/python/src/engine.rs:214`. The witness exercises the owned COG read
path but does not exercise production raster-source wiring.

The live manifest remained byte-equal, still had no D8 declaration, and the
decorator observed zero mutation attempts. The D8 declaration was injected
CLIENT-SIDE into a clone of the LIVE manifest solely by the read-only test
decorator. It was never written to the frozen `grit/hfx-v0.3.0` prefix.

The observed route domain was
`basin-delineations-public.upstream.tech`. Source code constructs it with
`AmazonS3Builder::new()` and `.with_skip_signature(true)` at
`source.rs:212-240`; completion with bogus credentials is consistent with
unsigned access. The serialized booleans `route.skip_signature` and
`route.ambient_aws_credentials_consulted` are unobserved literals, source-backed
by that constructor, which never calls `from_env()`. They are not runtime
observations, and no object-store implementation identity is asserted.
Decorator counts use ObjectStore API calls, not HTTP-request counts;
`get_ranges` can fan out into multiple HTTP requests.

### Resolution and descriptive areas

The supplied input coordinate `[8.5417, 47.3769]` resolved to
`[8.538953815312432, 47.38249957156025]` through `Snap` with `WeightFirst`.
The snap ID was `12,115,939`, weight `2234.2527`, mainstem status `mainstem`,
distance `652.6736170606541 m`, and candidate count `5`. The declaration was
named `reach-stems`, used `aux/snap_reaches.parquet`, referenced level `[1]`,
and declared weight semantics `drainage_area_km2_partitioned`.

The declared `hfx.aux.snap.v2` branch replaced the supplied coordinate with
`winner.nearest_coord`, and terminal refinement consumed
`resolved.resolved().resolved_coord` (`engine.rs:764-768`). WeightFirst,
distance within 1,000 m, and a positive candidate count record the successful
resolution path; they are not independent falsifiers because they follow from
`ResolverConfig::new()` and successful Snap resolution. The snap-declaration
fields are an independent recomputation over
`session.auxiliary_declarations().snaps`, equivalent by construction to the
engine's selection over `self.snap_stores`, not a readback of the engine's
selected store.

The areas are descriptive only:

- `1.5045751705518662 km²` unrefined terminal geodesic;
- `0.013500000000685453 km²` refined terminal geodesic; and
- `1.504575 km²` resolved-terminal HFX local area.

`PreMergeDrainageUnit::area()` returns `hfx::AreaKm2`, read with `.get()`;
`geodesic_area_multi` returns `pourpoint_core::algo::AreaKm2`, read with
`.as_f64()`. Both geometries are already longitude/latitude because refinement
inverse-projects before wrapping (`refinement.rs:172-176`), so the comparison
used no reprojection. There is no declared-area band, ratio, containment
assertion, Disabled control, or shrinkage requirement. The difference has no
required sign: roughly 30 m cells and the documented fraction-cell
polygonization overshoot (`refinement.rs:321-323`) make strict shrinkage unsafe.

### Second session and ObjectStore observations

The decorator was created once as `Arc<dyn ObjectStore>`; `Arc::clone` opened a
second `DatasetSession::open_remote_with_store(store, root, url)`
(`session.rs:591-599`). This was necessary because `Engine::builder` consumes
the first session by value (`engine.rs:522-530`), `DatasetSession` is not
`Clone` (`session.rs:89-117`), and `Engine` exposes no session accessor.

The initial-carve snapshot is frozen separately from the retained-session
delta. Preparation happens before the filesystem cache lookup
(`raster_cache.rs:52-68`), so manifest, snap-store, raster-metadata, header,
and tile-index ObjectStore reads recur. The cached localized windows prevent
repeated tile-payload ranges beyond the `24,507,158`-byte index region; this
does not mean the raster objects were never fetched again.

| Field | flow_dir | flow_acc |
|---|---:|---:|
| key | `aux/d8/flow_dir.tif` | `aux/d8/flow_acc.tif` |
| `get_opts_calls` | 8 | 8 |
| `head_calls` | 2 | 2 |
| `full_get_calls` | 0 | 0 |
| `ranged_get_calls` | 6 | 6 |
| `get_opts_range_bytes` | 808 | 808 |
| `get_ranges_calls` | 4 | 4 |
| `get_ranges_range_count` | 7 | 7 |
| `get_ranges_range_bytes` | 75,905 | 267,131 |
| maximum member range across all `get_ranges` calls | 75,749 | 266,975 |
| payload ranges beyond byte 24,507,158 | 1 | 1 |

`max_get_ranges_range_bytes` at `staged_r2_carve.rs:132-137` computes the
maximum over all `get_ranges` calls, not only the planned-tile slice and not a
coalesced HTTP range.

For each raster, the retained-session delta was `get_opts_calls=8`,
`head_calls=2`, `full_get_calls=0`, `ranged_get_calls=6`,
`get_opts_range_bytes=808`, `get_ranges_calls=3`,
`get_ranges_range_count=6`, `get_ranges_range_bytes=156`,
`max_get_ranges_range_bytes=48`, with zero payload ranges beyond the index.

### Telemetry and runtime ceilings

The telemetry seam at `refinement.rs:128-138` emitted exactly one event and
carried per-raster header bytes, tile bytes, tile count, and window pixels. The
fresh isolated cache made these initial values strictly positive before any
ceiling comparison. This is failure-capable cache-hit discrimination because
`LocalizedRasterWindow::cached` zeroes all four fields (`cog.rs:70-79`).
For both rasters, header bytes were `488`, tile count was `1`, and window pixels
were `4,108`; tile bytes were `75,749` for flow_dir and `266,975` for flow_acc.
The direct second-session paths exactly matched the paths captured from the
internal refinement spans. Their equality is the relevant observation; the
ephemeral absolute filesystem paths are not durable environmental
requirements.

The five runtime ceilings and their declaration and check locations are:

- `MAX_PLANNED_TILE_COUNT=65,536` (`cog.rs:32`, checked at `:245`);
- `MAX_COMPRESSED_CHUNK_BYTES=16,777,216` (`cog.rs:33`, checked at `:1307`);
- `MAX_COVERED_CHUNK_BYTES=1,073,741,824` (`cog.rs:34`, checked at `:1319`);
- `MAX_DECODED_CHUNK_BYTES=8,388,608` (`cog.rs:35`, checked at `:1456`);
- `MAX_WINDOW_ALLOCATION_BYTES=1,073,741,824` (`cog.rs:36`, checked at
  `:1424`).

| Ceiling | flow_dir observed | flow_dir margin | flow_acc observed | flow_acc margin |
|---|---:|---:|---:|---:|
| `MAX_PLANNED_TILE_COUNT` | 1 | 65,535 | 1 | 65,535 |
| `MAX_COMPRESSED_CHUNK_BYTES` | 75,749 | 16,701,467 | 266,975 | 16,510,241 |
| `MAX_COVERED_CHUNK_BYTES` | 75,749 | 1,073,666,075 | 266,975 | 1,073,474,849 |
| `MAX_DECODED_CHUNK_BYTES` | 262,144 | 786,432 | 1,048,576 | **0** |
| `MAX_WINDOW_ALLOCATION_BYTES` | 4,108 | 1,073,737,716 | 16,432 | 1,073,725,392 |

The `MAX_WINDOW_ALLOCATION_BYTES` values are derived allocation products, not
direct measurements: measured `window_pixels` multiplied by the sample width,
one byte for flow_dir and four bytes for flow_acc.

These are recorded compatibility measurements, not independent proofs:
exceeding a ceiling would have stopped localization before `Applied`. The
decoded-chunk observations `262,144` and `1,048,576` are derived constants,
hard-coded from the staged 512×512 tile geometry, not runtime remeasurements.
Against the then-current `1,048,576` ceiling, for F32,
`512 × 512 × 4 = 1,048,576`, exactly
`MAX_DECODED_CHUNK_BYTES`; the check at `cog.rs:1456` rejects only strict `>`,
so the staged F32 tile passes with **ZERO MARGIN**. This is compatibility,
never decoded-chunk headroom.

### Staged sample-domain evidence

The public `select_d8_raster_for_terminal` (`session.rs:701`) and
`localize_d8_raster_window` (`session.rs:777`) path reopened through
`LocalizedRasterWindow::path()` and `tiff::Decoder` the exact cached files used
by refinement.

The U8 flow_dir decoded as width `79`, height `52`, with sorted distinct values
exactly `[1,2,3,4,5,6,7,8]`; 255-nodata count and fraction were `0` and `0.0`;
legal non-nodata GRASS count and fraction were `4,108` and `1.0`. The enforced
bounds were width at least `19`, no more than `18` distinct byte values, and at
least `1.0%` of samples in `0..=8` or `248..=254`. Width 19 is the
combinatorial minimum at which one row can contain the 19 distinct values
needed to breach the cap; below 19, the per-row cumulative-sum discriminator
does not apply. The earlier staged probe measured `GDAL_NODATA=255`, but did
not measure direction values or encoding; the client-side declaration supplied
GRASS. The legal GRASS domain plus nodata has 17 values, so cap 18 has exactly
one value of correct-decode headroom. It rejects 19 or more values, while the
occupancy floor rejects all-nodata. A small structured cap breach would be an
encoding-convention finding requiring escalation, not automatically a
decode-failure diagnosis.

The F32 flow_acc decoded as width `79`, height `52`; NaN count and fraction
were `0` and `0.0`; non-NaN count and fraction were `4,108` and `1.0`;
non-NaN minimum was `0.0009` and maximum `2175.6726 km²`. Every sample had to
be NaN or finite with magnitude below `1,000,000,000.0`, with at least `1.0%`
non-NaN occupancy. The earlier probe measured `GDAL_NODATA=NaN` but no
accumulation samples; the declaration names km². One billion km² is a
deliberately generous backstop above Earth's roughly 510 million km² surface
area. These checks reject infinities, approximately 1e38-scale byte-plane or
differenced misassembly, and an all-NaN result.

These value-domain bounds falsify broad differenced or grossly misassembled
decoding, but no public staged-object oracle supplies bit-exact expected
samples. As a derivation rather than a measurement, a predictor-2 decode of the
witnessed 79-wide row of values from `{1..8}` would cumulative-sum modulo 256
into roughly 70 or more distinct bytes, far beyond cap 18. M3 did not prove
correct decoding of these staged objects. M3's known-value tests
(`cog.rs:3283`, `cog.rs:3313`) run against synthetic predictor-1 fixtures
(`cog.rs:2273`, `cog.rs:2419`); they prove the shipped decoder against fixture
oracles, not staged-byte assembly or decode.

### Residual limitations and deferred risks

1. **Single tile only.** Both rasters reported `tile_count: 1`, so cross-tile
   grid assembly was not exercised. Within one tile, a wrong sub-window offset
   can still yield legal GRASS bytes and pass every U8 bound.
2. **U8 zero-fill is not discriminated.** `decode_window` initializes U8
   output with `vec![0_u8; length]` at `cog.rs:1621`, whereas F32 initializes
   with nodata/NaN at `cog.rs:1638`. Byte 0 is a legal flow-direction value, so
   unwritten U8 regions cannot be distinguished from real data by these checks.
3. **Decoded-chunk observations are derived.** The
   `MAX_DECODED_CHUNK_BYTES` observed values are hard-coded from 512×512 tile
   geometry, not measured at runtime.
4. **Three route fields are unobserved.** `route.object_store_builder`,
   `route.skip_signature`, and `route.ambient_aws_credentials_consulted` are
   source-backed literals, not runtime observations;
   `staged_r2_carve.rs:1408` and `source.rs:212-240` support them.
5. **Snap declaration is recomputed.** The declaration evidence is an
   independent recomputation over `session.auxiliary_declarations().snaps`,
   equivalent by construction but not a readback of engine selection.
6. **No bit-exact ground truth exists.** No public oracle provides it. The
   value-domain checks catch broad differencing or gross misassembly, but a
   plausible wrong single-tile sub-window can pass them.

- Restoration of TIFF-spec defaults for absent optional `Compression`,
  `Predictor`, `PlanarConfiguration`, and `SampleFormat` tags was deferred in
  M4 because M4 did not edit `cog.rs` and both staged rasters explicitly carry
  those tags.
- The 4,096-byte and eight-call numeric backstops are gross-regression nets and
  cannot replace the preceding store-observed byte assertions.
- The converted witnesses retain an auditable chain from the historical names
  `planetary_window_locks_truncated_tile_byte_counts_failure` and
  `planetary_cache_window_locks_truncated_tile_byte_counts_failure` to the
  current success-oriented names
  `planetary_window_resolves_covered_tile_indexes_with_bounded_reads` and
  `planetary_cache_window_materializes_with_bounded_reads`. This cleanup
  renamed them without changing their converted-success assertions.
