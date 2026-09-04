# pourpoint

`pourpoint` is the Python package for the pourpoint watershed-delineation
engine. The current PyPI release is 0.3.0 and is classified Beta.

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
completely on a cold open. Required ranges and windows may be cached locally.

The live D8 declaration uses `hfx.aux.d8_raster.v2`, EPSG:8857, `grass`, and
`km2`, at `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`. See the
[D8 compatibility boundary](https://cooperbigfoot.github.io/pourpoint/guide/datasets/#d8-compatibility-and-remote-layout).

## Released and development API references

**Released 0.3.0 documentation:** use the
[tag-pinned Python README](https://github.com/CooperBigFoot/pourpoint/blob/pourpoint-v0.3.0/crates/python/README.md)
and [tag-pinned API reference](https://github.com/CooperBigFoot/pourpoint/blob/pourpoint-v0.3.0/crates/python/API.md).
Released 0.3.0 includes one-shot and batch calls, the staged API, GeoJSON
`Feature` output, and both GeoParquet writer classes.

**Main development documentation:** the files on the `main` branch and the
generated docs site describe the current checkout. They can include Unreleased
changes. In particular, `BestEffortSkipReason`,
`DelineationResult.refinement_skip_reason`,
`DelineationResult.refinement_seed_kind`, and
`Engine.unreadable_auxiliary_schemas` are main-only and are not in the 0.3.0
wheel.

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
