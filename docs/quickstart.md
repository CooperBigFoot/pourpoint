# Quickstart

This guide takes you from installing pyshed to your first delineated watershed.

## Terms

- **Outlet** — the point a watershed drains to, given as a `(latitude,
  longitude)` coordinate. Everything upstream of the outlet is its watershed.
- **Delineation** — computing that watershed: finding every piece of land
  upstream of the outlet and returning it as a polygon.
- **HFX dataset** — a *hydrofabric*: the pre-built river-network and catchment
  data shed reads to delineate. It follows the open
  [HFX](https://github.com/CooperBigFoot/hfx) format, and you point pyshed at one
  by path or URL. See [Datasets](guide/datasets.md).

## 1. Install

```bash
pip install pyshed
```

pyshed ships as a self-contained wheel with GDAL, PROJ, and GEOS bundled inside —
there is nothing else to install. Wheels are currently built for Apple Silicon
macOS only (`macosx_11_0_arm64`).

## 2. Delineate your first watershed

You do not need a local dataset. pyshed reads the hosted GRIT hydrofabric
directly over the network, fetching only the bytes it needs — the full dataset is
never downloaded to your machine.

```python
import pyshed

# Open the engine against the hosted GRIT hydrofabric (read over the network;
# nothing is copied to disk).
engine = pyshed.Engine("https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/")

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

The first open of the global hosted dataset fetches and validates metadata over
the network and can take a minute or two; keep the `engine` around and reuse it
for many delineations rather than reopening it.

## What just happened

`delineate` resolved your `(lat, lon)` to the catchment it falls in, walked the
river network upstream to gather every contributing catchment, and dissolved them
into one polygon. For the mechanics in plain language, see
[How it works](how-it-works.md). To run those steps yourself and inspect the
intermediate results, see the [Staged API](guide/staged-api.md).

## Next steps

- Point pyshed at other datasets, local or remote —
  [Datasets](guide/datasets.md).
- Export many basins to GeoParquet —
  [Basin GeoParquet Export](basin-geoparquet-export.md).
- Browse the full API — [API Reference](api-reference.md).
