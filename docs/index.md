# shed

**shed** is a watershed-delineation engine: give it an outlet — a single
`(latitude, longitude)` point on a river — and it returns the drainage basin, the
whole area of land that drains to that point. `pyshed` is its Python interface.

shed is *fabric-agnostic*. It reads any hydrofabric published in the open
[HFX (HydroFabric Exchange)](https://github.com/CooperBigFoot/hfx) format — GRIT,
MERIT-Basins, and others — and runs the same delineation over all of them. A
hosted GRIT dataset is available to point at directly, so you can delineate your
first watershed without downloading anything.

## Who it's for

Hydrologists, water-resource scientists, and engineers who need drainage-basin
polygons and areas from outlet coordinates — interactively, in batch, or inside a
pipeline. You work in Python; the heavy lifting runs in a compiled Rust core with
GDAL bundled inside the wheel, so there is no system GDAL to install.

## Install

```bash
pip install pyshed
```

Wheels are currently published for Apple Silicon macOS only
(`macosx_11_0_arm64`).

## Where to go next

- **[Quickstart](quickstart.md)** — from install to a first delineated watershed
  in one script.
- **[How it works](how-it-works.md)** — the hybrid vector + raster delineation
  method in plain language.
- **[Datasets](guide/datasets.md)** — what an HFX hydrofabric is and how to point
  pyshed at one.
- **[Staged API](guide/staged-api.md)** — run the delineation pipeline stage by
  stage.
- **[Basin GeoParquet Export](basin-geoparquet-export.md)** — write basins to
  GeoParquet for downstream analysis.
- **[API Reference](api-reference.md)** — the complete public `pyshed` surface.
- **[Credits & Citation](credits.md)** — the algorithm's origin and how to cite
  shed.
