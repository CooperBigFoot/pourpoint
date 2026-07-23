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

## pourpoint 0.2.0 stream status

**PREPARED — UNFIRED.** The release artifacts and human runbook are prepared,
but both the `pourpoint-v0.2.0` GitHub release and the resulting real-PyPI
publication remain human-gated and unfired. Review the
[tracked 0.2.0 release runbook](docs/releases/projected-crs-terminal-refinement.md)
before either action.
