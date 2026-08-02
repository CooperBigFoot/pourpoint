# MERIT unreadable-v1 reduction

This fixture is a reduction cut from the real dataset at
`/Users/nicolaslazaro/Library/CloudStorage/Dropbox-hydrosolutions/Nicolas Lazaro/merit-hfx-global`.
It is not a generated lookalike. The source manifest identifies
`format_version=0.3.0`, `crs=EPSG:4326`, `topology=tree`,
`fabric_name=merit_basins`, and `unit_count=2876771`.

## Source identity and reduction

| Source artifact | Bytes | SHA-256 |
|---|---:|---|
| `manifest.json` | 17,303 | `3c24b74b6191d3fb0915f8712cf0052e0e6c61110bf9dabbcc117a8c1e7479ce` |
| `graph.parquet` | 84,338,714 | `896fd14abe966383174b2894ec412c5feab6a398c02bf1dcbbd86116177c53d9` |
| `catchments.parquet` | 6,593,009,458 | `9577f028bbd05182083cf062701acecf2c0a1804db0d8439d72e346f5d037502` |
| `aux/snap_stems.parquet` | 1,848,449,226 | `6c15d4e12fe0f0df8acc4852aff98685e8283aed6cce0fb4e997eeecad33e472` |

Starting at full-data terminal unit `23017694`, the cut recursively follows
every real `upstream_ids` edge and retains the complete 43-unit closure. It
contains 43 graph rows, 43 catchment rows, and the 43 snap rows whose
`unit_id` belongs to that closure. No graph edge, geometry, catchment, or snap
feature is synthesized. The bbox union is exactly
`[8.399582862854004, 46.79708480834961, 9.43375015258789, 47.432918548583984]`.

PyArrow 23.0.1 filtered the source Parquet datasets and wrote each table with
Zstandard compression while preserving Arrow schema metadata. Structured JSON
copied the source manifest and changed only `unit_count` to 43, the bbox to the
selected catchment union, and the blessed snap artifact path from
`aux/snap_stems.parquet` to `snap.parquet`. The one `hfx.aux.snap.v2`
declaration and all 60 `hfx.aux.d8_raster.v1` occurrences remain in their
original order with their original JSON metadata. No referenced v1 raster is
committed because unreadable declarations are diagnostic-only.

## Committed identity and budget

| Machine artifact | Bytes | SHA-256 |
|---|---:|---|
| `manifest.json` | 17,336 | `d80cef2838ac844df7922eaf936ac1148ab99a8b1900602bd363ac5a7b328c77` |
| `graph.parquet` | 3,405 | `95478b94d3926bfc66272bb26651ed88dfd51912a5f447f87e4c1e6347bce625` |
| `catchments.parquet` | 59,765 | `ba2e31c0c80362349f1a82067183c1b5c6fa13461a69ee5911bb618e3305d744` |
| `snap.parquet` | 27,251 | `ecc7758ba4b74d33256857744a3625a9628cc124ef309c3f104cddd4c17db817` |

The machine-artifact total is 107,757 bytes. This README is 05041 bytes, so
the complete fixture is 112798 bytes against the 131,072-byte budget. The
README records its size but does not claim to checksum itself.

## Installed-wheel witnesses

BEFORE used ref `e9d4fc56723d22e7331b76087491981ace669622`, extracted with
`git archive`, and a separate Cargo target:

```console
before_dir="$(mktemp -d)"
git archive e9d4fc56723d22e7331b76087491981ace669622 | tar -x -C "$before_dir"
cd "$before_dir/crates/python"
CARGO_TARGET_DIR="$before_dir/target" maturin build --release --out dist
"$executor_python" -m pip install --force-reinstall dist/*.whl
"$executor_python" - <<'PY'
from importlib.metadata import distribution
from pathlib import Path
import pourpoint

package = Path(pourpoint.__file__).resolve()
installed = Path(distribution("pourpoint").locate_file("pourpoint")).resolve()
assert package.parent == installed
pourpoint.Engine("/Users/nicolaslazaro/Library/CloudStorage/Dropbox-hydrosolutions/Nicolas Lazaro/merit-hfx-global", refine=False)
PY
```

Exit code: `1`. Exact diagnostic:

```text
pourpoint.DatasetError: auxiliary schema "hfx.aux.d8_raster.v1" is no longer supported; recompile the dataset with a v2-emitting adapter that declares "hfx.aux.d8_raster.v2"
```

AFTER used ground ref `3db658f4d4348eb16d52878b211f454a87866cf5` and the shipped
wheel path:

```console
cd crates/python
maturin build --release --out dist
"$executor_python" -m pip install --force-reinstall dist/*.whl
cd ../..
"$executor_python" ci/verify_merit_hfx_global.py --dataset "/Users/nicolaslazaro/Library/CloudStorage/Dropbox-hydrosolutions/Nicolas Lazaro/merit-hfx-global"
```

Exit code: `0`. Exact JSON result:

```json
{"area_km2":2231.9425272184967,"auxiliary_entries":61,"d8_v1_occurrences":60,"snap_v2_occurrences":1,"terminal_unit_id":23017694,"unit_count":2876771}
```

## Corroborating binary evidence

The binary is secondary evidence, not a substitute for the installed wheel.
At the pre-fix ref it returned:

```json
{"error":"failed to open HFX dataset session: auxiliary schema \"hfx.aux.d8_raster.v1\" is no longer supported; recompile the dataset with a v2-emitting adapter that declares \"hfx.aux.d8_raster.v2\""}
```

At the ground ref, both the full dataset and this cut returned:

```json
{"failed":0,"failures":[],"succeeded":1,"successes":[{"area_km2":2231.9428245652666,"id":null,"lat":47.37,"lon":8.54,"terminal_unit_id":23017694}],"total":1}
```

The shipped Python result is `2231.9425272184967 km2`; the compiled CLI result
is `2231.9428245652666 km2`, an observed difference of approximately
`0.00029734677 km2`. No cause is asserted.
