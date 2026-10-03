# Snap-target layers

Export the Point and LineString features supplied by an HFX dataset to a
GeoPackage, then open it in QGIS. These are snap targets, not a reconstructed
river network. They do not contain snap-to-snap connectivity.

## CLI

The Rust CLI is available from the source/GitHub distribution. Installing the
Python package does not install it.

```bash
pourpoint export-snap \
  --dataset /data/hfx \
  --bbox 8.3 47.2 8.8 47.6 \
  --output snap-targets.gpkg

pourpoint export-snap \
  --dataset /data/hfx \
  --all \
  --snap-set reach-stems \
  --output all-reach-stems.gpkg
```

Supply exactly one of `--bbox WEST SOUTH EAST NORTH` or `--all`. Coordinates
are EPSG:4326. A bounding box selects actual geometry intersections, including
the boundary. Selected features retain their complete geometry; they are not
clipped. Supported local and remote HFX roots use the same dataset source syntax
as delineation.

All declared snap sets are exported unless `--snap-set NAME` selects one.
Each set produces `<name>_points` and `<name>_lines` layers. The reserved names
`sqlite` and `gpkg` receive a `snap_` prefix. Empty selections produce a valid
GeoPackage with empty layers. Open the file in QGIS and choose
the layers to display.

## Python

```python
import pourpoint

engine = pourpoint.Engine("/data/hfx")
targets = engine.snap_targets(bbox=(8.3, 47.2, 8.8, 47.6))
targets.write("snap-targets.gpkg")

gdf = targets.to_geodataframe()  # requires GeoPandas
print(gdf[["snap_set", "id", "unit_id", "weight"]])

engine.snap_targets(all=True, snap_set="reach-stems").write("all-stems.gpkg")
```

File export streams rows and does not collect the whole dataset in memory.
GeoDataFrame conversion explicitly materializes the selection. GeoPandas is
optional and is not needed for file export. Reading snap targets does not
require watershed delineation, raster refinement, or graph traversal.

## Attributes and metadata

Layers preserve `id`, `unit_id`, `weight`, optional `stem_role`, and the supplied
geometry. Optional source bounds are retained in nullable `bbox_xmin`,
`bbox_ymin`, `bbox_xmax`, and `bbox_ymax` fields; missing bounds stay null.
Python uses a nullable `bbox` tuple for the same supplied bounds. `snap_set`
identifies the source declaration, including in a combined GeoDataFrame. No per-feature level is invented: one declaration may reference
several levels. Weight is producer-defined; it is not necessarily drainage area.

The GeoPackage stores each declaration's name, description, referenced levels,
and weight semantics, together with source manifest metadata and any supplied
attribution. Extraction does not change the source dataset's license.

Missing snap declarations, unknown set names, unsupported selected declarations,
and read/schema failures are errors. An empty extent is a successful empty
selection. A failed export leaves an existing destination unchanged and does
not publish a partial GeoPackage.

Parquet bounds and row-group statistics reduce reads when available. Missing or
null bounds use a correct scan fallback, so a small extent does not guarantee a
small remote download.
