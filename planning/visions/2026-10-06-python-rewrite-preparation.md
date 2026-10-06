# Python rewrite preparation

Prepare `python-rewrite` for a clean Python implementation of pourpoint. Remove the old engine and its supporting machinery from this branch, retain a durable reference to it in Git, and provide a small working Python development environment following RivRetrieve. This work does not implement the new engine. Its design and behavior belong to the owner's separate rewrite issue, which is not part of this standalone vision.

## Preserve the reference, not a legacy directory

The old Rust engine, Python bindings, tests, fixtures, and documentation remain available at commit `60e3d6e383546ea6ed20e68fe4d77959c816a48c`:

https://github.com/CooperBigFoot/pourpoint/tree/60e3d6e383546ea6ed20e68fe4d77959c816a48c

At discovery, both local `main` and `python-rewrite`, and remote `main`, pointed to that commit. Use the pinned commit as the comparison authority rather than assuming `main` will never change. Record this reference and concise instructions for inspecting the old implementation in the new `AGENTS.md`. Agents can use `git show <commit>:<path>` or a separate checkout when execution is necessary. Do not retain a `legacy/` copy, rewrite history, or modify `main`.

## Clear the old branch contents

Remove the old Rust implementation and all supporting artifacts: `crates/` (including Rust-dependent Python wrappers), Rust `src/`, Cargo manifests and lockfile, old tests and fixtures, obsolete `scripts/`, and `target/`. New Python source and tests may reuse the conventional directory names.

Remove all existing CI workflows under `.github/workflows/`, `ci/`, and `docker-compose.ci.yml`. Do not replace them with Python CI, publishing, or documentation workflows. Do not change GitHub repository settings, branch protections, secrets, existing releases, or published packages.

Remove `studies/`, `site/`, `scratchpad/`, `docs/`, `mkdocs.yml`, the documentation-only `Makefile`, `.claude/`, `.mypy_cache/`, and obsolete test caches. Clear old `planning/` and `.worktrees/` contents. Remove `SECURITY.md`, `RELEASING.md`, `CHANGELOG.md`, `CONTEXT.md`, `CONTRIBUTING.md`, and obsolete third-party notices and `LICENSES/` associated with the removed bundled software. No repository changelog is needed; future release notes belong in GitHub Releases.

Keep the root project `LICENSE`, replace `README.md` with a short and accurate rewrite-status introduction, replace `AGENTS.md`, retain `CLAUDE.md` as its simple pointer, and adapt `.gitignore` for Python development. The README must not claim that the new branch already provides the old engine's API or capabilities.

**Planning cleanup excludes this canonical vision and any current handoff records required by the active PCE workflow.** Clear historical planning, not the instructions governing this work. Do not recreate the superseded `.pce/` machinery. At discovery, `.pce/repository-contract.json` was already deleted in the working tree; preserve that deletion rather than restoring the obsolete contract.

The owner explicitly requests removal of local ignored material in the listed cleanup directories, including items Git cannot restore. At discovery there were no registered secondary worktrees, but `.worktrees/` contained old logs and a test environment. Recheck before deletion. Do not delete the active implementation or review checkout or newly discovered unrelated work. Follow PCE's checkout-preservation rules and report any genuine conflict rather than discarding uncertain source or history.

## Match RivRetrieve's Python setup

Use `/Users/nicolaslazaro/Desktop/work/rivRetrieve/AGENTS.md` and `pyproject.toml` as the reference for development conventions, adapted to pourpoint. The inspected setup uses:

- Python 3.13 (`.python-version` is `3.13`, project requirement is `>=3.13`).
- `uv` exclusively for dependency management and execution, with a committed lockfile.
- A `src/` package layout and the `uv_build` backend (`>=0.12.1,<0.13` at discovery).
- Development tooling: pytest (`>=8.3`), pytest-cov (`>=6.2.1`), Ruff (`>=0.12`), and ty (`>=0.0.35`).
- Ruff line length 120, target `py313`, rules `E`, `F`, `W`, `N`, `I`, `UP`, `B`, `C4`, and `SIM`, with `E501`, `N803`, and `N806` ignored.
- ty targeting `src`.
- pytest using `src` as its Python path and `tests` as its test root, file patterns `test_*.py` and `*_test.py`, class pattern `Test*`, function pattern `test_*`, and options `--strict-markers`, `--strict-config`, and `-ra`.

Provide a minimal importable `pourpoint` package and a meaningful setup smoke test, with no watershed implementation or imitation of the old API. Match RivRetrieve's tooling, not its application: do not copy hydrology-provider dependencies, transport import bans, provider fixtures, `--logic-only` machinery, documentation dependencies, or research tools. Do not select geospatial runtime dependencies in this preparation. Adapt packaging exclusions to this repository so local planning, caches, credentials, and worktrees do not enter distributions.

The local verification commands must work without Rust, Cargo, maturin, or the old native extension:

```bash
uv sync
uv run pytest
uv run ruff format --check
uv run ruff check
uv run ty check src
uv build
```

Document RivRetrieve-style authoring commands too: `uv add`, `uv add --dev`, `uv run ruff format`, and `uv run ruff check --fix`. No release, version bump ceremony, or package publication is part of this work.

## Adapt agent instructions

Replace the Rust-specific doctrine with RivRetrieve's practical Python guidance: clear module responsibilities, narrow dependencies, explicit composition boundaries, parsing external inputs at their arrival boundaries, domain types where invariants or units matter, and enums or literals for named states without wrapping every primitive or bulk array. Mathematical module signatures are not mandatory.

Keep instructions proportional. Use NumPy-style public docstrings where explanation helps, document constraints, units, results, and failures, and avoid coverage-driven prose. Carry the useful plain-language principles into the root instructions without links to documentation being removed. Tests should protect named behavior with meaningful expectations at the simplest sufficient level. Failures must remain explicit; do not invent defaults for required inputs or silently discard errors. Do not transplant RivRetrieve's provider-specific partial-result model or private verification archive requirements.

State pourpoint's purpose as a fabric-agnostic watershed engine consuming HFX. Refer to sibling `../hfx/spec/HFX_SPEC.md` as the input-contract authority; do not freeze old Rust schema assumptions or design a new parallel contract here. Engine architecture, API compatibility, algorithms, and failure-policy choices belong to the later rewrite work.

PCE at `/Users/nicolaslazaro/Desktop/work/pce` supplies the installed skills. A short workflow reference is enough. Do not introduce a local workflow engine, copied skill definitions, or legacy `.pce/` files. Do not retain broken links to RivRetrieve's documentation or claim its project-specific commands exist here.

## Evidence of completion

The preparation is complete when the requested legacy content is gone from `python-rewrite`, the pinned Rust reference remains retrievable, and the new Python package can be synced, imported, tested, formatted, linted, type-checked, and built using the commands above. Check distribution contents for stale native binaries and excluded local material. New tests demonstrate setup only, not engine behavior or parity.

Review the final repository tree and instructions for stale Rust, CI, MkDocs, old API, and deleted-path references. Preserve deliberate historical references such as the pinned comparison commit and this vision. Verify the canonical vision survived cleanup and the user’s pre-existing deletion was not lost. Keep the resulting repository small: this is preparation for a radical rewrite, not an opportunity to add framework code or speculative abstractions.
