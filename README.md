# pourpoint

Give `pourpoint` a point on a river and it returns the whole upstream area that drains to it, the watershed.

`pourpoint` reads [HFX](https://github.com/CooperBigFoot/hfx), a folder of pre-built
river-network files in the open HydroFabric Exchange format. It finds the
catchment that contains the point, gathers every catchment upstream, and merges
them into one watershed polygon. The same engine works with any dataset in the
HFX format.

## Use it from Python

The Python wrapper [`pourpoint`](https://pypi.org/project/pourpoint/) is published
on PyPI as a self-contained wheel with GDAL, PROJ, and GEOS bundled inside, so
no system installs are needed.

```bash
uv add pourpoint
```

(or `pip install pourpoint`)

Prebuilt wheels are published for:

- macOS (Apple Silicon + Intel)
- Linux (x86_64 + aarch64)
- Windows (x86_64)

as `macosx_11_0_arm64`, `macosx_11_0_x86_64`, `manylinux_2_28_x86_64`, `manylinux_2_28_aarch64`, `win_amd64`.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for local builds.

```python
import pourpoint

engine = pourpoint.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)

print(result.area_km2)        # geodesic area in km²
print(result.terminal_unit_id)
geojson = result.to_geojson()
```

See [`crates/python/README.md`](crates/python/README.md) for the Python
quickstart and [`crates/python/API.md`](crates/python/API.md) for the full
developer API reference.

### Pending 0.2.0 raster-refinement contract

The prepared, not-yet-published pourpoint 0.2.0 release consumes only
`hfx.aux.d8_raster.v2` D8 auxiliaries. It supports EPSG:4326 and EPSG:8857 D8
rasters, performs declaration selection, carving, and snapping in the raster's
native CRS, and converts only the refined result back to EPSG:4326. Snap
thresholds expressed as `cells` remain cell counts; projected `km2` thresholds
are compared using projected pixel area. Version 0.2.0 rejects v1 auxiliaries
and rejects `km2` accumulation on EPSG:4326 rather than approximating angular
pixel area. For identical inputs, returned carve geometry is deterministic.

The curated notes, local evidence, and still-unfired human gates are in the
[0.2.0 release runbook](docs/releases/projected-crs-terminal-refinement.md).

## Dataset locations

`pourpoint` accepts local HFX dataset folders and remote URLs to datasets hosted
online, for example on Amazon S3 or Cloudflare R2. The root must contain the
HFX artifacts described by the
manifest: `manifest.json`, `catchments.parquet`, and `graph.parquet`.
Optional snap and D8 raster artifacts are declared in `manifest.json`
auxiliaries.

Supported dataset path forms:

| Form | Example |
|---|---|
| Local directory | `/data/hfx/rhine` |
| Local file URL | `file:///data/hfx/rhine` |
| Amazon S3 URL | `s3://bucket/path/to/hfx/rhine` |
| Cloudflare R2 HTTPS URL | `https://<account>.r2.cloudflarestorage.com/<bucket>/path/to/hfx/rhine` |
| Public R2 custom-domain URL | `https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/` |

For remote datasets, `pourpoint` reads only the parts needed for each watershed, so
you never download the whole dataset. It keeps a small cache on disk so repeat
opens are faster; set `HFX_CACHE_DIR` to choose where it lives. On macOS that is
typically `~/Library/Caches/hfx`. See
[`docs/raster-cache.md`](docs/raster-cache.md) for details.

### Canonical Hosted Dataset

The canonical public dataset for examples is GRIT (Global River Topology) 2.0.0, the source river network, compiled to the HFX v0.3.0 format, at:

```text
https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/
```

CLI example:

```bash
./target/release/pourpoint delineate \
    --dataset https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/ \
    --lat 47.3769 --lon 8.5417
```

Python example:

```python
import pourpoint

engine = pourpoint.Engine(
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"
)
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.terminal_unit_id, result.area_km2)
```

These examples use the default `refine=True`. GRIT does not declare a D8
raster auxiliary, so best-effort refinement safely skips with a
`best_effort_no_d8_aux_declared` outcome.

## Performance And Caching

The first open of a remote dataset fetches dataset metadata over the network and
is slower, so keep the engine around and reuse it. Repeated delineations in the
same session reuse data already fetched, so overlapping watersheds are faster.

## Use it from the CLI

```bash
git clone https://github.com/CooperBigFoot/pourpoint
cd pourpoint
cargo build --release

# Single outlet
./target/release/pourpoint delineate --dataset /path/to/hfx \
    --lat 47.3769 --lon 8.5417

# Batch via CSV
./target/release/pourpoint delineate --dataset /path/to/hfx \
    --outlets outlets.csv --output watersheds.geojson
```

`pourpoint delineate --help` for all flags (snap radius, accumulation threshold,
`--no-refine`, `--json` envelope, etc.).

## Repository layout

| Path | Purpose |
|---|---|
| `crates/core` | Pure-Rust algorithm core (HFX I/O, traversal, dissolve, repair) |
| `crates/gdal` | GDAL bridge for windowed raster reads + GEOS geometry repair |
| `crates/python` | Python bindings, published on PyPI as `pourpoint` |
| `src/main.rs` | The `pourpoint` CLI binary |
| `ci/`, `.github/` | Five-platform wheel build, repair, test, and publication pipeline |
| `scripts/` | Version-bump helpers; see `CLAUDE.md` for the workflow |

## Contributing

Build instructions, coding conventions, and the open call for community
wheel contributions (Linux / Intel macOS / Windows) live in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Acknowledgments

Public hosting of the canonical GRIT HFX dataset at
`https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/` is sponsored by
[Upstream Tech](https://www.upstream.tech/), who provide the hosting
infrastructure as an in-kind contribution to the open HFX ecosystem. Upstream
Tech is an infrastructure sponsor: `pourpoint` is independent open-source software,
and this acknowledgment implies no commercial relationship or endorsement.

## License

`pourpoint` is MIT-licensed (see [`LICENSE`](LICENSE)). Bundled
native libraries in the published wheel retain their own licenses; see
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) and the per-library
texts in [`LICENSES/`](LICENSES/).
