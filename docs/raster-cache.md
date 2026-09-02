# Raster cache

Remote terminal refinement reads raster windows. Pourpoint requests the TIFF
metadata entries and compressed byte ranges required for the selected window
instead of downloading the complete raster. Materialized windows can be cached
on disk for reuse across overlapping watersheds.

This bounded-read design does not mean every file is always read partially. A
cold engine open may fetch the small manifest and graph completely. Required
Parquet ranges may also be retained in an in-memory per-engine cache. Set
`HFX_CACHE_DIR` to choose the persistent metadata and raster-window cache
location; otherwise the operating-system cache directory is used.

## Refinement behavior

`refine=True` is best effort. If no usable D8 declaration exists, delineation
can return whole source units. `refine=False` disables terminal refinement:

```python
import pourpoint

engine = pourpoint.Engine("/data/hfx/local", refine=False)
```

The project-hosted GRIT manifest declares one compatible D8 pair at
`aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`. The engine returns a terminal
sub-polygon at the snapped raster cell when refinement succeeds.

For the complete released CRS, unit, data type, predictor, and COG boundary, see
[D8 compatibility and remote layout](guide/datasets.md#d8-compatibility-and-remote-layout).
