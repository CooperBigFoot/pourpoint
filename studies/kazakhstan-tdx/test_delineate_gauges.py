"""Study contracts: screening, immutable identity, complete export, lossless GIS roundtrip."""
import csv
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from shapely.geometry import box

spec = importlib.util.spec_from_file_location("delineate_gauges", Path(__file__).with_name("delineate_gauges.py"))
study = importlib.util.module_from_spec(spec)
spec.loader.exec_module(study)


def station(code="10000", lat="1", lon="2"):
    return dict(name_ru="Есиль у станции", name_en="Yesil station", station_code=code,
                latitude=lat, longitude=lon, country_ru="Казахстан", country_en="Kazakhstan", location_flag="0")


def test_screen_uses_original_polygon_not_country_attribute_or_bbox():
    # A polygon hole is outside even though within its rectangular bounds.
    country = box(0, 0, 4, 4).difference(box(1, 1, 3, 3))
    row = study.screen_station(station(lat="2", lon="2.5"), country, set())
    assert row["status"] == "excluded"
    assert "outside chosen Kazakhstan land boundary" in row["reason"]
    assert row["longitude"] == "2.5"


def test_exact_boundary_included_without_buffer():
    country = box(0, 0, 4, 4)
    row = study.screen_station(station(lat="1", lon="0"), country, set())
    assert row["status"] == "eligible"
    assert row["boundary_case"] == "exact"
    outside = study.screen_station(station(lat="1", lon="-0.00001"), country, set())
    assert outside["status"] == "excluded"
    assert outside["boundary_case"] == "within_100m"


@pytest.mark.parametrize("lat,lon", [("nan", "2"), ("91", "2"), ("1", "181"), ("bad", "2")])
def test_invalid_coordinates_are_excluded(lat, lon):
    assert study.screen_station(station(lat=lat, lon=lon), box(0, 0, 4, 4), set())["status"] == "excluded"


def test_explicit_and_suspect_locations_excluded():
    country = box(0, 0, 90, 80)
    for row, duplicates in [(station(code="11264"), set()),
                             (station(lat="52.4317", lon="52.4317"), set()),
                             (station(), {("1", "2")})]:
        assert study.screen_station(row, country, duplicates)["status"] == "excluded"


def test_input_change_fails_before_execution(tmp_path):
    path = tmp_path / "source.csv"
    with path.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=study.SOURCE_FIELDS)
        writer.writeheader()
        writer.writerow(station())
    with pytest.raises(ValueError, match="406"):
        study.read_stations(path)


def test_incomplete_batch_cannot_export(tmp_path):
    study.write_json(tmp_path / "screening.json", [{**station(), "status": "eligible"}])
    with pytest.raises(ValueError, match="Incomplete batch"):
        study.completed_records(SimpleNamespace(cache=tmp_path))


def test_checkpoint_identity_mismatch_rejected(tmp_path):
    (tmp_path / "stations").mkdir()
    study.write_json(tmp_path / "screening.json", [{**station(), "status": "eligible"}])
    study.write_json(tmp_path / "stations/10000.json", {**station(lon="3"), "status": "successful"})
    with pytest.raises(ValueError, match="identity"):
        study.completed_records(SimpleNamespace(cache=tmp_path))


def test_export_keeps_full_multipart_nested_geometries_and_unicode(tmp_path, monkeypatch):
    # Real Fiona writer/reopen path. Country clipping would remove the western component.
    import hashlib
    import shapely
    import fiona
    from shapely.geometry import MultiPolygon, shape
    cache = tmp_path / "cache"
    cache.mkdir()
    (cache / "stations").mkdir()
    rows = []
    geometries = [MultiPolygon([box(-5, 0, -3, 2), box(1, 0, 3, 2)]), box(1.2, 0.2, 2, 1)]
    for index, geometry in enumerate(geometries):
        code = str(10000 + index)
        row = {**station(code=code), "status": "successful", "reason": "", "area_km2": 123.456,
               "terminal_unit_id": str(10000000000 + index), "upstream_unit_count": 3,
               "geometry_sha256": hashlib.sha256(geometry.wkb).hexdigest()}
        (cache / "stations" / f"{code}.wkb").write_bytes(geometry.wkb)
        study.write_json(cache / "stations" / f"{code}.json", row)
        rows.append({**station(code=code), "status": "eligible"})
    study.write_json(cache / "screening.json", rows)
    monkeypatch.setattr(study, "check_identity", lambda args: {})
    args = SimpleNamespace(cache=cache, delivery=tmp_path / "delivery")
    study.export(args)
    with fiona.open(args.delivery / "kaz-basins-tdx.shp", encoding="UTF-8") as output:
        features = list(output)
        assert len(features) == 2
        for feature, expected in zip(features, geometries):
            assert shape(feature["geometry"]).normalize().equals_exact(expected.normalize(), 0)
            assert feature["properties"]["name_ru"] == "Есиль у станции"
    with pytest.raises(FileExistsError):
        study.export(args)


@pytest.mark.parametrize("changed", ["screening", "input", "boundary"])
def test_resume_rejects_tampered_identity_before_opening_engine(tmp_path, changed):
    import pourpoint._pourpoint as extension
    args = SimpleNamespace(input=tmp_path / "input.csv", boundary=tmp_path / "boundary.json", cache=tmp_path)
    args.input.write_text("original input")
    args.boundary.write_text("original boundary")
    screening = tmp_path / "screening.json"
    study.write_json(screening, [{**station(), "status": "eligible"}])
    provenance = dict(input_sha256=study.sha256(args.input), boundary_sha256=study.sha256(args.boundary),
        extension_sha256=study.sha256(Path(extension.__file__)), settings=study.SETTINGS,
        screening_sha256=study.sha256(screening), engine_source=study.ENGINE_SOURCE, dataset=study.DATASET)
    study.write_json(tmp_path / "provenance.json", provenance)
    assert study.check_identity(args) == provenance
    path = {"screening": screening, "input": args.input, "boundary": args.boundary}[changed]
    path.write_text("tampered content")
    with pytest.raises(ValueError, match="identity mismatch"):
        study.check_identity(args)


@pytest.mark.parametrize("name_ru", ["Есиль у станции", "  Есиль у станции  "])
def test_verify_accepts_lossless_single_part_shapefile_coercion(tmp_path, monkeypatch, name_ru):
    import hashlib
    from shapely.geometry import MultiPolygon
    cache = tmp_path / "cache"
    cache.mkdir()
    (cache / "stations").mkdir()
    source = [station(code=str(code)) for code in range(10000, 10405)] + [station(code="11264")]
    source[0]["name_ru"] = name_ru
    input_path = tmp_path / "input.csv"
    with input_path.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=study.SOURCE_FIELDS)
        writer.writeheader()
        writer.writerows(source)
    screening = [{**row, "status": "excluded", "reason": "fixture exclusion"} for row in source]
    screening[0]["status"] = "eligible"
    geometry = MultiPolygon([box(-5, 0, -3, 2)])
    wkb = geometry.wkb
    row = {**source[0], "status": "successful", "reason": "", "area_km2": 123.456,
           "terminal_unit_id": "10000000000", "upstream_unit_count": 3,
           "geometry_sha256": hashlib.sha256(wkb).hexdigest()}
    (cache / "stations/10000.wkb").write_bytes(wkb)
    study.write_json(cache / "stations/10000.json", row)
    study.write_json(cache / "screening.json", screening)
    # Remote identity is unrelated to this geometry regression; actual export and
    # verification use the native Fiona reader/writer and real engine-format WKB.
    monkeypatch.setattr(study, "check_identity", lambda args: {"remote": {}})
    monkeypatch.setattr(study, "configure_s3", lambda path: [])
    monkeypatch.setattr(study, "fetch_manifest", lambda: (b"", {}))
    args = SimpleNamespace(input=input_path, cache=cache, delivery=tmp_path / "delivery", credentials=None)
    study.export(args)
    study.verify(args)
    result = json.loads((args.delivery / "verification.json").read_text())
    assert result["exact_full_geometry_roundtrips"] == 1


def test_exact_component_comparison_rejects_coordinate_edits_and_dropped_parts():
    from shapely.geometry import MultiPolygon
    original = MultiPolygon([box(-5, 0, -3, 2), box(1, 0, 3, 2)])
    assert not study.exact_polygon_components(original, MultiPolygon([box(-5, 0, -3, 2)]))
    assert not study.exact_polygon_components(original,
        MultiPolygon([box(-5, 0, -3, 2), box(1, 0, 3.000000001, 2)]))
    assert study.exact_polygon_components(MultiPolygon([box(0, 0, 1, 1)]), box(0, 0, 1, 1))
