# pourpoint 0.2.0 release notes and human-gated register

**Release status: PREPARED — UNFIRED**

**Intended commit subject:** `chore(pourpoint): prepare 0.2.0`

This packet prepares the curated pourpoint 0.2.0 release. It does not create a
commit, tag, GitHub release, workflow dispatch, upload, or publication. The two
externally visible actions registered below require separate human approval.

## Compatibility and behavior

- `hfx.aux.d8_raster.v1` is de-blessed. Opening a v1 dataset fails with an
  error directing the user to recompile it with a v2-emitting adapter.
- EPSG:8857 Equal Earth terminal refinement is supported without raster
  reprojection. Declaration selection, carving, and snapping operate in the
  raster's native CRS; only the refined result returns to EPSG:4326.
- Public snap thresholds remain cell counts. For accumulation declared in
  `km2`, the comparison threshold is converted using projected pixel area.
  EPSG:4326 with `km2` is rejected rather than approximated.
- pourpoint 0.1.0 could return non-reproducible carve geometry for
  multi-component terminals, including non-reproducible canonical geometry and
  polygon counts where components share a vertex. At the fix's base commit, the
  separated-component probe had 15/15 repeats differ from the first. The
  diagonal probe had 199/199 differing outputs, two canonical WKBs, and polygon
  counts of 6 or 7. With ordered ring-origin selection, both probes converged
  to one raw geometry across 200 in-process calls and 15 separate processes.

## Public Rust enum derivation

The derivation compared `pourpoint-v0.1.0`
(`285f4f27af703ae0763a30b15e71987095006e67`) with the step base commit
`d0637d41d3cb4b5421121dd31c604dfbec16f1ae`. The release-prep diff contains no
Rust source changes, so its public enum surface is equivalent to the base
commit's surface.

Commands run:

```bash
git rev-parse pourpoint-v0.1.0
git grep -n -E 'pub enum |#\[non_exhaustive\]' pourpoint-v0.1.0 -- crates/core/src
git grep -n -E 'pub enum |#\[non_exhaustive\]' "$BASE_COMMIT" -- crates/core/src
git diff --find-renames --unified=12 pourpoint-v0.1.0 "$BASE_COMMIT" -- \
  crates/core/src crates/python/src/error.rs
git grep -n -E 'EngineError|SessionError|RefinementError|match .*Error|match e' \
  "$BASE_COMMIT" -- crates/python/src
git grep -n '#\[non_exhaustive\]' "$BASE_COMMIT" -- crates || true
```

The two public-enum greps found 37 baseline enums and 40 base-commit enums. The
diff contained 5,699 lines. Review of every public core enum and the exhaustive
Python `EngineError` mapping produced this complete in-scope result:

| Public enum | Status | Added variants |
|---|---|---|
| `RefinementError` | existing | `GeographicKm2Unsupported`, `InverseProjection` |
| `SessionError` | existing | `UnsupportedD8RasterV1`, `D8CrsIdentifierOutOfRange`, `UnsupportedD8Crs` |
| `ProjectionError` | new | `UnsupportedCrs`, `NonConvergence`, `OutOfDomain` |
| `Crs` | new | `Epsg4326`, `Epsg8857` |
| `InverseStage` | new | `Theta`, `GeodeticLatitude` |

The final `#[non_exhaustive]` grep produced no output. None of these enums, and
no enum under `crates/`, is marked `#[non_exhaustive]`; downstream Rust code
with exhaustive matches may require source changes.

The completeness diff for the unpublished GDAL crate was also run:

```bash
git diff --find-renames --unified=8 pourpoint-v0.1.0 "$BASE_COMMIT" -- \
  crates/gdal/src/error.rs
```

It showed the new `RasterReadError::UnsupportedSampleType` variant. This enum is
in unpublished `pourpoint-gdal` and is not reachable through the released
Python exception mapping, so it is outside the Python 0.2.0 semver disclosure.

## Version preparation

The repository bump script printed:

```text
pyproject.toml: 0.1.0 -> 0.2.0
Cargo.toml:     0.1.0 -> 0.2.0

Don't forget to update crates/python/CHANGELOG.md and tag with:
  git tag pourpoint-v0.2.0
```

The printed tag reminder was not executed. Full `cargo metadata
--format-version 1` regeneration changed only the `pourpoint-python` package
line in `Cargo.lock` from `version = "0.1.0"` to `version = "0.2.0"`.
Workspace `pourpoint` remains 0.1.189 and `pourpoint-core` remains 0.1.0. The
manifest and metadata inspection found no workspace dependency on
`pourpoint-python` with an exact version requirement.

## Known issues

1. `trace_upstream` has a reachable panic when a flow-direction tile's nodata
   byte decodes as a valid direction; out-of-bounds neighbor arithmetic can
   overflow.
2. `RasterSource` does not carry the manifest-declared flow-direction encoding.
   Test-side sources therefore cannot be selected from manifest metadata, and
   the bare `LocalTiffRasterSource` hard-codes ESRI decoding.
3. In the degenerate-terminal route, declaration artifact path resolution
   occurs before degeneracy is reported. An empty terminal with unresolvable
   paths therefore surfaces a path error instead of
   `DegenerateTerminalPolygon`.

These remain current issues; this release-prep step does not change production
code.

## Local wheel and sdist dry run

The Homebrew native-data prerequisites resolved to:

```text
/opt/homebrew/Cellar/gdal/3.12.2/share/gdal/gdalvrt.xsd
/opt/homebrew/Cellar/proj/9.7.1/share/proj/proj.db
```

Because `/opt/homebrew/share/gdal` and `/opt/homebrew/share/proj` are symlinks
on this host, the informational source counts used `find -L`. They observed 161
GDAL files and 469 PROJ files. Data was staged with:

```bash
export BUILD_PREFIX=/opt/homebrew
export GDAL_HOME=/opt/homebrew
export GDAL_INCLUDE_DIR=/opt/homebrew/include
export GDAL_LIB_DIR=/opt/homebrew/lib
export PROJ_DATA=/opt/homebrew/share/proj
export PKG_CONFIG_PATH=/opt/homebrew/lib/pkgconfig
export DYLD_LIBRARY_PATH=/opt/homebrew/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}
bash ./ci/copy_gdal_data.sh
```

Observed output:

```text
staging bundled data from /opt/homebrew into .../crates/python/python/pourpoint/_data
staged 630 data files
```

The staged tree contained 161 GDAL files and 469 PROJ files, including both
sentinels. It was gitignored and was not added.

The build-only commands were:

```bash
maturin build --release --manifest-path crates/python/Cargo.toml --out "$ARTIFACT_DIR"
maturin sdist --manifest-path crates/python/Cargo.toml --out "$ARTIFACT_DIR"
find "$ARTIFACT_DIR" -maxdepth 1 -type f -print
shasum -a 256 "$ARTIFACT_DIR"/*
ls -lh "$ARTIFACT_DIR"
```

Both commands exited 0 and produced:

| Artifact | Bytes | Display size | SHA-256 |
|---|---:|---:|---|
| `pourpoint-0.2.0-cp39-abi3-macosx_11_0_arm64.whl` | 802,079,047 | 765M | `41553331ac28ba0e86186596fd0fc7a0b03b9cdd02b6091235aa9f2a38bc37c9` |
| `pourpoint-0.2.0.tar.gz` | 666,016 | 650K | `5aa1da1235cb42309dc03abe7c8bf1a74f967c2da416d98ad2f52d5782a8431e` |

`unzip -p` of the wheel's `.dist-info/METADATA` and `tar -xOf` of the
sdist's `PKG-INFO` each reported:

```text
Name: pourpoint
Version: 0.2.0
```

The exact wheel was installed with `pip --no-deps` into a throwaway Python 3.14
environment. Import, installed distribution metadata, `pourpoint.__version__`,
both bundled-data sentinels, `_pourpoint._self_test_proj()`, and the
missing-dataset `DatasetError` smoke all passed. Its final output was:

```text
Successfully installed pourpoint-0.2.0
wheel dry-run passed; version=0.2.0
```

Both temporary directories were removed after recording the evidence. This
wheel is unrepaired, platform-local, and not publishable. It is not a release
asset. Only `.github/workflows/build-wheels.yaml` produces the authoritative
five repaired platform wheels, installed-wheel tests, and sdist.

## Ordered local gates

The required gates are recorded here after execution:

| Order | Command | Exit | Observed result |
|---:|---|---:|---|
| preflight | `cargo fmt` | 0 | completed without output |
| 1 | `cargo fmt --check` | 0 | completed without output |
| 2 | `cargo clippy --workspace -- -D warnings` | 0 | finished dev profile in 13.88s |
| 3 | `cargo test --workspace --exclude pourpoint-python` | 0 | 700 passed, 0 failed, 11 ignored; test profile finished in 22.16s |
| 4 | `cargo check -p pourpoint-python` | 0 | finished dev profile in 6.27s |
| 5 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | finished dev profile in 9.31s |

## G1 — GitHub tag and curated release

**Status: UNFIRED**

**Human Action:** A human creates `pourpoint-v0.2.0` at the reviewed, clean
release-prep commit and publishes the GitHub release using these tracked notes.
Publishing this clean, non-`rc` release also authorizes the workflow's real-PyPI
path. The human must approve both the GitHub release and that PyPI consequence
before publishing.

**Prerequisite Artifacts:** The exact reviewed release-prep commit once the
outer human-controlled commit step creates it; this exact eight-file diff; green
local gates; the enum derivation; the native-data staging transcript; the
wheel/sdist dry run; and these reviewed release notes.

**Verification Before Firing:**

- Verify the exact release-prep commit is clean, reviewed, on the intended
  branch, and contains exactly this step's one commit.
- Verify its subject is exactly `chore(pourpoint): prepare 0.2.0`, with no
  attribution or co-author footer.
- Verify `pourpoint-v0.2.0` is unused locally and remotely.
- Verify the GitHub release targets the approved release-prep commit.
- Verify every required gate and the local dry-run evidence are green. Any
  recorded local bundled-data failure remains a blocker unless the human
  explicitly moves reliance to the post-fire CI assertion.
- Verify the GitHub `pypi` environment and PyPI Trusted Publisher are configured
  for owner `CooperBigFoot`, repository `pourpoint`, workflow
  `build-wheels.yaml`, environment `pypi`.
- Verify PyPI version 0.2.0 is unused.
- Review these notes and acknowledge that publishing a tag starting with
  `pourpoint-v` and containing no `rc` triggers real PyPI.

**Post-Fire Assertions:**

- [ ] `git ls-remote --tags origin` reports exactly one
  `refs/tags/pourpoint-v0.2.0` at the approved release-prep commit.
- [ ] The GitHub release exists, targets that commit, and displays these tracked
  notes.
- [ ] No workspace `v0.2.0` tag was created as part of this action.

## G2 — PyPI publication resulting from the release workflow

**Status: UNFIRED**

**Human Action:** A human separately approves real-PyPI publication and then
fires G1. Publishing G1's clean GitHub release triggers
`.github/workflows/build-wheels.yaml`; its OIDC `publish` job performs the PyPI
publication. No agent or human runs an ad hoc upload command while the workflow
is operating.

**Prerequisite Artifacts:** All G1 prerequisites, plus confirmation at the
reviewed commit that the workflow defines five wheel matrix legs, an sdist job,
installed-wheel bundled-data tests, macOS/Linux/Windows repair, and the
clean-tag PyPI condition.

**Verification Before Firing:**

- Obtain explicit human approval for real-PyPI publication.
- Verify PyPI version 0.2.0 is unused.
- Verify the Trusted Publisher and `pypi` GitHub environment configuration
  listed in G1.
- Confirm that an `rc` is not intended. An `rc` tag routes to TestPyPI and is
  not this release.

**Post-Fire Assertions:**

- [ ] The tag-triggered workflow succeeds for macOS arm64, macOS x86_64, Linux
  x86_64, Linux aarch64, Windows amd64, sdist, and publish jobs.
- [ ] Every repaired-wheel test confirms import, version metadata, bundled
  `_data/gdal/gdalvrt.xsd`, bundled `_data/proj/proj.db`,
  `_self_test_proj()`, and missing-dataset `DatasetError`.
- [ ] The PyPI JSON API reports version 0.2.0 and the expected five wheels plus
  sdist, with filenames and hashes matching workflow artifacts.
- [ ] In clean supported environments, `pip install pourpoint==0.2.0` succeeds
  and the installed package passes import, `__version__ == "0.2.0"`,
  native-stack self-test, bundled-data checks, and missing-dataset Engine error
  smoke.

Both actions remain **UNFIRED**. Every post-fire assertion is intentionally
unchecked. Publication requires the human decisions described above.
