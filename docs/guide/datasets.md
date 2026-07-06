# Datasets

pourpoint reads a *hydrofabric*: a pre-built dataset of catchments and how they
connect upstream and downstream. pourpoint reads any hydrofabric published in the open
[HFX (HydroFabric Exchange)](https://github.com/CooperBigFoot/hfx) format, a
folder of pre-built river-network files, so the same engine works over GRIT (Global River Topology),
MERIT-Basins, and any other HFX dataset.

## What's in an HFX dataset

An HFX dataset is a directory, local or remote, containing:

| Artifact | Required | What it holds |
|---|---|---|
| `manifest.json` | Yes | Dataset metadata: fabric name, version, levels, and which optional auxiliaries are present. |
| `catchments.parquet` | Yes | The catchment ("unit") polygons. |
| `graph.parquet` | Yes | The upstream/downstream topology connecting the units. |
| `snap.parquet` | Optional | Precomputed snap points that pull an outlet onto the stream network. |
| D8 rasters (`flow_dir.tif`, `flow_acc.tif`) | Optional | Flow-direction and flow-accumulation grids used to sharpen the outlet's terminal catchment. |

`manifest.json` declares which optional artifacts a dataset carries. A dataset
that omits them still delineates; it just skips the corresponding step.

## The canonical hosted dataset

To get started without downloading anything, point pourpoint at the hosted GRIT
hydrofabric:

```text
https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/
```

This is GRIT 2.0.0, the source river network, compiled to the HFX v0.3.0 format. Public hosting is
sponsored by [Upstream Tech](https://www.upstream.tech/) as an in-kind
contribution to the open HFX ecosystem; pourpoint is independent open-source software
and the hosting implies no endorsement. The GRIT dataset ships no D8 raster, so
terminal refinement is skipped automatically (see
[Raster cache](../raster-cache.md)).

## Opening a dataset

The first argument to `Engine` is the dataset root, a local directory or a URL.
Your delineation code does not change when you switch datasets; you swap the path.

```python
import pourpoint

# Hosted dataset over HTTPS (read over the network, nothing downloaded).
engine = pourpoint.Engine("https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/")

# Local directory.
engine = pourpoint.Engine("/data/hfx/rhine")

# Local file URL.
engine = pourpoint.Engine("file:///data/hfx/rhine")

# Amazon S3.
engine = pourpoint.Engine("s3://bucket/path/to/hfx/rhine")
```

Only HFX v0.3.0 datasets load; older HFX format versions are rejected as an
unsupported version.

## Remote datasets

For remote datasets, pourpoint fetches only the pieces of each file it needs over
the network; the full dataset is never copied to your machine. Dataset metadata
is cached between runs under `HFX_CACHE_DIR`, or the OS cache directory if that
variable is unset. The first open of a large dataset fetches dataset metadata
over the network and is slower; keep the same `engine` around and reuse it for
many delineations.

See the [Quickstart](../quickstart.md) for a complete delineation and the
[API Reference](../api-reference.md) for every `Engine` option.
