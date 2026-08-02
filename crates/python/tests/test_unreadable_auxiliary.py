"""Installed-wheel evidence for unreadable auxiliaries in a real MERIT cut."""

import hashlib
import json
from pathlib import Path

import pyarrow.parquet as pq
import pytest

import pourpoint


EXPECTED_HASHES = {
    "manifest.json": "d80cef2838ac844df7922eaf936ac1148ab99a8b1900602bd363ac5a7b328c77",
    "graph.parquet": "95478b94d3926bfc66272bb26651ed88dfd51912a5f447f87e4c1e6347bce625",
    "catchments.parquet": "ba2e31c0c80362349f1a82067183c1b5c6fa13461a69ee5911bb618e3305d744",
    "snap.parquet": "ecc7758ba4b74d33256857744a3625a9628cc124ef309c3f104cddd4c17db817",
}
EXPECTED_BBOX = [
    8.399582862854004,
    46.79708480834961,
    9.43375015258789,
    47.432918548583984,
]


def read_parquet_table(path):
    with open(path, "rb") as fh:
        return pq.read_table(fh)


def test_merit_reduction_survives_sixty_unreadable_v1_declarations():
    fixture = Path(__file__).parent / "fixtures" / "merit-hfx-global-unreadable-v1"

    for name, expected_hash in EXPECTED_HASHES.items():
        assert hashlib.sha256((fixture / name).read_bytes()).hexdigest() == expected_hash

    manifest = json.loads((fixture / "manifest.json").read_text())
    assert manifest["format_version"] == "0.3.0"
    assert manifest["crs"] == "EPSG:4326"
    assert manifest["topology"] == "tree"
    assert manifest["fabric_name"] == "merit_basins"
    assert manifest["unit_count"] == 43
    assert manifest["bbox"] == EXPECTED_BBOX

    auxiliary = manifest["auxiliary"]
    assert len(auxiliary) == 61
    assert auxiliary[0]["schema"] == "hfx.aux.snap.v2"
    assert auxiliary[0]["artifacts"]["snap"] == "snap.parquet"
    assert [entry["schema"] for entry in auxiliary[1:]] == [
        "hfx.aux.d8_raster.v1"
    ] * 60

    graph = read_parquet_table(fixture / "graph.parquet")
    catchments = read_parquet_table(fixture / "catchments.parquet")
    snap = read_parquet_table(fixture / "snap.parquet")
    assert graph.num_rows == catchments.num_rows == snap.num_rows == 43

    graph_ids = set(graph["id"].to_pylist())
    assert graph_ids == set(catchments["id"].to_pylist())
    assert graph_ids == set(snap["unit_id"].to_pylist())
    upstream_by_id = dict(
        zip(graph["id"].to_pylist(), graph["upstream_ids"].to_pylist())
    )
    closure = set()
    pending = [23017694]
    while pending:
        unit_id = pending.pop()
        assert unit_id in graph_ids
        if unit_id not in closure:
            closure.add(unit_id)
            pending.extend(upstream_by_id[unit_id])
    assert closure == graph_ids

    engine = pourpoint.Engine(str(fixture), refine=False)
    assert engine.unreadable_auxiliary_schemas == [
        "hfx.aux.d8_raster.v1"
    ] * 60
    result = engine.delineate(lat=47.37, lon=8.54)
    assert result.terminal_unit_id == 23017694
    assert result.area_km2 == pytest.approx(2231.9425272184967, rel=0, abs=1e-9)
