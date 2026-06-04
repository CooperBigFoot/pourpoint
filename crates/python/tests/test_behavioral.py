"""Behavioral tests for pyshed bindings.

Exercises delineation, coordinate validation, batch exception mapping,
GeoJSON output, and repr — all against a synthetic 3-unit HFX dataset
created by the ``hfx_dataset`` fixture in conftest.py.

Unit layout (non-overlapping bboxes along x-axis):
    unit 1: x=[0.5, 0.9], y=[0.0, 0.4]  — headwater
    unit 2: x=[1.0, 1.4], y=[0.0, 0.4]  — drains unit 1
    unit 3: x=[1.5, 1.9], y=[0.0, 0.4]  — drains units 1+2 (outlet)

Test coordinate inside unit 3: lon=1.70, lat=0.20
Test coordinate inside unit 1: lon=0.70, lat=0.20
"""

import json

import pytest

import pyshed


class TestSingleDelineation:
    """Tests for engine.delineate()."""

    def test_delineate_accepts_weight_first_snap_strategy(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset, snap_strategy="weight-first")
        result = engine.delineate(lat=0.20, lon=1.70)
        assert result.area_km2 > 0

    def test_invalid_snap_strategy_raises_value_error(self, hfx_dataset):
        with pytest.raises(ValueError, match="invalid snap_strategy"):
            pyshed.Engine(hfx_dataset, snap_strategy="bogus")

    def test_delineate_returns_result(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        # Coordinate inside unit 3's bbox
        result = engine.delineate(lat=0.20, lon=1.70)
        assert result.area_km2 > 0
        assert isinstance(result.geometry_wkb, bytes)
        assert len(result.geometry_wkb) > 0
        assert len(result.upstream_unit_ids) >= 1
        assert result.terminal_unit_id > 0

    def test_delineate_input_outlet(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        lon, lat = result.input_outlet
        assert abs(lon - 1.70) < 1e-6
        assert abs(lat - 0.20) < 1e-6

    def test_delineate_resolved_outlet(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        lon, lat = result.resolved_outlet
        assert isinstance(lon, float)
        assert isinstance(lat, float)

    def test_delineate_outside_catchments_raises_resolution_error(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        # Geographically valid but outside all catchments
        with pytest.raises(pyshed.ResolutionError):
            engine.delineate(lat=50.0, lon=50.0)


class TestStagedDelineation:
    """Tests for the typed staged API."""

    def test_manual_staged_composition_matches_delineate(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        direct = engine.delineate(lat=0.20, lon=1.70)

        level = engine.select_level()
        outlet = engine.resolve_outlet(level, lat=0.20, lon=1.70)
        upstream = engine.traverse(outlet)
        units = engine.pre_merge_units(upstream)
        refinement = engine.refine(outlet, units)
        dissolved = engine.dissolve(units, refinement)
        manual = engine.compose_result(outlet, upstream, units, refinement, dissolved)

        assert manual.terminal_unit_id == direct.terminal_unit_id
        assert manual.input_outlet == direct.input_outlet
        assert manual.resolved_outlet == direct.resolved_outlet
        assert manual.upstream_unit_ids == direct.upstream_unit_ids
        assert manual.area_km2 == pytest.approx(direct.area_km2, rel=1e-9)
        assert manual.geometry_wkb == direct.geometry_wkb

    def test_staged_missing_kwargs_use_friendly_errors(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        level = engine.select_level()
        with pytest.raises(TypeError, match="lat"):
            engine.resolve_outlet(level, lon=1.70)
        with pytest.raises(TypeError, match="lat"):
            engine.resolve_outlet(level, lattitude=0.20, lon=1.70)

    def test_staged_wrong_intermediate_type_errors(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        level = engine.select_level()
        with pytest.raises(TypeError):
            engine.traverse(level)

    def test_staged_out_of_order_call_errors(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        level = engine.select_level()
        outlet = engine.resolve_outlet(level, lat=0.20, lon=1.70)
        upstream = engine.traverse(outlet)
        units = engine.pre_merge_units(upstream)
        with pytest.raises(TypeError):
            engine.dissolve(units, outlet)

    def test_merged_result_is_lean_and_pre_merge_bundle_is_heavy(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        level = engine.select_level()
        outlet = engine.resolve_outlet(level, lat=0.20, lon=1.70)
        upstream = engine.traverse(outlet)
        units = engine.pre_merge_units(upstream)

        assert isinstance(result.geometry_wkb, bytes)
        assert result.upstream_units
        first = result.upstream_units[0]
        assert isinstance(first, pyshed.DelineationUnitMetadata)
        assert first.id in result.upstream_unit_ids
        assert first.area_km2 > 0
        assert not hasattr(first, "geometry_wkb")
        assert not hasattr(result, "unit_geometry_wkb")

        whole_unit_wkb = units.unit_geometry_wkb
        assert len(whole_unit_wkb) == len(units.units)
        assert all(isinstance(wkb, bytes) and wkb for wkb in whole_unit_wkb)

    def test_pre_merge_bundle_documents_r3_divergence(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        direct = engine.delineate(lat=0.20, lon=1.70)
        level = engine.select_level()
        outlet = engine.resolve_outlet(level, lat=0.20, lon=1.70)
        upstream = engine.traverse(outlet)
        units = engine.pre_merge_units(upstream)

        assert "R3" in pyshed.PreMergeDrainageUnits.R3_NOTE
        assert "whole terminal" in pyshed.PreMergeDrainageUnits.R3_NOTE
        assert "r3_note" in repr(units)
        source_unit_area_sum = sum(unit.area_km2 for unit in units.units)
        assert source_unit_area_sum != pytest.approx(direct.area_km2, rel=1e-9)


class TestCoordinateValidation:
    """Tests for coordinate validation (issue 6 fix)."""

    def test_lat_too_high(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        with pytest.raises(ValueError, match="latitude.*outside"):
            engine.delineate(lat=91.0, lon=0.0)

    def test_lat_too_low(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        with pytest.raises(ValueError, match="latitude.*outside"):
            engine.delineate(lat=-91.0, lon=0.0)

    def test_lon_too_high(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        with pytest.raises(ValueError, match="longitude.*outside"):
            engine.delineate(lat=0.0, lon=181.0)

    def test_lon_too_low(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        with pytest.raises(ValueError, match="longitude.*outside"):
            engine.delineate(lat=0.0, lon=-181.0)

    def test_batch_validates_coords(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        with pytest.raises(ValueError, match="latitude.*outside"):
            engine.delineate_batch([{"lat": 91.0, "lon": 0.0}])


class TestBatchDelineation:
    """Tests for engine.delineate_batch()."""

    def test_batch_all_succeed(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        results = engine.delineate_batch([
            {"lat": 0.20, "lon": 1.70},
            {"lat": 0.20, "lon": 0.70},
        ])
        assert len(results) == 2
        for r in results:
            assert r.area_km2 > 0

    def test_batch_raises_typed_exception(self, hfx_dataset):
        """Batch errors must use typed exceptions, not generic RuntimeError (issue 2 fix)."""
        engine = pyshed.Engine(hfx_dataset)
        # Mix a valid outlet with one outside all catchments
        with pytest.raises(pyshed.ShedError) as exc_info:
            engine.delineate_batch([
                {"lat": 0.20, "lon": 1.70},
                {"lat": 50.0, "lon": 50.0},
            ])
        # Must be ResolutionError specifically, not a generic RuntimeError
        assert isinstance(exc_info.value, pyshed.ResolutionError)


class TestGeoJSON:
    """Tests for to_geojson() output."""

    def test_geojson_is_valid_json(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        geojson_str = result.to_geojson()
        data = json.loads(geojson_str)
        assert data["type"] == "Feature"
        assert data["geometry"]["type"] == "MultiPolygon"

    def test_geojson_has_expected_properties(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        data = json.loads(result.to_geojson())
        props = data["properties"]
        assert "area_km2" in props
        assert "terminal_unit_id" in props
        assert "resolution_method" in props
        assert "refinement" in props
        assert "upstream_unit_count" in props
        assert props["area_km2"] > 0


class TestRepr:
    """Tests for __repr__."""

    def test_repr_contains_key_info(self, hfx_dataset):
        engine = pyshed.Engine(hfx_dataset)
        result = engine.delineate(lat=0.20, lon=1.70)
        r = repr(result)
        assert "DelineationResult" in r
        assert "area_km2" in r
