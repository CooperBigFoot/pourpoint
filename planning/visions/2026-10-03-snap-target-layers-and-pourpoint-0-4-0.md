# Snap-target layers and pourpoint 0.4.0

## Outcome

Let users extract the snap-target geometries already supplied by an HFX dataset
as layers for QGIS, or inspect and plot them through Python. This is a small data
access/export utility, not a river-network reconstruction feature.

Ship this feature together with all accumulated changes since Python release
`pourpoint-v0.3.0` in **pourpoint 0.4.0** on PyPI. Keep documentation updates
focused on current usage and the complete release contents.

## User experience

The primary workflow produces a GeoPackage that users can open directly in QGIS.
The intended CLI surface is:

```bash
pourpoint export-snap \
  --dataset <hfx-root> \
  --bbox 8.3 47.2 8.8 47.6 \
  --output snap-targets.gpkg

pourpoint export-snap \
  --dataset <hfx-root> \
  --all \
  --output snap-targets.gpkg
```

Require exactly one spatial scope: a bounding box or explicit `--all`. Coordinates
are EPSG:4326 in west, south, east, north order. Never default to downloading an
entire global dataset. Select features whose actual geometry intersects the
extent, including its boundary; retain each selected feature's complete geometry
rather than clipping it.

Export all declared snap sets by default into distinguishable layers. Allow a
named-set selection such as `--snap-set reach-stems`. Preserve set identity and
separate Point and LineString layers when a set contains both. Layer naming must
be deterministic and usable in QGIS, without silently mixing sets or dropping
features.

The intended Python experience is:

```python
engine = pourpoint.Engine(hfx_root)
targets = engine.snap_targets(bbox=(8.3, 47.2, 8.8, 47.6))
targets.write("snap-targets.gpkg")
gdf = targets.to_geodataframe()

all_targets = engine.snap_targets(all=True)
all_targets.write("snap-targets.gpkg")
```

Provide equivalent set selection in Python. A combined GeoDataFrame retains the
snap-set identity. GeoPandas conversion may use an optional dependency; do not
make a plotting stack necessary for basic extraction or CLI export. These
examples define the desired small surface, not a mandate for a large collection
framework. Full-dataset file export must not require materializing all features
in memory; explicit GeoDataFrame conversion may do so.

The Rust CLI remains on the GitHub/source distribution path. **Do not install or
bundle the Rust CLI through PyPI**, and do not add a new binary-distribution
project as part of this work. Python wheels continue to distribute the Python
package and its native extension.

## HFX contract and repository evidence

The canonical input contract is `../hfx/spec/HFX_SPEC.md` and
`../hfx/spec/aux/snap/v2.md`, not legacy root-level snap filenames. Research for
this vision also inspected the GRIT adapter's declarations and HydroBASINS
compiled manifests in `../hfx`.

- HFX format 0.3.0 declares snap artifacts in `manifest.json` under
  `hfx.aux.snap.v2`. Resolve those relative paths through the existing dataset
  source facilities. Do not hard-code `snap.parquet` or source-fabric behavior.
- Multiple named declarations are valid. GRIT uses `segment-stems` and
  `reach-stems`; a set can reference more than one dataset-local level. Do not
  silently choose only the set used by outlet resolution or only the finest one.
- Snap rows contain `id`, `unit_id`, `weight`, WKB `geometry`, and optional
  `stem_role` and `bbox`. Allowed geometries are Point and LineString in
  EPSG:4326. Preserve their supplied values and geometries.
- Preserve declaration identity, description, referenced levels, and
  `weight_semantics` in the exported metadata. Weight is producer-defined, not
  universally drainage area. Snap rows have no level column; do not invent one
  from a declaration that can reference several levels.
- The optional bbox struct and GeoParquet covering statistics support pruning.
  Missing bounds require a correct fallback, not omitted features. Therefore
  small remote downloads cannot be promised for every conforming snap artifact.
- Snap features do not encode internal snap-to-snap connectivity. They are not
  unit outlets, and the unit adjacency graph is not a source of river geometry.

Pourpoint currently exposes snap bbox access internally through
`crates/core/src/reader/snap_store.rs`, but no public Python extraction API.
Reuse appropriate readers, source handling, and domain types rather than adding a
second HFX interpretation. Support local roots and the remote root forms already
supported by pourpoint. Use available range reads and spatial pruning. Extraction
must not require delineation, raster processing, or an upstream traversal; avoid
unnecessary whole-dataset reads just to export snap features.

Report absent snap declarations, unknown set names, unsupported declarations, and
read/schema failures clearly. An extent with no matching features is an empty
selection, not evidence that the dataset lacks snap data. Never silently skip a
broken selected set or claim a partial output is complete. Follow existing
project doctrine for errors, authority, types, and library boundaries.

## Scope limits

No HFX format or adapter changes are needed. Do not generate rivers, infer missing
lines from points, read raw source fabrics, add connectivity or routing, clip
geometries, select by watershed, add plotting functions, or build a QGIS plugin.
GeoPackage is the initial file output; additional export formats are not required.
Preserve data attribution where supplied and explain that extraction does not
change the source dataset's license.

## Release and focused documentation work

Live PyPI and GitHub checks during discovery both showed Python **0.3.0** as the
latest release. `crates/python/pyproject.toml` also remains at 0.3.0. The workspace
root version is independently 0.1.189; the HFX on-disk format is independently
0.3.0. Do not conflate these versions or bump the HFX format for this feature.

Target Python **0.4.0**, with release tag **`pourpoint-v0.4.0`**. Before preparing
the release, reconcile the complete Git range from `pourpoint-v0.3.0` through the
final release candidate against the changelogs. Existing accumulated work includes
watershed-area corrections, outlet/refinement behavior and provenance changes,
auxiliary-declaration diagnostics, and documentation/support updates. This is not
an exhaustive inventory: the Git range is authoritative, and older root
`Unreleased` bullets can describe changes already shipped in Python 0.3.0.
Document compatibility and behavior changes accurately, including changes that
can alter watershed results. Do not ship superseded intermediate behavior as a
new feature claim.

Use the existing release process in `RELEASING.md` and
`scripts/bump-pourpoint-version.sh` to synchronize Python package versions.
Update the Python changelog, affected API reference and type stubs, and concise
CLI/Python/QGIS usage examples. Update release/version statements where needed.
This is a focused documentation update, not another site-wide documentation sweep.

Validate with the documented Rust checks and Python tests in the project's own
environment. `CONTRIBUTING.md` documents the split Rust-test/PyO3-check approach
where full workspace testing hits extension-module linkage limits. Exercise
installed artifacts through the existing release workflow: it builds repaired
wheels for macOS arm64/x86_64, Linux arm64/x86_64, and Windows amd64, plus an sdist,
and performs installed-wheel checks. Local build output is not a substitute for
those publishable artifacts.

Publishing a GitHub Release tagged `pourpoint-v0.4.0` triggers
`.github/workflows/build-wheels.yaml` and real PyPI OIDC publication; `rc` tags
route to TestPyPI. A workspace `v*` release does not publish Python artifacts.
Follow normal review, checks, and environment approvals without bypassing them.
Under current repository policy, tag creation and release/PyPI publication remain
human-only unless the owner gives explicit per-release agent authorization.
This vision authorizes implementation and preparation when separately invoked,
not an automatic release fire. If authorization is absent, hand off the verified
release candidate and exact remaining publication steps. Do not call preparation
a completed PyPI release.

## Observable success

- A regional extraction from a supported local or remote HFX root opens in QGIS
  with correct CRS, complete source geometries, usable attributes, and separate
  named-set/geometry-type layers. Users can select just one named set.
- Python offers the same selection and export, and a GeoDataFrame usable by normal
  plotting tools. Extent and explicit-all behavior match the CLI.
- Evidence covers multi-set and mixed-geometry data, intersection versus bbox
  false positives, absent/null bounds, empty selections, absent snap data, and
  actionable failures. Large file export has bounded memory rather than an
  all-features collection requirement.
- Documentation demonstrates the actual shipped interfaces without promising a
  complete routable river network or pip-installed Rust CLI.
- The 0.4.0 release notes account for the entire change range, release artifacts
  pass the existing checks, and, after authorized publication, PyPI serves 0.4.0
  and the matching GitHub Release records it.
