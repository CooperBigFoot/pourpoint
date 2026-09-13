# Kazakhstan TDX gauge watersheds and overview map

## Outcome

Run pourpoint for the eligible discharge gauges in
`/Users/nicolaslazaro/Downloads/2025_07_23_ieh_hf_discharge_stations_KAZ.csv`
and deliver one combined watershed shapefile and a final overview map in
`/Users/nicolaslazaro/Desktop/kaz-basins-tdx/`.
This is a downstream delineation task using the existing engine and compiled
HFX data, not an HFX compilation or engine feature-development project.

## Station eligibility and watershed extent

Discovery found 406 rows and 406 unique `station_code` values. The UTF-8 CSV
has a BOM and columns `name_ru`, `name_en`, `station_code`, `latitude`,
`longitude`, `country_ru`, `country_en`, and `location_flag`. All observed
`location_flag` values are zero; this alone does not establish location quality.
Treat the coordinate fields as geographic longitude/latitude in EPSG:4326.

The user requires the **stations**, not their watershed polygons, to fall inside
Kazakhstan. Check original station coordinates against a documented Kazakhstan
country boundary. Do not infer eligibility from the country-name column or a
rectangular bounding box, and do not move an outside station inside by snapping.
Choose the boundary source and spatial-test mechanics from available evidence;
record their provenance and treatment of boundary cases.

Exclude suspect station locations rather than correcting them. Explicitly exclude
station `11264` (Yesil at Shoptykol), whose latitude and longitude both equal
`52.4317`. Exclude other identified suspect locations and outside-country or
invalid coordinates with a specific recorded reason. Do not invent replacement
coordinates or silently edit the input. Being inside Kazakhstan or producing a
successful delineation is not proof that a gauge location is correct. No claim
of a comprehensive independent geolocation audit is required; disclose the
checks actually performed and any remaining uncertainty.

For eligible gauges, retain complete upstream watersheds, including contributing
land in neighboring countries. Do not clip watershed geometry to Kazakhstan.

## Dataset and read-only access

Use the corrected global 62-basin TDX-Hydro dataset:

`s3://pourpoint-hfx/hfx/tdx-hydro-nga-20230126-global-62basin-corrected-hfx-0.3.0-d4d4c5e28df7/`

Hetzner endpoint: `https://fsn1.your-objectstorage.com`; region: `fsn1`.
Discovery used `hcloud context list` to confirm the configured active `pourpoint`
context. The installed `hcloud` CLI does not expose object-bucket listing;
S3 API access successfully listed the bucket and read this manifest. Existing
S3 credentials are available in `/Users/nicolaslazaro/secrets/pourpoint-hfx.env`.
Use them without printing, committing, or copying their values into deliverables.

The live manifest inspected during discovery declares:
- `fabric_name`: `tdx_hydro`;
- `fabric_version`: `NGA-TDX-Hydro-20230126`;
- `format_version`: `0.3.0`;
- CRS `EPSG:4326`, tree topology, 15,936,428 units;
- `hfx.aux.snap.v2` at `aux/snap_stems.parquet`, with inclusive drainage-area weights;
- **no D8 raster auxiliary**.

Recheck dataset identity at execution and record the actual manifest/build used.
Use this TDX dataset, not the bucket's HydroBASINS dataset or the publicly hosted
GRIT example. Leave remote data unchanged. No new paid cloud resources or changes
to existing servers are authorized by this vision. Prefer existing local consumer
capabilities and remote range reads; do not download the entire planetary dataset
as a default workaround.

## Delineation behavior and implementation context

Use pourpoint's normal defaults: 1,000 m vector snapping radius, weight-first
ranking, finest level, and best-effort refinement. Weight-first prioritizes greater
hydrologic drainage weight before distance; it is not nearest-river selection.
Do not silently expand the radius, change strategy, relocate failed stations, or
fabricate successful geometry. Since this dataset lacks refinement rasters, results
use whole drainage-unit polygons, including the terminal unit. State this limitation
in the delivery notes; these are not raster-refined station boundaries.

Reuse an engine across stations and retain successful results if an individual
station fails. Report every failure at one explicit per-station isolation point.
Choose resumable execution and caching details from engine behavior and actual
resource needs, without changing the agreed scientific settings.

Relevant repository evidence:
- `crates/python/API.md`, Python package exports/stubs, and `crates/core/src/resolver.rs`
  describe `Engine(dataset_path, ...)`, `delineate(lat=..., lon=...)`, snapping,
  result WKB, area, and requested/resolved outlet diagnostics.
- Python `delineate_batch` raises on the first input-order failure rather than
  returning independent success/error records. A per-station loop around a reused
  engine can preserve successful work and station identity.
- `src/main.rs` supports CSV batch input but expects `lat,lon` and optional `id,name`;
  it does not directly preserve the supplied CSV's full station schema. CLI output
  is GeoJSON, not shapefile. Python geometry output likewise needs a downstream
  shapefile writer/conversion step.
- Python and CLI geometry-repair defaults differ. Use one consistent interface and
  record the actual configuration and installed/build version. Main-only diagnostics
  must not be assumed present in the released 0.3.0 wheel.
- Read `../hfx/spec/HFX_SPEC.md` for the canonical contract. Required core artifacts
  include `manifest.json`, `catchments.parquet`, and `graph.parquet`; the reference
  to `graph.arrow` in project instructions is stale. Optional paths come from the
  manifest.

This task does not require maintained study tooling in the sibling HFX repository.
Do not modify that repository or recompile the fabric. If a genuine engine bug
blocks delivery, report it and follow the repository's regression-proof-before-fix
rule rather than silently patching or weakening correctness.

## Deliverables

Create the desktop folder if absent. Preserve any pre-existing user files and
inspect collisions before replacing outputs.

1. **One combined shapefile**, suggested basename `kaz-basins-tdx`, containing one
   final dissolved watershed feature per successfully delineated eligible station.
   Include all required companion files, CRS information, and encoding metadata.
   Preserve station codes, English/Russian names, and basin areas. Handle shapefile
   field-name, text-encoding, and numeric-width limits explicitly; retain full
   source attributes in the accompanying CSV where needed. Overlapping or nested
   watersheds remain distinct station features, not one union across all gauges.
2. **Station-level CSV report** accounting for all input rows with station identity,
   original coordinates, eligibility/status, exclusion or failure reason, requested
   and snapped coordinates where available, and area/engine diagnostics for successful
   results. Distinguish exclusions from attempted delineation failures.
3. **Final overview map**, produced after delineation/export, using Cartopy or an
   equivalent mapping tool. Show Kazakhstan and neighboring context, all successful
   watershed polygons including cross-border extent, and included gauge locations.
   Give excluded or failed stations distinct markers where coordinates are usable.
   Include a readable legend and scale. Export PNG and PDF into the same folder.
   Choose projection, styling, insets, and labels to keep the overview readable;
   do not suppress real watershed extent to make the map fit the national boundary.
4. **Concise run/provenance notes** recording dataset identity, boundary/map sources,
   engine version and settings, execution summary, limitations, and attribute mapping.
   Keep credentials and bulky caches out of the delivery folder and Git.

## Observable completion evidence

- Every one of the 406 observed input stations is accounted for exactly once in the
  report, or an input change is explicitly detected and reconciled before execution.
- Station 11264 and all identified suspect/outside-country stations are excluded
  with reasons, not silently dropped or relocated.
- The combined shapefile reopens successfully in a GIS reader, has valid readable
  polygonal geometry and a declared CRS, and its feature count and station identities
  match successful report rows. Text and identifiers survive export correctly.
- Watersheds retain their full upstream extent. Dataset limitations and snapping
  displacements are visible in the evidence rather than implied away by success.
- PNG and PDF maps exist, render correctly, and agree with exported geometries and
  station statuses. Visually inspect the map for extent, visibility, and legend issues.
- Final delivery states counts of successes, exclusions, and failures and points to
  the desktop outputs. A partial batch is not represented as all eligible gauges
  successfully completed; unresolved failures are explicit.
- Input CSV and remote HFX objects remain unchanged.

## Handoff status

This document is a local standalone draft created from discovery. It has not been
published or verified on the target branch. `implement-vision` must publish and
verify the new standalone draft before substantive execution. Writing this draft
does not itself start delineation, create desktop products, or provision compute.
