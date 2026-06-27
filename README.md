# shed

The watershed extraction engine that consumes compiled
[HFX](https://github.com/CooperBigFoot/hfx) datasets and returns watershed
polygons for any `(lat, lon)` outlet.

`shed` is fabric-agnostic by design: it reads the open HydroFabric Exchange
contract (`manifest.json`, `catchments.parquet`, `graph.parquet`, plus
manifest-declared snap and D8 raster auxiliaries — a D8 raster is the
eight-direction flow model in which each grid cell drains to whichever of its
eight neighbors lies steepest downhill) and runs outlet resolution (matching the
requested point to the drainage unit it falls in), upstream traversal, optional
terminal raster refinement (using the D8 flow grid to carve the precise
watershed boundary inside the outlet's own terminal unit), and final geometry
assembly without any source-fabric-specific logic in the hot path. The same
engine works for any HFX-compliant dataset.

## Use it from Python

The Python wrapper [`pyshed`](https://pypi.org/project/pyshed/) is published
on PyPI as a self-contained wheel with GDAL, PROJ, GEOS, and friends bundled
inside — no system dependencies required.

```bash
pip install pyshed
```

Current PyPI wheels are Apple Silicon macOS only (`macosx_11_0_arm64`). See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for local builds and platform notes.

```python
import pyshed

engine = pyshed.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)

print(result.area_km2)        # geodesic area in km²
print(result.terminal_unit_id)
geojson = result.to_geojson()
```

See [`crates/python/README.md`](crates/python/README.md) for the Python
quickstart and [`crates/python/API.md`](crates/python/API.md) for the full
developer API reference.

## Dataset locations

`shed` accepts local HFX dataset directories and remote object-store URLs for
the dataset root. The root must contain the HFX artifacts described by the
manifest: `manifest.json`, `catchments.parquet`, and `graph.parquet`.
Optional snap and D8 raster artifacts are declared in `manifest.json`
auxiliaries rather than fixed root filenames.

Supported dataset path forms:

| Form | Example |
|---|---|
| Local directory | `/data/hfx/rhine` |
| Local file URL | `file:///data/hfx/rhine` |
| Amazon S3 URL | `s3://bucket/path/to/hfx/rhine` |
| Cloudflare R2 HTTPS URL | `https://<account>.r2.cloudflarestorage.com/<bucket>/path/to/hfx/rhine` |
| Public R2 custom-domain URL | `https://basin-delineations-public.upstream.tech/grit/2.0.0/` |

For remote datasets, metadata and validation sidecars are cached under
`HFX_CACHE_DIR` or the OS cache directory joined with `hfx` by default. On
macOS that is typically `~/Library/Caches/hfx`; on Linux it is typically
the user's XDG cache directory with an `hfx` child. Parquet artifacts such as
`catchments.parquet` and `graph.parquet` are read through object-store range
reads instead of being downloaded wholesale. The per-engine Parquet row-group
cache defaults on for remote Python engines and off for local paths.

Remote raster refinement uses COG window reads when raster artifacts are present:
`shed` fetches TIFF metadata and only the compressed tile byte ranges needed for
the terminal catchment. See [`docs/raster-cache.md`](docs/raster-cache.md) for
details.

### Canonical Hosted Dataset

The canonical public dataset for examples is the GRIT HFX v0.2.1 fabric at:

```text
https://basin-delineations-public.upstream.tech/grit/2.0.0/
```

CLI example:

```bash
./target/release/shed delineate \
    --dataset https://basin-delineations-public.upstream.tech/grit/2.0.0/ \
    --lat 47.3769 --lon 8.5417
```

Python example:

```python
import pyshed

engine = pyshed.Engine(
    "https://basin-delineations-public.upstream.tech/grit/2.0.0/"
)
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.terminal_unit_id, result.area_km2)
```

These examples use the default `refine=True`. GRIT does not declare a D8
raster auxiliary, so best-effort refinement safely skips with a
`best_effort_no_d8_aux_declared` outcome.

## Performance And Caching

The first open of the global GRIT dataset over R2 is the expensive step: expect
about 2 minutes for the roughly 42 GB global fabric because metadata,
validation, and network round trips dominate. Warm repeat opens are much
cheaper, about 10 seconds when validation sidecars can be reused. A local
large-dataset open is also about 10 seconds.

Once an engine is open, delineation is much faster: about 80 ms for a single
outlet and about 10 ms per outlet when batched. Set `HFX_CACHE_DIR` to choose
the persistent remote metadata/validation cache root; otherwise the default on
macOS is typically `~/Library/Caches/hfx`. Remote Python engines also enable a
per-engine Parquet row-group cache by default, while local paths leave that
cache off unless requested.

## Use it from the CLI

```bash
git clone https://github.com/CooperBigFoot/shed
cd shed
cargo build --release

# Single outlet
./target/release/shed delineate --dataset /path/to/hfx \
    --lat 47.3769 --lon 8.5417

# Batch via CSV
./target/release/shed delineate --dataset /path/to/hfx \
    --outlets outlets.csv --output watersheds.geojson
```

`shed delineate --help` for all flags (snap radius, accumulation threshold,
`--no-refine`, `--json` envelope, etc.).

## Repository layout

| Path | Purpose |
|---|---|
| `crates/core` | Pure-Rust algorithm core (HFX I/O, traversal, dissolve, repair) |
| `crates/gdal` | GDAL bridge for windowed raster reads + GEOS geometry repair |
| `crates/python` | PyO3 bindings, published on PyPI as `pyshed` |
| `src/main.rs` | The `shed` CLI binary |
| `ci/`, `.github/` | macOS arm64 wheel build pipeline (cibuildwheel + bespoke native stack) |
| `scripts/` | Version-bump helpers — see `CLAUDE.md` for the workflow |

## Contributing

Build instructions, coding conventions, and the open call for community
wheel contributions (Linux / Intel macOS / Windows) live in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Acknowledgments

Public hosting of the canonical GRIT HFX dataset at
`https://basin-delineations-public.upstream.tech/grit/2.0.0/` is sponsored by
[Upstream Tech](https://www.upstream.tech/), who provide the object-storage
infrastructure as an in-kind contribution to the open HFX ecosystem. Upstream
Tech is an infrastructure sponsor: `shed` is independent open-source software,
and this acknowledgment implies no commercial relationship or endorsement.

## License

`shed` and `pyshed` are MIT-licensed (see [`LICENSE`](LICENSE)). Bundled
native libraries in the published wheel retain their own licenses; see
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) and the per-library
texts in [`LICENSES/`](LICENSES/).
