# Bounded remote reads are proven by tile-count independence, not a byte ceiling

Released pourpoint 0.2.0 bounded remote COG reads with two constants — 256 KiB
for extent selection, 16 MiB for the carve window — chosen against MERIT-scale
tile counts. The planetary GRIT D8 rasters carry 2,041,930 tiles per raster with
a tile index extending to byte ~24.5 MiB, so both bounds failed during the
2026-07-24 live fire without anyone having changed the reader.

Boundedness is therefore defined structurally: a terminal carve's byte and
request cost must not grow with the raster's tile count, asserted by running the
same carve against fixtures of deliberately different tile counts and observing
identical read accounting. A numeric ceiling is retained only as a coarse
backstop against gross regressions such as fetching whole tiles.

A ceiling re-picked against GRIT's tile counts was rejected: it carries the
identical latent failure, encoding today's largest known fabric into a reader
whose stated destination is global and fabric-agnostic. The structural assertion
fails the moment an eager index read is reintroduced, at any fabric size.
