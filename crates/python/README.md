# pyshed

Python bindings for the `shed` watershed delineation engine. `pyshed` loads
[HFX-format](https://github.com/CooperBigFoot/hfx) v0.2.1 datasets and returns
watershed polygons from a `(lat, lon)` outlet. Only HFX v0.2.1 datasets load;
HFX v0.1 datasets hard-error as an unsupported format version. The full native
stack (GDAL, PROJ, GEOS, libtiff, SQLite, and more) is bundled inside the wheel
— no system install required.

## Install

```bash
pip install pyshed
```

**Platform support:** Apple Silicon macOS only (`macosx_11_0_arm64`).
Linux, Intel macOS, and Windows wheels are not yet built — community
contributions are welcome. See
[CONTRIBUTING.md](https://github.com/CooperBigFoot/shed/blob/main/CONTRIBUTING.md)
if you want to help port the build.

## Zero-download quickstart (no local dataset)

`pyshed` delineates directly against a remote HFX dataset over the network —
the dataset payload is not downloaded or copied to disk first. Point the `Engine` at the live
public GRIT dataset on Cloudflare R2 and delineate straight away:

```python
import pyshed

# No local dataset: this reads the hosted GRIT dataset over the network.
engine = pyshed.Engine("https://basin-delineations-public.upstream.tech/grit/2.0.0/")
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
```

The engine fetches only the bytes it needs through object-store range reads
(reading byte ranges over HTTP instead of downloading the whole file), so the
full remote dataset never lands on your machine. This is a mechanism, not a
menu: point it at any HFX dataset URL and it delineates against that dataset.
GRIT `2.0.0` is the one public dataset hosted today; more may follow. You swap
datasets by swapping the URL — nothing else in your code changes.

## Quickstart

```python
import pyshed

engine = pyshed.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
```

Snapping options belong on the **constructor**, not on `delineate`:

```python
# Correct — snap_radius is an Engine constructor kwarg
engine = pyshed.Engine("/path/to/hfx/dataset", snap_radius=5000)
result = engine.delineate(lat=47.3769, lon=8.5417)
```

Geometry repair defaults to the pure-Rust topology cleaner. Pass
`repair_geometry="gdal"` to opt into the GDAL repairer; `repair_geometry="auto"`,
`"clean"`, `False`, and `None` all use the default cleaner.

`Engine` also accepts dataset root URLs backed by the object-store integration:

```python
local_engine = pyshed.Engine("/data/hfx/rhine")
file_url_engine = pyshed.Engine("file:///data/hfx/rhine")
s3_engine = pyshed.Engine("s3://bucket/path/to/hfx/rhine")
r2_engine = pyshed.Engine(
    "https://<account>.r2.cloudflarestorage.com/<bucket>/path/to/hfx/rhine"
)
public_r2_engine = pyshed.Engine(
    "https://basin-delineations-public.upstream.tech/grit/2.0.0/"
)
```

Remote dataset sessions cache persistent metadata and validation sidecars under
`HFX_CACHE_DIR` when set, otherwise under the OS cache directory (`~/Library/Caches/hfx`
on macOS, usually `$XDG_CACHE_HOME/hfx` or `/home/<user>/.cache/hfx` on Linux).
HFX roots contain `manifest.json`, `catchments.parquet`, and `graph.parquet`;
Parquet data is read with object-store range reads rather than copied wholesale.
This persistent remote-artifact cache is separate from the per-engine
in-memory Parquet row-group cache described below (Parquet stores rows in
horizontal blocks called row groups, each carrying its own column statistics;
the cache keeps recently read row groups in memory so overlapping watersheds
avoid re-fetching the same bytes).

GDAL raster URI and configuration plumbing is wired through the Python engine,
but public Cloudflare R2 raster access still depends on the target bucket,
credentials, and GDAL driver behavior. Verify the specific remote raster dataset
you plan to use.

The public GRIT `2.0.0` dataset ships no D8 raster — the D8 flow model routes
each grid cell to whichever of its 8 neighbours is steepest-downhill, stored as
a pair of rasters (`flow_dir.tif` for the per-cell direction, `flow_acc.tif` for
accumulated upstream cells). With no D8 raster present, the default best-effort
refinement safely skips terminal raster refinement (sharpening the outlet's own
boundary unit by tracing those flow directions through it) and returns whole
source units instead.

When a dataset does ship D8 rasters, the engine consumes them automatically. If
a terminal is covered by several overlapping per-Pfaf-02 D8 declarations (Pfaf-02
is the two-digit Pfafstetter basin code that partitions the globe into
hydrological regions), the engine selects the manifest-first covering tile — the
first tile in manifest order whose footprint covers the terminal. Those
overlapping entries are windows of one coherent fabric (the same continuous flow
grid, declared as overlapping tiles) and agree where they overlap, so the chosen
tile carves correctly.

### Verbose mode

Enable structured log output from both the Python and Rust layers:

```python
import pyshed

pyshed.set_log_level("info")
engine = pyshed.Engine("https://basin-delineations-public.upstream.tech/grit/2.0.0/")
# INFO lines stream during manifest/graph/catchment loading
result = engine.delineate(lat=47.3769, lon=8.5417)
```

Valid levels: `"trace"`, `"debug"`, `"info"`, `"warn"`/`"warning"`, and
`"error"`/`"critical"`. Set `PYSHED_LOG` to one of those values to opt in at
import time.

### Speeding up repeated delineations

Enable the in-memory Parquet column-chunk cache to avoid redundant range reads
across overlapping watersheds:

```python
engine = pyshed.Engine(
    "https://basin-delineations-public.upstream.tech/grit/2.0.0/",
    parquet_cache=True,
    parquet_cache_max_mb=512,
)
```

The cache is enabled by default for remote dataset URLs and disabled by default
for local paths. `parquet_cache_max_mb` defaults to `512` when caching is
enabled. This in-memory Parquet row-group cache is per-`Engine` instance and is
not persisted to disk; it is distinct from the persistent remote
metadata/validation cache under `HFX_CACHE_DIR` or the OS cache directory.

### Benchmark tracing

Capture stage-span timing records for one process with `bench_trace`:

```python
import pyshed

engine = pyshed.Engine("/path/to/hfx/dataset")

with pyshed.bench_trace("trace.jsonl"):
    result = engine.delineate(lat=47.3769, lon=8.5417)

# trace.jsonl now contains JSONL records with kind == "stage".
```

### Batch delineation with progress

```python
import pyshed

# tqdm is a user dependency — not bundled with pyshed
from tqdm.auto import tqdm

url = "https://basin-delineations-public.upstream.tech/grit/2.0.0/"
engine = pyshed.Engine(url)

outlets = [
    {"lat": 47.3769, "lon": 8.5417},
    {"lat": 46.9480, "lon": 7.4474},
    {"lat": 48.1351, "lon": 11.5820},
]

bar = tqdm(total=len(outlets), unit="outlet")

def on_progress(event):
    bar.update(1)
    bar.set_postfix(status=event.get("status"), ms=event.get("duration_ms"))

results = engine.delineate_batch(outlets, progress=on_progress)
bar.close()
```

The `progress` callback receives a dict with keys `index`, `total`, `lat`,
`lon`, `duration_ms`, `status` (`"ok"` or `"error"`), plus `n_catchments` on
success and `error` on failure. Exceptions raised inside the callback are
swallowed and logged; they do not interrupt the batch.

### Staged delineation

`delineate()` is the convenience composition of the staged API:

```python
level = engine.select_level(selection=pyshed.LevelSelection.FINEST)
outlet = engine.resolve_outlet(level, lat=47.3769, lon=8.5417)
upstream = engine.traverse(outlet)
units = engine.pre_merge_units(upstream)
refinement = engine.refine(outlet, units)
dissolved = engine.dissolve(units, refinement)
result = engine.compose_result(outlet, upstream, units, refinement, dissolved)
```

`LevelSelection.FINEST` is the only level selection currently supported;
multi-level selection is on the roadmap.

`result` matches `engine.delineate(lat=47.3769, lon=8.5417)`. The merged result
exposes final `geometry_wkb`, final `area_km2`, and light per-unit metadata
(`id`, `level`, `area_km2`, `up_area_km2`, `outlet`). Whole per-unit geometry is
available only on `PreMergeDrainageUnits.unit_geometry_wkb`.

Pre-merge units are whole source drainage units, including the whole terminal
unit. If terminal refinement is applied, summing or unioning those whole units
is not the same as the final merged `area_km2` or `geometry_wkb`.

### GeoParquet export

GeoParquet is the OGC standard for storing vector geometry inside Parquet files
(ordinary Parquet columns plus geometry metadata that spatial tools such as
GeoPandas, DuckDB spatial, and QGIS read natively). Exports are explicit
writer-object calls and write complete batches:

```python
basin_writer = pyshed.BasinGeoParquetWriter()
basin_writer.write(engine, "basins.parquet", [result], basin_ids=["rhine-basel"])

bundle_writer = pyshed.UnitBundleGeoParquetWriter()
bundle_writer.write(engine, "units.parquet", [units], [refinement])
```

`BasinGeoParquetWriter` writes one merged basin row per result. `basin_ids` are
caller-owned, filesystem-safe identifiers. Omitting `basin_ids` is allowed only
with `allow_default_basin_id=True` and exactly one result, where the terminal
unit ID becomes the basin ID.

`UnitBundleGeoParquetWriter` writes one row per pre-merge drainage unit. Unit
rows use dataset-local `unit_id`, include `terminal_unit_id` and `delineation`
grouping columns, and store whole-unit geometry.

Default `delineation` labels are `{fabric_name}/{fabric_version}/{method}`.
The default method is `d8-best-effort` when refinement is enabled and
`no-refine` when `refine=False`. The actual outcome is stored separately in
`refinement_status`.

## API Reference

For the full developer-oriented API surface, including argument types, return
types, and the exception hierarchy, see [API.md](https://github.com/CooperBigFoot/shed/blob/main/crates/python/API.md).

## What it does

- Resolves the outlet coordinate to a terminal HFX unit (via `snap.parquet`
  or point-in-polygon on `catchments.parquet`).
- Walks the upstream graph in `graph.parquet` collecting all contributing units.
- Optionally refines the terminal unit geometry using `flow_dir.tif` /
  `flow_acc.tif` rasters when present.
- Returns a dissolved `MultiPolygon` + geodesic area in km².
- Bundles GDAL / PROJ / GEOS / libtiff / SQLite — no system GDAL install
  needed.

## Links

- **Source & issues:** https://github.com/CooperBigFoot/shed
- **HFX dataset spec:** https://github.com/CooperBigFoot/hfx
- **License:** MIT for `pyshed`; bundled native libraries retain their own
  licenses — see
  [`LICENSES/`](https://github.com/CooperBigFoot/shed/tree/main/LICENSES).
