# Independent Kazakhstan original-coordinate eligibility audit

Date: 2026-09-13. No delineation, cloud changes, checkouts, or desktop products.

## Reproduction

From repository root:

```sh
uv run --with shapely==2.1.2 --with pyproj==3.8.0 python scratchpad/kazakhstan-eligibility-audit/screen.py
```

Original CSV SHA256: `4a5a9e524d724206667fe765d52035774775a8d7c8f51b6365f7f6500d08e1ba`.
Boundary SHA256: `244f11881bcbfcfc58f74e53db89f844f033d090a9d087b888a90688561a7107`.

## Boundary provenance

geoBoundaries gbOpen KAZ ADM0 `KAZ-ADM0-12445969`.
Full-resolution valid MultiPolygon, not the simplified polygon.
Source: OpenStreetMap, Wambacher; represented year 2017; source update
2023-01-19; build 2023-12-12; pinned git commit `9469f09`.
License: Open Data Commons Open Database License 1.0 (ODbL).
Attribute geoBoundaries and OpenStreetMap contributors; retain license/source
metadata with any redistributed boundary derivative.

Pinned data: https://github.com/wmgeolab/geoBoundaries/raw/9469f09/releaseData/gbOpen/KAZ/ADM0/geoBoundaries-KAZ-ADM0.geojson

Current API used to discover pinned URL:
https://www.geoboundaries.org/api/current/gbOpen/KAZ/ADM0/

`boundary-metadata.json` records API response. `pinned-boundary-metadata.json`
records release metadata retrieved from media.githubusercontent.com at commit
9469f09. The boundary is an open land ADM0 representation, not a surveyed
legal boundary or a contemporary marine-territory polygon.

## Mechanics and result

Parse original coordinates as EPSG:4326 (longitude, latitude). Use Shapely
`country.covers(Point(longitude, latitude))`: polygon interior and exact boundary
are included; polygon holes and exterior are excluded. No buffers, snapping,
coordinate repair, or name-column eligibility inference.

406 input rows and unique station codes. 401 eligible, 5 excluded.
All 5 excluded points are outside the chosen polygon. Explicit exclusion 11264
also has equal latitude and longitude. Other outside points are 15309, 16340,
97047, 97048. Detailed distances/reasons are in `summary.json` and
`independent-station-screen.csv`.

Diagnostic distance uses a WGS84 local azimuthal equidistant projection centered
on each original gauge and distance to projected polygon boundary segments.
It does not affect eligibility. Nine inside points are within 100 m:
12001, 12701, 14032, 14043, 14136, 19009, 19021, 19201, 97046.
No point falls exactly on the supplied polygon boundary. Closest is 19009 at
2.381 m inside. Outside 15309 is 159.715 m from the boundary. The 100 m flag is
an audit threshold, not a stated accuracy of the country boundary.

## Objective location checks and uncertainty

No missing required identity/name/coordinate fields, zero coordinates,
nonfinite or out-of-global-range coordinates, or duplicate coordinate pairs.
11264 is the sole equal-latitude-longitude pair. `location_flag` is zero for
all inputs and is not treated as quality evidence. The script records whether
swapping coordinates would also land inside only as a reproducible diagnostic;
this is NOT a correction rule or evidence of a transposition. No coordinates
were changed, and no additional inside station has been established as
mislocated by these limited checks.

Boundary inclusion does not verify river/gauge placement. Outside classification
does not prove a geolocation error: 97046, 97047, 97048 are explicitly named
Caspian Sea gauges in the input, and a historical land coastline can exclude
valid marine sites. Near-boundary positions, particularly 19009 (2.381 m), are
sensitive to coordinate rounding and boundary representation. Keep classifications
literal and disclose uncertainty rather than silently widening the country or
excluding all nearby stations. This is not a comprehensive independent gauge
geolocation audit. No manual speculative replacements are proposed.
