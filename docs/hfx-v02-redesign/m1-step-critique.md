# Adversarial Critique — M1 Step Plan (Durable v0.1 Parity Oracle)

Target: `docs/hfx-v02-redesign/m1-step-plan.md`
Contract: M1 section of `docs/hfx-v02-redesign/milestone-plan.md` + the three
owner-decided oracles (A non-refined real / B refined synthetic offline / C
refined real network-gated).
Reviewer stance: adversarial. Verified against the actual code, not the plan's
self-description. The already-converged milestone-plan objections (durable inert
goldens, byte-identical rasters, M1 canonical-WKB normalizer, real-data parity)
are honored as dispositions; this review checks the *step plan* delivers them.

---

## Ground truth established (read-only, cited)

**Engine + result accessors**

- `crates/core/src/engine.rs:385-445` — `delineate()` follows
  resolve → traverse → `try_refine` → assemble → compose, matching the plan's
  Assumption (line 14). Result fields: `terminal_atom_id`, `input_outlet`,
  `resolved_outlet`, `resolution_method`, `upstream_atom_ids`, `refinement`,
  `geometry`, `area_km2` (`engine.rs:48-57`).
- Accessors return `AtomId` (`engine.rs:61-63`, `81-83`). `RefinementOutcome`
  has `Applied{refined_outlet}`, `NoRastersAvailable`, `NoRasterSourceProvided`,
  `Disabled` (`engine.rs:30-42`).
- `try_refine` gates on `self.session.raster_paths().is_none()` →
  `NoRastersAvailable` (`engine.rs:515-516`), then on a missing attached source →
  `NoRasterSourceProvided` (`engine.rs:518-521`). So oracle A (no raster source)
  on a `has_rasters=false` dataset yields `NoRastersAvailable` — the plan's
  expectation (line 258) is **correct**.
- WKB is emitted little-endian (`engine.rs:915` asserts `wkb[0]==0x01`);
  `encode_wkb_multi_polygon` exists (`crates/core/src/algo/wkb.rs:100`) but there
  is **no** existing ring-orientation / start-vertex canonicalization — Step 1's
  normalizer is genuinely net-new.

**Refinement carve (containment claim)**

- `crates/core/src/algo/refine.rs:227-250` masks **both** flow-dir and
  accumulation tiles, traces on the masked flow-dir, polygonizes — so the refined
  polygon is a strict sub-polygon of the terminal. "Containment-clamped,
  both-tile" is **accurate** (module doc `refine.rs:11-21`). The strict shrink
  `refined_area < terminal_area` + bbox containment the plan asserts for B/C is
  exactly what `refine.rs` test `sub_polygon_within_terminal` (`refine.rs:462-522`)
  already proves on synthetic tiles.

**The production real-raster path is GDAL, not a core reader**

- The **only** non-test `RasterSource` is `GdalRasterSource`
  (`crates/gdal/src/raster_reader.rs:29,64`). The Rust **core** crate has no
  production raster source.
- The **only** code that attaches a raster source to a real engine is pyshed:
  `crates/python/src/engine.rs:207` —
  `Engine::builder(session).with_raster_source(GdalRasterSource::new())`.
- `crates/core/src/bin/bench_delineate.rs:216-217` builds the engine with **no**
  raster source, so the bench never refines. The plan's claim that
  `bench_delineate` "is not a sufficient template because it does not attach a
  raster source" (line 268) is **correct** — but the deeper consequence (Obj. 1)
  is unstated: *today's working real-data carve is the GDAL path.*
- `crates/gdal/Cargo.toml:9` — `gdal = "0.19"` with a system link. `shed-core`
  has **no** gdal dependency, so `cargo build -p shed-core` is genuinely
  GDAL-free; pulling `shed-gdal` into a core test would force a system-GDAL link
  into the offline compile graph. This is the real constraint behind the plan's
  GDAL avoidance.

**Remote windowing (oracle C feasibility + the missing guard)**

- `crates/core/src/session.rs:586-628` `localize_raster_window` is
  `pub(crate)`: remote sessions call `cache.get_or_fetch_window(...)` which reads
  only intersecting COG byte ranges and materializes a small cache-local GeoTIFF;
  local sessions return the full path.
- `crates/core/src/cog.rs:14-15` decode/encode use the `tiff` crate
  (`Decoder`, `TiffEncoder`). The localized window is a plain north-up GeoTIFF, so
  a `tiff`-crate reader *can* read it (supports Step 2's adapter premise).
- `LocalizedRasterWindow` exposes `tile_count`, `tile_bytes`, `window_pixels`
  (`cog.rs:79-85`, all `pub(crate)`); the engine forwards bytes/requests into
  telemetry surfaced as `Engine::http_stats()` (`engine.rs:367-369`,
  `573-576`). So a windowing-regression guard is *mechanically available* from an
  integration test — the plan just doesn't use it (Obj. 2).

**Pinned remote inputs**

- `bench_delineate.rs:17` pins `https://basin-delineations-public.upstream.tech/grit/1.0.0/`;
  named outlets `zurich = (8.5417, 47.3769)`, `repparfjord = (23.04, 69.97)`,
  `hammerfest = (23.6821, 70.6634)` (`bench_delineate.rs:502-512`), all
  `GeoCoord::new(lon, lat)` — the plan's oracle-A coords (lines 252-254) **match
  exactly**. Usage note `bench_delineate.rs:532` confirms hammerfest fails at the
  default 1000 m radius, justifying the plan's 5000 m recommendation (line 256).
- `crates/core/src/source.rs:212-241` configures the public R2 custom domain with
  `with_skip_signature(true)` — unsigned public access, as the plan assumes.
- `merit-basins/0.1.0` is **not** referenced anywhere in code; the plan's oracle-C
  URL (line 265) is asserted from the reference memory. That memory
  (`r2-canonical-hfx-datasets.md`, corrected 2026-06-02) states MERIT-Basins
  v0.1.0 carries real D8 rasters (`flow_dir.tif` ~13.2 GB etag `2717256c…-1577`,
  `flow_acc.tif` ~48.4 GB etag `61f057fe…-5774`), `format_version "0.1"`,
  `flow_dir_encoding "esri"`, `topology tree`, verified by manifest+HEAD. So
  oracle C is **realizable in principle** — but at ~61 GB the raster identity and
  windowing details below (Obj. 2, 5) become load-bearing.

**Builder / version pins (plan assumptions confirmed)**

- `testutil.rs:186-203` writes `format_version "0.1"`, `terminal_sink_id`,
  `atom_count`, `has_rasters`; `:234` writes `graph.arrow`; `:431-434`
  `write_raster_stubs` writes `b"stub"`. So `DatasetBuilder` is v0.1-only and
  `with_rasters()` cannot drive a real carve — the plan's Assumptions (lines
  21-23) are **correct**, and B as net-new real-GeoTIFF work is justified.
- Workspace version `0.1.109` (`Cargo.toml:9`); `hfx-core` resolves to crates.io
  `=0.2.0` (`Cargo.lock:1240`), the v0.1 contract. Plan Assumption (line 12) holds.

The plan is judged against this reality.

---

## Objections

### 1. [MAJOR] — Oracles B and C capture a brand-new `tiff`/core reader, but the *current working* real-raster carve runs through `GdalRasterSource`; the decode-fidelity gap is never proven, yet C is labeled "today's v0.1 engine"

Targets Steps 2, 3, 4 (lines 130-143, 271; Assumption line 5 "captures the
code; must not bend the engine").

The M1 charter is to capture *today's working v0.1 engine behavior*. For real-data
D8 refinement, today's engine path is the pyshed→core path that attaches
`GdalRasterSource::new()` (`crates/python/src/engine.rs:207`) — GDAL is the
production decoder. The core engine on its own never refines
(`bench_delineate.rs:216-217`). The plan instead drives B and C with a *new*
`test-fixtures`-gated `tiff`/`cog.rs` core source (lines 130-138, 271).

The core carve in `algo/refine.rs` is reader-agnostic — it consumes
`FlowDirectionTile`/`AccumulationTile` — so the *only* thing the reader controls is
the bytes→tile decode (sample interpretation, nodata, ESRI flow-dir decoding,
geotransform). If the `tiff` reader and `GdalRasterSource` decode the identical
localized window to identical tiles, the carve is identical and the oracle is
faithful. **The plan never establishes that they do.** It captures a reader that
has never run in production and that the production engine (pyshed→GDAL) does not
use, then calls C the "real D8-refined oracle" that "captures the current engine."
Step 5's cross-milestone note (lines 360-362) raises GDAL-vs-`tiff` tile-identity
**only for B**, and only as an M4 obligation — C's decode fidelity is left
entirely unaddressed.

This is the role's core question — *"could any step pass by bending engine
behavior instead of recording it?"* Yes: B/C freeze the decode of a reader the
real engine doesn't use.

**Fix demanded:** pick one and state it in Steps 3/4:
(a) Capture B and C with `GdalRasterSource` — the production decoder — isolating
the GDAL-linked capture **in the `shed-gdal` crate's own test target** (or behind a
`gdal` dev-feature) so `shed-core`'s offline compile graph stays GDAL-free; **or**
(b) Keep the core `tiff` reader but add a mandatory step that proves
**tile-identical** decode (same u8 flow-dir, same f32 accumulation, same
geotransform) between the `tiff` adapter and `GdalRasterSource` on at least one B
window **and** one C window, and explicitly re-scope C's description from "captures
today's engine" to "captures the core `tiff`-reader carve, proven tile-identical to
the GDAL production decode." Without (a) or (b), B/C do not record the current
engine.

### 2. [MAJOR] — Oracle C has no assertion that bounds bytes/tiles fetched, so a windowing regression that pulls the full ~61 GB MERIT mosaics would pass green

Targets Step 4 oracle C (lines 273-275) and the Out-Of-Scope guard (lines
295-300). Axis: never download whole datasets.

The C invariant ("engine must COG-window only terminal-bbox tiles", line 297) is
stated as prose with **no enforcing assertion**. The only C assertions are
geometry: `Applied{refined_outlet}`, `0 < refined_area < terminal_area`, refined
bbox ⊂ terminal bbox (lines 273-275). A regression that widened the window — or
fell back to localizing the whole raster — would still produce a correct carve and
**pass every C assertion**, after silently range-reading tens of GB. The
"containment-clamped carve metadata" guards the *geometry*, not the *fetch*.

The guard mechanism already exists: `LocalizedRasterWindow.tile_count/tile_bytes`
(`cog.rs:79-85`) and `Engine::http_stats()` (`engine.rs:367`) expose exactly the
bytes/requests fetched. An integration test can assert
`http_stats().total_bytes_in < <ceiling>` per C case.

**Fix demanded:** add to Step 4 a hard upper bound on bytes (or tile count)
fetched per C case via `Engine::http_stats()` — e.g. assert total bytes-in for the
two windows is below a few hundred MB — and record the *measured* fetched
bytes/tile-count in the C golden as provenance. This is the step that "would CATCH
a regression that accidentally pulls the full rasters"; the plan currently has
none.

### 3. [MAJOR] — `parity_golden_artifacts` durability hinges on an unstated assumption that M2 never mutates the committed B fixture in place; as written it can go RED at M2 when v0.1 inputs are converted

Targets Step 3 (lines 202-207) and Step 5 (lines 349-353). Axis: durability
across the cut.

Step 3 records "fixture byte sizes and hashes for `manifest.json`,
`catchments.parquet`, `graph.arrow`, `flow_dir.tif`, `flow_acc.tif`" in the B
golden, and Step 5 has `parity_golden_artifacts` "Verify B records committed
fixture hashes and byte-identical `flow_dir.tif`/`flow_acc.tif` metadata" (lines
349-350). The contract requires `parity_golden_artifacts` to survive M2 **with
zero edits**.

Two failure modes the plan leaves open:

- **If** `parity_golden_artifacts` re-hashes the on-disk fixture files (not just
  asserts the golden JSON *contains* hash fields), then when M2 "Convert[s] the M1
  parity fixture to HFX v0.2.1" (milestone-plan.md:143-144) by rewriting
  `manifest.json`/`catchments.parquet` and **deleting `graph.arrow`** (M2 rejects
  legacy `graph.arrow`, milestone-plan.md:170), the recorded v0.1 hashes no longer
  match the on-disk v0.2.1 files → the durable gate goes RED at M2.
- Even the raster re-hash is unsafe if M2 *relocates* the `.tif` into a v0.2.1
  fixture layout: the path the golden references moves.

The plan never states (a) which recorded hashes are *inert provenance* vs.
*actively re-checked against disk*, nor (b) that the committed M1 B fixture
directory is immutable and M2 must build its v0.2.1 fixture as a **separate copy**
reusing the same `.tif` bytes.

**Fix demanded:** pin the durable contract in Steps 3/5:
`parity_golden_artifacts` re-validates against disk **only** the byte-identical
`flow_dir.tif`/`flow_acc.tif` (at a committed path under
`tests/fixtures/parity/v01_synthetic_refined/` that M2 is forbidden to move or
delete) plus canonical-WKB idempotence; the v0.1-format input hashes
(`manifest.json`, `catchments.parquet`, `graph.arrow`) are recorded as **inert
provenance only, never re-checked against the mutable fixture tree**. State
explicitly that M2 creates its v0.2.1 fixture elsewhere and does not mutate the M1
B fixture in place.

### 4. [MAJOR] — Oracle C outlet "smallness" is asserted, not justified or measured; with no byte guard (Obj. 2) nothing protects the never-download invariant

Targets Step 4 oracle C (lines 269-272) and Choices-To-Review (lines 446-449).

The plan recommends `rhine_basel (7.5890, 47.5596)` and
`mekong_phnom_penh (104.9300, 11.5700)` "because they should window a small
terminal tile set" (line 447) and hedges "unless live capture disproves them"
(line 270) / "If live capture disproves either, update this plan" (lines 448-449).
That is an admission the smallness is **unverified**. The role demands: "Is the
chosen merit-basins outlet actually small, and is that justified (not asserted)?"
— it is not. A large terminal (e.g. a mainstem near the basin mouth) would window
a big tile set off a 61 GB mosaic.

**Fix demanded:** before blessing C goldens, measure and record the terminal
bbox, tile count, and fetched bytes for each chosen outlet (the
`http_stats()`/`tile_count` data from Obj. 2), and either confirm both are below
the Obj. 2 ceiling or replace the outlet. The justification must be a recorded
measurement in the C fixture README, not a recommendation.

### 5. [MINOR] — The C raster "identity record" lets an executor compute a content hash of a 61 GB raster, contradicting the never-download invariant

Targets Step 4 (lines 276-278) and Step 5 (lines 352-353).

The plan says record "ETag/byte identity **or** byte length plus content hash" for
`flow_dir.tif`/`flow_acc.tif` (line 277). A *content hash* of `flow_acc.tif`
requires downloading all ~48.4 GB — directly violating the Out-Of-Scope guard
(line 295). The MERIT etags are multipart (`…-1577`, `…-5774` per the reference
memory), i.e. **not** plain MD5 content hashes, so "etag" and "content hash" are
not interchangeable here.

**Fix demanded:** for the C *rasters specifically*, mandate identity =
remote ETag (multipart, recorded verbatim) **+** `Content-Length` from a HEAD
request **only**; explicitly forbid computing a content hash of the raster
artifacts. Content hashing remains fine for the small A/C parquet/manifest/graph
artifacts that are fully fetched anyway.

### 6. [MINOR] — Normalizer mandates determinism but does not pin the total-order tie-breaks; semantically-equal geometries could still differ under degenerate keys

Targets Step 1 (lines 65-76). Axis: normalizer completeness.

Step 1 enumerates the right knobs (coordinate precision, ring closure, exterior/
interior orientation, ring start vertex, hole ordering, component ordering,
little-endian 2D WKB, idempotence) — good. But it specifies each only as
"deterministic," not as a concrete **total order with a defined fallback when the
primary key ties**: two components with identical (minx, miny, area); two holes
with identical bbox; a start-vertex choice when two ring vertices collapse to the
same coordinate *after precision rounding* (duplicate vertices are then ambiguous).
Antimeridian crossing is also undefined (all chosen A/C outlets are far from ±180,
so this is bounded, but it should be stated).

**Fix demanded:** Step 1 must pin each ordering as a lexicographic total order
with an explicit final tie-break that is still total under equal primary keys
(e.g. component order by (minx, miny, then full vertex sequence); start vertex =
lexicographically smallest (x, y) after rounding, then by adjacent-vertex
sequence to break duplicate-coordinate ties), and the idempotence test must
include a degenerate case (duplicate vertex, equal-key multi-component). State
antimeridian geometry as out of scope for the chosen outlets.

### 7. [MINOR] — Step 3 stability check proves same-machine determinism only; committed canonical-WKB goldens are byte-compared on other machines/CI

Targets Step 3 (lines 192-197) and Choices-To-Review (lines 438-440).

The bless-time stability gate runs "at least three single-thread `delineate()`
calls for the same case" (line 192) — on one machine. That proves run-to-run
determinism, not cross-architecture determinism. Canonical WKB is then
**byte-exact** compared wherever `parity_golden_artifacts` runs. Geodesic
`area_km2` and dissolve/boolean-op intersection coordinates can round differently
across architectures/`geo` versions; the fixed-precision rounding tames most of it,
but the byte-exact WKB path is the fragile one (axis 10).

**Fix demanded:** either (a) validate the blessed goldens on at least two
architectures before committing, or (b) document that the chosen coordinate
precision carries enough margin that pre-rounding divergence cannot flip the
rounded digit, and keep the scalar-epsilon policy (not byte-exact) for `area_km2`.
Record which guarantee was chosen in the canonicalizer-version note.

### 8. [MINOR] — Oracle A reproducibility depends on the resolver search radius, which must be captured into the golden, not just passed at bless time

Targets Step 4 oracle A (lines 256-258) and the hammerfest choice (lines 442-444).

`bench_delineate.rs:532` confirms hammerfest fails at the default 1000 m radius;
the plan correctly recommends 5000 m (line 256). But the resolved outlet and
terminal depend on `ResolverConfig` search radius. If the golden records the
resolved/refined outlet without recording the **resolver config that produced
it**, a later run at a different default radius would mismatch.

**Fix demanded:** the A (and C) golden must record the exact `ResolverConfig`
(search radius) used, alongside the resolved outlet, so the capture is replayable.
The plan's required field list (lines 80-82) includes "resolution method" but not
the resolver radius input — add it.

---

## Axis-by-axis summary

1. **Parity integrity** — *Partially fails.* The carve claim is real
   (`refine.rs:227-250`) and A's `NoRastersAvailable` is correct. But B/C drive the
   carve with a `tiff`/core reader the production engine never uses (GDAL is
   production, `python/src/engine.rs:207`), with decode fidelity unproven (Obj. 1).
2. **Durability across the cut** — *At risk.* The capture test correctly retires at
   M2 and `cargo build -p shed-core` is genuinely GDAL-free, but
   `parity_golden_artifacts` can go RED at M2 if it re-hashes v0.1 input files M2
   converts; the immutability/inert-provenance contract is unstated (Obj. 3).
3. **Ordering & hidden deps** — *Sound.* Normalizer (Step 1) is front-loaded before
   B/C captures; fixture (Step 2) precedes B (Step 3); A/C (Step 4) follow. No
   circularity found.
4. **Network-dependence of gates** — *Sound in shape.* Offline set
   (`cargo build -p shed-core`, `parity_golden_artifacts`) is GDAL-free and
   network-free; A/C are env+`#[ignore]` gated and compile-and-skip. The plan
   correctly forbids `required-features` on the capture target (line 154), which
   would otherwise skip-while-green.
5. **Never-download invariant** — *Fails.* No assertion bounds C's fetched
   bytes/tiles (Obj. 2); outlet smallness is asserted not measured (Obj. 4); the
   raster identity wording invites a 61 GB content hash (Obj. 5).
6. **Synthetic fixture reality (B)** — *Mostly sound.* The plan correctly treats B
   as net-new real-GeoTIFF work (not the `b"stub"` builder, `testutil.rs:431-434`),
   commits the `.tif` bytes, and reads builder-independently — but it inherits the
   GDAL-vs-`tiff` fidelity gap (Obj. 1) and the durability ambiguity (Obj. 3).
7. **Normalizer completeness** — *Mostly sound, under-pinned.* All knobs
   enumerated; tie-break total-orders and degenerate cases not pinned (Obj. 6).
8. **Scope creep** — *Clean.* No v0.2 loader, no rename, no `grit/2.0.0`, no
   `merit/0.2.0` in M1; the `test-fixtures`-gated adapter is bounded test support,
   not a production backend.
9. **Verifiability & conventions** — *Sound.* Every step has runnable commands and
   the patch-bump+tag obligation (lines 38-43, per-step Commit Obligation).
10. **Unverifiable/fragile goldens** — *Residual risk.* Same-machine-only stability
    check vs. byte-exact WKB (Obj. 7); resolver radius not captured (Obj. 8). f32
    bbox storage is not a hazard since final `area_km2` derives from f64 geometry,
    not the f32 columns.

---

## VERDICT: NEEDS REVISION

0 BLOCKER, 4 MAJOR, 4 MINOR. The plan's structure is right — durable inert
artifact harness, front-loaded normalizer, offline GDAL-free gates, B as net-new
real-GeoTIFF work, correct A outcome — but four substantive defects must close
before it captures *today's* engine durably and provably.

Must-fix to converge:

- **[MAJOR 1]** B/C drive the carve with a non-production `tiff` reader while
  production uses `GdalRasterSource` (`python/src/engine.rs:207`). Either capture
  with GDAL isolated in `shed-gdal`'s test target, or prove tile-identical
  `tiff`-vs-GDAL decode on a B and a C window and re-scope C's "captures the
  current engine" claim.
- **[MAJOR 2]** Add a hard byte/tile ceiling on each C case via
  `Engine::http_stats()` so a windowing regression that pulls the full ~61 GB
  rasters is caught; record measured fetched bytes in the C golden.
- **[MAJOR 3]** Pin the durability contract: `parity_golden_artifacts` re-checks
  only byte-identical `.tif` (at an M2-immutable committed path) + WKB idempotence;
  v0.1 input hashes are inert provenance, never re-hashed against the mutable
  fixture; M2 builds its v0.2.1 fixture as a separate copy.
- **[MAJOR 4]** Replace asserted C-outlet smallness with a recorded measurement
  (terminal bbox, tile count, fetched bytes) before blessing; swap any outlet that
  exceeds the Obj. 2 ceiling.

Fold in the minors at the same pass: forbid content-hashing the C rasters (use
multipart ETag + Content-Length) (5); pin total-order normalizer tie-breaks +
degenerate idempotence case (6); resolve the same-machine-only stability gap (7);
capture the resolver search radius into A/C goldens (8). These are additive edits,
not structural rework.

---

# Re-review — Round 2 (revised plan)

Re-verified the revised `m1-step-plan.md` line-by-line against the round-1
must-fix list and re-checked the same ground truth. No new code reads were
needed; the changes are plan-text, validated against the facts already cited.

## Round-1 objections — disposition

- **[MAJOR 1] B/C capture a non-production `tiff` reader, GDAL fidelity unproven**
  — *Resolved.* Assumptions now name `GdalRasterSource` as the production decode
  and require tile-identity before blessing (lines 21-25). Step 2 adds a mandatory
  isolated `shed-gdal` proof of B-window tile identity (same u8 flow-dir, f32 acc,
  nodata, geotransform), kept outside the `shed-core` graph (lines 164-170), with
  a hard "if identity cannot be proven, replan; do not bless a `tiff`-only oracle
  as production behavior" guard (lines 181-183). Step 4 requires the same proof
  for the localized C windows and **re-scopes C's claim** to "core TIFF-reader
  carve proven tile-identical to the GDAL production decode" (lines 338-342). The
  GDAL proofs are `--ignored`/network-gated and explicitly forbidden from being an
  unconditional `shed-core` dev-dependency (lines 202, 388-392). Gate-mapping row
  added (line 496). The fidelity gap is now closed *and* the overclaim corrected.

- **[MAJOR 2] No byte/tile ceiling on Oracle C** — *Resolved.* Step 4 now requires
  measuring and recording terminal bbox, per-raster tile count/bytes, total
  `Engine::http_stats().total_bytes_in`, and search radius before blessing, and
  enforces a hard per-outlet ceiling (recommended 500 MB) that fails the run even
  when the geometry is correct (lines 317-329). Step 5 verifies these fields are
  present in the C golden (lines 432-434); gate-mapping "Never-download guard for
  C" row added (line 500). This is the missing regression catch.

- **[MAJOR 3] `parity_golden_artifacts` durability could break at M2** —
  *Resolved.* Step 3 declares the v0.1 input hashes inert provenance and forbids
  re-hashing `manifest.json`/`catchments.parquet`/`graph.arrow` in the durable
  test (lines 241-243). Step 5 restricts the on-disk re-check to the M2-immutable
  B `flow_dir.tif`/`flow_acc.tif` at their committed path (lines 422-426). Both
  Step 3 and Step 5 require M2 to build its v0.2.1 fixture as a separate copy and
  never mutate/move the M1 B fixture in place (lines 255-257, 441-442). The
  durable gate no longer depends on files M2 rewrites or deletes.

- **[MAJOR 4] C outlet smallness asserted, not measured** — *Resolved.* The two
  outlets are now explicitly candidates accepted "only if live measurement proves
  they are small-window cases," with the measured bbox/tiles/bytes — not the names
  — as the justification, and any outlet exceeding the ceiling is replaced (lines
  312-321, 538-541).

- **[MINOR 5] 61 GB raster content-hash** — *Resolved.* C rasters now record
  multipart ETag verbatim + `Content-Length` from HEAD only; content hashing of
  `flow_dir.tif`/`flow_acc.tif` is forbidden in Step 4, the Out-Of-Scope guard,
  and verified in Step 5 (lines 330-334, 361-362, 430-431). The small
  fully-fetched artifacts may still use content hashes — correct.

- **[MINOR 6] Normalizer tie-breaks under-pinned** — *Resolved.* Step 1 pins start
  vertex, hole ordering, and component ordering as explicit lexicographic total
  orders bottoming out in the full rounded vertex sequence, adds degenerate
  idempotence cases (duplicate-after-rounding vertices, equal-primary-key
  multi-component), and scopes out antimeridian geometry (lines 75-89).

- **[MINOR 7] Same-machine-only stability** — *Resolved.* The canonicalizer-version
  note must state either ≥2-architecture validation or a documented pre-rounding
  margin, and `area_km2` uses the scalar-epsilon policy rather than byte-exact
  equality (lines 96-100, 525-529).

- **[MINOR 8] Resolver radius not captured** — *Resolved.* Resolver configuration
  including search radius is a required golden field (line 95), recorded for C
  (line 319) and verified in Step 5 (lines 416-417).

## New checks on the revisions (no regressions found)

- The B/C GDAL proofs reuse the `test-fixtures`-gated core source via a
  `shed-gdal` dev-dependency; this does not pull GDAL into `shed-core`'s own
  offline graph (`shed-core` has no gdal dep — `crates/gdal/Cargo.toml:9` is the
  only system-GDAL link), so `cargo build -p shed-core` /
  `cargo test -p shed-core` stay GDAL-free. Sound.
- Step 5 line 420 ("Verify B records committed fixture hashes") no longer conflicts
  with durability because lines 422-426 scope the *on-disk re-check* to the `.tif`
  only; "records ... hashes" now reads as golden-internal provenance presence.
- The retired-at-M2 capture test (`parity_v01_oracle_capture`) remains correctly
  outside the must-survive set (lines 509-511); only `cargo build -p shed-core`
  and `parity_golden_artifacts` must survive, and both are loader-/AtomId-free.

One soft, non-blocking note for the executor (not a must-fix): `localize_raster_window`
is `pub(crate)` (`session.rs:586`), so the `shed-gdal` C-window tile-identity proof
cannot call it directly — it must obtain the materialized window by running a
capture `delineate()` (which writes the windowed `.tif` under `HFX_CACHE_DIR`) and
then pointing both readers at that cached file. The plan's intent supports this;
worth stating the mechanism in the test README.

## Axis recheck

1. **Parity integrity** — Now sound: A's `NoRastersAvailable` is faithful, and
   B/C are bound to the production GDAL decode by a mandatory tile-identity proof
   with the claim re-scoped honestly.
2. **Durability across the cut** — Sound: durable test re-checks only immutable
   `.tif` + WKB canonicality; v0.1 inputs are inert; M2 copies rather than mutates.
3. **Ordering & hidden deps** — Sound, unchanged.
4. **Network-dependence of gates** — Sound: offline set is GDAL-/network-free;
   GDAL/network proofs are isolated and `--ignored`.
5. **Never-download invariant** — Now enforced: measured windows + hard ceiling +
   ETag/Content-Length-only raster identity.
6. **Synthetic fixture reality (B)** — Sound, with the GDAL fidelity gap closed.
7. **Normalizer completeness** — Sound: total-order tie-breaks + degenerate cases.
8. **Scope creep** — Clean, unchanged.
9. **Verifiability & conventions** — Sound: runnable commands, per-step bump+tag.
10. **Fragile goldens** — Addressed: cross-arch precision policy + resolver radius
    captured; f32 bbox remains a non-hazard (area derives from f64 geometry).

## VERDICT: CONVERGED

0 BLOCKER, 0 MAJOR. All four round-1 MAJORs and all four MINORs are closed with
specific, verifiable plan text, and the revisions introduce no new defects. The
oracle now provably captures today's engine (GDAL decode bound by tile-identity),
the never-download invariant is enforced by a measured byte ceiling, and the
durable artifact test no longer depends on anything M2 mutates. The plan is sound
to execute.
