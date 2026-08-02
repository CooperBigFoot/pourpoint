"""Opt-in installed-wheel verifier for the full MERIT HFX dataset."""

import argparse
import importlib.metadata
import json
import math
from pathlib import Path

import pourpoint


def require_installed_wheel() -> None:
    """Reject source-tree, editable, and mismatched package imports."""
    checkout_package = (
        Path(__file__).parents[1] / "crates" / "python" / "python" / "pourpoint"
    ).resolve()
    package_file = Path(pourpoint.__file__).resolve()
    extension_file = Path(pourpoint._pourpoint.__file__).resolve()
    try:
        distribution = importlib.metadata.distribution("pourpoint")
    except importlib.metadata.PackageNotFoundError as error:
        raise AssertionError("pourpoint must be installed from a maturin-built wheel") from error

    installed_package = Path(distribution.locate_file("pourpoint")).resolve()
    assert not package_file.is_relative_to(checkout_package)
    assert not extension_file.is_relative_to(checkout_package)
    assert package_file.parent == installed_package
    assert extension_file.parent == installed_package


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dataset", required=True, type=Path)
    args = parser.parse_args()
    dataset = args.dataset.resolve(strict=True)

    require_installed_wheel()
    manifest_path = dataset / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    assert manifest["format_version"] == "0.3.0"
    assert manifest["crs"] == "EPSG:4326"
    assert manifest["topology"] == "tree"
    assert manifest["fabric_name"] == "merit_basins"
    assert manifest["unit_count"] == 2876771

    auxiliary = manifest["auxiliary"]
    d8_v1 = [entry for entry in auxiliary if entry["schema"] == "hfx.aux.d8_raster.v1"]
    snap_v2 = [entry for entry in auxiliary if entry["schema"] == "hfx.aux.snap.v2"]
    assert len(auxiliary) == 61
    assert len(d8_v1) == 60
    assert len(snap_v2) == 1
    assert manifest_path.stat().st_size == 17303
    assert (dataset / "graph.parquet").stat().st_size == 84338714
    assert (dataset / "catchments.parquet").stat().st_size == 6593009458
    snap_path = dataset / snap_v2[0]["artifacts"]["snap"]
    assert snap_path.stat().st_size == 1848449226

    engine = pourpoint.Engine(str(dataset), refine=False)
    assert engine.unreadable_auxiliary_schemas == ["hfx.aux.d8_raster.v1"] * 60
    result = engine.delineate(lat=47.37, lon=8.54)
    assert result.terminal_unit_id == 23017694
    assert math.isclose(result.area_km2, 2231.9425272184967, rel_tol=0, abs_tol=1e-9)

    payload = {
        "area_km2": result.area_km2,
        "auxiliary_entries": len(auxiliary),
        "d8_v1_occurrences": len(d8_v1),
        "snap_v2_occurrences": len(snap_v2),
        "terminal_unit_id": result.terminal_unit_id,
        "unit_count": manifest["unit_count"],
    }
    print(json.dumps(payload, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
