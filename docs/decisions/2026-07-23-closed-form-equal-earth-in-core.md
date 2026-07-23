# Closed-form Equal Earth transforms in core, not GDAL/PROJ

`hfx.aux.d8_raster.v2` lets a declaration name any EPSG code, and the GRIT
planetary D8 entry declares EPSG:8857, so pourpoint must transform between the
dataset CRS (EPSG:4326) and a raster's native CRS at the projection seam.
`pourpoint-core` has no GDAL dependency even though the shipped Python wheel
links GDAL through `pourpoint-gdal`, so a PROJ-backed transform would either
pull GDAL into core or cross the boundary through an injected trait.

We implement the 4326 ↔ 8857 transform in core as closed-form Equal Earth
(forward closed-form, inverse by Newton iteration) with EPSG:4326 as identity,
and reject any other declared EPSG code with an explicit unsupported-CRS error.
A PROJ-backed transform would support arbitrary EPSG codes exactly, but it would
make every golden coordinate a function of the installed PROJ data version and
would add a `proj.db` runtime dependency to core's offline parity fixture. We
accept that adding a projection is a code change rather than a configuration
change, in exchange for a fully offline, version-stable golden suite.

Because a golden fixture proves stability rather than correctness — a wrong
coefficient still round-trips cleanly — correctness is established against a
coordinate oracle table generated once with PROJ and committed alongside its
generator script, not against our own arithmetic.
