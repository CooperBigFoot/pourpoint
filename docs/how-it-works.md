# How it works

`shed` draws the **drainage basin** — also called a **watershed** — for any point
you give it: the entire area of land where every drop of rain eventually flows past
that point. The point you choose is the **outlet**, or **pour point**. The outlet is
the one place the whole basin drains through, usually a spot on a river.

This page explains, in plain terms, the method `shed` uses to do that both quickly
and accurately, and it names each stage of the pipeline so you can follow along in
the [Staged API](guide/staged-api.md). The method is not original to `shed`: it is
Matthew Heberger's hybrid approach, and the [Credits & Citation](credits.md) page
gives him full credit.

## Two classic ways to delineate a watershed

Hydrologists have long had two families of methods for this, and each has a real
drawback.

### The raster (grid) method: accurate but slow

A **raster** is a grid of square cells laid over the landscape, like the pixels in
an image. Two grids drive this method:

- **Flow direction** — often stored as a **D8** grid — records, for each cell, the
  single neighbour (out of the eight surrounding cells) that lies steepest downhill.
  That is the direction water leaves the cell. "D8" simply means "eight directions."
- **Flow accumulation** records, for each cell, how many uphill cells ultimately
  drain into it. Cells with large accumulation values are where water has collected —
  in other words, the river channels.

To delineate, you start at the outlet and follow the flow-direction arrows
*backwards*, gathering every cell that eventually drains to the outlet. The answer is
exact down to the size of a single grid cell. The catch is scale: a continent-sized
basin can contain hundreds of millions of cells, so walking them one by one is slow
and memory-hungry.

### The vector (polygon) method: fast but coarse at the outlet

The **vector** method starts from a **hydrofabric** — a pre-computed dataset that has
already carved the landscape into small pieces and worked out how they connect. Each
piece is a **unit catchment**: the patch of land that drains directly into one
segment of river, typically the stretch between two confluences. The hydrofabric also
records the **river network** — which unit catchment drains into which, all the way
downstream.

To delineate, you find the unit catchment your outlet sits in, then collect every
unit catchment upstream of it and glue them together. This is fast, because the hard
work was done in advance. The drawback is at the outlet itself: your point almost
never lands exactly on the edge of a unit catchment. It lands somewhere *inside* the
outlet's own catchment, so that last piece gets included whole — and the basin
boundary near the outlet is only as precise as that one coarse polygon.

## The hybrid idea: vector for the bulk, raster for the outlet

Heberger's insight is that you rarely need the raster for the *whole* basin — only
for the one piece the outlet falls inside. So the hybrid method uses each approach
where it is strongest:

1. Use the **vector** method for everything upstream: walk the river network upward
   from the outlet and assemble all the complete upstream unit catchments at once.
   They are already the right shape, so no grid work is needed for them.
2. Use the **raster** method for the single **terminal catchment** — the outlet's own
   "home" piece. Follow the flow-direction grid to keep only the part of that home
   catchment that actually drains to the exact outlet, cutting it precisely at the
   point instead of including it whole.
3. **Dissolve** all the pieces — every upstream unit catchment plus the trimmed home
   catchment — into a single polygon: the finished watershed.

The result has the accuracy of the raster method exactly where it matters most (right
at the outlet) and the speed of the vector method everywhere else.

Two supporting ideas make steps 1 and 2 reliable:

- **Snapping.** A point typed or clicked by hand rarely sits exactly on the river.
  Snapping nudges it onto the true channel — the nearest cell with high flow
  accumulation — so the delineation starts from the real stream rather than a nearby
  hillside.
- **Best-effort refinement.** The raster trim in step 2 needs a D8 grid to be present
  in the dataset. When a hydrofabric ships without one, `shed` safely skips the trim
  and keeps the whole home catchment rather than failing — so a watershed is always
  returned.

## How this maps onto shed's pipeline

`shed` runs the hybrid method as a sequence of named stages. You can run the whole
thing at once with `Engine.delineate(...)`, or call the stages one at a time (see the
[Staged API](guide/staged-api.md)). In order:

| Stage | What it does |
|---|---|
| `resolve_outlet` | Snaps your `(lat, lon)` onto the river network and finds the **terminal unit** — the unit catchment the outlet falls in. |
| `traverse` | Walks the river network upstream from the terminal unit and lists every unit catchment that drains to it. |
| `pre_merge_units` | Gathers those upstream unit-catchment polygons, ready to be merged. |
| `refine` | Uses the D8 flow-direction grid to trim the terminal (home) catchment down to just the part above the exact outlet. Skips safely when the dataset has no D8 grid. |
| `dissolve` | Merges every upstream unit catchment and the trimmed home catchment into one watershed polygon. |
| `compose_result` | Packages the polygon with its area, the outlet coordinates, and the contributing unit ids into the final result. |

`traverse` and `pre_merge_units` are the **vector** half of the method (the bulk of
the basin); `refine` is the **raster** half (the outlet's own catchment); `dissolve`
joins them.

## Where this idea comes from

The hybrid raster-and-vector method is Matthew Heberger's, released as a preprint in
2025 and implemented in the open-source, MIT-licensed `delineator` tool. `shed` is an independent
Rust reimplementation of that method, generalized to run on any HFX-compliant
hydrofabric rather than one specific dataset. The idea of combining the two
approaches traces back further, to Djokic and Ye (1999); Heberger's contribution is
the fast, free, global open-source implementation. Full credit and citations are on
the [Credits & Citation](credits.md) page.
