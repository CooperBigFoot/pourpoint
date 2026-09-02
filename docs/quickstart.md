# Quickstart

This guide uses the released `pourpoint` 0.3.0 Python package and the project's
single hosted dataset.

## 1. Install

```bash
uv add pourpoint
```

(or `pip install pourpoint`)

PyPI publishes five self-contained `cp39-abi3` wheels for macOS 11+
arm64/x86_64, `manylinux_2_28` arm64/x86_64, and Windows amd64, plus an sdist.

## 2. Open the hosted HFX dataset

```python
import pourpoint

engine = pourpoint.Engine(
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"  # Reader floor: pourpoint 0.3.0
)
result = engine.delineate(lat=47.3769, lon=8.5417)

print(result.area_km2)
print(result.terminal_unit_id)
geojson_feature = result.to_geojson()
```

HFX is the normalized input contract. Raw hydrofabrics require an adapter
compile step first. The example is the **GRIT 2.0.0 HFX dataset**, compiled from
GRIT v1.0 source data. It is the only dataset currently hosted by this project.
See [Datasets](guide/datasets.md) for the distinct fabric, format, and adapter
versions.

The remote reader fetches required byte ranges and raster windows instead of
the complete roughly 299 GB dataset. The small manifest and graph may be
fetched completely on a cold open. Required ranges and windows may be cached
locally, so reuse an `Engine` for repeated work.

Outlet resolution uses declared snap features and the configured strategy. The
default weight-first strategy is not simply a nearest-feature search. The hosted
manifest declares `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`. When that D8
auxiliary is used, refinement produces a terminal sub-polygon at the snapped
raster cell. See [D8 compatibility and remote
layout](guide/datasets.md#d8-compatibility-and-remote-layout).

## License and citation

The engine is MIT-licensed. The hosted GRIT dataset is separately
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/) for
NonCommercial use. Installing the engine grants no commercial rights to it.
Cite the [GRIT vector data](https://doi.org/10.5281/zenodo.17435232),
[GRIT raster data](https://doi.org/10.5281/zenodo.15715535), and
[GRIT paper](https://doi.org/10.1029/2024WR038308).

## Next steps

- [Staged API](guide/staged-api.md)
- [Basin GeoParquet Export](basin-geoparquet-export.md)
- [API Reference](api-reference.md), generated from the current checkout
