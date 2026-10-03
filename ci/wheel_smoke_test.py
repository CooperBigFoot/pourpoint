"""Installed repaired-wheel checks shared by every cibuildwheel platform.

PyArrow is a test-only fixture writer. No plotting or GDAL Python package is used.
"""

import importlib.metadata
import json
from pathlib import Path
import sqlite3
import struct
import tempfile

import pyarrow as pa
import pyarrow.parquet as pq
import tomli
import pourpoint
from pourpoint import _pourpoint


def write_snap_fixture(root):
    # Deliberately omit graph/catchments: extraction must not load them.
    declarations = []
    point = struct.pack("<BIdd", 1, 1, 1.0, 1.0)
    line = struct.pack("<BII4d", 1, 2, 2, -2.0, 0.5, 3.0, 0.5)
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("unit_id", pa.int64(), nullable=False),
        pa.field("weight", pa.float32(), nullable=False),
        pa.field("stem_role", pa.string()),
        pa.field("geometry", pa.binary(), nullable=False),
    ])
    for name, geometries in [("reach-stems", [point, line]), ("segment-stems", [point])]:
        declarations.append({
            "schema": "hfx.aux.snap.v2",
            "artifacts": {"snap": name + ".parquet"},
            "metadata": {
                "name": name,
                "description": "Synthetic wheel-test targets",
                "references_levels": [0],
                "weight_semantics": "Synthetic producer preference",
            },
        })
        count = len(geometries)
        table = pa.Table.from_pydict({
            "id": list(range(1, count + 1)), "unit_id": [1] * count,
            "weight": [2.5] * count, "stem_role": [None] * count,
            "geometry": geometries,
        }, schema=schema)
        with (root / (name + ".parquet")).open("wb") as stream:
            pq.write_table(table, stream)
    (root / "manifest.json").write_text(json.dumps({
        "format_version": "0.3.0", "fabric_name": "wheel-test",
        "crs": "EPSG:4326", "topology": "tree", "has_up_area": False,
        "bbox": [-2, 0, 3, 1], "unit_count": 1,
        "created_at": "2026-10-03T00:00:00Z", "adapter_version": "test",
        "auxiliary": declarations,
    }), encoding="utf-8")


def check_snap_export(root):
    write_snap_fixture(root)
    engine = pourpoint.Engine(str(root))
    targets = engine.snap_targets(bbox=(0, 0, 1, 1))
    assert isinstance(targets, pourpoint.SnapTargets)
    output = root / "targets.gpkg"
    targets.write(output)
    with sqlite3.connect(str(output)) as db:
        assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
        layers = db.execute(
            "SELECT table_name FROM gpkg_contents WHERE data_type='features'"
        ).fetchall()
        counts = {name: db.execute('SELECT count(*) FROM "' + name + '"').fetchone()[0]
                  for (name,) in layers}
        assert counts == {"reach-stems_points": 1, "reach-stems_lines": 1,
                          "segment-stems_points": 1, "segment-stems_lines": 0}, counts
        assert db.execute("SELECT DISTINCT srs_id FROM gpkg_geometry_columns").fetchall() == [(4326,)]
        assert db.execute(
            'SELECT snap_set, id, unit_id, weight, stem_role FROM "reach-stems_lines"'
        ).fetchone() == ("reach-stems", 2, 1, 2.5, None)
        # Verify full line coordinates in the GeoPackage geometry, not clipping.
        geometry_column = db.execute(
            "SELECT column_name FROM gpkg_geometry_columns WHERE table_name='reach-stems_lines'"
        ).fetchone()[0]
        blob = db.execute('SELECT "' + geometry_column + '" FROM "reach-stems_lines"').fetchone()[0]
        envelope_sizes = {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}
        wkb = blob[8 + envelope_sizes[(blob[3] >> 1) & 7]:]
        endian = "<" if wkb[0] == 1 else ">"
        assert struct.unpack(endian + "II4d", wkb[1:]) == (2, 2, -2.0, 0.5, 3.0, 0.5)
    named = root / "named.gpkg"
    engine.snap_targets(all=True, snap_set="segment-stems").write(named)
    with sqlite3.connect(str(named)) as db:
        assert db.execute("SELECT count(*) FROM gpkg_contents WHERE data_type='features'").fetchone() == (2,)


def main():
    source = Path(__file__).resolve().parents[1]
    with (source / "crates/python/pyproject.toml").open("rb") as stream:
        expected = tomli.load(stream)["project"]["version"]
    assert importlib.metadata.version("pourpoint") == expected, "wrong wheel metadata version"
    assert pourpoint.__version__ == expected, "wrong extension version"
    pkg = Path(pourpoint.__file__).resolve().parent
    assert pkg != source / "crates/python/python/pourpoint", "source-tree import"
    assert (pkg / "_data/gdal/gdalvrt.xsd").is_file(), "missing bundled gdal_data"
    assert (pkg / "_data/proj/proj.db").is_file(), "missing bundled proj.db"
    with tempfile.TemporaryDirectory(prefix="pourpoint-wheel-") as directory:
        root = Path(directory)
        try:
            pourpoint.Engine(str(root / "missing"))
        except pourpoint.DatasetError:
            pass
        else:
            raise AssertionError("Engine should reject missing datasets")
        check_snap_export(root)
    _pourpoint._self_test_proj()
    print("wheel self-test passed; version=" + expected)


if __name__ == "__main__":
    main()
