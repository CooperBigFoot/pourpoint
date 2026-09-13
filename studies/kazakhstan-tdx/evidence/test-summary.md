# Validation before export

- `.venv-study/bin/python -m pytest studies/kazakhstan-tdx -q`: 18 passed, 4 subtests passed.
- `cargo test --workspace --exclude pourpoint-python`: exit 0; 841 passed,
  15 ignored across test/doc-test result blocks. This is the repository CI command.
- Plain `cargo test --workspace`: exit 101, expected macOS PyO3 extension-module
  link failure (unresolved Python symbols when linking its lib test binary).
  No engine source was changed. Python study tests run against the built extension.
- Screening identity hardening: red proof in `screening-regression-red.log`,
  current passing result in `tests.log`. Test mutates real screening/input/boundary
  files and calls actual `check_identity`, without substituting an engine path.

Full build/command logs and exact executed study-source snapshots are retained in
the canonical project `scratchpad/kazakhstan-tdx` after batch finalization.
