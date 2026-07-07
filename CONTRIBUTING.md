# Contributing to pourpoint

## Building from source

### Prerequisites

- Rust toolchain (stable) — install via [rustup](https://rustup.rs)
- [maturin](https://github.com/PyO3/maturin) ≥ 1.7 (`pip install maturin`)
- System GDAL — on macOS with Homebrew: `brew install gdal`

### Build the Python extension

```bash
cd crates/python
maturin develop --release
```

This compiles the Rust extension against your system GDAL and installs it into
the active virtual environment. On macOS, Homebrew's GDAL is picked up
automatically via `pkg-config`.

## Running tests

Rust workspace tests:

```bash
cargo test --workspace
```

Python extension tests:

```bash
cd crates/python
pytest tests/ -q
```

## Coding conventions

See [`CLAUDE.md`](CLAUDE.md) for the full coding conventions this project uses
(tracing not log, type-driven design, surgical changes, etc.). All contributions
are expected to follow those conventions.

## Commit and version policy

### Workspace Rust crates

Use conventional commit messages. Regular commits carry no version bump and no
tag. The workspace version changes only as part of a curated release prepared
by maintainers; `./scripts/bump-version.sh` is invoked only during release
preparation. Release tags use the `v*` namespace and are created by a human at
release time.

### Pourpoint release process (standalone)

`crates/python/` (`pourpoint`) has its own standalone release process. Its version
changes only on intentional PyPI releases and uses a separate tag namespace
(`pourpoint-v*`) so it does not collide with the workspace `v*` tags.

```bash
# Stable release
./scripts/bump-pourpoint-version.sh patch   # 0.1.0 → 0.1.1

# Release candidate (PEP 440 input, SemVer 2.0 written to Cargo.toml)
./scripts/bump-pourpoint-version.sh set 0.1.0rc1

# Final release after rc
./scripts/bump-pourpoint-version.sh set 0.1.0
```

The `set` mode is required for prereleases because `cargo metadata` rejects
PEP 440 prerelease syntax (`0.1.0rc1`) but accepts SemVer 2.0 (`0.1.0-rc.1`).
The script writes the PEP 440 form to `pyproject.toml` and the SemVer 2.0
equivalent to `Cargo.toml` automatically.

Update `crates/python/CHANGELOG.md` for every pourpoint version bump, then tag:

```bash
git tag pourpoint-v0.1.0rc1   # use the PEP 440 form for the tag
```

## Maintainers: first-time PyPI setup

These steps are performed once, then both release paths (TestPyPI for
release candidates, PyPI for real releases) run automatically on tag push.

### 1. Create a PyPI project-scoped API token

Go to https://pypi.org/manage/account/token/ and create a token scoped to
the `pourpoint` project (create the project first by uploading once manually,
or use the account-scoped token and tighten after first release). Copy the
token (starts with `pypi-`).

### 2. Create a TestPyPI token

Same flow on https://test.pypi.org/manage/account/token/. Copy that token.

### 3. Store both tokens as GitHub repository secrets

From the repo root:

```bash
gh secret set PYPI_TOKEN     --repo CooperBigFoot/pourpoint  # paste PyPI token
gh secret set TESTPYPI_TOKEN --repo CooperBigFoot/pourpoint  # paste TestPyPI token
```

The `build-wheels.yaml` workflow reads these via `secrets.PYPI_TOKEN` and
`secrets.TESTPYPI_TOKEN`. No GitHub environments are required.

### Rotation

Rotate both tokens on a cadence you're comfortable with (or immediately
after exposure — e.g. if a token ever leaks into a commit, PR, or chat
transcript). Rotation means: revoke the old token on PyPI/TestPyPI, create
a new one, re-run `gh secret set` with the new value.
