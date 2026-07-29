# Guard ceilings are fixed with defended headroom, never derived from the file

The
[bounded-read ADR](2026-07-24-bounded-reads-are-tile-count-independent.md)
rejected ceilings whose adequacy scales with a raster's tile count and retained
numeric ceilings only as coarse backstops. It does not distinguish that class
from a second one: a per-item allocation guard that protects the process from a
malformed or hostile file. `MAX_DECODED_CHUNK_BYTES` is the second class. Its
adequacy does not degrade as a fabric grows — a planetary raster and a regional
one decode one tile at a time — but it was nonetheless set to 1,048,576, exactly
the `512 x 512 x 4` bytes the staged GRIT `flow_acc` tile decodes to, and the
check rejects only strict `>`. The evidence register recorded that margin as
`ZERO MARGIN`. A GRIT rebuild at 1024 x 1024 tiles, or any two-band raster,
would fail every carve in a released reader against hosted data.

A guard ceiling is therefore fixed, file-independent, and sized with recorded
headroom over the largest artifact class the reader intends to serve, and a test
asserts that the shipped fabric's observed value sits below it by a required
margin rather than merely recording what was observed. `MAX_DECODED_CHUNK_BYTES`
becomes 8,388,608, covering a 1024 x 1024 float64 tile: double GRIT's tile
dimension at double its sample width. This is not the rejected move of re-picking
a bound against today's largest known fabric, because the quantity guarded does
not grow with fabric size; what changes with a new fabric is tile geometry, and
the margin assertion fails loudly when that geometry creeps toward the cliff.

Deriving the ceiling from the declared tile geometry was rejected outright. The
declaration is the untrusted input the guard exists to bound, so a derived
ceiling lets a file authorize its own allocation and removes the guard while
appearing to make it principled. Deleting the ceiling, the remedy the
bounded-read ADR required for `EXTENT_HEADER_RANGE_BYTES`, was rejected for the
same reason: that constant was wrong to exist because it encoded tile counts,
whereas this one bounds an allocation nothing else bounds.
