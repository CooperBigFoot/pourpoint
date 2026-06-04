# M5 Migration Notes

## HFX v0.2.1 Hard Cut

HFX v0.2.1 required; HFX v0.1 input unsupported.

HFX v0.2.1 is required for shed M5. HFX v0.1 input datasets are unsupported:
the loader rejects v0.1 manifests before attempting later required field checks.

## Public Key Mapping

| Surface | Old key | New key |
|---|---|---|
| CLI JSON envelope | `terminal_atom_id` | `terminal_unit_id` |
| CLI GeoJSON properties | `terminal_atom_id` | `terminal_unit_id` |
| CLI GeoJSON properties | `upstream_atom_count` | `upstream_unit_count` |
| pyshed result property | `terminal_atom_id` | `terminal_unit_id` |
| pyshed result property | `upstream_atom_ids` | `upstream_unit_ids` |
| pyshed GeoJSON properties | `terminal_atom_id` | `terminal_unit_id` |
| pyshed GeoJSON properties | `upstream_atom_count` | `upstream_unit_count` |

## Basin GeoParquet Export Surface

M5 adds a basin GeoParquet export surface for shed-produced basin outlines. This
is additive to delineation; it does not change HFX input loading, staged
inspection, refinement, or final watershed geometry behavior.

The export is a documented shed output format, not a versioned spec. It writes
OGC WKB basin geometries with a GeoParquet footer and deterministic Hilbert
ordering, and `docs/basin-geoparquet-export.md` is the reader-facing format
document for M5.

## Deferral Ledger

- full pyshed redesign (inspectable stages, strategy menu, Python-authored strategy callbacks, Python export API) - deferred
- full aux-to-strategy binding - deferred
- Python-authored strategies - deferred
- level-selection strategies beyond default finest - deferred
- additional blessed refinement strategies - deferred
- versioned export spec / conformance suite - deferred

## Live-Surface Gates

The no-domain-atom gates scan only live/public surface: core source and README,
CLI source, pyshed Rust bindings, pyshed stubs, and pyshed public docs. Planning,
critique, and migration notes are intentionally outside the hard no-leak scope
because they must record the rename.

```bash
test -z "$(rg -n '\b[Aa]tom' crates/core/src crates/core/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n '\b[Aa]tom|terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python/src crates/python/python/pyshed/__init__.pyi crates/python/API.md crates/python/README.md | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -z "$(rg -n 'terminal_atom_id|upstream_atom_ids|upstream_atom_count' src crates/python)"
```

The negative proof confirms the filter does not hide real domain leaks:

```bash
test -n "$(printf 'crates/core/README.md:1:terminal atom leak\n' | rg -v '\bAtomic[A-Za-z0-9_]*|\batomic\b')"
test -n "$(printf 'src/main.rs:1:terminal_atom_id\n' | rg 'terminal_atom_id|upstream_atom_ids|upstream_atom_count')"
```
