# Reader support claims

This maintainer document describes the current main checkout. References to an
installed wheel mean a wheel built from that checkout, not PyPI release 0.3.0.
`DelineationResult.refinement_skip_reason` and
`Engine.unreadable_auxiliary_schemas` are Unreleased/main-only.

> Every catalogued declaration that production behavior distinguishes has one
> independent evidence witness, and every witness names a declared integration
> test. Inventory aggregation and claim/witness correspondence fail the build.

## Enforced catalog boundary

`crates/core/src/support_claims.rs` contains four deliberately separate claim
inventories. `READER_SUPPORT_CLAIM_INVENTORIES` references them in this order:

1. `CORE_MANIFEST_SUPPORT_CLAIMS` (two rows)
2. `FLOW_DIRECTION_ENCODING_SUPPORT_CLAIMS` (three rows)
3. `AUXILIARY_SCHEMA_SUPPORT_CLAIMS` (four rows)
4. `D8_METADATA_SUPPORT_CLAIMS` (four rows)

The aggregate therefore contains 13 rows. Stable claim IDs join these production
claims to an independent, test-local witness table. The structural gate enforces
inventory/aggregate membership and catalog/witness membership in both set
directions, rejects duplicate catalog IDs, witness IDs, and aggregate entries,
and enforces witness-to-declared-test existence in one direction. Several claims
may name the same evidence test.

Completeness is bounded by production branching. A declaration belongs in the
catalog only when a production `match`, `if`, or comparison distinguishes its
parsed value. Parsing, storing, exposing, or logging a value does not create a
support-claim obligation.

## Evidence correspondence

“Typed inventory only” means an in-process assertion checks the exact
`ReaderSupportValue`. It is not shipped-path typed-value discrimination. In
particular, `hfx::Crs::Epsg4326` is not discriminating typed evidence because
HFX 0.5.0 has only that `Crs` variant.

| Claim ID | Exact evidence test | Observable declaration evidence | Typed-value evidence | Evidence strength |
|---|---|---|---|---|
| `core-format-version-0.3.0` | `format_version_claim_has_shipped_cli_evidence` | Compiled CLI accepts `0.3.0` and rejects a temporary `0.2.1` manifest. | Typed inventory only. | Declaration-discriminating compiled-CLI evidence. |
| `core-dataset-crs-epsg-4326` | `dataset_crs_claim_has_shipped_cli_evidence` | Compiled CLI accepts `EPSG:4326` and rejects a temporary `EPSG:3857` manifest. | Typed inventory only; the HFX enum itself is not discriminating. | Declaration-discriminating compiled-CLI evidence. |
| `core-flow-dir-encoding-esri` | `esri_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` | Temporary manifest-only ESRI declaration yields the exact invalid-nodata refinement outcome, area, and ordered ring counts while TIFF bytes remain equal. | Typed inventory only. | Declaration-discriminating compiled-CLI evidence. |
| `core-flow-dir-encoding-taudem` | `taudem_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` | Temporary manifest-only TauDEM declaration yields its expected refinement, area, and ordered ring counts while TIFF bytes remain equal. | Typed inventory only. | Declaration-discriminating compiled-CLI evidence. |
| `core-flow-dir-encoding-grass` | `grass_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` | The unchanged tracked GRASS declaration yields its expected refinement, area, and ordered ring counts; changing only that declaration to TauDEM makes the test red. | Typed inventory only. | Declaration-discriminating compiled-CLI evidence. |
| `aux-schema-d8-raster-v2` | `auxiliary_schema_claims_have_shipped_cli_evidence` | An unmodified tracked D8-v2 fixture succeeds through the compiled CLI. | Typed inventory only; no shipped-path typed-value discrimination. | Declaration-discriminating compiled-CLI behavior. |
| `aux-schema-snap-v2` | `auxiliary_schema_claims_have_shipped_cli_evidence` | A builder-created snap-v2 dataset succeeds through the compiled CLI. | Typed inventory only; no shipped-path typed-value discrimination. | Declaration-discriminating compiled-CLI behavior. |
| `aux-schema-generic` | `auxiliary_schema_claims_have_shipped_cli_evidence` | Present/missing provisional and third-party artifacts exercise generic behavior; the representative literal is checked against the catalog after all eleven CLI calls. | Typed inventory only. | Shipped-CLI behavior coupled only to independent catalog-literal correspondence, not representative-declaration discrimination. |
| `aux-schema-d8-raster-v1-unsupported` | `auxiliary_schema_claims_have_shipped_cli_evidence` | A builder-created manifest changed to D8-v1 opens and delineates through the compiled CLI; the shipped MERIT reduction with 60 D8-v1 declarations does the same and refined GeoJSON names the exact first unreadable D8-family schema. The installed-wheel test separately asserts `DelineationResult.refinement_skip_reason` and that `Engine.unreadable_auxiliary_schemas` retains all 60 declarations in manifest order. | Typed inventory only; `claimed_auxiliary_schema` cannot return this row, so D8-v1 remains unsupported for decoding and is retained outside the typed manifest. | Declaration-discriminating compiled-CLI evidence plus separate typed installed-wheel evidence. |
| `core-d8-crs-epsg-4326` | `geographic_d8_claims_have_shipped_cli_evidence` | The tracked geographic/cells dataset refines through the compiled CLI. | Typed inventory only. | Declaration-discriminating compiled-CLI behavior. |
| `core-d8-crs-epsg-8857` | `projected_d8_claims_have_shipped_cli_evidence` | The unchanged projected/km2 fixture refines through the compiled CLI. | Typed inventory only. | Declaration-discriminating compiled-CLI behavior. |
| `core-d8-flow-acc-units-cells` | `geographic_d8_claims_have_shipped_cli_evidence` | Geographic/cells refinement succeeds, then a copied manifest changed only to `km2` follows the rejection behavior with raster bytes unchanged. | Typed inventory only. | Declaration-discriminating compiled-CLI behavior. |
| `core-d8-flow-acc-units-km2` | `projected_d8_claims_have_shipped_cli_evidence` | The unchanged projected/km2 fixture refines through the compiled CLI. | Typed inventory only. | Declaration-discriminating compiled-CLI behavior. |

The generic representative is deliberately weaker than declaration-discriminating
evidence: its provisional and third-party CLI outcomes do not discriminate
`hfx.x.experimental.v1`; catalog equality after shipped behavior does. Typed-value
only mutations can remain CLI-green and are caught by separate in-process
inventory assertions. This is also the boundary for D8-v2 and snap-v2: their
witnesses do not prove their typed variants through the shipped path. D8-v1 has
a different named non-support outcome: the reader retains each unreadable
declaration outside the typed manifest, the dataset remains openable, and the
Python `Engine` reports the retained schema names to callers. Best-effort
refinement reports only the first unreadable declaration whose name begins with
the exact `hfx.aux.d8_raster.` prefix, in manifest order. This routing
classification does not verify that a referenced artifact is a D8 raster.

The removed flow-direction pairwise-distinctness helper was a decorative fourth
copy of outcomes and supplied no protection. The consumed production cases and
manifest-only mutation reds are the evidence. GRASS and TauDEM share an outlet
string but differ in ordered ring counts; ESRI has the distinct invalid-nodata
outcome. Area uses a tolerance of `0.025_f64` and a stable failure diagnostic.

## Source-level mechanism and limits

The permanent integration test uses a small standard-library lexer. It scans
only `crates/core/src/support_claims.rs`, skipping whitespace, comments, string
and character literals, and lifetimes. It detects private inventories and those
with `pub`, `pub(crate)`, `pub(super)`, or `pub(in path)` visibility when they use
`const` or `static` and declare `&[ReaderSupportClaim]`,
`&[ReaderSupportClaim; N]`, or `[ReaderSupportClaim; N]`. Every detected
inventory is required exactly once in the aggregate. Because lifetime tokens
are skipped, `pub const X: &'static [ReaderSupportClaim] =` is also covered. The
aggregate initializer may contain only comma-separated bare identifiers.

The mechanically covered declaration forms above are the mandatory authoring
convention. These forms are known, unenforced limitations:

- Form C: a type alias followed by `pub const X: &ClaimSlice = ...`.
- Form D: a parameterized `macro_rules!` expansion whose body declares
  `pub const $name: &[ReaderSupportClaim] = ...`.
- Form F: `pub const X: &[ReaderSupportClaim] = ...` in another file, such as
  `crates/core/src/snap_claims.rs`.
- A raw-identifier inventory name, such as
  `pub const r#SNAP_CLAIMS: &[ReaderSupportClaim] = ...`.
- A path-qualified element type, such as
  `pub const X: &[self::ReaderSupportClaim] = ...`.
- A witness-named `#[test]` declared inside a `#[cfg(...)]`-gated `mod`; the
  module and test may be compiled out without failing the gate.
- An `ignore` or `cfg` introduced indirectly through `cfg_attr` on a
  witness-named test.
- Evidence in `crates/python/tests/` is not scanned by the structural gate. A
  witness row can name a Rust test while a substantive assertion, such as the
  typed installed-wheel accessor assertion, lives only in a Python test.

The same lexer scans `tests/reader_support_claims.rs` for attribute runs followed
by `fn <IDENT>(`. A run must contain `#[test]` and must not contain `#[ignore]` or
`#[cfg(...)]`, regardless of attribute order. This check reads only the attribute
run attached to the test function itself; attributes on enclosing items are not
considered. Every witness's `evidence_test` must be one valid identifier and must
occur in the declared-test set computed this way. Deleting a named test outright,
or disabling it with `ignore` or `cfg` in its own attribute run, fails the gate.
The check does not inspect test bodies, prove that a named test still invokes the
CLI, validate evidence semantics, or evaluate `ignore` or `cfg` introduced
indirectly through `cfg_attr`. The current compiled-CLI bodies, the full focused
suite, and mutation controls establish those properties for the current
implementation. In particular, the D8-v1 Rust witness proves compiled-CLI
acceptance, delineation, and exact first-schema disclosure in refined GeoJSON.
The installed-wheel Python test separately proves the public typed accessor and
is outside the structural scan.

## Independence and anti-circularity

Witness IDs and fixture declaration literals must remain independent values,
never values read from the claim catalog. `DatasetBuilder` provides independent
evidence inputs: at ground ref
`fc6ea4a60b970883bf6bd4a4df601499c49c9c54`,
`crates/core/src/testutil.rs:313-325` contains literal
`"format_version": "0.3.0"` and `"crs": "EPSG:4326"` values.

Sourcing any fixture manifest value from the claim catalog is permanently
prohibited. Such a DRY refactor would mutate claim and evidence together, make
the witness circular, and recreate the false-green class this correspondence
mechanism exists to prevent.

## D8 compatibility boundary

D8 CRS and accumulation-unit lookups compare complete declaration strings with
no case folding, normalization, wildcard, default, or fallback. The supported
matrix is EPSG:4326/cells, EPSG:8857/cells, and EPSG:8857/km2; EPSG:4326/km2 is
incompatible. The exact diagnostic remains
`flow accumulation units km2 require projected pixel area, but EPSG:4326 is geographic`.
The shipped evidence establishes the outcome, but does not claim to verify the
guard's pre-snapping ordering.

## Topology exclusion trace

At governing ref `fc6ea4a60b970883bf6bd4a4df601499c49c9c54`, production parses
topology at `crates/core/src/reader/manifest.rs:177-185`, stores it through
`ManifestBuilder` at `:237-247`, logs it at
`crates/core/src/session.rs:310-315` and `:566-572`, and exposes it at `:612-615`.
The only workspace occurrences of `Topology::Tree` or `Topology::Dag` are tests;
the example at `crates/core/src/export/identity.rs:428` is within the
`#[cfg(test)]` module beginning at `:318`. The comment at
`crates/core/src/algo/upstream.rs:121-124` states that one visited-set traversal
handles both tree and DAG datasets.

No production `match`, `if`, or comparison distinguishes the two parsed topology
values. Under the branch-on-values completeness rule, topology is intentionally
uncatalogued. Future maintainers must not “fix” the correspondence gate by adding
a topology claim or witness unless production behavior first gains such a branch.
