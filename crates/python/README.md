# pourpoint

`pourpoint` is the Python package for the `pourpoint` watershed delineation engine.
Give it a point on a river and it returns the whole upstream watershed as a
polygon. It reads HFX datasets, which are folders of pre-built river-network
files. Only HFX v0.3.0 datasets load; older versions report a clear unsupported
format error.

The wheel bundles the full native stack, including GDAL, PROJ, GEOS, libtiff,
SQLite, and more, so no system GDAL install is needed.

## Install

```bash
uv add pourpoint
```

(or `pip install pourpoint`)

Prebuilt wheels are published for:

- macOS (Apple Silicon + Intel)
- Linux (x86_64 + aarch64)
- Windows (x86_64)

as `macosx_11_0_arm64`, `macosx_11_0_x86_64`, `manylinux_2_28_x86_64`, `manylinux_2_28_aarch64`, `win_amd64`.

## Zero-download quickstart

Use the hosted public GRIT (Global River Topology) dataset without downloading it first:

```python
import pourpoint

# No local dataset: this reads the hosted GRIT dataset over the network.
engine = pourpoint.Engine("https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/")
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
```

The engine fetches only the pieces of the dataset it needs, so the full dataset
never lands on your machine. The one public dataset hosted today is GRIT 2.0.0
(the source river network), compiled to HFX v0.3.0 (the format version). To use a
different HFX dataset, change the URL and nothing else in your code changes.

## Local quickstart

```python
import pourpoint

engine = pourpoint.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
```

`Engine` accepts local paths and remote dataset URLs such as `s3://` and
`https://`. For constructor options such as snap search radius and geometry
repair, see the
[Tuning Knobs](https://github.com/CooperBigFoot/pourpoint/blob/main/crates/python/API.md#tuning-knobs)
section of `API.md`.

## Reuse the Engine

The first open fetches dataset metadata over the network and is slower, so keep
the engine around and reuse it. Repeated delineations in the same session reuse
data already fetched, so overlapping watersheds are faster.

## Going further

Logging and verbose output, batch delineation with a progress callback, the
staged step-by-step API, and GeoParquet export are documented in
[`API.md`](https://github.com/CooperBigFoot/pourpoint/blob/main/crates/python/API.md)
and on the docs site at https://cooperbigfoot.github.io/pourpoint/.

## What it does

- Finds the catchment your point sits in.
- Gathers every catchment upstream.
- Merges them into one watershed polygon.
- Returns the geometry plus geodesic area in km².

## API Reference

For the full developer-oriented API surface, including argument types, return
types, and the exception hierarchy, see
[API.md](https://github.com/CooperBigFoot/pourpoint/blob/main/crates/python/API.md).

## Links

- **Source & issues:** https://github.com/CooperBigFoot/pourpoint
- **HFX dataset spec:** https://github.com/CooperBigFoot/hfx
- **License:** MIT for `pourpoint`; bundled native libraries retain their own
  licenses. See
  [`LICENSES/`](https://github.com/CooperBigFoot/pourpoint/tree/main/LICENSES).
