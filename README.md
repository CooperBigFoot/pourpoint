# pourpoint

`pourpoint` is an independent, MIT-licensed watershed-delineation engine. Given
an outlet coordinate and an HFX dataset, it resolves the outlet, traverses the
upstream hydrofabric graph, and returns watershed geometry and area through
Rust, Python, and CLI interfaces.

## Release status

The released Python package is version 0.3.0. It is classified
`Development Status :: 4 - Beta`, is available from
[PyPI](https://pypi.org/project/pourpoint/), and is recorded as
[`pourpoint-v0.3.0`](https://github.com/CooperBigFoot/pourpoint/releases/tag/pourpoint-v0.3.0)
in GitHub Releases. PyPI hosts the package artifacts. The GitHub Release records
the release and does not host wheels.

Version 0.3.0 provides one-shot and batch delineation, a staged Python API,
GeoJSON `Feature` serialization, and Python GeoParquet writers. Changes under
the changelog's Unreleased section and APIs identified as main-only in the
[Python API reference](crates/python/API.md) are not part of 0.3.0.

Install the release:

```bash
uv add pourpoint
```

(or `pip install pourpoint`)

PyPI provides five `cp39-abi3` wheels and an sdist for 0.3.0:

- macOS 11+ arm64 and x86_64;
- `manylinux_2_28` arm64 and x86_64;
- Windows amd64.

The wheels bundle GDAL, PROJ, and GEOS. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for source builds.

```python
import pourpoint

engine = pourpoint.Engine("/path/to/hfx/dataset")
result = engine.delineate(lat=47.3769, lon=8.5417)
print(result.area_km2)
print(result.terminal_unit_id)
geojson_feature = result.to_geojson()
```

See the [Python quickstart](crates/python/README.md), the
[tag-pinned 0.3.0 API reference](https://github.com/CooperBigFoot/pourpoint/blob/pourpoint-v0.3.0/crates/python/API.md),
and the [main development API reference](crates/python/API.md).

## HFX dataset boundary

[HFX](https://github.com/CooperBigFoot/hfx) is the normalized input contract.
Pourpoint does not read arbitrary raw or source hydrofabrics. Every source
hydrofabric must first pass through an adapter compile step that emits HFX.
Named adapters indicate available compilation paths, not public hosting.

An HFX dataset root contains these required core artifacts:

- `manifest.json`
- `catchments.parquet`
- `graph.parquet`

Optional snap and D8 artifact paths are declared by `manifest.json`; their file
names and locations are not assumed by the engine.

Supported roots include local directories, `file://` URLs, `s3://` URLs, and
Cloudflare R2 HTTP(S) URLs on either the project public custom domain or an
`<account>.r2.cloudflarestorage.com` endpoint. For a remote dataset, pourpoint
fetches required byte ranges and raster windows instead of the complete dataset, which is about 299 GB for the
hosted example. The small manifest and graph may be fetched completely on a
cold open. Required ranges and materialized raster windows may be cached
locally. See [Raster cache](docs/raster-cache.md).

Outlet resolution uses the dataset's declared snap features and the configured
strategy. The default weight-first strategy ranks hydrologic weight before
distance; it is not simply a nearest-feature search.

## Hosted GRIT dataset

There is exactly one dataset hosted by this project: the **GRIT 2.0.0 HFX
dataset** at this engine root:

```text
https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/
Reader floor: pourpoint 0.3.0
```

A bare root can return 404 in a browser. Use this resolvable manifest as the
authority:
[`manifest.json`](https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/manifest.json)
(Reader floor: pourpoint 0.3.0).
The distinct identities are:

- hosted distribution title: GRIT 2.0.0 HFX dataset;
- source data: GRIT v1.0;
- `fabric_version`: `1.0.0`;
- HFX `format_version`: `0.3.0`;
- current `adapter_version`: `grit-global-2.1.0`.

```python
import pourpoint

engine = pourpoint.Engine(
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"  # Reader floor: pourpoint 0.3.0
)
result = engine.delineate(lat=47.3769, lon=8.5417)
```

The live GRIT manifest declares `hfx.aux.d8_raster.v2` in EPSG:8857 with
`grass` direction encoding and `km2` accumulation units. Its paths are
`aux/d8/flow_dir.tif` and `aux/d8/flow_acc.tif`. See the bounded
[D8 compatibility and remote layout](docs/guide/datasets.md#d8-compatibility-and-remote-layout)
section for the released limits. Refinement returns a terminal sub-polygon at
one explicit raster seed. A vector-resolved outlet is quantized to its unique
containing cell and guarded in place. A containment-only outlet uses the fixed
threshold candidate ranker. This is not a claim of an exact hydrologic
boundary.

### Data license and citations

The pourpoint engine is MIT-licensed. The hosted GRIT data is separately
licensed [CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/) for
NonCommercial use. Installing pourpoint does not grant commercial rights to
the hosted GRIT data.

Users of hosted GRIT must cite:

- [GRIT vector data](https://doi.org/10.5281/zenodo.17435232);
- [GRIT raster data](https://doi.org/10.5281/zenodo.15715535);
- [the GRIT paper](https://doi.org/10.1029/2024WR038308).

[Upstream Tech](https://www.upstream.tech/) provides only the hosted-data
infrastructure as an in-kind sponsor. It is not the project owner, dataset
vendor, or commercial partner.

## CLI

Build the CLI from source:

```bash
git clone https://github.com/CooperBigFoot/pourpoint
cd pourpoint
cargo build --release
./target/release/pourpoint delineate --dataset /path/to/hfx \
    --lat 47.3769 --lon 8.5417
```

The CLI can write a GeoJSON `FeatureCollection` for CSV batch input. The
GeoParquet writers are Python APIs; the CLI does not provide GeoParquet output.
Run `pourpoint delineate --help` for current flags.

## Evaluation and collaboration

Technical evaluations and unpaid case-study collaboration are welcome. Open a
[GitHub issue](https://github.com/CooperBigFoot/pourpoint/issues) or email
[business.coopernick@gmail.com](mailto:business.coopernick@gmail.com).

## Repository layout

| Path | Purpose |
|---|---|
| `crates/core` | Rust algorithm core and HFX I/O |
| `crates/gdal` | GDAL raster bridge and GEOS geometry repair |
| `crates/python` | Python bindings published as `pourpoint` |
| `src/main.rs` | CLI composition root |
| `ci/`, `.github/` | Tests, wheels, publication, and documentation workflows |

## License

The engine is MIT-licensed; see [`LICENSE`](LICENSE). Bundled native libraries
retain their own licenses; see [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md)
and [`LICENSES/`](LICENSES/).
