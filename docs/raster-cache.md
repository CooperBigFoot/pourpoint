# Raster cache

When a dataset ships D8 flow-direction rasters, pourpoint can sharpen the outlet's own
terminal catchment by tracing those flow directions through it (terminal
refinement). For remote datasets it reads only the raster tiles it needs —
fetching the compressed byte ranges that cover the terminal catchment rather than
downloading whole rasters — and caches the materialized window on disk for reuse
across overlapping watersheds.

Remote window reads resolve only the tile-offset and tile-byte-count entries for tiles covered by the requested window, then fetch only those tiles’ compressed chunks. Boundedness is structural: byte and request cost must not grow with the raster’s tile count; the numeric test ceiling is only a coarse gross-regression backstop.

## When refinement is skipped

The default `refine=True` is best-effort: if a dataset declares no D8 raster, the
engine skips terminal refinement and returns whole source units. The canonical public GRIT (Global River Topology) manifest declares both flow
direction and flow accumulation in one `hfx.aux.d8_raster.v2` entry. The live
rasters are `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`, so pourpoint 0.3.0
can discover them and apply built-in terminal refinement.

When several D8 declarations overlap and each fully covers the terminal catchment
— the expected case for a per-Pfafstetter-basin fabric such as MERIT-Basins — the
engine selects the manifest-first covering declaration and carves normally under
the default `refine=True`. Overlapping declarations are windows of one coherent D8
fabric that agree where they overlap, so the choice is immaterial and you do not
need to disable refinement.

`refine=False` is the escape hatch for the cases refinement genuinely cannot
handle: a terminal whose bbox straddles a tile boundary with no single fully
covering tile (`TerminalSpansD8Tiles`), a terminal with no covering tile at all
(`NoCoveringD8Tile`), or a fabric whose overlapping declarations are not
guaranteed to agree. Opening the engine with `refine=False` takes the whole-unit
result and skips refinement:

```python
import pourpoint

# Escape hatch: skip refinement and take the whole-unit result.
engine = pourpoint.Engine("/data/hfx/local", refine=False)
```

## Supported raster layout

Remote refinement expects Cloud-Optimized GeoTIFFs (COGs): one band, 512×512
tiled, `u8` flow direction and `f32` flow accumulation. GeoTIFF scale/tiepoint
values are interpreted in the raster's declared native CRS. The scale/tiepoint
georeferencing arithmetic is not EPSG:4326-specific. pourpoint supports
geographic EPSG:4326 and projected EPSG:8857 declared D8 CRSs, and rejects any
other declared EPSG code loudly. The canonical GRIT D8 declaration uses
projected EPSG:8857. Unsupported remote TIFF layouts fail loudly rather than
silently downloading multi-gigabyte rasters.
