# Guard ceilings are fixed with defended headroom, never derived from the file

The
[bounded-read ADR](2026-07-24-bounded-reads-are-tile-count-independent.md)
rejected ceilings whose adequacy scales with a raster's tile count and retained
numeric ceilings only as coarse backstops. It does not distinguish that class
from a second one: a per-item allocation guard that protects the process from a
malformed or hostile file. `MAX_DECODED_CHUNK_BYTES` is the second class. Its
adequacy does not degrade as a fabric grows — a planetary raster and a regional
one decode one tile at a time — but the earlier implementation was nonetheless
set to the then-current source literal 1,048,576. The staged GRIT `flow_acc`
decoded-size field was the hard-coded derivation
`512 x 512 x 4 = 1,048,576`, not a runtime measurement, and the check rejected
only strict `>`. The evidence register recorded that historical margin as
`ZERO MARGIN`. A GRIT rebuild at 1024 x 1024 tiles, or any two-band raster,
would have failed every carve in that reader against hosted data.

A guard ceiling is therefore fixed, file-independent, and sized with recorded
headroom over the largest artifact class the reader intends to serve, and a
test asserts a required margin. The shipped, source-backed
`MAX_DECODED_CHUNK_BYTES` is fixed at `8,388,608` bytes in
`crates/core/src/cog.rs:41`, covering a `1024 x 1024` Float64 tile: double
GRIT's tile dimension at double its sample width. This is not the rejected move
of re-picking a bound against today's largest known fabric, because the
quantity guarded does not grow with fabric size; what changes with a new fabric
is tile geometry, and the margin assertion fails loudly when that geometry
creeps toward the cliff.

Deriving the ceiling from the declared tile geometry was rejected outright. The
declaration is the untrusted input the guard exists to bound, so a derived
ceiling lets a file authorize its own allocation and removes the guard while
appearing to make it principled. Deleting the ceiling, the remedy the
bounded-read ADR required for `EXTENT_HEADER_RANGE_BYTES`, was rejected for the
same reason: that constant was wrong to exist because it encoded tile counts,
whereas this one bounds an allocation nothing else bounds.

The shipped evidence has two distinct paths. The offline core regression
derives decoded tile pixels at runtime from parsed tile width and parsed tile
height at `crates/core/src/cog.rs:2555-2557`, then multiplies by a sample width
the test supplies as the source literal `1` for U8 at
`crates/core/src/cog.rs:4227` or `4` for F32 at
`crates/core/src/cog.rs:4266`. Its repository-level retirement of the old
evidence limitation applies to the parsed tile-geometry half of this
derivation, and the regression asserts the defended margin.

The live carve uses a separate hard-coded-literal derivation:
`512 * 512 * 4 = 1,048,576` decoded bytes, followed by the margin derivation
`8,388,608 - 1,048,576 = 7,340,032` bytes. These are not runtime measurements.
Separately, the live run witnesses that real staged F32 tiles decode
successfully through the production ceiling check under the raised ceiling.

Adjacent allocation protection is also shipped. U8/I8 `decode_window`
unconditionally parses declared nodata before allocation and before the tile
loop, prefilling every output position with that parsed byte; invalid or
unrepresentable U8/I8 declarations fail loudly. The localized GeoTIFF emits
`GDAL_NODATA` through `normalized_nodata`: U8 and F32 use `metadata.nodata`, I8
uses its stored U8 byte representation, and I32 uses `nan`. When remote F32
metadata omits nodata, the remote reader synthesizes `"-1"`, which is then
carried through `normalized_nodata`.
