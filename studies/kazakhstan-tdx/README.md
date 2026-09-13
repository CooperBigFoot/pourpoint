# Kazakhstan TDX gauge watershed study

Downstream, fixed-setting study delivery. This directory does not add engine APIs.
The approved outcome is [the standalone vision](../../planning/visions/2026-09-13-kazakhstan-tdx-gauge-watersheds-and-overview-map.md).

## Reproduce

Use engine source `c172e04a71bdca3a8eb09cd7227f90b1e38fe1f3` or the study PR
(which leaves engine source unchanged). The package version remains `0.3.0`;
the release wheel is **not** the tested implementation. The provenance records
the source revision, compiled extension SHA256 and installed dependency versions.

```sh
uv venv .venv-study --python 3.12
uv pip install --python .venv-study/bin/python -r studies/kazakhstan-tdx/requirements.txt ./crates/python
.venv-study/bin/python -m pytest studies/kazakhstan-tdx -q
```

Download the full-resolution boundary from the pinned `BOUNDARY_URL` in
`delineate_gauges.py`, using a client that follows redirects. Verify SHA256
`244f11881bcbfcfc58f74e53db89f844f033d090a9d087b888a90688561a7107`.
Do not substitute the simplified file. Boundary metadata is in `evidence/`.
Set the following paths to your input, boundary, external cache and delivery directory:

```sh
.venv-study/bin/python studies/kazakhstan-tdx/delineate_gauges.py prepare \
  --input /path/to/2025_07_23_ieh_hf_discharge_stations_KAZ.csv \
  --boundary /path/to/geoBoundaries-KAZ-ADM0.geojson \
  --cache /path/to/study-cache --delivery /path/to/delivery
```

Run the same command with `delineate`, then `export`, then `verify` in place
of `prepare`. Credentials default to `~/secrets/pourpoint-hfx.env`; override
with `--credentials` if needed. Only S3 GET requests are made. The explicit
Hetzner endpoint/region are set for both boto3 and object_store. The latter's
`AmazonS3Builder::from_env` consumes `AWS_ENDPOINT`; the script also sets its
endpoint aliases and path-style request flag. No remote writes are implemented.

`delineate --limit 1` is an optional initial timing probe. A subsequent
`delineate` resumes using all existing success/failure checkpoints. Each process
opens one engine, reused across its remaining stations. Ordinary station failures
are recorded once and never silently retried. Interruptions resume missing
checkpoints without losing completed basins. Do not change the input, boundary,
engine extension, settings or manifest in an existing cache. A new scientific
configuration requires a separate cache, not edits to recorded successes.
Checkpoint WKB and metadata are atomically renamed separately; metadata is the
commit marker. Disk errors stop the batch rather than being treated as stations.
Incomplete batches cannot export. Existing delivery collisions stop export.

After final shapefile export:

```sh
.venv-study/bin/python studies/kazakhstan-tdx/map_overview.py \
  --delivery /path/to/delivery --boundary /path/to/geoBoundaries-KAZ-ADM0.geojson
```

Run `verify` again after adding notes and maps to capture final file hashes.
Independently open PNG/PDF and inspect extent, markers, legend and scale.

## Eligibility and scientific settings

- Original UTF-8 BOM CSV, 406 unique station codes, EPSG:4326 coordinates.
- Country test is exact `covers` against full-resolution geoBoundaries KAZ ADM0.
  Polygon boundary is included; holes and exterior are excluded. No buffers.
- Nonfinite, out-of-range and nonnumeric coordinates are excluded. Identical
  coordinate pairs shared by distinct stations and equal latitude/longitude are
  suspect. Station 11264 is explicitly excluded regardless of other tests.
- Distance to boundary in local WGS84 AEQD is diagnostic only. `within_100m`
  does not exclude an inside station and is not a source-accuracy claim.
- `location_flag=0`, an inside coordinate, and successful delineation do not prove
  accurate gauge placement. This is not a comprehensive geolocation audit.
- Historical **land** ADM0 excludes some named Caspian marine gauges. Exclusion
  means outside this polygon, not proven erroneous coordinates or a marine
  sovereignty decision. Boundary date and coordinate rounding affect edge cases.
- One reused engine per process; default 1,000 m vector radius, weight-first,
  finest level, best-effort refinement, Python `repair_geometry="auto"` (Rust
  topology cleaner), bounded 512 MiB Parquet cache. No retry at relaxed settings.
- The corrected global TDX dataset has snap v2 and **no D8 auxiliary**. Every
  result includes the whole terminal drainage unit, not a raster-refined outlet
  boundary. The whole upstream graph is traversed. No country clipping.
- Each successful basin is a separate dissolved feature. Nested and overlapping
  stations stay separate. The exporter makes no geometry repairs or simplification.
  Map display simplification is separate and documented in map provenance.

## Deliverable fields and validation

Shapefile fields (all names at most 10 characters): `station_id` (text, 20),
`name_en`, `name_ru` (UTF-8 text, 254-byte limit checked), `area_km2` (24,8),
`term_id` (text, 20), `n_units` (12). `.prj` declares EPSG:4326; `.cpg`
declares UTF-8. Source names are retained exactly, including internal tabs.
The BOM CSV keeps every original attribute and coordinate as supplied, plus
status/reason, boundary flags/distance, requested/resolved outlet, geodesic snap
displacement, area, terminal unit, upstream count, resolution and refinement
provenance, timing, geometry hash/bounds and a cross-border diagnostic.

`verify` reopens the CSV and shapefile in a separate process, checks all 406
identities and original attributes, confirms successful feature membership and
Unicode text, CRS, area, validity and exact normalized coordinate equality
against saved engine WKB. This proves the export retains all upstream geometry,
not merely its bounds. It records file hashes and total extent. It does not
independently validate the source fabric's hydrology.

## Source attribution and licensing boundaries

Eligibility/Kazakhstan outline: geoBoundaries gbOpen KAZ ADM0
`KAZ-ADM0-12445969`, represented year 2017, OpenStreetMap/Wambacher, pinned
`9469f09`; © OpenStreetMap contributors, Open Data Commons Open Database License
1.0 (https://www.openstreetmap.org/copyright). See retained metadata for source
and build dates. The boundary is not a surveyed legal or current marine boundary.
Natural Earth context is public domain; exact download source/hash are in map
provenance. These statements describe boundary/context sources, not a blanket
license for the source station CSV, TDX fabric, or generated watersheds. The
engine is MIT-licensed; dataset usage rights remain with their respective sources.

Bulk WKB, boundary data, caches and desktop GIS products are not committed.
Compact run evidence belongs in `evidence/`.
