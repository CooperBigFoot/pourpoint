# Reader support claims

> Every declared value the reader branches on has a support claim, and a test asserts each claim has evidence constructed through the production path. A claim without such evidence fails the build.

This step introduces the shared vocabulary and the first production-owned
inventory. It does not implement the complete claim-to-evidence check. That
build-failing correspondence gate belongs to m3-s5.

## Vocabulary and layering

A claim ID is a stable correspondence key for the independent shipped-evidence
table that m3-s5 will enforce. The canonical declaration is the exact on-disk
string accepted by production. Its typed value is the HFX domain value passed
into the production reader. A claim row therefore means that exact declaration
is implemented; it is not a catalog of every rejected string and has no support
boolean.

The vocabulary lives in the neutral crate-root `support_claims` module. Reader
code and future algorithm code may import `crate::support_claims`; algorithm
code must not import `crate::reader`. Later steps must extend
`ReaderSupportValue` and the existing catalog structure for auxiliary
classification, flow-direction encoding, and D8 metadata. They must not create
a second vocabulary.

The bounded core-manifest inventory contains exactly these rows, in order:

| Claim ID | Canonical declaration | Typed value |
|---|---|---|
| `core-format-version-0.3.0` | `0.3.0` | `hfx::FormatVersion::V0_3_0` |
| `core-dataset-crs-epsg-4326` | `EPSG:4326` | `hfx::Crs::Epsg4326` |

`format_version_claim_has_shipped_cli_evidence` and
`dataset_crs_claim_has_shipped_cli_evidence` each use `DatasetBuilder` and the
compiled `pourpoint` binary. Each first proves the claimed declaration succeeds,
then changes only its temporary manifest field and proves that the caller
receives the exact rejection as JSON. The inventory assertion separately proves
the ID, declaration, and typed value of both rows. m3-s5 will compare the
complete catalog with independent evidence and make an unwitnessed claim fail
the build; this step deliberately does not perform that global comparison.

The separate flow-direction encoding sub-inventory contains these rows, in
order:

| Claim ID | Canonical declaration | Typed HFX value |
|---|---|---|
| `core-flow-dir-encoding-esri` | `esri` | `hfx::FlowDirEncoding::Esri` |
| `core-flow-dir-encoding-taudem` | `taudem` | `hfx::FlowDirEncoding::Taudem` |
| `core-flow-dir-encoding-grass` | `grass` | `hfx::FlowDirEncoding::Grass` |

Each row has discriminating evidence from the compiled `pourpoint` binary:

| Claim ID | Reference outcome `(refinement, area_km2, ring_vertex_counts)` | Exact test name |
|---|---|---|
| `core-flow-dir-encoding-esri` | `("best_effort_skipped(BestEffortSkipped { strategy: BestEffortD8IfPresent, why: MisDeclaration { source: RasterLoad, diagnostic: \"flow-direction nodata byte 128 decodes as a legal direction under Esri encoding\" } })", 36922.8059387193, [25])` | `esri_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` |
| `core-flow-dir-encoding-taudem` | `("applied(lon=0.986445, lat=0.416385)", 24613.14053443639, [13, 13])` | `taudem_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` |
| `core-flow-dir-encoding-grass` | `("applied(lon=0.986445, lat=0.416385)", 24986.140564067347, [13, 1289])` | `grass_flow_direction_encoding_claim_has_discriminating_shipped_cli_evidence` |

Refinement strings and ordered ring counts use exact equality. Area uses
`(actual_area_km2 - expected_area_km2).abs() <= 0.025_f64`. GRASS and TauDEM
share the formatted refined outlet, but the ordered ring counts alone
discriminate all three encodings. The area tolerance is far narrower than the
smallest inter-encoding area gap. ESRI is additionally distinguished by its
full invalid-nodata best-effort diagnostic.

The CLI rows witness independent declaration literals through production
behavior. Separate in-process assertions witness the IDs, canonical
declarations, and typed HFX values. CLI behavior alone does not detect a
typed-value-only catalog mutation.

## D8 metadata inventory and compatibility

D8 raster metadata has a separate bounded inventory in the same neutral
vocabulary. It contains exactly these four rows, in order:

| Claim ID | Canonical declaration | Typed value |
|---|---|---|
| `core-d8-crs-epsg-4326` | `EPSG:4326` | `algo::projection::Crs::Epsg4326` |
| `core-d8-crs-epsg-8857` | `EPSG:8857` | `algo::projection::Crs::Epsg8857` |
| `core-d8-flow-acc-units-cells` | `cells` | `hfx::FlowAccumulationUnits::Cells` |
| `core-d8-flow-acc-units-km2` | `km2` | `hfx::FlowAccumulationUnits::Km2` |

The D8 CRS and accumulation-unit lookups compare the complete caller-supplied
declaration byte for byte with these canonical declarations. They perform no
case folding, normalization, wildcard matching, defaulting, or fallback. The
declaration-string compatibility policy resolves both sides through those
lookups and implements this matrix:

| D8 CRS | `cells` | `km2` |
|---|---:|---:|
| `EPSG:4326` | compatible | incompatible |
| `EPSG:8857` | compatible | compatible |

An unclaimed declaration on either side is incompatible. `DatasetSession`
continues to parse the numeric EPSG identifier locally so out-of-range and
unsupported identifiers retain distinct errors. D8 CRS admission is delegated
to the claim lookup, while the original, unnormalized declaration is retained
unchanged in caller-visible errors.

Terminal refinement reconstructs canonical lookup keys from its typed inputs
and asks the neutral compatibility policy about the pair. The rejection branch
and `GeographicKm2Unsupported` error remain in `refine_terminal` before tile
alignment, masking, and snapping. Thus the EPSG:4326/km2 diagnostic remains
`flow accumulation units km2 require projected pixel area, but EPSG:4326 is geographic`.
The checked layering command requires every file under `crates/core/src/algo/`
to import no `crate::reader` module; algorithm code may depend on the neutral
`crate::support_claims` module.

The shipped CLI evidence uses two accepted witnesses and one rejected pair.
The tracked `v021_synthetic_refined` fixture proves EPSG:4326/cells refinement.
Its rejection arm copies only its five tracked files to a temporary directory,
round-trips the manifest as `serde_json::Value`, proves the copied cells case
still refines, and then changes only the copied `flow_acc_units` declaration to
`km2`. Assertions compare both copied raster byte streams with their tracked
sources before and after the manifest writes. The projected EPSG:8857/km2
witness uses `tiny-with-aux-d8-projected-grass` directly and unmodified.

Normal-refinement versus `--no-refine` CLI comparisons discriminate the four
canonical declarations and the compatibility rejection branch. The
in-process inventory assertion separately catches typed-value corruption; the
shipped comparisons are not evidence for typed-value identity or refinement
provenance beyond those observables. Global claim-to-evidence build enforcement
remains outside this step.

## Topology exclusion trace

Topology is excluded under the bounded rule “declared values the reader branches
on.” At ref `1e859814061a8aeda2272e58acb49e09e5f7cc73`, the complete source trace
is:

- `crates/core/src/reader/manifest.rs:182-190` requires `topology` and delegates
  parsing to `Topology::from_str`, whose accepted declarations are `"tree"` and
  `"dag"`.
- `crates/core/src/reader/manifest.rs:242-251` stores the parsed value in
  `ManifestBuilder`.
- `crates/core/src/session.rs:611-613` exposes it, while
  `crates/core/src/session.rs:309-313` and `:565-570` only log it.
- Production code only parses topology at
  `crates/core/src/reader/manifest.rs:182-190`, stores it at
  `crates/core/src/reader/manifest.rs:242-251`, exposes it at
  `crates/core/src/session.rs:611-613`, and logs it at
  `crates/core/src/session.rs:309-313` and `:565-570`; no production `match`,
  `if`, or comparison distinguishes `Topology::Tree` from `Topology::Dag`, and
  every occurrence of either variant in the workspace is test code. For
  example, `Topology::Tree` at `crates/core/src/export/identity.rs:428` is
  guarded by `#[cfg(test)]` at `crates/core/src/export/identity.rs:318`.
- `crates/core/src/algo/upstream.rs:121-124` explicitly says one visited-set
  traversal serves both tree and DAG datasets.

Topology parsing validates the HFX domain, but the accepted values do not select
different reader behavior, decode, or traversal. It therefore has no claim row
or CLI topology witness in this step.
