# CONTEXT

Canonical domain language for the pourpoint engine. Shared by humans and agent
sessions; update when a term is resolved, not speculatively.

## Language

**Hydrofabric**:
A source river-network dataset describing catchments and their
upstream/downstream topology (e.g. MERIT-Hydro/MERIT-Basins, GRIT,
HydroBASINS). The pourpoint engine is fabric-agnostic: any hydrofabric
compiled to the HFX format drives the same delineation, so fabrics become a
swappable, comparable input rather than a fixed dependency — the property that
distinguishes pourpoint from single-fabric tools like `delineator` (MERIT-only).
_Avoid_: "dataset" (overloaded — also names an on-disk HFX folder); "fabric"
alone where the source-network sense is not clear from context.

**Projection seam**:
The engine boundary where dataset-CRS (EPSG:4326) coordinates — resolved
outlet, terminal polygon, selection bboxes — are transformed into a D8
auxiliary raster's declared native CRS before the grid-space carve, and the
carved polygon is transformed back afterward. The carve itself (rasterize,
mask, snap, trace, polygonize) is CRS-agnostic grid arithmetic.
_Avoid_: "reprojection" (implies resampling the raster; a D8 grid is never
warped — its values are neighbor pointers valid only on the grid they were
derived on)

**Supported carve CRS**:
The closed set of declared D8 raster CRSs the engine can transform to and from
in core without GDAL or PROJ: EPSG:4326 as identity and EPSG:8857 by
closed-form Equal Earth. A declaration naming any other EPSG code is rejected
with an explicit unsupported-CRS error rather than silently skipped, so the
engine stays agnostic about hydrofabrics while being explicit about which
projections it can carve on.
_Avoid_: "supported projection" (ambiguous between a raster's declared CRS and
the dataset CRS); "any EPSG" (the v2 schema permits any EPSG; the engine does
not)

**Planetary D8 entry**:
A single global D8 auxiliary declaration whose COG pair mosaics every source
tile of a fabric onto one native-CRS grid, so a covering declaration exists for
every terminal catchment. GRIT ships one planetary `hfx.aux.d8_raster.v2`
entry on EPSG:8857.
_Avoid_: "per-region entries", "D8 tiles" (tile-grained declarations make
`TerminalSpansD8Tiles` a routine failure)

**Reader-gated publish**:
Publishing a manifest change to a live dataset prefix only after a released
reader is proven to read the actual staged artifacts, not merely to parse the
entry's schema. Extends the manifest-last upload discipline across software
releases: artifacts may stage early, the manifest is the atomic switch.
_Avoid_: "additive update" (an addition the deployed parser rejects is a
breaking change, not an addition); "gate satisfied" on schema support alone
(the gate is behavioral — 0.2.0 parsed the v2 entry yet could not open the
planetary COGs)

**Bounded remote read**:
A remote raster read whose byte and request cost does not grow with the
raster's tile count — selecting an extent costs the same against a planetary
COG as against a regional one, and fetching a window costs in proportion to the
tiles the window covers rather than to the tiles the file contains. Boundedness
is a structural property of the reader, not a byte ceiling chosen against the
largest fabric currently known.
_Avoid_: "header prefix budget", "byte ceiling" (both name the mechanism that
failed — a constant picked against MERIT's tile counts that a planetary fabric
invalidated without the reader changing)

**Frozen prefix**:
A live dataset prefix whose manifest never gains an entry that a deployed
released reader rejects; such an entry ships under a successor prefix instead,
leaving the frozen prefix byte-stable for the readers already pointing at it.
`grit/hfx-v0.3.0` is frozen; the planetary D8 entry ships under a successor
prefix.
_Avoid_: "re-fire" (amending the frozen prefix in place), "additive
amendment"

**Window assembly**:
Placing each covered tile's decoded samples at its correct offset in the
output window raster. Distinct from planning (which tiles a window covers)
and from decoding (turning one tile's compressed bytes into samples): a
window can plan and decode correctly and still assemble wrong, yielding a
spatially scrambled raster whose values are all individually legal.
_Avoid_: "assembly" unqualified (`EngineError::Assembly` names watershed
geometry assembly, an unrelated late stage)

**Terminal refinement**:
The optional engine stage that replaces the terminal (outlet-containing)
drainage unit's whole geometry with a D8-carved sub-polygon upstream of the
snapped outlet. Only the terminal unit is ever refined; upstream units are
used whole. Refinement is engine behavior; HFX only declares the raster
contract.
_Avoid_: "catchment refinement" (ambiguous about which catchment; only the
terminal is refined)
