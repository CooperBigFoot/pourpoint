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
by name: fatally in `RequireD8`, and as a diagnosable unsupported-CRS skip
reason under best-effort totality, never as an unexplained absence of
refinement. The engine stays agnostic about hydrofabrics while being explicit
about which projections it can carve on.
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
invalidated without the reader changing); do not extend that condemnation to a
**Guard ceiling**, whose adequacy does not scale with tile count

**Frozen prefix**:
A live dataset prefix whose manifest never gains an entry that a deployed
released reader rejects; such an entry ships under a successor prefix instead,
leaving the frozen prefix byte-stable for the readers already pointing at it.
Freezing buys exactly one thing — protection for readers already pointing at
the prefix — so its value is proportional to that population and is zero when
the population is empty. On 2026-08-06 the repository owner attested that
pourpoint has no third-party installs, superseding the freeze of
`grit/hfx-v0.3.0` and the successor prefix `grit/hfx-v0.3.1` by decision: the
planetary D8 entry is to be declared in place once released 0.2.0 and 0.2.1 are
yanked, and no successor prefix is minted. Neither the yank nor the in-place
declaration has been performed; until they are, the freeze still describes the
live prefix. The underlying rule — never change data under a reader already
reading it — is unchanged; what replaces enacting it by immobilising the data is
stating a **reader floor**. The attestation is owner recollection, not a
measurement: the 2026-08-01 BigQuery counts (37 and 36 installer-attributed
downloads) were never attributed to a person, in either direction.
_Avoid_: "re-fire" (amending the frozen prefix in place), "additive
amendment"; "the freeze was wrong" (it was correct for the population it was
believed to protect; the population is what changed)

**Reader floor**:
The lowest released pourpoint version that reads a given dataset address
correctly, published as a fact beside every address the repository hands out,
rather than reasoned out from context each time an address is shared. It
replaces immobilising data with declaring who may read it, and it is the
general form of the three protections this Program reached for in sequence —
frozen prefix, support claims, and the straggler question. A floor is a
**support claim** and inherits that term's evidence discipline: the floor for
the GRIT address is 0.3.0 because the declaration-authority and window-assembly
fixes landed there, which is a derivation from the claim catalog and not an
observation of a reader reading that address. Only a page that hands an address
to a user carries a floor; test fixtures, goldens, and dated evidence records
cite addresses as evidence and are not offers.
_Avoid_: "minimum version" unqualified (says nothing about which address);
"supported version" (a floor is the reader side of one address, not a support
policy for the package)

**Straggler reader**:
A deployed released reader that *accepts* a declaration it predates and decodes
it under its own defaults, yielding a wrong result rather than a rejection.
Distinct from the case a frozen prefix protects against: moving an entry to a
successor prefix shields the reader that stays put, but a straggler that follows
the successor prefix is not shielded by anything the prefix discipline provides.
Released 0.2.1 is the straggler for a declared-`grass` D8 entry — it parses
`flow_dir_encoding` and then decodes ESRI anyway, and #63 taught it to open the
planetary COGs while the encoding fix did not ship until 0.3.0. 0.2.0 shares the
decode defect but cannot open those COGs at all, so it fails loudly and is not
the hazard *for those particular files*; against a smaller GRASS-declared D8
raster it lies as quietly as 0.2.1, so its loudness is a property of the file
size and not of the release. "0.2.x" blurs the two failure modes. A straggler is
removed, not protected: on 2026-08-06 the owner decided to yank both 0.2.0 and
0.2.1, which ends the category here because a yank leaves no default resolution
path to either. A yank is not a deletion — it changes what a version range
resolves to and leaves an exact pin installable — so it reaches future installs
and never an install already on disk; it is sufficient here only because the
owner attests the installed base is his own.
_Avoid_: "old client", "unsupported reader" (it is not unsupported; it accepts);
"straggler" unqualified (the Map uses it for the affected user population, not
for the failure mode)

**Window assembly**:
Placing each covered tile's decoded samples at its correct offset in the
output window raster. Distinct from planning (which tiles a window covers)
and from decoding (turning one tile's compressed bytes into samples): a
window can plan and decode correctly and still assemble wrong, yielding a
spatially scrambled raster whose values are all individually legal.
_Avoid_: "assembly" unqualified (`EngineError::Assembly` names watershed
geometry assembly, an unrelated late stage)

**Bounds erasure**:
Answering with a value that is legal in the raster's own vocabulary where the
truthful answer is that no cell was there, so a spatial fact is destroyed by a
byte that decodes cleanly. The historical defect occurred at two sites. First,
an out-of-window neighbor probe answered with the raster's nodata *value*
rather than explicit absence, making "outside the window" and "nodata inside
the window" the same byte. Checked probes now preserve absence. Second, a
window buffer pre-filled with legal flow-direction code 0 made an output cell
no tile ever wrote indistinguishable from a decoded terminating cell. U8 and
I8 window buffers now prefill every not-yet-written position with the parsed
declared nodata byte. `direction_nodata_byte` runs unconditionally before
allocation and before the tile loop, so an invalid or unrepresentable U8/I8
declaration fails loudly even when later writes would cover every output cell.
This prefill alone discriminates an unwritten cell only when the declared
sentinel is not itself a legal direction code, but it is not the last line of
defense. `FlowDirectionTile::from_raw` rejects a tile whose header nodata byte
decodes as a legal direction under the declared encoding, and the production
`GdalRasterSource::load_flow_direction` goes through it, so a raster declaring
nodata `1` through `8` fails loudly at tile construction rather than tracing a
prefilled cell as a real direction. The shipped staged flow-direction raster
declares `255`, as does the U8 fallback when the nodata tag is absent.
_Avoid_: "nodata handling" (the sentinel is handled correctly; what is lost is
the bounds fact), "off-by-one" (the arithmetic is right; its input is a cell
that does not exist), "zero-initialized buffer" (names the mechanism, not the
erased fact)

**Localized window handoff**:
The seam where an assembled window stops being pourpoint's in-memory raster and
becomes a local GeoTIFF that a `RasterSource` reopens by bounding box. Two
independent spatial arithmetics meet here — the tile paste that writes the file
and its geotransform, and the backend's box-to-pixel read out of it — and
production runs GDAL on the second while a GDAL-free `crates/core` test can only
substitute a pure-Rust reader. Evidence that covers only one side leaves a
placement defect on the other unwitnessed. The localizer emits `GDAL_NODATA`
through `normalized_nodata`: U8 and F32 use `metadata.nodata`, I8 is normalized
to its stored U8 byte representation, and I32 is normalized to `nan`. The
remote metadata reader synthesizes `"-1"` when F32 nodata is absent.
_Avoid_: "the raster source reads the COG" (it never touches the remote object;
it reads the localized window), "cache path" (naming the storage, not the
reader boundary)

**Overlap agreement**:
Proving a multi-tile window read against a single-tile read of the same pixels,
requiring the shared region to come back sample-identical. It manufactures an
oracle where no published ground truth exists, because the single-tile path is
already witnessed: a tile landing at a wrong offset shifts the overlap and the
comparison breaks. Value-domain bounds are not a substitute — every sample in a
transposed window is individually legal.
_Avoid_: "sanity check", "plausibility bound" (both name the weaker
value-domain checks that a wrong sub-window already passes)

**Guard ceiling**:
A fixed, file-independent cap that bounds a single allocation against a
malformed or hostile declaration. The source-backed
`MAX_DECODED_CHUNK_BYTES = 8_388_608` ceiling covers the largest artifact class
the reader intends to serve. A tempfile-backed synthetic four-tile regression
asserts required margin from parsed fixture geometry. The live carve instead
checks only that hard-coded `512 * 512 * 4` decoded bytes do not exceed the
ceiling; its `7,340,032`-byte margin is a derivation, not a shipped-fabric
runtime observation. This is distinct from a bound whose adequacy scales with
tile count, which is a boundedness defect to delete rather than resize (see
**Bounded remote read**). A guard ceiling derived from declared geometry is not
a guard: the declaration is the untrusted input it exists to bound.
_Avoid_: "byte ceiling" unqualified (condemned for the tile-count-scaling
class); "derived limit", "adaptive ceiling" (a file authorizing its own
allocation)

**Declaration authority**:
The rule that a D8 declaration's metadata — CRS, flow-direction encoding,
accumulation units — is the sole authority the engine decodes by, and that a
reader carrying its own default for any of them is a second source of truth
whose disagreement is silent. A declared value the reader never reads is
indistinguishable from an undeclared one.
_Avoid_: "default encoding" (a reader default is the defect, not a fallback);
"honors the declaration" on parse alone (`flow_dir_encoding()` was parsed and
exposed on the handle for two releases without any decode path reading it)

**Support claim**:
A reader's assertion that it implements a declared contract — a schema name, a
flow-direction encoding, a CRS, an accumulation unit. The assertion does not
validate itself. Pourpoint 0.2.0 claimed `hfx.aux.d8_raster.v2`, shipped a
passing projected-GRASS golden, and still decoded GRASS as ESRI in production:
the golden records its source as
`LocalTiffRasterSource::with_encoding(hfx::FlowDirEncoding::Grass) ->
EncodedLocalTiffRasterSource` (tag `pourpoint-v0.2.0`:
`crates/core/tests/parity_golden_artifacts.rs:354`), while both shipped entry
points construct `GdalRasterSource::new()`, whose default was
`FlowDirEncoding::Esri` (`crates/gdal/src/raster_reader.rs:41`,
`crates/python/src/engine.rs:214`, `src/main.rs:185`, all at tag
`pourpoint-v0.2.0`). A support claim counts
only when its evidence runs through the construction path shipped code takes;
evidence through a hand-configured object proves a component nobody runs. That
particular hole is closed — #62 made the declared encoding a required argument
of `RasterSource::load_flow_direction` and deleted both
`EncodedLocalTiffRasterSource` and the encoding constructors — but the rule
generalizes past that one field, and a naming discipline does not reach it: on
2026-07-24 every version token in play was correct. #100 turned the rule into a
catalog: every declared value the reader branches on carries a row in
`crates/core/src/support_claims.rs`, and a source-derived gate requires each row
to name a shipped-CLI witness (ADR:
`docs/decisions/2026-08-01-support-claims-not-schema-names.md`).
_Avoid_: "supports GRASS" on the strength of a passing test (name the
construction path the evidence ran through); "conformance matrix" unqualified
(hfx's `conformance/` corpus validates datasets, not readers)

**Evidence strength**:
How much a support claim's witness actually proves, as distinct from the claim
it is attached to. Two properties are routinely collapsed and are not the same.
*Declaration-discriminating* means a different declared value produces a
different observed outcome through the compiled CLI or an installed wheel;
twelve of the thirteen catalogued witnesses carry that strength. *Typed-value
discrimination through the shipped path* means the reader's own typed value is
what the evidence separates — no witness carries it, because "Typed inventory
only" marks an in-process assertion, not a shipped-path one
(`docs/reader-support-claims.md:31-34`). A recorded strength is itself a claim
and can overstate: the `crs` and `flow_acc_units` witnesses carry the
declaration-discriminating label without a mutation control proving a wrong
value turns them red.
_Avoid_: "discriminating" unqualified (say which property); "the gate proves the
claim" (the correspondence gate proves a named test *exists*, not that it
exercises the claim, and it does not scan `crates/python/tests/`)

**Declaration erasure**:
Answering that a dataset lacks a capability where the truthful answer is that
the capability was declared under a name the reader could not read — **bounds
erasure** one level up, with a manifest entry in place of a cell. Two sites were
open through pourpoint 0.3.0 and #100 closed both. First,
`AuxiliarySchemaId::parse` returned `MalformedSchemaId` for any unblessed
`hfx.aux.*` name, which pourpoint raised as `SessionError::AuxiliaryDeclParse`,
so an unrecognised *optional* auxiliary destroyed the mandatory core; a
stranger's reverse-DNS name was treated more gently, classifying as `Generic`
and leaving the dataset openable. Second, a tolerated unrecognised declaration
landed in `AuxDeclarations::generic` and refinement then asked
`d8_rasters.is_empty()`, which could not separate "this dataset declares no D8
raster" from "this dataset declares one under a schema I do not implement". The
shipped rule: an unrecognised auxiliary costs its own entry and nothing more,
retained on `AuxDeclarations::unreadable`, and best-effort refinement reports
`BestEffortSkipReason::UnreadableD8AuxDeclared` naming the schema it could not
read. Two cases remain uncovered — a readable D8 pair alongside an unreadable
D8 declaration produces no typed skip, and `RequireD8` names the schema it
wanted rather than the one it found. The witness is `merit-hfx-global`, written
2026-07-16 by adapter 0.2.0 — 2,876,771 units, one still-blessed
`hfx.aux.snap.v2`, and 60 `hfx.aux.d8_raster.v1` entries retired four days later
— which `hfx-cli` calls malformed and released pourpoint 0.3.0 refuses to open
at all; a 43-unit committed cut of it opens and delineates on `main` (hfx ADR:
`docs/decisions/2026-08-01-unrecognised-auxiliary-costs-its-entry.md`).
_Avoid_: "unsupported auxiliary" (the dataset is well-formed; the reader is
what lacks support); "ignore unknown entries" (names only the tolerant half —
retention is what keeps the skip diagnosable)

**Best-effort totality**:
The property that `RefinementMode::BestEffort` has no failing outcome: every
D8-path failure, whether the raster is unavailable or its declaration lies,
becomes a typed `BestEffortSkipped` reason on the result rather than an
engine error. Hard failure lives in `RefinementMode::RequireD8`. The skip
reason distinguishes availability from mis-declaration, so a degraded carve
still names which of the two occurred.
_Avoid_: "graceful degradation" (unqualified; the reason must be diagnosable,
not merely non-fatal); "best effort" read as "attempt and possibly abort"

**Terminal refinement**:
The optional engine stage that replaces the terminal (outlet-containing)
drainage unit's whole geometry with a D8-carved sub-polygon upstream of the
snapped outlet. Only the terminal unit is ever refined; upstream units are
used whole. Refinement is engine behavior; HFX only declares the raster
contract.
_Avoid_: "catchment refinement" (ambiguous about which catchment; only the
terminal is refined)
