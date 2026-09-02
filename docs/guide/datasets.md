# Datasets

## HFX is the engine boundary

[HFX](https://github.com/CooperBigFoot/hfx) is the normalized input contract.
It is not a native or raw hydrofabric format. Every raw or source hydrofabric must first be compiled
by an adapter before pourpoint can read it. A named adapter documents a compilation path; it does not imply that its output is
publicly hosted.

Every HFX v0.3.0 dataset root contains:

| Artifact | Requirement | Purpose |
|---|---|---|
| `manifest.json` | Required | Contract and dataset metadata, levels, and auxiliary declarations |
| `catchments.parquet` | Required | Drainage-unit polygons and attributes |
| `graph.parquet` | Required | Same-level topology |

Snap features and D8 rasters are optional. Their paths are declared in
`manifest.json`; consumers must not assume `snap.parquet`, `flow_dir.tif`, or
`flow_acc.tif` locations. Pourpoint rejects unsupported HFX format versions.

## Hosted GRIT identity

The project currently offers exactly one hosted dataset, titled the **GRIT
2.0.0 HFX dataset**:

```text
https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/
Reader floor: pourpoint 0.3.0
```

The dataset root is intended for engines and may return 404 when opened in a
browser. Use its resolvable
Reader floor: pourpoint 0.3.0.
[`manifest.json`](https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/manifest.json)
as the authority.

Do not conflate these identities:

| Identity | Current value |
|---|---|
| Hosted distribution title | GRIT 2.0.0 HFX dataset |
| Source data release | GRIT v1.0 |
| Manifest `fabric_version` | `1.0.0` |
| HFX `format_version` | `0.3.0` |
| Manifest `adapter_version` | `grit-global-2.1.0` |

The live auxiliary declaration uses `hfx.aux.d8_raster.v2`, EPSG:8857, `grass`
flow direction, and `km2` accumulation. Its manifest-declared paths are
`aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`.

## Opening local and remote roots

```python
import pourpoint

remote = pourpoint.Engine(
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"  # Reader floor: pourpoint 0.3.0
)
local = pourpoint.Engine("/data/hfx/local")
file_url = pourpoint.Engine("file:///data/hfx/local")
s3 = pourpoint.Engine("s3://bucket/path/to/hfx")
```

For remote roots, pourpoint fetches required byte ranges and raster windows
instead of the complete dataset. The hosted dataset is roughly 299 GB. The
small manifest and graph may be fetched completely on a cold open. Required
ranges and materialized raster windows may be cached locally. This is bounded
remote access, not a promise that every request is a partial-file request or
that no bytes are written to disk. See [Raster cache](../raster-cache.md).

## Snapping

Outlet resolution uses the snap features and metadata declared by the dataset,
plus the configured strategy. The default is weight-first, which ranks
hydrologic weight before distance. Distance-first is also available. The
default therefore does not mean “choose the nearest feature.”

## D8 compatibility and remote layout

Released pourpoint 0.3.0 consumes only `hfx.aux.d8_raster.v2` for built-in D8
terminal refinement. The supported CRS/unit combinations are exactly:

- EPSG:4326 with `cells`;
- EPSG:8857 with `cells`;
- EPSG:8857 with `km2`.

EPSG:4326 with `km2` is rejected because angular pixel area is not approximated.
Other D8 CRSs are unsupported.

The remote D8 reader accepts one-band, internally tiled 512 by 512, DEFLATE COGs.
Direction samples may be `uint8` or `int8`, with TIFF predictor 1 or 2.
Accumulation samples may be `float32` or `int32`; `cells` requires `float32`.
For `int32`, predictors 1 and 2 are accepted. For `float32`, predictors 1 and 3
are accepted. Unsupported layouts fail rather than falling back to a complete
raster download.

Refinement traces within the selected terminal unit and returns a terminal
sub-polygon at the snapped raster cell. It does not claim an exact boundary at
the original point.

## Hosted-data license and citations

The engine is MIT-licensed. The hosted GRIT dataset is separately licensed
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/) for
NonCommercial use. Installing pourpoint does not grant commercial rights to the
hosted GRIT data. Cite all three sources:

- [GRIT vector dataset](https://doi.org/10.5281/zenodo.17435232)
- [GRIT raster dataset](https://doi.org/10.5281/zenodo.15715535)
- [GRIT paper](https://doi.org/10.1029/2024WR038308)

[Upstream Tech](https://www.upstream.tech/) provides only hosting infrastructure
as an in-kind sponsor. It is not the project owner, dataset vendor, or
commercial partner.
