# Pourpoint public documentation sweep

Prepare pourpoint for its first deliberate public announcement by making every current public-facing description accurate, consistent, and credible to a technical reader arriving without prior context.

Pourpoint is already public and released. Version 0.3.0 is available on PyPI and GitHub Releases, with self-contained wheels for supported macOS, Linux, and Windows targets. Its engineering evidence is substantial, but its external adoption is still early and the Python package classifies itself as Beta. The documentation should therefore present pourpoint as a released, usable beta rather than either an experiment or an externally proven production standard.

## Intended public understanding

A reader should leave with this accurate model:

- Pourpoint is an independent, MIT-licensed open-source watershed-delineation engine.
- Given an outlet coordinate and an HFX dataset, it resolves the outlet, traverses the upstream hydrofabric graph, and returns the watershed geometry and area through Rust, Python, and CLI surfaces.
- HFX is the normalized input contract. Pourpoint does not consume arbitrary raw hydrofabrics directly; each source hydrofabric first needs an HFX adapter.
- The released Python package is installed as `pourpoint` and the current public release is 0.3.0.
- The canonical hosted example is the global GRIT-derived HFX dataset. Remote reads fetch the required ranges and raster windows rather than downloading the complete dataset.
- GRIT is the only publicly hosted dataset offered by this project. Other adapters and local build paths do not imply that equivalent public hosted datasets exist.
- Optional built-in terminal refinement is D8-based and has explicit format and CRS limits. Documentation must describe the released limits rather than implying arbitrary raster or CRS support.
- The engine code and hosted data have different licenses. The engine is MIT licensed; the hosted GRIT data is CC BY-NC 4.0 and carries citation requirements. Installing the engine does not grant commercial rights to that hosted data.
- Upstream Tech is credited only as the in-kind infrastructure sponsor for hosted data. It is not presented as the project owner or as a commercial partner.
- The project welcomes technical evaluations and unpaid case-study collaboration. Interested parties can open a GitHub issue or contact `business.coopernick@gmail.com`.

The public voice must remain general. It must not name or imply a relationship with SCALGO or any other prospective collaborator.

## Scope of the sweep

Audit and correct all current, authoritative surfaces that a prospective user, contributor, or collaborator may rely on. This includes the root README, documentation site sources, active technical reference pages, Python package metadata, current contributor and release guidance, and current repository metadata such as the GitHub description. Include generated documentation output only if the repository's established publication convention treats it as committed source of truth.

The sweep is broader than replacing one stale string. Known defects and risks include:

- The root README still describes a “Pending 0.2.1” contract even though 0.3.0 is released.
- The GitHub repository description ends with `pip install pyshed` instead of `pip install pourpoint`.
- `docs/README.md` links to the missing `docs/benchmarks/delineate-harness.md`.
- Some current descriptions blur the source GRIT release, the HFX format version, the adapter version, and manifest metadata. The live hosted manifest currently supplies the runtime facts, while `../hfx/spec/HFX_SPEC.md` defines their meaning. Reconcile these concepts rather than replacing one version label mechanically. Preserve an accurate distinction among the GRIT source release, HFX `format_version`, `fabric_version`, and `adapter_version`.
- Main contains post-0.3.0 changes under the changelog's Unreleased section. Public material must distinguish behavior available from `pip install pourpoint` 0.3.0 from behavior that exists only on current main.
- Fabric-independence claims can be misread as support for arbitrary raw data. State the HFX boundary and adapter requirement.
- Adapter support, hosted availability, D8 refinement, and supported CRSs must not be conflated.
- Existing benchmark material is not suitable for a headline latency claim. Some results are historical, use stale dataset addresses, or lack a fresh released-wheel protocol.

Use the sibling HFX repository and its canonical specification to verify contract terminology and supported artifact semantics. Do not create a competing description of HFX inside pourpoint. If the sweep finds stale public HFX documentation outside this repository, record it clearly for separate work rather than silently expanding this change into a cross-repository rewrite.

## Claims discipline

Every current claim should be traceable to released behavior, the live hosted manifest, repository tests/evidence, or the canonical HFX contract. Prefer precise and bounded statements over promotional language.

Do not claim:

- support for raw hydrofabrics without compilation to HFX;
- public hosting for multiple hydrofabrics;
- support for arbitrary D8 formats or coordinate reference systems;
- independent hydrologic-accuracy validation that has not occurred;
- superiority to SCALGO or any competing system;
- “instant” response or a specific latency without a fresh, reproducible released-wheel benchmark;
- production adoption that external evidence does not show;
- commercial permission for the hosted GRIT data.

It is valid to describe demonstrated properties such as the released Python API, cross-platform wheels, HFX-driven execution, bounded remote access, deterministic behavior where proven, batch and staged interfaces, GeoJSON/GeoParquet outputs, and the hosted zero-prior-download quickstart. Phrase each property at the scope its evidence supports.

## Observable completion

The sweep is complete when a fresh reader can move among GitHub, PyPI-facing metadata, the README, and the documentation site without encountering conflicting package names, release status, versions, dataset identity, support boundaries, or licensing claims.

The public quickstart must use the released package name and a working current hosted dataset address. Current links must resolve or be intentionally removed. Documentation build and repository documentation checks must pass. Search-based review should find no active “pending 0.2.1” statement, stale `pyshed` installation instruction, unsupported hosting claim, or present-tense reference to retired dataset addresses outside clearly historical material.

The result should also give a prospective case-study collaborator a direct, neutral route to respond without making the project appear affiliated with any organization that has not agreed to participate.

## Explicit exclusions

This vision does not authorize or include:

- changes to engine behavior, public APIs, data formats, or algorithms;
- a new package release or dataset publication;
- rewriting dated evidence, release records, or historical planning documents to sound current;
- edits to the sibling HFX repository;
- creation or publication of benchmark results unless a separate effort establishes an appropriate protocol;
- the LinkedIn announcement;
- contacting SCALGO;
- drafting or sending the SCALGO email;
- running a comparison or publishing a case study.

The later communication sequence remains: finish and verify this documentation sweep, then publish a personal LinkedIn announcement, then contact SCALGO privately. Any possible comparison is unpaid, exploratory, private by default, and publishable only after both sides approve the specific public account.
