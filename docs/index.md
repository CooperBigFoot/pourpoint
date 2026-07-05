# pourpoint

**pourpoint** is a watershed-delineation engine: give `pourpoint` a point on a river and
it returns the whole upstream area that drains to it, the watershed. `pourpoint` is
its Python interface.

A *hydrofabric* is a pre-built map of a river network — its streams, the land
patches (catchments) that drain into each stream, and which catchment flows into
which downstream — of the same kind hydrologists already work with in NHDPlus,
HydroSHEDS, or MERIT-Hydro. pourpoint reads any hydrofabric published in the open
[HFX (HydroFabric Exchange)](https://github.com/CooperBigFoot/hfx) format, a
folder of pre-built river-network files. The same delineation runs over GRIT
(Global River Topology; [Wortmann et al.
2025](https://doi.org/10.1029/2024WR038308)), MERIT-Basins, and any other HFX
dataset. A hosted GRIT dataset is available to point at directly, so you can
delineate your first watershed without downloading anything.

## Who it's for

Hydrologists, water-resource scientists, and engineers who need watershed
polygons and areas from outlet coordinates, interactively, in batch, or inside a
pipeline. You work in Python; the heavy lifting runs in a compiled Rust core
with GDAL bundled inside the wheel, so there is no system GDAL to install.

## Install

```bash
uv add pourpoint
```

(or pip install pourpoint)

Wheels are currently published for Apple Silicon macOS only
(`macosx_11_0_arm64`).

## Where to go next

- **[Quickstart](quickstart.md)**: from install to a first delineated watershed
  in one script.
- **[How it works](how-it-works.md)**: how pourpoint finds every catchment upstream
  of your point and merges them into one watershed, using connections the
  hydrofabric has already computed.
- **[Datasets](guide/datasets.md)**: what an HFX hydrofabric is and how to point
  pourpoint at one.
- **[Staged API](guide/staged-api.md)**: run the delineation pipeline stage by
  stage.
- **[Basin GeoParquet Export](basin-geoparquet-export.md)**: write basins to
  GeoParquet for downstream analysis.
- **[API Reference](api-reference.md)**: the complete public `pourpoint` surface.
- **[Credits & Citation](credits.md)**: the algorithm's origin and how to cite
  pourpoint.
