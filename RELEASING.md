# Releasing

This repository has two independent release streams. Do not confuse them:

- **Workspace (`pourpoint` / `pourpoint-core`)** — tagged `v*`. Publishing a `v*` GitHub
  Release does **not** build or publish Python artifacts:
  `.github/workflows/build-wheels.yaml` runs release builds only when the
  published release tag starts with `pourpoint-v`.
- **pourpoint (Python bindings)** — tagged `pourpoint-v*`. Publishing a `pourpoint-v*`
  GitHub Release builds and ships Python artifacts to PyPI / TestPyPI via OIDC
  Trusted Publishing (no stored tokens). A tag containing `rc` routes to
  TestPyPI; a clean tag routes to real PyPI.

Versions change **only** on intentional, curated releases — never per commit.
Agents never create or push tags; a human cuts every release.

## Cutting a pourpoint release

1. **Bump the version.** Use the standalone pourpoint bump script (it edits
   `crates/python/pyproject.toml` and `crates/python/Cargo.toml`; it does not
   commit or tag):

   ```bash
   ./scripts/bump-pourpoint-version.sh patch          # 0.3.0 -> 0.3.1
   ./scripts/bump-pourpoint-version.sh minor          # 0.3.1 -> 0.4.0
   ./scripts/bump-pourpoint-version.sh set 0.4.0rc1   # required for prereleases
   ```

   Prereleases **must** use `set` mode with the PEP 440 form (e.g. `0.4.0rc1`);
   the script writes the SemVer 2.0 equivalent (`0.4.0-rc.1`) to `Cargo.toml` and
   the PEP 440 form to `pyproject.toml`.

2. **Update the changelog.** Add the release entry to `crates/python/CHANGELOG.md`.

3. **Commit and merge.** Commit the bump on a branch (conventional message, e.g.
   `chore(pourpoint): prepare 0.4.0rc1`) and merge it to `main` via PR. The commit
   itself creates no tag.

4. **Create and publish the GitHub Release.** Tag it `pourpoint-vX.Y.Z[rcN]` (e.g.
   `pourpoint-v0.4.0rc1` or `pourpoint-v0.4.0`), targeting the merged commit on `main`,
   and click **Publish**. The tag string is the single source of truth for
   routing:

   | Release tag        | Publishes to |
   |--------------------|--------------|
   | contains `rc`      | TestPyPI     |
   | no `rc`            | real PyPI    |

   Publishing the Release triggers `build-wheels.yaml`. It builds and repairs
   wheels for macOS arm64, macOS x86_64, Linux x86_64, Linux aarch64, and
   Windows amd64, plus an sdist. Each platform stages bundled GDAL/PROJ data;
   repaired wheels undergo installed-wheel import, version, bundled-data,
   native-stack, and missing-dataset smoke tests before the configured GitHub
   environment permits OIDC publication.

   A local `maturin build` dry run is unrepaired and platform-local. It is
   evidence only and is never an artifact uploaded to PyPI; the workflow-built
   and repaired artifacts are authoritative.

> **First release after adopting OIDC:** cut an **rc** (routes to TestPyPI) to
> prove the Trusted-Publishing handshake end-to-end **before** a clean version
> goes to real PyPI. PyPI versions are permanent and immutable — a bad clean
> publish cannot be undone.

### Manual dispatch (recovery)

`build-wheels.yaml` keeps a `workflow_dispatch` with an `upload` input
(`0` = build only, `1` = TestPyPI, `2` = PyPI) for re-running a publish without
cutting a new Release.

## One-time maintainer setup (prerequisites)

OIDC Trusted Publishing and the docs site need GitHub + PyPI configuration that
only a repository admin can do. Do this **once**:

- [ ] **PyPI Trusted Publisher** — on <https://pypi.org>, add a GitHub trusted
      publisher to the `pourpoint` project:
      - Owner: `CooperBigFoot`
      - Repository: `pourpoint`
      - Workflow filename: `build-wheels.yaml`
      - Environment: `pypi`
- [ ] **TestPyPI Trusted Publisher** — on <https://test.pypi.org>, same project,
      owner, repo, and workflow filename, but Environment: `testpypi`.
- [ ] **GitHub environments** — create `pypi`, `testpypi`, and `github-pages`
      (Settings -> Environments). The publish jobs gate on `pypi` / `testpypi`;
      the docs deploy uses `github-pages`.
- [ ] **Enable GitHub Pages** — Settings -> Pages -> Source: **GitHub Actions**.

Until both Trusted Publishers and the `pypi` / `testpypi` environments exist, a
published `pourpoint-v*` Release will build the wheel but the publish step will fail
the OIDC handshake.

## pourpoint 0.3.0 stream status

**FIRED — PUBLISHED TO PyPI.** This status applies to the Python
package release stream, whose tags are `pourpoint-v*`. The independent
workspace (`pourpoint` / `pourpoint-core`) release stream continues to use
`v*` tags. The previous Python package tag `pourpoint-v0.2.1` points at
`68ac80c`; tag `pourpoint-v0.3.0` points at `6e29331`.

The historical 0.2.0 packet at
`docs/releases/projected-crs-terminal-refinement.md` is a pre-fire record. Its
line 3 `Release status: PREPARED — UNFIRED`, line 242 `Status: UNFIRED`, line
283 `Status: UNFIRED`, and line 319 `Both actions remain UNFIRED` claims all
predate the 2026-07-24 fire under tag `pourpoint-v0.2.0` and no longer describe
reality. This stream-status statement supersedes those four historical status
claims without rewriting the packet.

Tag `pourpoint-v0.3.0` exists and points at merge commit `6e29331` on `main`.
The GitHub Release `pourpoint 0.3.0` is published against that tag, and the
`build-wheels.yaml` run it triggered reported `Publish to PyPI: success` with
`Publish to TestPyPI: skipped`, matching the routing rule for a clean tag.
PyPI serves `pourpoint` 0.3.0 as six artifacts uploaded 2026-07-31T23:47Z:
macOS arm64, macOS x86_64, manylinux aarch64, manylinux x86_64, and Windows
amd64 `cp39-abi3` wheels, plus the sdist.

The tag, the Release, and the PyPI publication were created by an agent at the
repository owner's explicit instruction, overriding the standing rule above
that agents never create or push tags and that a human cuts every release.
That rule is unchanged and still governs by default; this release is a recorded
exception to it, not a revision of it. Any future agent reading this file must
treat tag creation, Release publication, and PyPI publication as human-only
unless it holds the same explicit, per-release instruction.

## pourpoint 0.2.x publication decision and 0.3.0 successor mechanism

The prior premise was that pourpoint 0.2.0 and 0.2.1 had an empty installed
base and could be yanked. BigQuery measurements from
`bigquery-public-data.pypi.file_downloads`, covering downloads since 2026-07-24
(the 0.2.0 fire date) and measured on 2026-08-01, falsified that premise. The
installer-attributed measurements were:

- pourpoint 0.2.0: 37 installer-attributed downloads (pip 35, uv 2).
- pourpoint 0.2.1: 36 installer-attributed downloads (pip 34, uv 2).

The raw counts were 639 for 0.2.0 and 658 for 0.2.1. They are mostly
bandersnatch, Browser, and unattributed traffic, so raw traffic is not a
measure of installed base.

**Decision: DO NOT YANK.** pourpoint 0.2.0 and 0.2.1 remain published on PyPI by deliberate decision.

The repository owner made that explicit keep-published decision after being
shown these measurements. These releases remain published on purpose rather
than by oversight.

For pourpoint 0.3.0, `grit/hfx-v0.3.0` is the frozen source prefix, and its
manifest carries no `hfx.aux.d8_raster.v2` entry. The existing record in
`CONTEXT.md` names only a generic “successor prefix”; under that explicit
decision by the repository owner, this release record fixes and establishes
`grit/hfx-v0.3.1` as the successor-prefix name. That named successor prefix is
created by a server-side copy of frozen `grit/hfx-v0.3.0`, and the copy carries
the staged `aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif` rasters. The
successor manifest is where the D8 entry will be declared. “Unchanged” means its
schema version remains `hfx.aux.d8_raster.v2` rather than being bumped; it does
not mean that the declaration is inherited from the frozen prefix.

**SERVER-SIDE COPY: UNFIRED. LIVE CARVE: UNFIRED.**

Neither action has been performed.

Because pourpoint 0.2.0 and 0.2.1 remain published, released 0.2.x remains a
straggler for a declared-`grass` D8 entry: the frozen-prefix discipline shields
a reader that stays on `grit/hfx-v0.3.0`, but nothing it provides protects a
0.2.x reader that follows `grit/hfx-v0.3.1`, so the successor declaration and
its publication timing must be chosen with that unprotected reader in mind.
