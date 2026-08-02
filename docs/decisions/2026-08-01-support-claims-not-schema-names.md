# A reader's support claim, not the schema name, protects the caller

The 2026-07-24 live fire left a residual hazard framed as a naming problem: a
declaration that outruns deployed readers should be named so those readers fail
loudly instead of silently carving the wrong geometry. That framing does not
survive its own evidence. Every version token in play on 2026-07-24 was
correct. `hfx.aux.d8_raster.v1` had already moved to `v2` for the
declarative-CRS change, and pourpoint 0.2.0 genuinely implemented v2 — in the
object its tests constructed. The projected-GRASS golden records its source as
`LocalTiffRasterSource::with_encoding(hfx::FlowDirEncoding::Grass) ->
EncodedLocalTiffRasterSource`
(`crates/core/tests/parity_golden_artifacts.rs:354` at tag
`pourpoint-v0.2.0`), while both shipped entry points constructed
`GdalRasterSource::new()`, whose default was `FlowDirEncoding::Esri`
(`crates/gdal/src/raster_reader.rs:41`, `crates/python/src/engine.rs:214`, `src/main.rs:185`, all at tag
`pourpoint-v0.2.0`). The declared `flow_dir_encoding: "grass"` reached a builder
no shipped code path called. The golden was green, the release was green, and
the geometry was wrong. No naming rule reaches that defect: a rename only helps
a reader that is honest about what it implements, and a reader that over-claims
keeps over-claiming under every name anyone invents.

The protection therefore sits on the claim side. A reader holds an explicit
**support claim** for each declared value it branches on, and a claim counts
only when its evidence runs through the construction path shipped code takes.
Evidence through a hand-configured object proves a component nobody runs. The
claim surface is bounded by a property that has an answer in the code rather
than in judgement — does anything branch on this value — which puts
`format_version`, the dataset CRS, the auxiliary schema names, and the D8
metadata fields `crs`, `flow_dir_encoding`, and `flow_acc_units` close to the
whole set. A value that is only recorded, logged, or passed through needs no
claim; it acquires one at the moment it becomes load-bearing.

A declared value the reader holds no claim for must change what the caller
receives, never only what is logged. It produces the typed, diagnosable outcome
that `RefinementMode::BestEffort` already establishes for every other D8-path
failure, so a degraded result names its own degradation. A warning printed
beside a result that still reports `Applied` is silence with extra steps: the
batch job and the notebook cell that consume the geometry never read stderr.

Schema-name discipline survives as a consequence of this rather than as the
mechanism. Naming remains how a *dataset* signals that its contract moved; it
is simply not what protects a caller from a reader that misreads the contract
it claims to implement. Rejected alternatives: a dataset-side rule obliging a
rename on every semantic change, which would not have prevented the fire it is
meant to prevent; and distribution control over old releases — advisory wording
or a package-index yank — which cannot reach an already-installed environment,
the exact population that produces the wrong shape.

The cost is an evidence obligation on every branch over a declared value, paid
at the point the branch is written. The benefit is that over-claiming stops
being an accident a passing test can hide and becomes a deliberate act. The
specific 2026-07-24 hole is already closed — #62 made the declared encoding a
required argument of `RasterSource::load_flow_direction` and deleted both
`EncodedLocalTiffRasterSource` and the encoding constructors — so this record
governs the next declared field, not that one.

Decided 2026-08-01 during discovery for Effort ticket #100. Implemented and
shipped at `c7413a182077984bc335b7c094e62af6c5eb228e` in three layers. First,
the reader retains unrecognized `hfx.aux.*` declarations in
`AuxDeclarations::unreadable`, preserves their raw data, and excludes their
paths from required local and remote artifact validation
(`crates/core/src/reader/manifest.rs:67`,
`crates/core/src/reader/manifest.rs:81`,
`crates/core/src/reader/manifest.rs:239`,
`crates/core/src/reader/manifest.rs:311`,
`crates/core/src/session.rs:1009`, and `crates/core/src/session.rs:1051`).
Second, best-effort refinement reports the typed
`BestEffortSkipReason::UnreadableD8AuxDeclared`, selected by the engine and
exposed by the Python result accessor (`crates/core/src/refinement.rs:456`,
`crates/core/src/engine.rs:732`, and `crates/python/src/result.rs:158`).
Third, production support claims live in the core-manifest,
flow-direction-encoding, auxiliary-schema, and D8-metadata inventories and
their aggregate (`crates/core/src/support_claims.rs:179`,
`crates/core/src/support_claims.rs:136`,
`crates/core/src/support_claims.rs:171`,
`crates/core/src/support_claims.rs:211`, and
`crates/core/src/support_claims.rs:219`); the manifest reader consults those
claims (`crates/core/src/reader/manifest.rs:174`,
`crates/core/src/reader/manifest.rs:188`, and
`crates/core/src/reader/manifest.rs:342`), with compiled-CLI witnesses
beginning at `tests/reader_support_claims.rs:880`,
`tests/reader_support_claims.rs:902`,
`tests/reader_support_claims.rs:924`,
`tests/reader_support_claims.rs:986`,
`tests/reader_support_claims.rs:1118`,
`tests/reader_support_claims.rs:1136`,
`tests/reader_support_claims.rs:1156`, and
`tests/reader_support_claims.rs:1189`.

The source-derived correspondence gate finds supported inventory declaration
forms, requires every inventory in the aggregate, checks claim and witness
membership in both directions, rejects duplicate claim IDs, witness IDs, and
aggregate entries, and requires every witness to name a declared Rust test
(`tests/reader_support_claims.rs:718-822`). This is a structural obligation,
not proof of evidence semantics: it does not inspect test bodies, prove that a
named test exercises its claim, or scan `crates/python/tests/`
(`docs/reader-support-claims.md:99-117`).

The shipped evidence remains deliberately bounded. Across all thirteen
catalogued rows, “Typed inventory only” means an in-process assertion of the
exact `ReaderSupportValue`, not typed-value discrimination through the shipped
path (`docs/reader-support-claims.md:31-34`). The generic representative has
only shipped-CLI behavior coupled to catalog-literal correspondence; D8-v2
and snap-v2 likewise do not prove their typed variants through the shipped
path (`docs/reader-support-claims.md:52-57`). The other witnesses' recorded
`DeclarationDiscriminatingCompiledCli` strength must not be read as a blanket
claim that every typed value is discriminated through the shipped path.

Unreadable-D8 degradation eligibility is only a diagnostic routing heuristic:
the declared name must begin with the exact `hfx.aux.d8_raster.` family
prefix, no code verifies that a referenced raster exists or contains D8 data,
and only the first matching declaration in manifest order is reported
(`crates/core/src/refinement.rs:461-469`,
`crates/core/src/session.rs:612-619`, and
`docs/reader-support-claims.md:58-63`). The typed outcome arises only under
`RefinementMode::BestEffort` when no readable D8 auxiliary is declared
(`crates/core/src/engine.rs:741-747`). A readable `hfx.aux.d8_raster.v2` pair
alongside an unreadable D8-family declaration therefore yields no typed
degradation; refinement proceeds, while the unreadable declaration is
reported only through `Engine.unreadable_auxiliary_schemas`. Under
`RefinementMode::RequireD8`, the caller instead receives
`SessionError::MissingRequiredD8Aux`, whose message names the required v2
schema rather than the unreadable declaration
(`crates/core/src/error.rs:483-485`). In both cases, the norm above that an
unsupported declaration must change what the caller receives is not yet met
by the reader. A schema name therefore remains a routing signal, not
capability proof.
