"""Installed-wheel coverage for manifest-only snap selection and atomic export."""

import builtins
import json
import sqlite3
import struct
from pathlib import Path

import pyarrow as pa
import pyarrow.parquet as pq
import pytest
import pourpoint

from conftest import _write_manifest, make_wkb_point


def _line(points):
    return struct.pack("<BII", 1, 2, len(points)) + b"".join(
        struct.pack("<dd", *point) for point in points
    )


def _declaration(name):
    return {
        "schema": "hfx.aux.snap.v2",
        "artifacts": {"snap": f"snap/{name}.parquet"},
        "metadata": {
            "name": name,
            "description": f"Supplied {name}",
            "references_levels": [0, 1],
            "weight_semantics": "Producer preference, not drainage area",
        },
    }


def _write_targets(path, geometries):
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("unit_id", pa.int64(), nullable=False),
        pa.field("weight", pa.float32(), nullable=False),
        pa.field("stem_role", pa.string()),
        pa.field("geometry", pa.binary(), nullable=False),
    ])
    count = len(geometries)
    table = pa.Table.from_pydict({
        "id": list(range(1, count + 1)),
        "unit_id": [1] * count,
        "weight": [2.5] * count,
        "stem_role": [None] * count,
        "geometry": geometries,
    }, schema=schema)
    with path.open("wb") as output:
        pq.write_table(table, output, row_group_size=1)


@pytest.fixture
def snap_dataset(tmp_path):
    # No graph/catchments: snap extraction must not open delineation artifacts.
    _write_manifest(tmp_path, [_declaration("reach-stems"), _declaration("segment-stems")])
    manifest = json.loads((tmp_path / "manifest.json").read_text())
    manifest["attribution"] = {"license": "source-license", "producer": "test"}
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))
    (tmp_path / "snap").mkdir()
    _write_targets(tmp_path / "snap/reach-stems.parquet", [
        make_wkb_point(1, 1),
        _line([(-2, 1), (3, 1)]),
        # Bounding rectangle intersects [0,0,1,1], actual line does not.
        _line([(-1, 0.5), (0.5, 2)]),
    ])
    _write_targets(tmp_path / "snap/segment-stems.parquet", [make_wkb_point(0.5, 0.5)])
    return tmp_path


def _counts(path):
    with sqlite3.connect(path) as db:
        layers = db.execute("SELECT table_name FROM gpkg_contents WHERE data_type='features'").fetchall()
        return {name: db.execute(f'SELECT count(*) FROM "{name}"').fetchone()[0] for (name,) in layers}


def test_export_snap_only_dataset(snap_dataset, tmp_path):
    engine = pourpoint.Engine(str(snap_dataset))
    targets = engine.snap_targets(bbox=(0, 0, 1, 1))
    assert isinstance(targets, pourpoint.SnapTargets)
    output = tmp_path / "targets.gpkg"
    targets.write(output)
    assert _counts(output) == {
        "reach-stems_points": 1, "reach-stems_lines": 1,
        "segment-stems_points": 1, "segment-stems_lines": 0,
    }
    with sqlite3.connect(output) as db:
        assert db.execute('SELECT snap_set, id, unit_id, weight, stem_role FROM "reach-stems_points"').fetchone() == (
            "reach-stems", 1, 1, 2.5, None
        )
        assert db.execute("SELECT DISTINCT srs_id FROM gpkg_geometry_columns").fetchall() == [(4326,)]
        assert db.execute('SELECT bbox_xmin, bbox_ymin, bbox_xmax, bbox_ymax FROM "reach-stems_points"').fetchone() == (None, None, None, None)
    # Selection can be used again after export.
    targets.write(output)
    with pytest.raises(pourpoint.DatasetError):
        engine.select_level()


def test_scope_and_named_set(snap_dataset, tmp_path):
    engine = pourpoint.Engine(str(snap_dataset))
    for options in ({}, {"all": False}, {"all": True, "bbox": (0, 0, 1, 1)},
                    {"bbox": (1, 0, 0, 1)}, {"bbox": (0, 0, float("nan"), 1)},
                    {"bbox": (-181, 0, 1, 1)}):
        with pytest.raises(ValueError):
            engine.snap_targets(**options)
    with pytest.raises(pourpoint.DatasetError, match="unknown snap set"):
        engine.snap_targets(all=True, snap_set="absent")
    output = tmp_path / "one.gpkg"
    engine.snap_targets(all=True, snap_set="reach-stems").write(output)
    assert _counts(output) == {"reach-stems_points": 1, "reach-stems_lines": 2}
    engine.snap_targets(bbox=(10, 10, 11, 11), snap_set="reach-stems").write(output)
    assert _counts(output) == {"reach-stems_points": 0, "reach-stems_lines": 0}


def test_missing_and_unsupported_snap(hfx_dataset):
    engine = pourpoint.Engine(hfx_dataset)
    with pytest.raises(pourpoint.DatasetError, match="no snap declarations"):
        engine.snap_targets(all=True)
    root = Path(hfx_dataset)
    declaration = _declaration("future-stems")
    declaration["schema"] = "hfx.aux.snap.v3"
    _write_manifest(root, [declaration])
    engine = pourpoint.Engine(hfx_dataset)
    with pytest.raises(pourpoint.DatasetError, match="unsupported snap declaration"):
        engine.snap_targets(all=True)


def test_failed_export_keeps_destination(snap_dataset, tmp_path):
    _write_targets(snap_dataset / "snap/segment-stems.parquet", [b"bad-wkb"])
    targets = pourpoint.Engine(str(snap_dataset)).snap_targets(all=True)
    output = tmp_path / "existing.gpkg"
    output.write_bytes(b"existing destination")
    with pytest.raises(pourpoint.DatasetError):
        targets.write(output)
    assert output.read_bytes() == b"existing destination"
    assert not list(tmp_path.glob(".pourpoint-snap-*"))
    absent = tmp_path / "absent.gpkg"
    with pytest.raises(pourpoint.DatasetError):
        targets.write(absent)
    assert not absent.exists()
    assert not list(tmp_path.glob(".pourpoint-snap-*"))
    with pytest.raises(ValueError, match=".gpkg"):
        targets.write(tmp_path / "wrong.json")


def test_geodataframe(snap_dataset):
    pytest.importorskip("geopandas")
    targets = pourpoint.Engine(str(snap_dataset)).snap_targets(bbox=(0, 0, 1, 1))
    frame = targets.to_geodataframe()
    assert list(frame.snap_set) == ["reach-stems", "reach-stems", "segment-stems"]
    assert list(frame.id) == [1, 2, 1]
    assert frame.crs.to_epsg() == 4326
    assert frame.geometry_wkb.iloc[1] == _line([(-2, 1), (3, 1)])
    assert list(frame.geometry.iloc[1].coords) == [(-2, 1), (3, 1)]
    assert frame.stem_role.isna().all()
    assert list(frame.bbox) == [None, None, None]
    assert frame.attrs["snap_sets"][0]["references_levels"] == [0, 1]
    assert frame.attrs["manifest"]["attribution"]["license"] == "source-license"
    empty = pourpoint.Engine(str(snap_dataset)).snap_targets(bbox=(10, 10, 11, 11)).to_geodataframe()
    assert empty.empty
    assert empty.crs.to_epsg() == 4326
    assert "snap_set" in empty.columns


def test_geopandas_optional(snap_dataset, tmp_path, monkeypatch):
    original_import = builtins.__import__

    def no_geopandas(name, *args, **kwargs):
        if name == "geopandas":
            raise ImportError("blocked for optional-dependency test")
        return original_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", no_geopandas)
    targets = pourpoint.Engine(str(snap_dataset)).snap_targets(all=True)
    targets.write(tmp_path / "without-geopandas.gpkg")
    with pytest.raises(ImportError, match=r"pourpoint\[geopandas\]"):
        targets.to_geodataframe()


def test_constructor_validates_manifest_and_source(snap_dataset):
    with pytest.raises(pourpoint.DatasetError, match="unsupported"):
        pourpoint.Engine("ftp://example.org/hfx")
    path = snap_dataset / "manifest.json"
    manifest = json.loads(path.read_text())
    manifest["format_version"] = "0.1.0"
    path.write_text(json.dumps(manifest))
    with pytest.raises(pourpoint.DatasetError):
        pourpoint.Engine(str(snap_dataset))


def test_supplied_bbox_preserved(snap_dataset, tmp_path):
    pytest.importorskip("geopandas")
    path = snap_dataset / "snap/reach-stems.parquet"
    bbox_fields = [pa.field(name, pa.float32(), nullable=False)
                   for name in ("xmin", "ymin", "xmax", "ymax")]
    bbox_values = [None, {"xmin": 0.25, "ymin": 0.25, "xmax": 0.75, "ymax": 0.75},
                   {"xmin": 180.0, "ymin": 90.0, "xmax": 180.0, "ymax": 90.0},
                   {"xmin": -180.0, "ymin": -90.0, "xmax": -180.0, "ymax": -90.0}]
    geo = {"version": "1.1.0", "primary_column": "geometry", "columns": {
        "geometry": {"encoding": "WKB", "geometry_types": ["Point"], "covering": {
            "bbox": {name: ["bbox", name] for name in ("xmin", "ymin", "xmax", "ymax")}
        }}
    }}
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("unit_id", pa.int64(), nullable=False),
        pa.field("weight", pa.float32(), nullable=False),
        pa.field("geometry", pa.binary(), nullable=False),
        pa.field("bbox", pa.struct(bbox_fields)),
    ], metadata={b"geo": json.dumps(geo).encode()})
    table = pa.Table.from_pydict({
        "id": [1, 2, 3, 4], "unit_id": [1, 1, 1, 1], "weight": [2.5] * 4,
        "geometry": [make_wkb_point(0, 0), make_wkb_point(0.5, 0.5), make_wkb_point(180, 90), make_wkb_point(-180, -90)],
        "bbox": bbox_values,
    }, schema=schema)
    with path.open("wb") as output:
        pq.write_table(table, output, row_group_size=1)
    targets = pourpoint.Engine(str(snap_dataset)).snap_targets(all=True, snap_set="reach-stems")
    frame = targets.to_geodataframe()
    expected = [None, (0.25, 0.25, 0.75, 0.75), (180.0, 90.0, 180.0, 90.0), (-180.0, -90.0, -180.0, -90.0)]
    assert list(frame.bbox) == expected
    output = tmp_path / "bounds.gpkg"
    targets.write(output)
    with sqlite3.connect(output) as db:
        rows = db.execute('SELECT bbox_xmin, bbox_ymin, bbox_xmax, bbox_ymax FROM "reach-stems_points" ORDER BY id').fetchall()
    assert rows == [(None, None, None, None), *expected[1:]]


def test_lazy_delineation_retry(hfx_dataset):
    root = Path(hfx_dataset)
    graph = root / "graph.parquet"
    graph_bytes = graph.read_bytes()
    graph.write_bytes(b"invalid parquet")
    engine = pourpoint.Engine(hfx_dataset, refine=False)
    with pytest.raises(pourpoint.DatasetError):
        engine.select_level()
    graph.write_bytes(graph_bytes)
    assert engine.select_level().level == 0
    # Repeated operations reuse the initialized engine.
    assert engine.select_level().level == 0


def test_relative_source_stays_anchored_after_chdir(hfx_dataset, tmp_path, monkeypatch):
    from conftest import _write_graph, _write_catchments, _write_snap

    original = Path(hfx_dataset)
    declaration = _declaration("stems")
    declaration["artifacts"]["snap"] = "snap.parquet"
    declaration["metadata"]["references_levels"] = [0]
    _write_manifest(original, [declaration])
    _write_snap(original)

    alternate_parent = tmp_path / "other"
    alternate = alternate_parent / original.name
    alternate.mkdir(parents=True)
    _write_manifest(alternate, [declaration])
    _write_graph(alternate)
    _write_catchments(alternate)
    _write_snap(alternate)
    # Both roots are valid, but the alternate has a different snap ID and an
    # isolated terminal unit, so its delineation would return only one unit.
    with (alternate / "snap.parquet").open("rb") as source:
        table = pq.read_table(source)
    table = table.set_column(0, table.schema.field("id"), pa.array([4001], type=pa.int64()))
    with (alternate / "snap.parquet").open("wb") as output:
        pq.write_table(table, output)
    with (alternate / "graph.parquet").open("rb") as source:
        table = pq.read_table(source)
    index = table.schema.get_field_index("upstream_ids")
    table = table.set_column(index, table.schema.field(index),
                             pa.array([[], [1], []], type=table.schema.field(index).type))
    with (alternate / "graph.parquet").open("wb") as output:
        pq.write_table(table, output)

    monkeypatch.chdir(original.parent)
    engine = pourpoint.Engine(original.name, refine=False)
    monkeypatch.chdir(alternate_parent)
    output = tmp_path / "anchored.gpkg"
    engine.snap_targets(all=True).write(output)
    with sqlite3.connect(output) as db:
        assert db.execute('SELECT id FROM "stems_points"').fetchall() == [(3001,)]
    assert len(engine.delineate(lat=0.20, lon=1.70).upstream_unit_ids) == 3
    assert len(pourpoint.Engine(original.name, refine=False).delineate(lat=0.20, lon=1.70).upstream_unit_ids) == 1
