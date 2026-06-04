# Adversarial Critique — M5 Basin GeoParquet Export Plan

Target: `docs/hfx-v02-redesign/m5-export-plan.md` (folded into Milestone 5).
Reviewer stance: attack the design. Vague approval is failure.

Repos read: `shed` (`engine.rs`, `assembly.rs`, `wkb.rs`, `src/main.rs`), `../hfx`
(`spec/HFX_SPEC.md`, `crates/hfx-core/src/manifest.rs`, `crates/hfx-core/src/id.rs`,
`crates/hfx-validator/README.md`). Reuse surface formed independently before judging.

Ground-truth I verified before objecting:

- `DelineationResult` exposes exactly the accessors the plan names (`engine.rs:73-128`):
  `terminal_unit_id() -> UnitId`, `resolved_outlet()`, `resolution_method()`,
  `upstream_unit_ids() -> &[UnitId]`, `refinement() -> &RefinementOutcome`,
  `geometry() -> &MultiPolygon<f64>`, `area_km2()`, `geometry_wkb()`. Good — the input
  surface is real and stable enough to build on.
- `RefinementOutcome` is already an enum with three variants (`Applied`,
  `BestEffortSkipped`, `Disabled`) — `engine.rs:41-56`. The plan's three string states
  map 1:1. Good.
- `encode_wkb_multi_polygon` writes 2D (`CoordDimensions::xy()`) and the geometry is
  always `MultiPolygon<f64>` — so `geometry_types: ["MultiPolygon"]` (2D, no Z) is
  honest. Good.
- The "assembly.rs writer to mirror" is `write_catchments_fixture`, which lives **inside
  `#[cfg(test)] mod tests`** (`assembly.rs:271-272, 690`). It is a test fixture helper,
  not a production writer. The plan half-acknowledges this but then repeatedly says
  "mirror the assembly.rs writer."
- **`hfx-core::Manifest` has BOTH `fabric_version() -> Option<&str>`
  (`manifest.rs:214`) AND `adapter_version() -> &str` (`manifest.rs:257`). They are
  distinct fields.** The plan proposes to source the label from `adapter_version()` and
  *alias* it as `fabric_version`. This is wrong (Objection 1).
- `UnitId` is `UnitId(i64)` (`id.rs:23`) — **signed**, not `u64` (Objection 6).
- HFX **defers Hilbert ordering in v0.2.1**: validator README line 100 — "Curve
  parameters not yet specified in the spec; ... not emitted in v0.2.1"; HFX_SPEC — "Hilbert
  ordering remains deferred until curve [parameters specified]." There is no canonical
  curve to mirror and no shed code that implements one (`rg hilbert` → zero hits in
  `shed/`). (Objection 3, the central one.)
- No `crates/core/src/export/` exists yet; no precedent anywhere in `shed` for writing a
  Parquet `geo` file-level key/value metadata entry (existing readers/fixtures never do).
  The metadata-write mechanism is genuinely net-new and under-specified (Objection 2).
- A real CLI already exists (`shed/src/main.rs`, clap `Subcommand` with `Delineate`), so
  E7 has a real home but is also `anyhow` application glue (Objection 11).

---

## BLOCKERS

### 1. [BLOCKER] `delineation` label sources the WRONG manifest field — `adapter_version` is not the fabric data version

`manifest.rs` exposes two distinct strings: `fabric_version() -> Option<&str>` (the
**source-fabric data version**, e.g. GRIT `2.0.0`, MERIT-Basins `0.1.0`) and
`adapter_version() -> &str` (the **adapter/tooling version** that produced the HFX
dataset). The plan (lines 9, 71-72, 82) proposes deriving the `delineation` label from
`adapter_version()` and "calling it `fabric_version` only as an export alias."

This breaks the central HDX requirement — *same caller `basin_id`, distinct rows by
delineation* — at the semantic level:

- The whole point of `delineation` is to distinguish the **fabric/data** that produced a
  basin's geometry. Two different source-fabric data versions processed by the **same
  adapter build** would collapse to the *same* `adapter_version` string and produce
  **identical `delineation` labels** → a `(basin_id, delineation)` collision the writer
  then hard-rejects, OR two semantically different delineations silently mislabeled as one.
- Conversely, re-running a newer adapter build over the *same* fabric data bumps
  `adapter_version` and spuriously forks the delineation label, fragmenting "same basin
  across the same fabric."

Fix demanded: derive the version component of `delineation` from **`fabric_version()`**,
not `adapter_version()`. Because it is `Option`, the plan MUST specify the `None`
fallback (reject? substitute a sentinel? fall back to `adapter_version`?) — this is a
required, documented decision, not an implementation detail. If the downstream team
actually wants adapter provenance *too*, carry it as a separate optional column, not as a
relabeling of the identity-bearing `delineation` field. **This also resolves the plan's
own hedge ("if that is the agreed public wording") — see ESCALATE-A.**

### 2. [BLOCKER] GeoParquet validity hinges on a `geo` footer entry the plan never specifies how to write — and never tests at the Parquet layer

Axis 2 is "will a STANDARD GeoParquet reader accept this." A standard reader
(GeoPandas/pyogrio/GDAL) reads the **Parquet file-level `key_value_metadata`** for the
`geo` key. The plan asserts the `geo` JSON content (lines 42-53) but never says *how it
reaches the Parquet footer*, and there is **no precedent in the repo** for doing so. With
`arrow-rs` there are two non-equivalent paths (embed in the Arrow `Schema` metadata
HashMap vs. `ArrowWriter::append_key_value_metadata`), and a naïve writer that only sets
`WriterProperties` will emit a parquet-with-a-binary-column that **no GeoParquet reader
recognizes** — exactly the "just a parquet file with a geometry column" failure the axis
warns against.

Compounding this, the E4 gate ("primary geometry column metadata ... WKB encoding ...
EPSG:4326 PROJJSON") reads as an *in-memory schema* assertion. That can pass while the
footer is empty.

Fix demanded: (a) name the concrete mechanism for writing the `geo` key into the Parquet
`key_value_metadata`; (b) the round-trip gate (E5/E6) MUST re-open the written file with a
**Parquet reader**, pull `key_value_metadata`, parse the `geo` JSON, and assert
`version`, `primary_column`, `encoding=WKB`, `geometry_types`, and the EPSG:4326 PROJJSON
`id` — *from the footer*, not from the schema object held in memory. Strongly recommend
also validating one file with an actual external GeoParquet reader (e.g. a pyshed/pyogrio
smoke check) at least once before calling axis 2 satisfied; otherwise "GeoParquet-valid"
is asserted, not proven.

---

## MAJORS

### 3. [MAJOR] The Hilbert sort has no spec to mirror, no reference implementation, and an under-defined, non-deterministic normalization

The plan calls Hilbert "the only net-new algorithmic piece" and says to "mirror the
assembly.rs writer" / "HFX house style." Both reuse claims are hollow:

- **HFX defers Hilbert in v0.2.1** (validator README l.100; HFX_SPEC "remains deferred
  until curve parameters specified"). HFX itself does not emit or validate it. There is no
  canonical curve, bit depth, or x/y bit-interleave order to conform to.
- `assembly.rs` contains **no Hilbert code**; `rg hilbert shed/` is empty. There is
  nothing to mirror. The writer to imitate is a `#[cfg(test)]` fixture that sorts nothing.

So the plan is silently *inventing* an ordering and badging it "house style." Worse, the
one normalization it does specify — "Hilbert index from centroid coordinates **normalized
over the file-level bbox extent**" (line 120, 127) — is **batch-dependent**: change the
set of basins in the export and every centroid renormalizes, so the same physical basin
lands at a different Hilbert index and a different row position run-to-run. For a "delineate
once, extract many" catalog this is a determinism trap if anyone treats row order or index
as stable.

Fix demanded, pick one and write it down:
  (a) **Drop Hilbert** for M5. Since HFX emits none, a plain deterministic sort (e.g.
      `(bbox_minx, bbox_miny, basin_id, delineation)`) gives reproducible locality without
      pretending to a curve that the contract it cites doesn't define. This is the
      lower-risk, lower-ceremony choice and matches "documented format, not a spec."
  (b) **Own the curve explicitly**: fix the bit depth, the normalization extent
      (recommend a **fixed global extent `[-180,-90,180,90]`**, not the file bbox, so the
      index is basin-stable), and the bit-interleave order in the documented format, and
      add a test with hand-computed expected indices for 3-4 known centroids. Then state
      plainly that shed is *ahead of* the HFX spec here, not mirroring it.
Either way, delete the "mirror assembly.rs / HFX house style" framing for Hilbert — it is
not accurate. **This choice is partly the downstream team's — see ESCALATE-B.**

### 4. [MAJOR] f64→f32 bbox narrowing must round outward, or row-group pruning silently drops intersecting rows

`bbox_*` columns are `Float32` (HFX spec, l.69-72, correctly matched). But the geometry is
`f64`. Default `as f32` / nearest-rounding can **shrink** the box: `minx` rounds up,
`maxx` rounds down. A consumer doing row-group/page pruning on the bbox statistics will
then **skip a row group that actually intersects the query** — a silent wrong-results bug,
not a perf nit. The plan says nothing about rounding direction.

Fix demanded: round outward — `bbox_minx/miny = next_down(f32)`, `bbox_maxx/maxy =
next_up(f32)` (or `f32::from_bits` nudge / `floor`/`ceil` at f32 ULP). Add a test that an
f64 box whose bounds are not exactly representable in f32 yields f32 columns that
*contain* the true box. The dataset-level `geo.bbox` covering value should likewise be a
true cover (f64 is fine there).

### 5. [MAJOR] Default `basin_id = terminal_unit_id` silently collides when a catalog has multiple outlets in one drainage unit

The convenience default (lines 24, 66) mints `basin_id` from
`DelineationResult::terminal_unit_id()`. Two distinct caller pour-points that resolve to
the **same terminal unit** (entirely normal for a dense catalog) produce the **same
default `basin_id`** → the writer's uniqueness validator rejects the *entire* export. The
failure is far from the cause (the user gave two valid points; the export blows up on a
derived id they never saw). Filesystem-safety of the decimal string is fine; **collision**
is the real hazard and the plan doesn't name it.

Fix demanded: (a) document that the terminal-unit default is only safe for catalogs with
at most one outlet per terminal unit; (b) on a default-id collision, error with a message
that names *both* originating outlets and tells the caller to supply explicit `basin_id`s
— do not surface it as an opaque "duplicate row." Add a test for the two-outlets-one-unit
case asserting that diagnostic.

### 6. [MAJOR] `BasinId` validity is a fragile blocklist, and `UnitId` is signed

Two problems in the identity rules (lines 57-66):

- The plan defines `BasinId` by **what it forbids** ("free of `/`, traversal, control
  chars, ... platform-hostile punctuation"). Blocklists rot: it omits Windows reserved
  device names (`CON`, `NUL`, `COM1`…), trailing dot/space, the `=` that structures the
  Hive `basin=<id>` segment itself, and percent/URL-hostile bytes. "Platform-hostile
  punctuation" is not a specification. For a contract that becomes a **directory name**,
  use a strict **allowlist** (recommend `^[A-Za-z0-9._-]+$`, non-empty, not `.`/`..`,
  bounded length), which is enumerable and testable.
- `UnitId` is `i64` (signed). The terminal-unit default "formatted as decimal" can emit a
  leading `-`. Decide and document whether negative ids are possible in HFX and whether
  `-` is acceptable in a `basin=<id>` segment; if not, reject at the default boundary.

Fix demanded: replace the blocklist with an allowlist regex + explicit reserved-name and
length rules in `BasinId::parse`; add rejection tests for reserved names, trailing dot,
`=`, empty, `.`/`..`; define the signed-id default behavior.

### 7. [MAJOR] Row-group policy invents a "non-final" exemption the spec text doesn't grant, and the gate never tests the realistic 50k case

The plan (line 133) requires "every **non-final** row group has 4,096–8,192 rows." HFX
spec (l.96-98) says: "Files with 4,096 or more rows **must** use row groups of 4,096–8,192
rows" — with **no final-group exemption in the normative sentence**. For the stated
~50k-basin target: `50000 = 6×8192 + 848`, so the final group is **848 rows < 4,096** —
a violation under the literal rule. The plan papers over this by inserting "non-final,"
and its E5 gate only tests "tiny and 4,096+", never a count whose remainder underflows
4,096.

Fix demanded: (a) state explicitly how the final short group is handled and reconcile it
with the spec wording — note that HFX itself classifies row-group sizing as a **WARN**
diagnostic (HFX_SPEC l.101), so a short tail is tolerable *if you say so*; don't smuggle in
"non-final" as if the spec granted it; (b) add a row-group test at a realistic count with
an awkward remainder (e.g. 9,000 or 50,000 synthetic rows is too big — use ~9,000) and
assert the actual per-group sizes, including the tail. Also pin down the
`ArrowWriter`/`set_max_row_group_size` feeding pattern (one big batch vs. streamed
batches) because it determines whether you even get the group boundaries you assert.

---

## MINORS

### 8. [MINOR] "Mirror the assembly.rs writer" overstates a `#[cfg(test)]` helper

`write_catchments_fixture` is test-only and writes the *catchments* schema, not basin
exports. Lifting a test fixture into a production writer is fine as a starting reference,
but say that plainly ("adapt the WriterProperties/EnabledStatistics::Chunk pattern from
the test fixture") instead of "mirror the assembly.rs writer," which implies a production
writer that doesn't exist. Keeps the next implementer from hunting for it.

### 9. [MINOR] Empty-input behavior is named as an error variant but never decided

`ExportError` lists an "empty input policy" variant (line 88) and the `geo.bbox` is
"computed ... when at least one row is written" (line 51). But is zero rows an **error** or
a **valid empty GeoParquet** (no `bbox`)? HDX "delineate once" pipelines can legitimately
produce an empty shard. Decide and test it; don't leave it as a variant with no policy.

### 10. [MINOR] Geometry winding/orientation metadata unaddressed

The plan omits `columns.geometry.orientation`. Omitting it is *legal* in GeoParquet 1.1
(means "unspecified"), but if HDX assumes RFC7946 CCW-exterior winding, an unspecified
file is a latent interop mismatch. Either declare the orientation you actually guarantee
(and confirm the dissolve/M1 normalizer produces it) or note in the documented format that
winding is unspecified so consumers don't assume.

### 11. [MINOR] E7 CLI step lands in `anyhow` glue and must map errors + validate IDs pre-delineation

A CLI already exists (`shed/src/main.rs`, `anyhow`). The plan's E7 gate says "rejects
unsafe basin IDs before delineation where practical." Make that **not** "where practical":
parsing `BasinId` is cheap and must happen *before* the expensive delineation loop so a
50k-row catalog with one bad id fails in milliseconds, not after hours. Also specify the
`ExportError -> anyhow` mapping so library `thiserror` variants surface as useful CLI
context. Deferring E7 entirely (as the plan permits) is acceptable.

### 12. [MINOR] `geo` covering columns not advertised to GeoParquet readers

The bbox_* columns satisfy HFX house style, but GeoParquet 1.1 has a first-class
`columns.geometry.covering.bbox` metadata pointer that lets *GeoParquet* readers use those
columns for spatial pruning. Optional, but cheap to declare and turns dead weight into a
reader-usable index. Consider adding it; flag as a documented choice if you skip it.

---

## Axis scorecard (independent judgment)

1. **HDX conformance** — One-row-per-(basin,delineation): schema OK. Filesystem-safe
   unique id: weakened by blocklist (Obj 6) and default-collision (Obj 5). Multi-fabric
   "same id, distinct delineation": **broken at the source field** (Obj 1). Not yet
   conformant.
2. **GeoParquet validity** — Achievable, but the `geo`-footer write mechanism is
   unspecified and untested at the Parquet layer (Obj 2). Currently "asserted, not proven."
3. **HFX house style** — Hilbert claim is not grounded in any spec or code (Obj 3); bbox
   f32 narrowing is unsafe (Obj 4); row-group rule diverges from spec wording and is
   undertested (Obj 7). The columns/types themselves match the spec.
4. **Ceremony discipline** — Mostly right: documented-format-not-spec stance, no validator,
   no conformance suite. No over-build. The under-specification is in the wrong places
   (Hilbert, geo-footer, id charset), not over-engineering.
5. **Scope/deferral leakage** — Clean. Additive `export/` module; M1 canonicalizer/goldens,
   M3 stages, M4 carve untouched; pyshed/Phase-4/aux-binding stay deferred; no delineation
   behavior change. E6 even asserts no M1/M3/M4 fixtures are touched. Good.
6. **M5 fold quality** — Sequencing (after result-type stabilization, before
   migration-doc closure) is sound. Gates are real `cargo test` filters, not hand-waves —
   but several gates test the wrong layer (Obj 2 schema-vs-footer, Obj 7 missing realistic
   row count). Round-trip is real in shape; make it read the footer.
7. **Type-driven / conventions** — `BasinId`/`DelineationLabel`/`ExportMethod` newtypes,
   parse-at-boundary, `RefinementOutcome` enum reuse, `thiserror` with when-fired docs,
   batch API off the hot path: all correct in intent. Per-commit version + `Cargo.lock` +
   tag is carried in Boundaries. Tighten `BasinId` parsing (Obj 6) and the signed-id
   default.

---

## Escalations (genuinely a human / downstream-team joint call)

- **ESCALATE-A — `delineation` version semantics.** Whether the label carries source
  **fabric data version** (`fabric_version()`, my position) vs. **adapter version**
  (`adapter_version()`), and the `None`-fabric-version fallback, determines whether the
  multi-fabric HDX layout is even correct. The plan itself hedges ("if that is the agreed
  public wording"). This is the downstream HDX team's contract decision, not the
  implementer's. Resolve before E2.
- **ESCALATE-B — Hilbert ownership.** Because HFX explicitly defers the curve, shed must
  either drop Hilbert (treat as deterministic locality sort) or **own and publish** curve
  parameters ahead of the spec. That is a contract-direction call shared with the HFX/HDX
  owners — shed should not unilaterally mint a "house-style" curve and imply conformance.

---

## VERDICT: SEND BACK

Two BLOCKERs (wrong manifest version field driving the identity-bearing `delineation`
label; unspecified + untested GeoParquet `geo`-footer write) plus four MAJORs (ungrounded
Hilbert with non-deterministic normalization; unsafe f32 bbox narrowing; silent
default-id collisions; fragile blocklist id rules + signed-id default; row-group rule that
diverges from spec wording and dodges the realistic count) are enough that the schema and
its conformance claims are not yet safe to implement as written. None are architectural —
the module placement, reuse surface, type-driven shape, deferral discipline, and step
sequencing are sound — but the identity semantics, GeoParquet-validity proof, and the
house-style claims must be corrected and the two escalations resolved before dispatch.

Re-submit with: (1) `fabric_version()`-sourced label + `None` policy; (2) named geo-footer
mechanism and a Parquet-footer round-trip assertion; (3) Hilbert decision (drop or
fully-specified+global-extent) with the "mirror HFX/assembly" framing removed; (4)
outward f32 bbox rounding + test; (5) default-id collision diagnostic + test; (6)
allowlist `BasinId` parse + signed-id default policy; (7) row-group tail handling
reconciled with spec wording + a realistic-remainder test. Escalations A and B answered by
the downstream owners.
