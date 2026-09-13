#!/usr/bin/env python3
"""Kazakhstan overview: exported station watersheds × statuses → projected PNG/PDF.

Run only after the shapefile and complete status CSV exist::

    .venv-study/bin/python studies/kazakhstan-tdx/map_overview.py \
        --delivery /Users/nicolaslazaro/Desktop/kaz-basins-tdx \
        --boundary /absolute/path/to/full-resolution-KAZ.geojson

The boundary must contain Kazakhstan only in EPSG:4326. Natural Earth admin-0
context is downloaded from the explicit URL below into an external cache.
Use --cache to override that location. Existing map outputs are refused unless
--overwrite is supplied after inspection. No input geometry or CSV is modified.
Map-only simplification is 150 projected metres, with topology preserved.
"""

from __future__ import annotations

import argparse
from collections import Counter
import csv
import hashlib
import json
import math
from pathlib import Path
import urllib.request

import fiona
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.lines import Line2D
from matplotlib.patches import Patch, PathPatch
from matplotlib.path import Path as PlotPath
import numpy as np
from pyproj import CRS, Geod, Transformer
from shapely.geometry import Point, box, shape
from shapely.geometry.polygon import orient
from shapely.ops import transform, unary_union

CONTEXT_URL = (
    "https://naturalearth.s3.amazonaws.com/10m_cultural/"
    "ne_10m_admin_0_countries.zip"
)
MAP_CRS = CRS.from_proj4(
    "+proj=aea +lat_1=40 +lat_2=55 +lat_0=48 +lon_0=68 +datum=WGS84 +units=m"
)
STATUS_STYLE = {
    "successful": ("o", "#154c75", "Successful gauges"),
    "excluded": ("x", "#ae3d14", "Excluded stations"),
    "failed": ("^", "#b20b63", "Failed delineations"),
}


def usable_coordinate(row):
    """Return an unchanged geographic point, or None for unusable coordinates."""
    try:
        longitude, latitude = float(row["longitude"]), float(row["latitude"])
    except (ValueError, TypeError):
        return None
    if not (math.isfinite(longitude) and math.isfinite(latitude)):
        return None
    if not (-180 <= longitude <= 180 and -90 <= latitude <= 90):
        return None
    return Point(longitude, latitude)


def polygon_parts(geometry):
    if geometry.geom_type == "Polygon":
        yield geometry
    elif geometry.geom_type in ("MultiPolygon", "GeometryCollection"):
        for part in geometry.geoms:
            yield from polygon_parts(part)


def draw_polygons(axes, geometry, **style):
    """Draw polygon holes correctly using oppositely oriented ring paths."""
    for polygon in polygon_parts(geometry):
        polygon = orient(polygon, sign=1.0)
        vertices, codes = [], []
        for ring in (polygon.exterior, *polygon.interiors):
            coordinates = list(ring.coords)
            vertices.extend(coordinates)
            codes.extend([PlotPath.MOVETO] + [PlotPath.LINETO] * (len(coordinates) - 2)
                         + [PlotPath.CLOSEPOLY])
        axes.add_patch(PathPatch(PlotPath(np.asarray(vertices), codes), **style))


def full_extent(geometries, padding=0.065):
    """Bounds of every unsimplified basin, boundary, and usable station."""
    bounds = [geometry.bounds for geometry in geometries if not geometry.is_empty]
    if not bounds or not all(math.isfinite(v) for item in bounds for v in item):
        raise ValueError("Map inputs have empty or non-finite projected bounds")
    west, south = min(b[0] for b in bounds), min(b[1] for b in bounds)
    east, north = max(b[2] for b in bounds), max(b[3] for b in bounds)
    dx, dy = max(east - west, 1), max(north - south, 1)
    return west - dx * padding, south - dy * padding, east + dx * padding, north + dy * padding


def read_inputs(delivery, boundary_path):
    with (delivery / "station-status.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    if not rows:
        raise ValueError("Empty station report")
    required = {"station_code", "longitude", "latitude", "status", "reason"}
    if not required.issubset(rows[0]):
        raise ValueError(f"Station report lacks fields: {required - rows[0].keys()}")
    ids = [row["station_code"] for row in rows]
    if len(ids) != len(set(ids)):
        raise ValueError("Station report contains duplicate station codes")
    unknown = {row["status"] for row in rows} - STATUS_STYLE.keys()
    if unknown:
        raise ValueError(f"Unknown or incomplete station statuses: {unknown}")
    with fiona.open(delivery / "kaz-basins-tdx.shp") as features:
        if CRS(features.crs) != CRS.from_epsg(4326):
            raise ValueError("Watershed shapefile must declare EPSG:4326")
        basins = [(str(f["properties"]["station_id"]), shape(f["geometry"])) for f in features]
    success_ids = {row["station_code"] for row in rows if row["status"] == "successful"}
    if len(basins) != len(success_ids) or {item[0] for item in basins} != success_ids:
        raise ValueError("Shapefile station identities do not match successful CSV rows")
    if any(g.is_empty or not g.is_valid or g.geom_type not in ("Polygon", "MultiPolygon")
           for _, g in basins):
        raise ValueError("Shapefile contains empty, invalid, or non-polygonal geometry")
    with fiona.open(boundary_path) as features:
        if CRS(features.crs) != CRS.from_epsg(4326):
            raise ValueError("Kazakhstan boundary must declare EPSG:4326")
        boundary = unary_union([shape(f["geometry"]) for f in features])
    if boundary.is_empty or not boundary.is_valid:
        raise ValueError("Kazakhstan boundary is empty or invalid")
    if any(usable_coordinate(row) is None for row in rows if row["status"] == "successful"):
        raise ValueError("Successful station has unusable original coordinates")
    return rows, basins, boundary


def context_archive(cache):
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "ne_10m_admin_0_countries.zip"
    if not archive.exists():
        temporary = archive.with_suffix(".download")
        with urllib.request.urlopen(CONTEXT_URL, timeout=120) as response:
            with temporary.open("wb") as stream:
                while chunk := response.read(1024 * 1024):
                    stream.write(chunk)
        temporary.replace(archive)
    return archive


def add_scale(axes, extent, inverse):
    west, south, east, north = extent
    x, y = west + 0.06 * (east - west), south + 0.045 * (north - south)
    length = 500_000 if east - west > 2_500_000 else 200_000
    lon1, lat1 = inverse.transform(x, y)
    lon2, lat2 = inverse.transform(x + length, y)
    _, _, metres = Geod(ellps="WGS84").inv(lon1, lat1, lon2, lat2)
    axes.plot([x, x + length], [y, y], color="#27313b", linewidth=3, zorder=20)
    for xx in (x, x + length):
        axes.plot([xx, xx], [y - (north - south) * 0.006, y + (north - south) * 0.006],
                  color="#27313b", linewidth=1.5, zorder=20)
    axes.text(x + length / 2, y + (north - south) * 0.014,
              f"≈ {metres / 1000:.0f} km", ha="center", fontsize=10, zorder=21,
              bbox={"facecolor": "white", "alpha": 0.85, "edgecolor": "none", "pad": 2})


def render(delivery, boundary_path, cache, overwrite=False):
    outputs = [delivery / f"kaz-basins-tdx-overview.{suffix}" for suffix in ("png", "pdf")]
    provenance_path = delivery / "map-provenance.json"
    collisions = [str(path) for path in [*outputs, provenance_path] if path.exists()]
    if collisions and not overwrite:
        raise FileExistsError(f"Inspect existing outputs before using --overwrite: {collisions}")
    rows, basins, boundary = read_inputs(delivery, boundary_path)
    forward = Transformer.from_crs(4326, MAP_CRS, always_xy=True)
    inverse = Transformer.from_crs(MAP_CRS, 4326, always_xy=True)
    project = lambda geometry: transform(forward.transform, geometry)
    projected_boundary = project(boundary)
    projected_basins = [(code, project(geometry)) for code, geometry in basins]
    stations = [(row, usable_coordinate(row)) for row in rows]
    plotted = [(row, project(point)) for row, point in stations if point is not None]
    extent = full_extent([projected_boundary, *(g for _, g in projected_basins),
                          *(p for _, p in plotted)])
    archive = context_archive(cache)
    counts = Counter(row["status"] for row in rows)
    omitted = sum(point is None for _, point in stations)
    fig, axes = plt.subplots(figsize=(16, 11.5))
    fig.subplots_adjust(left=0.025, right=0.975, bottom=0.14, top=0.885)
    axes.set_facecolor("#eaf1f5")
    # Context alone is cropped before projection to avoid world-wrap artefacts.
    # Neither watershed polygons nor the Kazakhstan boundary are clipped.
    context_bounds = inverse.transform_bounds(*extent, densify_pts=41)
    geographic_window = box(*context_bounds).buffer(2)
    with fiona.open(f"zip://{archive}") as countries:
        for feature in countries:
            geometry = shape(feature["geometry"])
            if not geometry.intersects(geographic_window):
                continue
            local = project(geometry.intersection(geographic_window))
            draw_polygons(axes, local.simplify(800, preserve_topology=True),
                          facecolor="#f5f2eb", edgecolor="#a5a7a3", linewidth=0.65, zorder=1)
            properties = feature["properties"]
            if properties.get("ADM0_A3") == "KAZ":
                continue
            visible = local.intersection(box(*extent))
            if not visible.is_empty and visible.area > 0.018 * (extent[2]-extent[0]) * (extent[3]-extent[1]):
                label = visible.representative_point()
                axes.text(label.x, label.y, properties.get("NAME_EN", properties["NAME"]),
                          ha="center", fontsize=10, color="#767570", zorder=2)
    draw_polygons(axes, projected_boundary.simplify(150, preserve_topology=True),
                  facecolor="#fffdf7", edgecolor="none", linewidth=0, zorder=2)
    for _, geometry in sorted(projected_basins, key=lambda item: item[1].area, reverse=True):
        draw_polygons(axes, geometry.simplify(150, preserve_topology=True),
                      facecolor="#3a91b7", edgecolor="#176586", linewidth=0.35,
                      alpha=0.25, zorder=3)
    draw_polygons(axes, projected_boundary.simplify(150, preserve_topology=True),
                  facecolor="none", edgecolor="#41413f", linewidth=1.3, zorder=4)
    for status, (marker, color, _) in STATUS_STYLE.items():
        points = [point for row, point in plotted if row["status"] == status]
        axes.scatter([p.x for p in points], [p.y for p in points], marker=marker,
                     color=color, s=14 if status == "successful" else 65,
                     linewidths=0.7 if status == "successful" else 1.5, zorder=6)
    for row, point in plotted:
        if row["station_code"] == "11264":
            axes.annotate("11264: excluded\nsuspect original coordinates", (point.x, point.y),
                          xytext=(12, 14), textcoords="offset points", fontsize=9,
                          color=STATUS_STYLE["excluded"][1], zorder=10,
                          bbox={"facecolor": "white", "alpha": 0.85, "edgecolor": "none", "pad": 3})
    axes.set_xlim(extent[0], extent[2])
    axes.set_ylim(extent[1], extent[3])
    axes.set_aspect("equal")
    axes.set_xticks([])
    axes.set_yticks([])
    for spine in axes.spines.values():
        spine.set_color("#b7c0c5")
    add_scale(axes, extent, inverse)
    fig.suptitle("Kazakhstan discharge gauges | TDX-Hydro watersheds", fontsize=21, y=0.974)
    fig.text(0.5, 0.928,
             f"{len(rows)} stations  •  {counts['successful']} successful  •  "
             f"{counts['excluded']} excluded  •  {counts['failed']} failed",
             ha="center", fontsize=13, color="#334454")
    handles = [Patch(facecolor="#89bfd4", edgecolor="#176586", label="Whole-unit watersheds (overlap retained)"),
               Line2D([], [], color="#41413f", linewidth=1.3, label="Kazakhstan boundary")]
    handles.extend(Line2D([], [], marker=marker, color=color, linestyle="none",
                          markersize=5 if status == "successful" else 8, label=label)
                   for status, (marker, color, label) in STATUS_STYLE.items())
    fig.legend(handles=handles, loc="lower center", bbox_to_anchor=(0.5, 0.085),
               ncol=3, frameon=False, fontsize=10)
    fig.text(0.5, 0.058,
             "Full upstream extent; not clipped to Kazakhstan. Whole drainage units including terminal unit; no D8 refinement.",
             ha="center", fontsize=10)
    fig.text(0.5, 0.038,
             "Original gauge coordinates shown. Darker fill indicates overlapping watersheds. "
             f"Unusable coordinates not plotted: {omitted}.", ha="center", fontsize=9, color="#4c5861")
    fig.text(0.5, 0.019,
             "Albers equal-area (40° / 55°N; 68°E), WGS 84 • Natural Earth context: public domain\n"
             "KAZ land outline: geoBoundaries / © OpenStreetMap contributors (ODbL), source year 2017, "
             "pinned 9469f09 • map-provenance.json",
             ha="center", fontsize=8, color="#65717a")
    for output in outputs:
        fig.savefig(output, dpi=240, facecolor="white")
    plt.close(fig)
    hashed_inputs = [boundary_path, delivery / "station-status.csv", archive]
    hashed_inputs.extend(sorted(delivery.glob("kaz-basins-tdx.*")))
    provenance = {
        "context_url": CONTEXT_URL,
        "context_license": "Natural Earth: public domain",
        "kazakhstan_boundary_attribution": {
            "source": "geoBoundaries / © OpenStreetMap contributors",
            "license": "ODbL",
            "source_year": 2017,
            "pinned_revision": "9469f09",
            "scope": "Kazakhstan land outline only; not a license for watershed data",
            "details": "See accompanying delivery boundary metadata",
        },
        "projection_wkt": MAP_CRS.to_wkt(),
        "display_simplification_metres": 150,
        "context_simplification_metres": 800,
        "counts": dict(counts),
        "unusable_coordinates_not_plotted": omitted,
        "extent_projected_metres": extent,
        "watersheds_clipped": False,
        "sha256": {str(path): file_sha256(path) for path in hashed_inputs if path.is_file()},
    }
    provenance_path.write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"outputs": [str(p) for p in outputs], "counts": dict(counts),
                      "unplotted_unusable_coordinates": omitted}, indent=2))


def file_sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--delivery", type=Path, required=True)
    parser.add_argument("--boundary", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=Path.home() / ".cache" / "pourpoint-kaz-study" / "maps")
    parser.add_argument("--overwrite", action="store_true", help="Replace map outputs after inspecting collisions")
    args = parser.parse_args()
    render(args.delivery.resolve(), args.boundary.resolve(), args.cache.resolve(), args.overwrite)


if __name__ == "__main__":
    main()
