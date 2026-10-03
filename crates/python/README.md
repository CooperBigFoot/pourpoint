# pourpoint

`pourpoint` is the Python package for the pourpoint watershed-delineation
engine. Prepared 0.4.0 adds snap-target layers and result corrections; it is
not yet published. The current PyPI release is 0.3.0 and is classified Beta.

## Install

```bash
uv add pourpoint
```

(or `pip install pourpoint`)

Release 0.3.0 has five `cp39-abi3` wheels for macOS 11+ arm64/x86_64,
`manylinux_2_28` arm64/x86_64, and Windows amd64, plus an sdist. The wheels
bundle GDAL, PROJ, GEOS, and their runtime dependencies.

## Hosted quickstart

```python
import pourpoint

engine = pourpoint.Engine(
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"  # Reader floor: pourpoint 0.3.0
)
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
geojson_feature = result.to_geojson()
```

HFX is the normalized input contract. Every raw or source hydrofabric needs an
adapter compile step before pourpoint can read it. Adapter availability does not
imply hosted availability. There is exactly one dataset hosted by this project:
the **GRIT 2.0.0 HFX dataset**, compiled from GRIT v1.0 source data. Its live
manifest reports `fabric_version` 1.0.0, HFX `format_version` 0.3.0, and
`adapter_version` `grit-global-2.1.0`.

Remote operation fetches required byte ranges and raster windows instead of the
complete roughly 299 GB dataset. The small manifest and graph may be fetched
completely when first needed. Construction reads the manifest; delineation
loads and validates the graph and catchments on first use. Snap extraction does
not load them. Required ranges and windows may be cached locally.

The live D8 declaration uses `hfx.aux.d8_raster.v2`, EPSG:8857, `grass`, and
`km2`, at `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`. See the
[D8 compatibility boundary](https://cooperbigfoot.github.io/pourpoint/guide/datasets/#d8-compatibility-and-remote-layout).

## Released and development API references

**Released 0.3.0 documentation:** use the
[tag-pinned Python README](https://github.com/CooperBigFoot/pourpoint/blob/pourpoint-v0.3.0/crates/python/README.md)
and [tag-pinned API reference](https://github.com/CooperBigFoot/pourpoint/blob/pourpoint-v0.3.0/crates/python/API.md).
Released 0.3.0 includes one-shot and batch calls, the staged API, GeoJSON
`Feature` output, and both GeoParquet writer classes.

**Prepared 0.4.0 documentation:** this checkout describes the upcoming version,
including snap-target extraction, typed refinement diagnostics and auxiliary
schema diagnostics. See the [changelog](CHANGELOG.md) for behavior changes.
These additions are not in the published 0.3.0 wheel. Build this checkout using
[CONTRIBUTING.md](../../CONTRIBUTING.md) to use them before publication.

## Snap-target layers

Export the complete snap geometries supplied by a dataset for use in QGIS:

```python
targets = engine.snap_targets(bbox=(8.3, 47.2, 8.8, 47.6))
targets.write("snap-targets.gpkg")

# Select one declared set, or explicitly select the whole dataset.
reach_targets = engine.snap_targets(
    bbox=(8.3, 47.2, 8.8, 47.6), snap_set="reach-stems"
)
engine.snap_targets(all=True).write("all-snap-targets.gpkg")
```

Specify exactly one of `bbox` or `all=True`. Bounds are EPSG:4326 coordinates in
west, south, east, north order. Selection tests actual geometry intersection,
including the boundary, and never clips geometries. Each set has separate
`<set>_points` and `<set>_lines` GeoPackage layers, including empty layers.
`write()` streams rows and replaces an existing file only after a complete,
successful export. It needs no GeoPandas installation.

For a combined GeoDataFrame, install `pourpoint[geopandas]`:

```python
gdf = targets.to_geodataframe()
gdf.plot()
```

The frame includes `snap_set`, `id`, `unit_id`, `weight`, `stem_role`, `bbox`,
`geometry_wkb` (the original bytes), and `geometry`. Its `attrs["snap_sets"]`
retains declaration descriptions, referenced levels, and weight semantics;
`attrs["manifest"]` retains the source manifest, including supplied attribution.
GeoDataFrame conversion loads the selection into memory.

Opening an `Engine` validates its source, manifest, and options. Graph and
catchment reads begin on the first delineation operation, not on snap export.
Snap reads use available spatial pruning; artifacts without bounds can require
larger reads. Snap features are not a reconstructed or routable river network.
Extraction does not change the source dataset's license.

## Local use

```python
import pourpoint

engine = pourpoint.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)
```

Outlet resolution uses declared snap features and the configured strategy. The
default weight-first strategy is not simply nearest. Keep an `Engine` for
repeated delineations so its caches can be reused.

## License and citation

The engine is MIT-licensed. The hosted GRIT dataset is separately
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/) for
NonCommercial use. Installing pourpoint grants no commercial rights to hosted
GRIT. Cite the [vector data](https://doi.org/10.5281/zenodo.17435232),
[raster data](https://doi.org/10.5281/zenodo.15715535), and
[paper](https://doi.org/10.1029/2024WR038308).

[Upstream Tech](https://www.upstream.tech/) is only the in-kind hosting
infrastructure sponsor, not the owner, vendor, or commercial partner.

## Links

- [Source and issues](https://github.com/CooperBigFoot/pourpoint)
- [HFX specification](https://github.com/CooperBigFoot/hfx)
- [Main development API](API.md)
