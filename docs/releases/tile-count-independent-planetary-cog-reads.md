# Tile-count-independent planetary COG reads: M1 evidence register

This document records evidence established by milestone M1. It is a milestone
evidence register, not a release packet: no production read-path behavior,
version, tag, or publication changed in M1. M5 owns release preparation.

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

The test-only owned walker prototype in
[`crates/core/src/cog.rs`](../../crates/core/src/cog.rs) reached dimensions and
GeoTIFF scale/tiepoint with four requests and 274 bytes for classic TIFF, and
four requests and 476 bytes for BigTIFF. Both tile indexes remained typed
descriptors. The BigTIFF total of 476 bytes is below the first index offset of
3,998, substantiating that the prototype did not materialize either index.

The test-only decode prototype in the same file expanded a known-value
zlib-framed `Compression=8` payload under predictor 1 to
`[1,2,3,4,5,6,7,8]`, with no horizontal differencing.

The same test module contains two assertions explicitly labelled
`TRANSITIONAL`. The extent assertion locks the current 262,144-byte failure;
M2 must convert it to green success. The window assertion locks the truncated
`TileByteCounts` failure; M3 must convert it to green success. Neither
assertion may be deleted.

It also contains one `DURABLE` generated-fixture invariant that must survive M2
and M3. The invariant derives the index end from the fixture's own
`TileByteCounts` IFD entry and asserts that the result exceeds both legacy
bounds. Its M1-S2a falsification check set the fixture tile count to 1,000 and
made the invariant fail; it passes only with the measured count of 2,041,930.

`flate2 = "=1.1.9"` is committed in `[dev-dependencies]` for the test-only
decode prototype. The parser-ownership ADR binds M3 to promote it to
`[dependencies]` when owned decode ships in library code.

## Network-gated layout guard and its limitation

The
[`staged_cog_layout_probe.rs`](../../crates/core/tests/staged_cog_layout_probe.rs)
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
- M4 owns the live carve proof and extends this same register with its
  live-carve evidence.
- M5 owns release preparation.
