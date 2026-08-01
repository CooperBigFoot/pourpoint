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
