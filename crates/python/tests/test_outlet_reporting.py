"""Outlet coordinates and ranked provenance through installed Python surfaces."""

import json
import shutil
import struct
from pathlib import Path

import pourpoint
import pyarrow as pa
import pyarrow.parquet as pq
import pytest


@pytest.fixture
def vector_raster_dataset(tmp_path):
    source = (
        Path(__file__).parents[2]
        / "core/tests/fixtures/parity/tiny-with-aux-d8-projected-grass"
    )
    shutil.copytree(source, tmp_path, dirs_exist_ok=True)
    manifest_path = tmp_path / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["auxiliary"].append(
        {
            "schema": "hfx.aux.snap.v2",
            "artifacts": {"snap": "snap.parquet"},
            "metadata": {
                "name": "outlet-reference",
                "description": "Synthetic vector reference",
                "weight_semantics": "higher is preferred",
                "references_levels": [1],
            },
        }
    )
    manifest_path.write_text(json.dumps(manifest))
    schema = pa.schema(
        [
            pa.field("id", pa.int64(), nullable=False),
            pa.field("unit_id", pa.int64(), nullable=False),
            pa.field("weight", pa.float32(), nullable=False),
            pa.field("geometry", pa.binary(), nullable=False),
        ]
    )
    table = pa.Table.from_pydict(
        {
            "id": [401],
            "unit_id": [4],
            "weight": [1.0],
            "geometry": [
                struct.pack("<BIdd", 1, 1, 0.9833333333333333, 0.4166666666666667)
            ],
        },
        schema=schema,
    )
    with (tmp_path / "snap.parquet").open("wb") as output:
        pq.write_table(table, output)
    return str(tmp_path)


def test_vector_reference_and_ranked_center_survive_python_surfaces(
    vector_raster_dataset,
):
    engine = pourpoint.Engine(vector_raster_dataset)
    request = (0.98, 0.42)
    result = engine.delineate(lon=request[0], lat=request[1])
    assert result.input_outlet == request
    assert result.resolved_outlet == (0.9833333333333333, 0.4166666666666667)
    assert result.refined_outlet != result.resolved_outlet
    # The vector reference equals the existing projected fixture's containment
    # reference. The same nearest threshold-qualified seed therefore wins.
    assert result.refined_outlet == pytest.approx(
        (0.9864447364836884, 0.4163847890060862), rel=0, abs=1e-14
    )
    assert result.refinement_seed_kind == "raster_ranked"
    assert result.refinement_skip_reason is None
    props = json.loads(result.to_geojson())["properties"]
    assert (props["input_lon"], props["input_lat"]) == request
    assert (props["resolved_lon"], props["resolved_lat"]) == result.resolved_outlet
    assert (props["refined_lon"], props["refined_lat"]) == result.refined_outlet
    assert "RasterOutletRanked" in props["refinement"]
    assert "VectorOutletQuantized" not in props["refinement"]

    level = engine.select_level(pourpoint.LevelSelection.FINEST)
    outlet = engine.resolve_outlet(lon=request[0], lat=request[1], level=level)
    upstream = engine.traverse(outlet)
    units = engine.pre_merge_units(upstream)
    refinement = engine.refine(outlet, units)
    dissolved = engine.dissolve(units, refinement)
    staged = engine.compose_result(outlet, upstream, units, refinement, dissolved)
    assert staged.refined_outlet == result.refined_outlet
    assert staged.resolved_outlet == result.resolved_outlet
    assert staged.refinement_seed_kind == result.refinement_seed_kind
    assert staged.terminal_unit_id == result.terminal_unit_id == 4
    assert staged.upstream_unit_ids == result.upstream_unit_ids
    assert staged.geometry_wkb == result.geometry_wkb


def test_disabled_refinement_exports_null_center(vector_raster_dataset):
    result = pourpoint.Engine(vector_raster_dataset, refine=False).delineate(
        lon=0.98, lat=0.42
    )
    assert result.resolved_outlet == (0.9833333333333333, 0.4166666666666667)
    assert result.refined_outlet is None
    assert result.refinement_seed_kind == "disabled"
    props = json.loads(result.to_geojson())["properties"]
    assert props["refined_lon"] is None
    assert props["refined_lat"] is None


def test_coarse_fallback_exports_null_center(hfx_dataset):
    result = pourpoint.Engine(hfx_dataset).delineate(lon=1.70, lat=0.20)
    assert result.refined_outlet is None
    assert result.refinement_seed_kind == "coarse"
    assert result.refinement_skip_reason is not None
    props = json.loads(result.to_geojson())["properties"]
    assert props["refined_lon"] is None
    assert props["refined_lat"] is None
    assert props["refinement"].startswith("best_effort_skipped(")
