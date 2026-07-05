# How it works

Give `pourpoint` a point on a river and it returns the whole upstream area that drains to it, the watershed.
The point you choose is the outlet, or pour point, of that watershed.
It is the place where all upstream water leaves the area you want to trace.

## The connections are already computed

`pourpoint` reads a hydrofabric, a dataset that has already divided the landscape into unit catchments.
A unit catchment is the patch of land that drains directly into one river segment.
The hydrofabric also records which catchment flows into which downstream catchment.

`pourpoint` reads hydrofabrics in the HFX format, a folder of pre-built river-network files.
Because the landscape units and their connections are prepared ahead of time, delineation becomes a lookup and a merge.

## What pourpoint does with your point

First, `pourpoint` finds the unit catchment that contains your point.
Before that lookup, it nudges the point onto the nearest river channel so the watershed starts from the stream and not a nearby hillside.

Next, `pourpoint` follows the recorded catchment connections upstream.
It gathers every unit catchment whose water drains toward the point.

Finally, `pourpoint` merges those unit catchments into one polygon.
That polygon is the watershed.
Repeated delineations in the same session reuse data already fetched, so overlapping watersheds are faster.

Developers who need each stage separately can use the [Staged API](guide/staged-api.md).

As one optional last refinement step, when a dataset includes a D8 flow-direction raster `pourpoint` trims the outlet's own catchment to the exact point, and since the hosted GRIT (Global River Topology) dataset ships no raster `pourpoint` keeps that catchment whole and still returns the full watershed.

This approach comes from Matthew Heberger's open-source `delineator` project, which inspired `pourpoint`.
Full credit and citations are on the [Credits & Citation](credits.md) page.
