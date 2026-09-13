"""Gauge study: source stations × country boundary × HFX → checkpointed watersheds.

This is a downstream study script, not an engine API. The only per-station
failure boundary is in delineate_stations. No coordinates or geometry are repaired.
"""
from __future__ import annotations

import argparse
from collections import Counter
import csv
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import time

import boto3
from dotenv import dotenv_values
import fiona
from pyproj import CRS, Geod, Transformer
import shapely
from shapely.geometry import MultiPolygon, Point, mapping, shape
from shapely.ops import transform, unary_union

DATASET = "s3://pourpoint-hfx/hfx/tdx-hydro-nga-20230126-global-62basin-corrected-hfx-0.3.0-d4d4c5e28df7/"
ENDPOINT = "https://fsn1.your-objectstorage.com"
REGION = "fsn1"
ENGINE_SOURCE = "c172e04a71bdca3a8eb09cd7227f90b1e38fe1f3"
BOUNDARY_URL = "https://github.com/wmgeolab/geoBoundaries/raw/9469f09/releaseData/gbOpen/KAZ/ADM0/geoBoundaries-KAZ-ADM0.geojson"
SETTINGS = dict(snap_radius=1000.0, snap_strategy="weight-first", refine=True,
                repair_geometry="auto", parquet_cache=True, parquet_cache_max_mb=512)
GEOD = Geod(ellps="WGS84")
SOURCE_FIELDS = ["name_ru", "name_en", "station_code", "latitude", "longitude",
                 "country_ru", "country_en", "location_flag"]
REPORT_FIELDS = SOURCE_FIELDS + ["status", "reason", "country_covers", "boundary_distance_m",
    "boundary_case", "requested_lon", "requested_lat", "resolved_lon", "resolved_lat",
    "snap_displacement_m", "area_km2", "terminal_unit_id", "upstream_unit_count",
    "resolution_method", "refinement_skip_reason", "refinement_seed_kind", "duration_s",
    "geometry_sha256", "bounds", "cross_border"]


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value: object) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def read_stations(path: Path) -> list[dict]:
    with path.open(encoding="utf-8-sig", newline="") as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != SOURCE_FIELDS:
            raise ValueError(f"Input schema changed: {reader.fieldnames}")
        rows = list(reader)
    codes = [r["station_code"] for r in rows]
    if len(rows) != 406 or len(set(codes)) != 406:
        raise ValueError("Input changed: expected exactly 406 unique station codes")
    if any(not code.isdecimal() for code in codes):
        raise ValueError("Station codes must be decimal strings safe for checkpoint filenames")
    return rows


def country_geometry(path: Path):
    data = json.loads(path.read_text())
    country = unary_union([shape(f["geometry"]) for f in data["features"]])
    if not country.is_valid or country.is_empty or country.geom_type not in ("Polygon", "MultiPolygon"):
        raise ValueError("Country boundary must be valid nonempty polygonal geometry")
    return country


def screen_station(row: dict, country, duplicate_coordinates: set[tuple[str, str]]) -> dict:
    result = {**row, "status": "eligible", "reason": "", "country_covers": "",
              "boundary_distance_m": "", "boundary_case": ""}
    try:
        lat, lon = float(row["latitude"]), float(row["longitude"])
    except ValueError:
        lat = lon = math.nan
    valid = math.isfinite(lat) and math.isfinite(lon) and -90 <= lat <= 90 and -180 <= lon <= 180
    reasons = []
    if not valid:
        reasons.append("invalid geographic coordinates")
    else:
        point = Point(lon, lat)
        covered = country.covers(point)
        # Boundary inclusion is exact covers, not buffering. Local AEQD distance is a diagnostic.
        projection = Transformer.from_crs("EPSG:4326",
            f"+proj=aeqd +lat_0={lat} +lon_0={lon} +datum=WGS84 +units=m", always_xy=True)
        distance = transform(projection.transform, country.boundary).distance(Point(0, 0))
        result.update(country_covers=covered, boundary_distance_m=distance,
                      boundary_case="exact" if country.boundary.covers(point) else
                                    "within_100m" if distance <= 100 else "not_near")
        if not covered:
            reasons.append("original coordinate outside chosen Kazakhstan land boundary")
        if lat == lon:
            reasons.append("suspect equal latitude and longitude")
    if row["station_code"] == "11264":
        reasons.insert(0, "explicit exclusion: 11264 Yesil at Shoptykol has copied coordinates")
    if (row["latitude"], row["longitude"]) in duplicate_coordinates:
        reasons.append("suspect coordinate shared by distinct station codes")
    if reasons:
        result.update(status="excluded", reason="; ".join(reasons))
    return result


def configure_s3(credentials: Path) -> list[str]:
    values = dotenv_values(credentials)
    secrets = []
    for key in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"):
        if not values.get(key):
            raise ValueError(f"Missing required credential variable {key}")
        os.environ[key] = values[key]
        secrets.append(values[key])
    # object_store AmazonS3Builder::from_env uses AWS_ENDPOINT. Force consistent
    # endpoint aliases and path-style requests; never inherit a different endpoint.
    os.environ.update(AWS_ENDPOINT=ENDPOINT, AWS_ENDPOINT_URL=ENDPOINT,
                      AWS_ENDPOINT_URL_S3=ENDPOINT, AWS_REGION=REGION,
                      AWS_DEFAULT_REGION=REGION, AWS_VIRTUAL_HOSTED_STYLE_REQUEST="false")
    os.environ.pop("AWS_SESSION_TOKEN", None)
    return secrets


def fetch_manifest() -> tuple[bytes, dict]:
    client = boto3.client("s3", endpoint_url=ENDPOINT, region_name=REGION)
    key = DATASET.removeprefix("s3://pourpoint-hfx/") + "manifest.json"
    response = client.get_object(Bucket="pourpoint-hfx", Key=key)
    raw = response["Body"].read()
    manifest = json.loads(raw)
    expected = dict(fabric_name="tdx_hydro", fabric_version="NGA-TDX-Hydro-20230126",
                    format_version="0.3.0", crs="EPSG:4326", topology="tree", unit_count=15936428)
    for key, value in expected.items():
        if manifest.get(key) != value:
            raise ValueError(f"Dataset identity changed: {key} expected {value}")
    schemas = [entry["schema"] for entry in manifest.get("auxiliary", [])]
    if "hfx.aux.snap.v2" not in schemas or any("d8_raster" in schema for schema in schemas):
        raise ValueError(f"Dataset auxiliary declarations changed: {schemas}")
    return raw, {"sha256": hashlib.sha256(raw).hexdigest(), "etag": response["ETag"],
                 "last_modified": response["LastModified"].isoformat(), "manifest": manifest}


def prepare(args) -> None:
    import pourpoint
    import pourpoint._pourpoint as extension
    args.cache.mkdir(parents=True, exist_ok=True)
    lock = args.cache / "provenance.json"
    if lock.exists():
        raise FileExistsError("Already prepared; use delineate/export to resume")
    rows = read_stations(args.input)
    country = country_geometry(args.boundary)
    counts = Counter((r["latitude"], r["longitude"]) for r in rows)
    duplicates = {xy for xy, count in counts.items() if count > 1}
    screening = [screen_station(r, country, duplicates) for r in rows]
    configure_s3(args.credentials)
    raw, remote = fetch_manifest()
    if not hasattr(pourpoint.DelineationResult, "refinement_skip_reason"):
        raise ValueError("Requires source-built main, not released 0.3.0")
    packages = {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()}
    provenance = dict(input_sha256=sha256(args.input), boundary_sha256=sha256(args.boundary),
        boundary_url=BOUNDARY_URL, engine_source=ENGINE_SOURCE, engine_version=pourpoint.__version__,
        extension_sha256=sha256(Path(extension.__file__)), settings=SETTINGS, dataset=DATASET,
        endpoint=ENDPOINT, region=REGION, remote=remote, python=platform.python_version(),
        packages=packages, created_utc=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
    (args.cache / "manifest.json").write_bytes(raw)
    write_json(args.cache / "screening.json", screening)
    provenance["screening_sha256"] = sha256(args.cache / "screening.json")
    write_json(lock, provenance)
    print(json.dumps(dict(screening=Counter(r["status"] for r in screening),
                          excluded=[r for r in screening if r["status"] == "excluded"])), flush=True)


def check_identity(args) -> dict:
    import pourpoint._pourpoint as extension
    provenance = json.loads((args.cache / "provenance.json").read_text())
    for key, value in dict(input_sha256=sha256(args.input), boundary_sha256=sha256(args.boundary),
                           extension_sha256=sha256(Path(extension.__file__)), settings=SETTINGS,
                           screening_sha256=sha256(args.cache / "screening.json"),
                           engine_source=ENGINE_SOURCE, dataset=DATASET).items():
        if provenance[key] != value:
            raise ValueError(f"Checkpoint identity mismatch: {key}")
    return provenance


def delineate_stations(args) -> None:
    import pourpoint
    provenance = check_identity(args)
    secrets = configure_s3(args.credentials)
    _, remote = fetch_manifest()
    if remote != provenance["remote"]:
        raise ValueError("Remote manifest changed since preparation")
    checkpoints = args.cache / "stations"
    checkpoints.mkdir(exist_ok=True)
    country = country_geometry(args.boundary)
    screening = json.loads((args.cache / "screening.json").read_text())
    os.environ["HFX_CACHE_DIR"] = str(args.cache / "hfx-metadata")
    started = time.monotonic()
    print("opening one Engine", flush=True)
    engine = pourpoint.Engine(DATASET, **SETTINGS)
    print(json.dumps(dict(engine_open_seconds=time.monotonic() - started,
                          unreadable_auxiliary_schemas=engine.unreadable_auxiliary_schemas)), flush=True)
    attempted = 0
    for row in screening:
        code = row["station_code"]
        checkpoint = checkpoints / f"{code}.json"
        if row["status"] == "excluded" or checkpoint.exists():
            continue
        started = time.monotonic()
        record = {**row, "requested_lon": float(row["longitude"]), "requested_lat": float(row["latitude"])}
        # The single named per-station isolation point includes engine calls and output validation.
        # Checkpoint I/O stays outside it: disk failures must stop rather than lose successful work.
        wkb = None
        try:
            result = engine.delineate(lat=record["requested_lat"], lon=record["requested_lon"])
            wkb = result.geometry_wkb
            geometry = shapely.from_wkb(wkb)
            if geometry.is_empty or not geometry.is_valid or geometry.geom_type not in ("Polygon", "MultiPolygon"):
                raise ValueError(f"Engine geometry is not a valid nonempty polygon: {shapely.is_valid_reason(geometry)}")
            if not math.isfinite(result.area_km2) or result.area_km2 <= 0:
                raise ValueError(f"Invalid engine area: {result.area_km2}")
            lon, lat = result.resolved_outlet
            skip = result.refinement_skip_reason
            record.update(status="successful", reason="", resolved_lon=lon, resolved_lat=lat,
                snap_displacement_m=GEOD.inv(record["requested_lon"], record["requested_lat"], lon, lat)[2],
                area_km2=result.area_km2, terminal_unit_id=str(result.terminal_unit_id),
                upstream_unit_count=len(result.upstream_unit_ids), resolution_method=result.resolution_method,
                refinement_skip_reason=skip.kind if skip is not None else "", refinement_seed_kind=result.refinement_seed_kind,
                geometry_sha256=hashlib.sha256(wkb).hexdigest(), bounds=list(geometry.bounds),
                cross_border=not country.covers(geometry))
        except Exception as error:
            message = f"{type(error).__name__}: {error}"
            for secret in secrets:
                message = message.replace(secret, "[REDACTED]")
            record.update(status="failed", reason=message)
            wkb = None
        record["duration_s"] = time.monotonic() - started
        if wkb is not None:
            temporary = checkpoints / f"{code}.wkb.tmp"
            temporary.write_bytes(wkb)
            temporary.replace(checkpoints / f"{code}.wkb")
        write_json(checkpoint, record)
        print(json.dumps({k: record.get(k) for k in ("station_code", "status", "reason", "duration_s", "area_km2", "upstream_unit_count")}), flush=True)
        attempted += 1
        if args.limit and attempted >= args.limit:
            break


def completed_records(args) -> list[dict]:
    records = []
    for row in json.loads((args.cache / "screening.json").read_text()):
        if row["status"] == "excluded":
            records.append(row)
        else:
            path = args.cache / "stations" / f"{row['station_code']}.json"
            if not path.exists():
                raise ValueError(f"Incomplete batch: no checkpoint for {row['station_code']}")
            record = json.loads(path.read_text())
            if any(record[k] != row[k] for k in SOURCE_FIELDS) or record["status"] not in ("successful", "failed"):
                raise ValueError(f"Checkpoint station identity/status mismatch: {path}")
            records.append(record)
    return records


def export(args) -> None:
    provenance = check_identity(args)
    records = completed_records(args)
    args.delivery.mkdir(parents=True, exist_ok=True)
    names = ["kaz-basins-tdx" + suffix for suffix in (".shp", ".shx", ".dbf", ".prj", ".cpg")]
    names += ["station-status.csv", "provenance.json"]
    if any((args.delivery / name).exists() for name in names):
        raise FileExistsError("Delivery collision: refusing to overwrite existing outputs")
    schema = {"geometry": "Polygon", "properties": {"station_id": "str:20", "name_en": "str:254",
               "name_ru": "str:254", "area_km2": "float:24.8", "term_id": "str:20", "n_units": "int:12"}}
    with fiona.open(args.delivery / "kaz-basins-tdx.shp", "w", driver="ESRI Shapefile",
                    crs="EPSG:4326", encoding="UTF-8", schema=schema) as output:
        for row in records:
            if row["status"] != "successful":
                continue
            for field in ("name_en", "name_ru"):
                if len(row[field].encode("utf-8")) > 254:
                    raise ValueError(f"Shapefile UTF-8 byte limit exceeded: {row['station_code']} {field}")
            path = args.cache / "stations" / f"{row['station_code']}.wkb"
            if sha256(path) != row["geometry_sha256"]:
                raise ValueError(f"Geometry checkpoint hash mismatch: {path}")
            output.write({"geometry": mapping(shapely.from_wkb(path.read_bytes())), "properties": {
                "station_id": row["station_code"], "name_en": row["name_en"], "name_ru": row["name_ru"],
                "area_km2": row["area_km2"], "term_id": row["terminal_unit_id"], "n_units": row["upstream_unit_count"]}})
    with (args.delivery / "station-status.csv").open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=REPORT_FIELDS)
        writer.writeheader()
        writer.writerows(records)
    provenance["counts"] = dict(Counter(r["status"] for r in records))
    provenance["cross_border_successes"] = sum(r.get("cross_border", False) for r in records)
    write_json(args.delivery / "provenance.json", provenance)
    print(json.dumps(provenance["counts"]), flush=True)


def exact_polygon_components(left, right) -> bool:
    """Compare exact coordinates/parts, allowing only Shapefile's single-part type coercion."""
    left = MultiPolygon([left]) if left.geom_type == "Polygon" else left
    right = MultiPolygon([right]) if right.geom_type == "Polygon" else right
    return left.normalize().equals_exact(right.normalize(), 0)


def verify(args) -> None:
    provenance = check_identity(args)
    configure_s3(args.credentials)
    _, remote = fetch_manifest()
    assert remote == provenance["remote"], "Remote manifest changed after run"
    source = read_stations(args.input)
    with (args.delivery / "station-status.csv").open(encoding="utf-8-sig", newline="") as stream:
        records = list(csv.DictReader(stream))
    assert len(records) == 406 and len({r["station_code"] for r in records}) == 406
    assert [{k: r[k] for k in SOURCE_FIELDS} for r in records] == source
    assert next(r for r in records if r["station_code"] == "11264")["status"] == "excluded"
    successes = {r["station_code"]: r for r in records if r["status"] == "successful"}
    seen = set()
    bounds = []
    with fiona.open(args.delivery / "kaz-basins-tdx.shp", encoding="UTF-8") as collection:
        assert CRS(collection.crs) == CRS("EPSG:4326")
        for feature in collection:
            props = feature["properties"]
            code = props["station_id"]
            assert code in successes and code not in seen
            seen.add(code)
            row = successes[code]
            assert props["name_ru"] == row["name_ru"] and props["name_en"] == row["name_en"]
            assert props["term_id"] == row["terminal_unit_id"]
            assert props["n_units"] == int(row["upstream_unit_count"])
            assert abs(props["area_km2"] - float(row["area_km2"])) < 1e-7
            geometry = shape(feature["geometry"])
            original = shapely.from_wkb((args.cache / "stations" / f"{code}.wkb").read_bytes())
            assert geometry.is_valid and not geometry.is_empty and geometry.geom_type in ("Polygon", "MultiPolygon")
            # Structural equality after ring/order normalization proves all vertices survive,
            # not merely that output bounds resemble a potentially clipped input.
            assert exact_polygon_components(geometry, original)
            bounds.append(geometry.bounds)
    assert seen == set(successes)
    assert (args.delivery / "kaz-basins-tdx.cpg").read_text().strip().upper() == "UTF-8"
    summary = dict(stations=len(records), counts=dict(Counter(r["status"] for r in records)),
                   shapefile_features=len(seen), valid_polygonal_features=len(seen),
                   exact_full_geometry_roundtrips=len(seen), text_and_identifiers_roundtrip=True,
                   crs="EPSG:4326", bounds=[min(b[0] for b in bounds), min(b[1] for b in bounds),
                                             max(b[2] for b in bounds), max(b[3] for b in bounds)],
                   files={p.name: dict(bytes=p.stat().st_size, sha256=sha256(p)) for p in args.delivery.iterdir() if p.is_file() and p.name != "verification.json"})
    write_json(args.delivery / "verification.json", summary)
    print(json.dumps(summary), flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "delineate", "export", "verify"))
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--boundary", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--delivery", type=Path, required=True)
    parser.add_argument("--credentials", type=Path, default=Path.home() / "secrets/pourpoint-hfx.env")
    parser.add_argument("--limit", type=int, help="Checkpoint only this many new attempts; export rejects incomplete batches")
    args = parser.parse_args()
    {"prepare": prepare, "delineate": delineate_stations, "export": export, "verify": verify}[args.action](args)


if __name__ == "__main__":
    main()
