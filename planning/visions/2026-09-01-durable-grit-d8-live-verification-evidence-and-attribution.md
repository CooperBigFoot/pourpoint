# Durable GRIT D8 live verification evidence and attribution

Program: https://github.com/CooperBigFoot/pourpoint/issues/42
Effort: https://github.com/CooperBigFoot/pourpoint/issues/46

## Outcome

Finish the interrupted GRIT D8 live-verification Effort by turning an already successful public deployment into reviewed, durable repository state across `pourpoint` and the sibling `../hfx` repository.

A fresh reader of either repository should be able to establish that the public `grit/hfx-v0.3.0` dataset declares its native EPSG:8857 GRIT D8 rasters, that released pourpoint 0.3.0 reproducibly applies terminal refinement across a real raster row seam and in a distant GRIT region, that the planetary reads remain bounded, and that the raster source is attributed correctly. The evidence must survive outside the ignored legacy planning directory and local work-package branches.

## Why this remains unfinished

The public operation succeeded on 2026-08-19, but the old graph workflow stopped before delivery. All 19 active work packages completed individually. Final graph assembly failed because a later hardening package superseded an earlier amendment, making the earlier amendment's counterfactual proof non-discriminating. This was an orchestration/composition failure, not a failed product criterion. The multi-repository graph also had no promotion route, so no aggregate branch or pull request was created.

Only these preparatory parts reached remote `main`:

- HFX authority preparation and publication tooling through HFX PRs #198, #200, and #201.
- Pourpoint test-instrumentation isolation through PR #147.
- The released-wheel proof harness through pourpoint PR #148.

The accepted live evidence, offline refined golden, publication audit records, final attribution enforcement, and active-state documentation remain on local legacy package refs.

## Established public state

Treat the following as an observed state to verify, not a future publication plan:

- Public base: `https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/`
- Canonical manifest: 1,426 bytes, SHA-256 `02339ff92cbfd1d2ea57bb5332cb843b98115cd7a7395f64c14fac78d2ed643c`
- Publication time: 2026-08-19T15:59:29Z
- Declaration: exactly one `hfx.aux.d8_raster.v2`
- Raster CRS: `EPSG:8857`
- Flow-direction encoding: `grass`
- Accumulation units: `km2`
- Artifacts: `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`
- Publication used a one-shot compare-and-swap write followed by full-body read-back. The accepted identity remained public and no rollback occurred.
- The content-addressed staging sibling was subsequently removed.
- Hosted `NOTICE` and `CITATION.txt` identify the GRIT raster archive, Wortmann et al. 2025, and CC BY-NC 4.0.

Any drift from this state is a blocker. It does not authorize an implementing agent to rewrite hosted storage.

For current read-only identity checks, hash the small manifest and attribution objects in full. For the planetary rasters, verify the recorded content length and ETag plus the bounded range-read evidence; do not download either full raster merely to recompute its historical SHA-256:

- `flow_dir.tif`: 50,686,516,478 bytes, ETag `"bc48d1013cf6908fb44c325dd2ad10ab-1511"`
- `flow_acc.tif`: 205,069,870,081 bytes, ETag `"49eab3942a26036aa49e72ea33a1b724-6112"`

## Binding proof cases and released artifact

Repeat the accepted cases rather than selecting replacements:

- Horizontal row-seam: discovery seed `[8.5417, 47.3769]`; input and resolved outlet `[8.616505182125767, 47.26531170766501]`; terminal `13882784`; canonical geometry SHA-256 `611f5ae1fac750cdc5a3531adef993e4fde71e305fe778b496a32fff040da6da`.
- Distant region: discovery seed `[10.405, 63.44]`; input and resolved outlet `[10.68514408032242, 63.26919011070941]`; terminal `14676694`; canonical geometry SHA-256 `93fa926041070606ed858cf4c844c791ac199ec115532026ae0469be12a1f9de`.
- Released Darwin arm64 witness: `pourpoint-0.3.0-cp39-abi3-macosx_11_0_arm64.whl`; 22,310,060 bytes; SHA-256 `a79ebc38be0cdc39247fd07eb608750536c982999954bd68e3ccf5599fefdabe`; package metadata version `0.3.0`, `Requires-Python >=3.9`.

If implementation runs on another platform, use the corresponding published pourpoint 0.3.0 wheel and pin its published identity, while retaining the Darwin arm64 record above as the historical witness. Do not substitute a locally rebuilt wheel.

## Required durable evidence

Deliver reviewed repository records that demonstrate all of the following:

1. **Observed released-reader refinement.** Released pourpoint 0.3.0, installed from the recorded wheel in a fresh environment, reads the public address without declaration injection and returns built-in `Applied` refinement under `RequireD8`.
2. **Real row-seam placement.** The production direction and accumulation windows for the fixed horizontal-boundary case cross distinct tile rows. Retained observations cover real samples on both sides of the boundary after localization and read-back.
3. **Regional reach.** A fixed outlet in a distant GRIT region also returns `Applied` refinement through a separate fresh process and empty cache.
4. **Reproducibility.** Repeated isolated runs preserve terminal identity, upstream identities, refined outlet, provenance, and canonical geometry.
5. **Declaration discrimination.** A deliberately false flow-direction interpretation cannot reproduce the accepted canonical result.
6. **Bounded access.** Object-read telemetry shows bounded range reads tied to the selected windows, no complete planetary raster download, and positive allocation-guard margin.
7. **Offline continuity.** A refined GRIT golden validates without network access and is pinned to the accepted manifest, raster identities, released wheel, outlets, provenance, and canonical results. Existing historical non-refined GRIT goldens remain byte-for-byte historical records.
8. **Publication accountability.** HFX retains reviewable candidate, former-manifest, staging, one-shot publication, containment, rollback-rehearsal, cleanup, and final-state records.
9. **Complete attribution.** User-facing locations that offer the hosted GRIT dataset identify the raster archive DOI `10.5281/zenodo.15715535`, the Wortmann et al. paper DOI `10.1029/2024WR038308`, and CC BY-NC 4.0 as applicable. Automated checks reject stale claims that the hosted dataset has no refinement rasters.
10. **Loud live gating.** The opt-in live proof fails when authorization is absent or when no released-worker raster reads complete. A skipped or degraded run is not success.

Before delivery, repeat the two fixed released-0.3.0 cases using read-only network access, separate fresh processes, and empty caches. Independently validate the retained historical evidence against the current public object identities. The existing live evidence is recovery material, not an exemption from fresh verification.

## Recovery sources

The ignored legacy record is at:

`planning/2026-08-07-declare-grit-d8-live-and-prove-released-reader-refinement-across-a-row-seam/`

Its final composed tips were:

- pourpoint: `fbbc2c9c50aede627e838ee5173b7290370f9bad`
- hfx: `ad41cc7b7bf74aff51aaba8ce41e9878bc48c0d1`

Useful package refs include:

- retained public proof: pourpoint `19f9bb9941264c979f4ced226972ff8e75337749`
- offline refined golden and accepted indexes: pourpoint `f3c8b96d1d9d8062030415dbc1c6b4213f7b2492`
- final pourpoint hardening lineage: `76560703b5ba6749b012690eb66248b61d7586ce`
- final pourpoint attribution lineage: `f4c5a345db02507cb6d1ce0a0937e1926c25cfbe`
- canonical publication hardening: hfx `cf31cc26f209763e1e39c82f0a4f047db79183bf`
- final HFX evidence lineage: `745a16aefc0f31b59851910d2b2a8268c40ebc0a`
- final HFX attribution lineage: `e366822265128cdda3d74404abdcea31a3cd3cec`

These refs form a large, repeatedly amended attempt history. Do not merge the lineage wholesale. Recover or reimplement coherent changes from current target branches, preserve relevant regression coverage, and subject the result to normal review. If local refs are unavailable, reproduce the evidence from the accepted public state rather than weakening the outcome.

## Constraints

- Hosted storage is read-only for this implementation. Do not rewrite, republish, delete, or automatically repair the canonical manifest or any hosted object.
- Stop and request explicit authorization if the public identity has drifted or a remote mutation appears necessary.
- Keep `grit/hfx-v0.3.0` as the sole dataset address. Do not create `grit/hfx-v0.3.1` or another prefix.
- Do not re-upload the planetary rasters.
- Do not publish a new pourpoint version, wheel, or tag. Pourpoint 0.3.0 remains the released-reader witness.
- Preserve the failed 2026-07-24 live-fire and rollback evidence as immutable history.
- Preserve unrelated current work. In particular, inspect the actual HFX target before branching; the local HFX checkout was one commit ahead of `origin/main` during discovery.
- Follow each repository's normal branch, test, review, and pull-request process. The product delivery spans both repositories even though this vision is owned and linked from pourpoint Effort #46.
- Treat the old PCE assembly failure as historical execution context. Repairing PCE's amendment-proof semantics is not part of this Effort.

## Exclusions

This Effort does not include:

- hydrologic accuracy comparison against an external scientific oracle;
- exhaustive proof across all seven GRIT regions;
- future GRIT refresh cadence or prefix policy;
- rebuilding `merit-hfx-global`;
- HFX vector-contract or `format_version` changes;
- deriving or reprojecting D8 rasters;
- expanding supported carve CRS behavior;
- a new pourpoint release; or
- rewriting dated failure evidence.

## Done

The Effort is ready for delivery review when both repositories contain coherent, reviewed changes that match the observed public state; the fresh read-only released-reader cases pass; the row-seam, distant-region, reproducibility, declaration-discrimination, and bounded-read evidence is independently inspectable; the offline refined golden passes without network access; active documentation and attribution are consistent; and no historical evidence or hosted object was silently changed.
