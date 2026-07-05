# Staged API

`Engine.delineate()` runs the whole delineation in one call. The **staged API**
runs the same pipeline one step at a time, handing you the typed intermediate
after each stage. Reach for it when you want to inspect what happens between steps
— the resolved outlet, the set of upstream units, the pre-merge geometries — or
to export those intermediates.

## The pipeline

`delineate()` is exactly this composition:

```python
import pourpoint

engine = pourpoint.Engine("https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/")

level = engine.select_level(selection=pourpoint.LevelSelection.FINEST)
outlet = engine.resolve_outlet(level, lat=47.3769, lon=8.5417)
upstream = engine.traverse(outlet)
units = engine.pre_merge_units(upstream)
refinement = engine.refine(outlet, units)
dissolved = engine.dissolve(units, refinement)
result = engine.compose_result(outlet, upstream, units, refinement, dissolved)
```

`result` here is identical to `engine.delineate(lat=47.3769, lon=8.5417)`.

## What each stage does

| Stage | Returns | What it does |
|---|---|---|
| `select_level(selection=LevelSelection.FINEST)` | `SelectedLevel` | Picks the HFX resolution level to work at. `FINEST` is the only supported selection today. |
| `resolve_outlet(level, *, lat, lon)` | `ResolvedOutlet` | Matches your `(lat, lon)` to the terminal catchment (the "home" unit) it falls in, snapping to the stream network when the dataset supports it. |
| `traverse(outlet)` | `UpstreamUnits` | Walks the river-network graph upstream from the terminal unit, collecting the IDs of every contributing unit. |
| `pre_merge_units(upstream)` | `PreMergeDrainageUnits` | Materializes the whole-catchment geometry for each collected unit, including the whole terminal unit. |
| `refine(outlet, units)` | `TerminalRefinement` | Sharpens the terminal unit's boundary at the outlet using D8 rasters when the dataset declares them; otherwise records that refinement was skipped. |
| `dissolve(units, refinement)` | `DissolvedWatershed` | Merges the units, with any refinement applied, into the single final watershed polygon and its geodesic area. |
| `compose_result(...)` | `DelineationResult` | Packages the same result shape `delineate()` returns. |

Each stage accepts the typed object from the previous one; passing the wrong type
raises `TypeError`.

## Staged vs. one-shot

Use one-shot `delineate()` for the common case — you want the watershed and its
area. Use the staged API when you need the intermediates: for example, to read
`upstream.unit_ids` before geometry is built, or to feed `PreMergeDrainageUnits`
and `TerminalRefinement` to the `UnitBundleGeoParquetWriter`.

One caveat: `PreMergeDrainageUnits` holds *whole* source units, including the
whole terminal unit. When terminal refinement is applied, summing or unioning
those whole units is **not** the same as the final merged `area_km2` or geometry —
use `dissolved` (or `delineate()`) for the authoritative watershed.

See [How it works](../how-it-works.md) for the hydrology behind these stages and
the [API Reference](../api-reference.md) for the full type definitions.
