# How it works

Given an outlet coordinate, pourpoint resolves a terminal HFX drainage unit,
walks the same-level graph upstream, and dissolves the contributing unit
geometries into a watershed.

## Prepared input

HFX is a normalized exchange contract. An adapter compiles each raw or source
hydrofabric into required `manifest.json`, `catchments.parquet`, and
`graph.parquet` artifacts before the engine reads it. The engine does not carry
source-fabric-specific logic.

## Outlet resolution and traversal

Resolution uses the snap features declared by the HFX manifest and the engine's
configured strategy. Weight-first is the default and ranks hydrologic weight
before distance. Distance-first is available, so snapping should not be
described as an unconditional nearest-channel operation.

After resolving the terminal unit, pourpoint follows graph edges upstream and
collects all contributing units. It then dissolves their polygons.

## Optional terminal refinement

When a compatible `hfx.aux.d8_raster.v2` auxiliary is declared, the engine can
replace the whole terminal unit with a D8-derived terminal sub-polygon at the
snapped raster cell. This does not assert an exact watershed boundary at the
input coordinate. See the bounded [D8 compatibility and remote
layout](guide/datasets.md#d8-compatibility-and-remote-layout) section.

Developers can inspect each operation through the [Staged API](guide/staged-api.md).
Algorithm lineage and citations are on [Credits & Citation](credits.md).
