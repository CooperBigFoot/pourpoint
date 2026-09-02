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

## Maintainers: publishing setup

Python releases use PyPI and TestPyPI **OIDC Trusted Publishing**. Do not create
or store project API tokens in repository secrets. Repository administrators
configure the trusted publishers and GitHub environments once.

See [`RELEASING.md`](RELEASING.md#one-time-maintainer-setup-prerequisites) for
the current publisher configuration, release routing, and human-gated release
procedure.
