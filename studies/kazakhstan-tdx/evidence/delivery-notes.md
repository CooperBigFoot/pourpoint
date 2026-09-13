# Kazakhstan TDX gauge watershed delivery

Completed 2026-09-13. **406 input stations: 388 successful, 5 excluded, 13 failed.**
All 401 eligible stations were attempted. The 13 failures are all no vector snap
candidates within the fixed 1,000 m radius; none was retried at relaxed settings.
Failed station codes: 11002, 11903, 11907, 11961, 11964, 14016, 14904, 14919, 16052, 16374, 19512, 77819, 96009.

## Files

- `kaz-basins-tdx.shp/.shx/.dbf/.prj/.cpg`: 388 distinct dissolved basin features,
  EPSG:4326, UTF-8. Full upstream geometry, including cross-border contributing
  land; never clipped to Kazakhstan. Overlap and nesting remain distinct features.
- `station-status.csv`: all406 rows, all original source fields/coordinates,
  exclusion/failure reason, requested/resolved outlets, geodesic displacement,
  area, terminal/upstream-unit diagnostics and whole-unit refinement limitation.
- `kaz-basins-tdx-overview.png/.pdf`: final full-extent projected maps from exported
  shapefile and statuses; 150m display-only simplification, not GIS simplification.
- `provenance.json`, `boundary-metadata.json`, `map-provenance.json`,
  `verification.json`: source identity, settings, attribution and independent
  file-readback validation/hashes. Boundary source not redistributed in this folder.

## Method and limits

Corrected global62-basin TDX-Hydro HFX dataset:
s3://pourpoint-hfx/hfx/tdx-hydro-nga-20230126-global-62basin-corrected-hfx-0.3.0-d4d4c5e28df7/
Manifest SHA256 `af443be357742550ea76ef774b83a1f86e683ea828bb796c9ca893425777be85`.
Engine source `c172e04a71bdca3a8eb09cd7227f90b1e38fe1f3`, locally built Python interface (package
label0.3.0 is not sufficient to identify this newer source). Defaults explicitly
fixed:1000m weight-first snapping, finest level, best-effort refinement,
Python auto geometry cleaning,512MiB memory Parquet cache. Weight-first selects
higher drainage weight before distance, not necessarily the nearest river.
No D8 raster auxiliary exists: all388 results use whole drainage-unit polygons,
including the whole terminal unit. These are **not raster-refined gauge boundaries**.
There are 145 successful basin geometries extending outside the chosen Kazakhstan land polygon.
Maximum geodesic requested-to-resolved displacement: 985.372m;
all station displacements are in CSV. Engine resolution distance uses its own
planar metric; the report's independent WGS84 geodesic diagnostic can differ.

Eligibility uses unmodified EPSG:4326 station coordinates and exact boundary-
inclusive `covers` against full-resolution geoBoundaries KAZ ADM0, not country
name or bbox. Exclusions:11264 (explicit suspect copied lat/lon, also outside),
15309,16340,97047,97048 (outside chosen **land** boundary). The last two are
named Caspian Sea sites: land-boundary exclusion is not proof of bad coordinates
or a statement on marine territory. No coordinates were corrected.
Nine inside stations within100m of the polygon boundary stay included and flagged:
12001,12701,14032,14043,14136,19009,19021,19201,97046.
The100m flag is diagnostic, not claimed boundary accuracy; no inclusion buffer.
19009 is only2.381m inside. Historical coastline, coordinate rounding and border
representation affect these cases. Limited checks found no other duplicate pairs,
nonfinite/out-of-range coordinates, or established inside mislocations.
An inside point, location_flag=0, and successful basin do not validate gauge
placement. This is not a comprehensive independent geolocation audit.

## Attribution and attribute mapping

Country outline/eligibility: geoBoundaries gbOpen KAZ ADM0 KAZ-ADM0-12445969,
OpenStreetMap/Wambacher, represented2017, pinned9469f09. © OpenStreetMap
contributors, Open Data Commons Open Database License1.0:
https://www.openstreetmap.org/copyright . Boundary source/build metadata is
in boundary-metadata.json. Natural Earth map context is public domain; exact
source/hash in map-provenance.json. These licenses describe boundary/context,
not a blanket license for the station CSV, TDX fabric or generated watersheds.

Shapefile fields: station_id(text20)=station_code; name_en/name_ru(UTF-8,254-byte
limit checked); area_km2(decimal24,8)=geodesic basin area; term_id(text20)=terminal
HFX unit; n_units(integer12)=complete upstream unit count. Full source schema is
in BOM CSV. Codes and Russian names are checked after native GIS readback.
Normalized coordinates/rings/parts must match saved engine WKB exactly, with
comparison-only Polygon/one-part MultiPolygon coercion allowed for Shapefile.
No hidden geometry repair or export simplification.

Input CSV remains unchanged: SHA256 `4a5a9e524d724206667fe765d52035774775a8d7c8f51b6365f7f6500d08e1ba`.
Remote operations were read-only GET/range reads; final manifest identity is
rechecked. No paid resources/server changes and no HFX edits.
Batch command elapsed2131.959s; initial probe was separate (cold-open77.983s,
first basin12.436s) and its success reused. Canonical resumable evidence:
`/Users/nicolaslazaro/Desktop/work/pourpoint/scratchpad/kazakhstan-tdx`. This contains exact execution source snapshots, logs,
identity locks and individual JSON/WKB checkpoints, not credentials, build dirs,
virtual environments or planetary HFX downloads. Reproducibility source/tests
are under repository studies/kazakhstan-tdx on the delivery PR.

DBF text-format limit: the Shapefile driver removes leading/trailing ASCII spaces.
The explicit name_en/name_ru mapping trims only those edge spaces; internal tabs,
letters and Unicode remain exact. All original strings including edge spaces
remain unchanged in station-status.csv. Affected station IDs (29):
11063, 11087, 11089, 11094, 11110, 11124, 11130, 11131, 11136, 11143, 11151, 11155, 11160, 11164, 11189, 11213, 11221, 11407, 11904, 12029, 13005, 15208, 15347, 15368, 19014, 19208, 19218, 19513, 19808.
