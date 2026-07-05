# Quickstart

This guide takes you from installing pourpoint to your first delineated watershed.

## Terms

- **Outlet**: the point a watershed drains to, given as a `(latitude,
  longitude)` coordinate. Everything upstream of the outlet is its watershed.
- **Delineation**: computing that watershed by finding every piece of land
  upstream of the outlet and returning it as a polygon.
- **Hydrofabric**: a pre-built map of a river network — its streams, the
  catchments that drain into them, and how those catchments connect — of the same
  kind as NHDPlus, HydroSHEDS, or MERIT-Hydro. pourpoint reads one to delineate.
- **HFX dataset**: a *hydrofabric*, the pre-built river-network and catchment
  data pourpoint reads to delineate. It follows the open
  [HFX](https://github.com/CooperBigFoot/hfx) format, and you point pourpoint at one
  by path or URL. See [Datasets](guide/datasets.md).
- **GRIT**: Global River Topology, a global river-network dataset ([Wortmann et
  al. 2025](https://doi.org/10.1029/2024WR038308)). The hosted example is GRIT
  2.0.0 (the source data) compiled to the HFX v0.3.0 format.

## 1. Install

```bash
uv add pourpoint
```

(or pip install pourpoint)

pourpoint ships as a self-contained wheel with GDAL, PROJ, and GEOS bundled inside,
so there is nothing else to install. Wheels are currently built for Apple Silicon
macOS only (`macosx_11_0_arm64`).

## 2. Delineate your first watershed

You do not need a local dataset. pourpoint reads the hosted GRIT hydrofabric
directly over the network, fetching only the bytes it needs; the full dataset is
never downloaded to your machine.

```python
import pourpoint

# Open the engine against the hosted GRIT hydrofabric (read over the network;
# nothing is copied to disk).
engine = pourpoint.Engine("https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/")

# Delineate the watershed draining to an outlet near Zurich, Switzerland.
result = engine.delineate(lat=47.3769, lon=8.5417)

print(result.area_km2)          # geodesic drainage area, in km²
print(result.terminal_unit_id)  # the HFX unit the outlet resolved into

# The watershed boundary, as a GeoJSON Feature string.
geojson = result.to_geojson()
```

That is the whole flow: open an `Engine`, call `delineate` with an outlet, and
read the result. `result.area_km2` is the drainage area; `result.to_geojson()`
returns the boundary polygon ready to write to a file or load into GeoPandas,
QGIS, or a web map.

The first open of the hosted dataset fetches dataset metadata over the network
and is slower; keep the `engine` around and reuse it for many delineations.
Repeated delineations in the same session reuse data already fetched, so
overlapping watersheds are faster.

## What just happened

The hydrofabric already records which catchment flows into which downstream
catchment. `delineate` nudged your point onto the nearest river channel, found
the unit catchment that contains it, followed the recorded catchment connections
upstream, and merged the gathered catchments into one polygon, the watershed.
For the mechanics in plain language, see [How it works](how-it-works.md). To run
those steps yourself and inspect the intermediate results, see the
[Staged API](guide/staged-api.md).

## Next steps

- Point pourpoint at other datasets, local or remote:
  [Datasets](guide/datasets.md).
- Export many basins to GeoParquet:
  [Basin GeoParquet Export](basin-geoparquet-export.md).
- Browse the full API: [API Reference](api-reference.md).
