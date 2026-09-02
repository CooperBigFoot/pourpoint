# pourpoint

**pourpoint** is an independent watershed-delineation engine. Give it an outlet
coordinate and an HFX dataset, and it resolves the outlet, traverses the
upstream graph, and returns watershed geometry and area.

The Python package's released version 0.3.0 is available on PyPI and classified
Beta. It ships `cp39-abi3` wheels for macOS 11+ arm64/x86_64,
`manylinux_2_28` arm64/x86_64, and Windows amd64, plus an sdist.

## HFX inputs

[HFX](https://github.com/CooperBigFoot/hfx) is the normalized input contract,
not a raw hydrofabric format. Every raw or source hydrofabric requires an
adapter compile step before pourpoint can read it. An adapter's presence does
not mean this project publicly hosts its output.

The project currently offers one hosted dataset, the GRIT 2.0.0 HFX dataset.
It was compiled from GRIT v1.0 source data. See [Datasets](guide/datasets.md)
for its manifest identity, remote-read behavior, D8 limits, license, and
citations.

## Capabilities

Released 0.3.0 supports one-shot and batch Python calls, a staged API, GeoJSON
`Feature` output, Python GeoParquet writers, and a source-built CLI that emits
GeoJSON `FeatureCollection` output for batch input. The CLI does not emit
GeoParquet.

## Where to go next

- [Quickstart](quickstart.md)
- [How it works](how-it-works.md)
- [Datasets](guide/datasets.md)
- [Staged API](guide/staged-api.md)
- [Basin GeoParquet Export](basin-geoparquet-export.md)
- [API Reference](api-reference.md), which renders the current checkout
- [Credits & Citation](credits.md)

## Evaluation and collaboration

Technical evaluations and unpaid case-study collaboration are welcome. Open a
[GitHub issue](https://github.com/CooperBigFoot/pourpoint/issues) or email
[business.coopernick@gmail.com](mailto:business.coopernick@gmail.com).
