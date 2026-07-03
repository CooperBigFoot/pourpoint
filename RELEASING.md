# Releasing

This repository has two independent release streams. Do not confuse them:

- **Workspace (`shed` / `shed-core`)** — tagged `v*`. Publishing a `v*` GitHub
  Release does **not** publish pyshed: `build-wheels.yaml` guards every publish
  path (and the build job) on the `pyshed-v` tag prefix.
- **pyshed (Python bindings)** — tagged `pyshed-v*`. Publishing a `pyshed-v*`
  GitHub Release is what builds and ships the wheel to PyPI / TestPyPI via OIDC
  Trusted Publishing (no stored tokens).

Versions change **only** on intentional, curated releases — never per commit.
Agents never create or push tags; a human cuts every release.

## Cutting a pyshed release

1. **Bump the version.** Use the standalone pyshed bump script (it edits
   `crates/python/pyproject.toml` and `crates/python/Cargo.toml`; it does not
   commit or tag):

   ```bash
   ./scripts/bump-pyshed-version.sh patch          # 0.3.0 -> 0.3.1
   ./scripts/bump-pyshed-version.sh minor          # 0.3.1 -> 0.4.0
   ./scripts/bump-pyshed-version.sh set 0.4.0rc1   # required for prereleases
   ```

   Prereleases **must** use `set` mode with the PEP 440 form (e.g. `0.4.0rc1`);
   the script writes the SemVer 2.0 equivalent (`0.4.0-rc.1`) to `Cargo.toml` and
   the PEP 440 form to `pyproject.toml`.

2. **Update the changelog.** Add the release entry to `crates/python/CHANGELOG.md`.

3. **Commit and merge.** Commit the bump on a branch (conventional message, e.g.
   `chore(pyshed): prepare 0.4.0rc1`) and merge it to `main` via PR. The commit
   itself creates no tag.

4. **Create and publish the GitHub Release.** Tag it `pyshed-vX.Y.Z[rcN]` (e.g.
   `pyshed-v0.4.0rc1` or `pyshed-v0.4.0`), targeting the merged commit on `main`,
   and click **Publish**. The tag string is the single source of truth for
   routing:

   | Release tag        | Publishes to |
   |--------------------|--------------|
   | contains `rc`      | TestPyPI     |
   | no `rc`            | real PyPI    |

   Publishing the Release triggers `build-wheels.yaml`, which builds the macOS
   arm64 wheel and publishes it via OIDC — no stored tokens.

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
      publisher to the `pyshed` project:
      - Owner: `CooperBigFoot`
      - Repository: `shed`
      - Workflow filename: `build-wheels.yaml`
      - Environment: `pypi`
- [ ] **TestPyPI Trusted Publisher** — on <https://test.pypi.org>, same project,
      owner, repo, and workflow filename, but Environment: `testpypi`.
- [ ] **GitHub environments** — create `pypi`, `testpypi`, and `github-pages`
      (Settings -> Environments). The publish jobs gate on `pypi` / `testpypi`;
      the docs deploy uses `github-pages`.
- [ ] **Enable GitHub Pages** — Settings -> Pages -> Source: **GitHub Actions**.

Until both Trusted Publishers and the `pypi` / `testpypi` environments exist, a
published `pyshed-v*` Release will build the wheel but the publish step will fail
the OIDC handshake.
