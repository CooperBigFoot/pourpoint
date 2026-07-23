#!/usr/bin/env python3
"""Generate the pinned PROJ coordinate oracle for WGS 84 / Equal Earth."""

import argparse
import json
import math
import platform
from pathlib import Path

import pyproj


PYTHON_VERSION = "3.14.6"
PYPROJ_VERSION = "3.7.2"
PROJ_VERSION = "9.5.1"
GENERATOR_COMMAND = (
    "uv run --python 3.14.6 --with pyproj==3.7.2 python "
    "scripts/generate-equal-earth-oracle.py --output "
    "crates/core/tests/fixtures/projection/equal_earth_proj_oracle.json"
)

A = 6378137.0
RECIPROCAL_FLATTENING = 298.257223563
M = math.sqrt(3) / 2
A1 = 1.340264
A2 = -0.081106
A3 = 0.000893
A4 = 0.003796

FORWARD_CASES = [
    ("equator", 0, 0),
    ("axis_order_control", 23.5, -17.25),
    ("north_15", 0, +15),
    ("south_15", 0, -15),
    ("north_45", 0, +45),
    ("south_45", 0, -45),
    ("north_60", 0, +60),
    ("south_60", 0, -60),
    ("north_89", 0, +89),
    ("south_89", 0, -89),
    ("north_89_99", 0, +89.99),
    ("south_89_99", 0, -89.99),
    ("north_89_999", 0, +89.999),
    ("south_89_999", 0, -89.999),
    ("antimeridian_east", 180, 0),
    ("antimeridian_west", -180, 0),
    ("north_pole", 0, +90),
    ("south_pole", 0, -90),
]

INVERSE_CASE_NAMES = {
    "equator",
    "axis_order_control",
    "north_15",
    "south_15",
    "north_45",
    "south_45",
    "north_60",
    "south_60",
    "north_89",
    "south_89",
    "north_89_99",
    "south_89_99",
    "antimeridian_east",
    "antimeridian_west",
}

GRIT_CORNERS = [
    ("top_left", -15000000, 8400000),
    ("top_right", 17100000, 8400000),
    ("bottom_left", -15000000, -6600000),
    ("bottom_right", 17100000, -6600000),
]


def require_versions() -> None:
    """Require the process versions recorded in the generated artifact."""
    versions = {
        "Python": (platform.python_version(), PYTHON_VERSION),
        "pyproj": (pyproj.__version__, PYPROJ_VERSION),
        "PROJ": (pyproj.proj_version_str, PROJ_VERSION),
    }
    mismatches = [
        f"{name}: running {actual}, required {required}"
        for name, (actual, required) in versions.items()
        if actual != required
    ]
    if mismatches:
        raise RuntimeError("toolchain version mismatch: " + "; ".join(mismatches))


def require_finite(pair: tuple[float, float], context: str) -> tuple[float, float]:
    """Reject a transform result containing a non-finite ordinate."""
    if not all(math.isfinite(value) for value in pair):
        raise RuntimeError(f"{context} returned non-finite coordinates: {pair!r}")
    return pair


def require_serializable_finite(value: object, context: str = "payload") -> None:
    """Reject every non-finite float before JSON serialization."""
    if isinstance(value, float) and not math.isfinite(value):
        raise RuntimeError(f"{context} contains non-finite value: {value!r}")
    if isinstance(value, dict):
        for key, nested in value.items():
            require_serializable_finite(nested, f"{context}.{key}")
    elif isinstance(value, list):
        for index, nested in enumerate(value):
            require_serializable_finite(nested, f"{context}[{index}]")


def reference_values() -> dict[str, float | str]:
    """Calculate and verify the normative binary64 Equal Earth quantities."""
    flattening = 1 / RECIPROCAL_FLATTENING
    eccentricity_squared = flattening * (2 - flattening)
    eccentricity = math.sqrt(eccentricity_squared)

    def authalic_quantity(phi: float) -> float:
        s = math.sin(phi)
        den = 1 - eccentricity_squared * s * s
        return (1 - eccentricity_squared) * (
            s / den + math.atanh(eccentricity * s) / eccentricity
        )

    polar_authalic_quantity = authalic_quantity(math.pi / 2)
    authalic_radius = A * math.sqrt(polar_authalic_quantity / 2)
    theta_max = math.pi / 3
    y_max = (
        authalic_radius
        * theta_max
        * (
            A1
            + A2 * theta_max**2
            + theta_max**6 * (A3 + A4 * theta_max**2)
        )
    )

    diagnostics = (
        (flattening, 0.0033528106647474805, "flattening"),
        (eccentricity_squared, 0.0066943799901413165, "eccentricity_squared"),
        (eccentricity, 0.08181919084262149, "eccentricity"),
        (
            polar_authalic_quantity,
            1.9955310875028367,
            "polar_authalic_quantity",
        ),
        (authalic_radius, 6371007.180918474, "authalic_radius_metres"),
        (theta_max, 1.0471975511965976, "theta_max_radians"),
        (y_max, 8392927.598466454, "y_max_metres"),
    )
    for actual, expected, name in diagnostics:
        if actual != expected:
            raise RuntimeError(
                f"{name} binary64 diagnostic mismatch: {actual!r} != {expected!r}"
            )

    return {
        "a_metres": A,
        "a1": A1,
        "a2": A2,
        "a3": A3,
        "a4": A4,
        "authalic_radius_metres": authalic_radius,
        "eccentricity": eccentricity,
        "eccentricity_squared": eccentricity_squared,
        "flattening": flattening,
        "m_expression": "sqrt(3)/2",
        "polar_authalic_quantity": polar_authalic_quantity,
        "reciprocal_flattening": RECIPROCAL_FLATTENING,
        "theta_max_radians": theta_max,
        "y_max_metres": y_max,
    }


def equal_earth_derivative(theta: float) -> float:
    """Evaluate the derivative used by inverse Equal Earth Newton updates."""
    return (
        A1
        + 3 * A2 * theta**2
        + theta**6 * (7 * A3 + 9 * A4 * theta**2)
    )


def is_in_canonical_domain(
    x: float, y: float, authalic_radius: float, y_max: float
) -> bool:
    """Classify a projected point against the canonical Equal Earth image."""
    if abs(y) > y_max:
        return False

    target = y / authalic_radius
    theta = y / (authalic_radius * A1)
    for _ in range(12):
        residual = (
            theta
            * (A1 + A2 * theta**2 + theta**6 * (A3 + A4 * theta**2))
            - target
        )
        dtheta = residual / equal_earth_derivative(theta)
        theta -= dtheta
        if abs(dtheta) <= 1e-14:
            break
    else:
        raise RuntimeError(
            f"canonical-domain Newton solve did not converge for ({x!r}, {y!r})"
        )

    derivative = equal_earth_derivative(theta)
    x_half = math.pi * authalic_radius * math.cos(theta) / (M * derivative)
    return abs(x) <= x_half


def build_payload() -> dict[str, object]:
    """Build the complete deterministic oracle payload."""
    require_versions()
    reference = reference_values()
    forward_transformer = pyproj.Transformer.from_crs(
        "EPSG:4326", "EPSG:8857", always_xy=True
    )
    inverse_transformer = pyproj.Transformer.from_crs(
        "EPSG:8857", "EPSG:4326", always_xy=True
    )

    forward_rows = []
    projected_by_name = {}
    for name, lon, lat in FORWARD_CASES:
        projected = require_finite(
            forward_transformer.transform(lon, lat, errcheck=True),
            f"forward transform {name}",
        )
        projected_by_name[name] = projected
        forward_rows.append(
            {
                "geographic_lon_lat_degrees": [lon, lat],
                "name": name,
                "projected_xy_metres": list(projected),
            }
        )

    transposed_geographic = (-17.25, 23.5)
    transposed_projected = require_finite(
        forward_transformer.transform(*transposed_geographic, errcheck=True),
        "forward transform transposition control",
    )
    ordered_projected = projected_by_name["axis_order_control"]
    if ordered_projected == transposed_projected:
        raise RuntimeError("axis-order control did not distinguish transposed input")
    if ordered_projected != (2203165.970658505, -2199534.790026376):
        raise RuntimeError(
            f"ordered axis-control diagnostic mismatch: {ordered_projected!r}"
        )
    if transposed_projected != (-1587191.7091611568, 2976483.336501383):
        raise RuntimeError(
            f"transposed axis-control diagnostic mismatch: {transposed_projected!r}"
        )

    inverse_rows = []
    for name, _, _ in FORWARD_CASES:
        if name not in INVERSE_CASE_NAMES:
            continue
        projected = projected_by_name[name]
        geographic = require_finite(
            inverse_transformer.transform(*projected, errcheck=True),
            f"inverse transform {name}",
        )
        inverse_rows.append(
            {
                "geographic_lon_lat_degrees": list(geographic),
                "name": name,
                "projected_xy_metres": list(projected),
            }
        )

    authalic_radius = reference["authalic_radius_metres"]
    y_max = reference["y_max_metres"]
    if not isinstance(authalic_radius, float) or not isinstance(y_max, float):
        raise RuntimeError("reference values have unexpected types")
    negative_rows = []
    for name, x, y in GRIT_CORNERS:
        if is_in_canonical_domain(x, y, authalic_radius, y_max):
            raise RuntimeError(f"GRIT corner unexpectedly inside domain: {name}")
        negative_rows.append(
            {
                "expected": "outside_projection_domain",
                "name": name,
                "projected_xy_metres": [x, y],
            }
        )

    return {
        "axis_order": {
            "always_xy": True,
            "geographic_pair": "lon_lat_degrees",
            "projected_pair": "x_y_metres",
            "transposition_control": {
                "ordered_geographic_lon_lat_degrees": [23.5, -17.25],
                "ordered_projected_xy_metres": list(ordered_projected),
                "transposed_geographic_lon_lat_degrees": [-17.25, 23.5],
                "transposed_projected_xy_metres": list(transposed_projected),
            },
        },
        "forward_cases": forward_rows,
        "generator": {
            "command": GENERATOR_COMMAND,
            "proj_version": PROJ_VERSION,
            "pyproj_version": PYPROJ_VERSION,
            "python_version": PYTHON_VERSION,
        },
        "grit_grid": {
            "height": 500000,
            "negative_inverse_cases": negative_rows,
            "transform": [30.0, 0.0, -15000000.0, 0.0, -30.0, 8400000.0],
            "width": 1070000,
        },
        "inverse_cases": inverse_rows,
        "reference": reference,
        "schema_version": 1,
    }


def main() -> None:
    """Parse arguments and write the oracle JSON."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    payload = build_payload()
    require_serializable_finite(payload)
    serialized = (
        json.dumps(
            payload,
            sort_keys=True,
            indent=2,
            separators=(",", ": "),
            ensure_ascii=True,
            allow_nan=False,
        )
        + "\n"
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(serialized, encoding="ascii")


if __name__ == "__main__":
    main()
