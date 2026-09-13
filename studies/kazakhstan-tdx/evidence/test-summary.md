# Validation before export

- `.venv-study/bin/python -m pytest studies/kazakhstan-tdx -q`: 21 passed, 4 subtests passed.
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

- Shapefile verifier coercion: real Fiona export/readback red proof in
  `shapefile-regression-red.log`; exact comparison-only wrapping fixes legal
  one-part MultiPolygon to Polygon coercion. Tests reject dropped parts and
  coordinate edits. No broad topological equality is used.

- DBF padding: real driver readback trims edge ASCII spaces in29names, including
  two leading-space cases. Regression red/green in dbf-padding-regression-red.log
  and tests.log. Explicit mapping is limited to ASCII edge spaces; source CSV
  remains byte-for-byte field-equivalent.
