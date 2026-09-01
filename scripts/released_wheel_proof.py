#!/usr/bin/env python3
"""proof_case : ReleasedWheel × CandidateDeclaration × ReadOnlyHostedPrefix × SelectionMode → Evidence | LoudFailure.

This module is deliberately standard-library-only.  Live mode is a controller
for an externally provisioned wheel; self-test mode exercises the guards with
generated fixtures and installs a fail-closed network opener.
"""

from __future__ import annotations

import argparse
import contextlib
import dataclasses
import enum
import hashlib
import http.client
import http.server
import io
import json
import math
import os
import platform
import re
import shutil
import struct
import subprocess
import sys
import sysconfig
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile
from pathlib import Path
from typing import Any, BinaryIO, Callable, Iterable, NoReturn, Protocol


class FailureCode(enum.IntEnum):
    CONFIG = 2
    WHEEL = 3
    OUTPUT = 4
    IDENTITY = 5
    WORKER = 6
    ZERO_READS = 7
    REQUIRE_D8 = 8
    BOUNDS = 9
    EVIDENCE = 10
    EXHAUSTED = 11
    UNRESOLVED = 12
    INTERNAL = 70


class ProofFailure(Exception):
    def __init__(self, code: FailureCode, message: str) -> None:
        super().__init__(message)
        self.code = code


class OfflineNetworkViolation(RuntimeError):
    pass


class CaseMode(enum.Enum):
    HORIZONTAL = "horizontal-boundary"
    DISTANT = "distant-region"
    NEGATIVE = "negative-flow-direction"


class CandidateRejectionCode(enum.Enum):
    """Stable reason codes for candidates rejected by the live predicate."""

    BOUNDS = "BOUNDS"
    DISTANCE = "DISTANCE"
    REQUIRE_D8 = "REQUIRE_D8"
    UNRESOLVED = "UNRESOLVED"


@dataclasses.dataclass(frozen=True)
class CandidateRejection:
    rank: int
    coordinate: tuple[float, float]
    code: CandidateRejectionCode

    def evidence(self) -> dict[str, Any]:
        return {"coordinate": list(self.coordinate), "rank": self.rank,
                "rejection_code": self.code.value}


class CandidateExhaustion(ProofFailure):
    """Typed exhaustion failure carrying the complete rejection ledger."""

    def __init__(self, ledger: list[dict[str, Any]]) -> None:
        self.ledger = tuple(ledger)
        super().__init__(
            FailureCode.EXHAUSTED,
            "deterministic candidate budget exhausted; rejection ledger="
            + json.dumps(ledger, sort_keys=True, separators=(",", ":")),
        )


class CandidateDiagnostics:
    """One rejection reason for every ordered candidate that was not accepted."""

    def __init__(self) -> None:
        self._attempts: list[tuple[int, tuple[float, float]]] = []
        self._rejections: list[CandidateRejection] = []
        self._open: tuple[int, tuple[float, float]] | None = None
        self._accepted = False

    def start(self, rank: int, coordinate_value: tuple[float, float]) -> None:
        if (self._accepted or self._open is not None
                or rank != len(self._attempts) + 1 or not coordinate(coordinate_value)):
            fail(FailureCode.EVIDENCE, "candidate diagnostic ordering differs")
        coordinate_pair = (float(coordinate_value[0]), float(coordinate_value[1]))
        self._attempts.append((rank, coordinate_pair))
        self._open = (rank, coordinate_pair)

    def reject(self, code: CandidateRejectionCode) -> None:
        if self._open is None:
            fail(FailureCode.EVIDENCE, "candidate rejection has no matching attempt")
        rank, coordinate_value = self._open
        self._rejections.append(CandidateRejection(rank, coordinate_value, code))
        self._open = None

    def accept(self) -> None:
        if self._open is None:
            fail(FailureCode.EVIDENCE, "candidate acceptance has no matching attempt")
        self._open = None
        self._accepted = True

    @property
    def attempted_count(self) -> int:
        return len(self._attempts)

    def evidence(self) -> list[dict[str, Any]]:
        rejected = [(item.rank, item.coordinate) for item in self._rejections]
        expected = self._attempts[:-1] if self._accepted else self._attempts
        if rejected != expected:
            fail(FailureCode.EVIDENCE, "candidate dropped without a recorded rejection reason")
        return [item.evidence() for item in self._rejections]

    def exhausted(self) -> NoReturn:
        ledger = self.evidence()
        if self._open is not None or self._accepted:
            fail(FailureCode.EVIDENCE, "candidate exhaustion state differs")
        raise CandidateExhaustion(ledger)


def candidate_rejection_code(failure: ProofFailure,
                             case: CaseMode) -> CandidateRejectionCode | None:
    if case is CaseMode.NEGATIVE:
        return None
    return {
        FailureCode.BOUNDS: CandidateRejectionCode.BOUNDS,
        FailureCode.REQUIRE_D8: CandidateRejectionCode.REQUIRE_D8,
        FailureCode.UNRESOLVED: CandidateRejectionCode.UNRESOLVED,
    }.get(failure.code)


@dataclasses.dataclass(frozen=True)
class ObjectIdentity:
    content_length: int
    etag: str | None = None
    sha256: str | None = None


@dataclasses.dataclass(frozen=True)
class ClosedByteRange:
    start: int
    end_inclusive: int

    @property
    def length(self) -> int:
        return self.end_inclusive - self.start + 1


@dataclasses.dataclass(frozen=True)
class CompletedRead:
    seq: int
    key: str
    byte_range: ClosedByteRange | None
    bytes_received: int


@dataclasses.dataclass(frozen=True)
class CeilingObservation:
    observed: int
    ceiling: int

    def evidence(self) -> dict[str, int]:
        margin = self.ceiling - self.observed
        if margin < 1:
            fail(FailureCode.BOUNDS, "ceiling requires positive margin")
        return {"ceiling": self.ceiling, "margin": margin, "observed": self.observed}


def fail(code: FailureCode, message: str) -> NoReturn:
    raise ProofFailure(code, message)


HOSTED_BASE = "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"
AUTHORIZATION = "I_AUTHORIZE_READ_ONLY_BOUNDED_NETWORK_V1"
AUTH_ENV = "POURPOINT_LIVE_READ_AUTHORIZATION"
WHEEL_ENV = "POURPOINT_RELEASE_WHEEL"
NETWORK_DENIED_MARKER = "POURPOINT_PROOF_NETWORK_DENIED"
AWS_NAMES = (
    "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "AWS_PROFILE",
    "AWS_SHARED_CREDENTIALS_FILE", "AWS_CONFIG_FILE", "AWS_WEB_IDENTITY_TOKEN_FILE",
    "AWS_ROLE_ARN", "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
)

WHEEL_ALLOWLIST = {
    "pourpoint-0.3.0-cp39-abi3-macosx_11_0_arm64.whl": (22310060, "a79ebc38be0cdc39247fd07eb608750536c982999954bd68e3ccf5599fefdabe"),
    "pourpoint-0.3.0-cp39-abi3-macosx_11_0_x86_64.whl": (23453105, "3f5638a6a7f921bc850e3e3a43c74746a9b0770fdae84767dab8d95e34f01327"),
    "pourpoint-0.3.0-cp39-abi3-manylinux_2_28_aarch64.whl": (22316138, "3d5ce30a9f9b0cbdd0a595cdb269c891bd59e300549506e4f28b3d21059caa8d"),
    "pourpoint-0.3.0-cp39-abi3-manylinux_2_28_x86_64.whl": (23329548, "1ecf0ea1ae684935a9ba7114f1c5c4960f26c551fc0f2c286afbd966c9230b87"),
    "pourpoint-0.3.0-cp39-abi3-win_amd64.whl": (48735137, "205977f0a86b632b0f15b2bf314f2acc4149c7579f5db73c41e8ade3ac5dedd0"),
    "pourpoint-0.3.0.tar.gz": (739756, "7393788dfcce173a5fd0944a04addb175eb5018ce83d94b45ca59b187c6ae7ac"),
}

FORMER_MANIFEST = ObjectIdentity(1132, sha256="0935a7bc09b7c2636786082fd9fd9a669ea1b32c6e2e4d92cb3f8da531c083c4")
PUBLISHED_MANIFEST = ObjectIdentity(1426, sha256="02339ff92cbfd1d2ea57bb5332cb843b98115cd7a7395f64c14fac78d2ed643c")
HOSTED_USER_AGENT = "pourpoint-released-wheel-proof/0.3.0"
COG_IDENTITIES = {
    "flow_dir": ObjectIdentity(50686516478, '"bc48d1013cf6908fb44c325dd2ad10ab-1511"'),
    "flow_acc": ObjectIdentity(205069870081, '"49eab3942a26036aa49e72ea33a1b724-6112"'),
}
HISTORICAL_SHA256 = {
    "flow_dir": "eace32b63c4bc09e8172f03cce6dacfbf09a86c6b51c42b50c6cccd498d4d656",
    "flow_acc": "30f16ba3238085289d87e72f3386fa152da7e9b56063f5d610422d20a79fc98b",
}

MAX_PLANNED_TILE_COUNT = 65536
MAX_COMPRESSED_CHUNK_BYTES = 16777216
MAX_COVERED_CHUNK_BYTES = 1073741824
MAX_DECODED_CHUNK_BYTES = 8388608
MAX_WINDOW_ALLOCATION_BYTES = 1073741824
CANDIDATE_BUDGET = 128
TILE_SIZE = 512
BAND_HALF_WIDTH_PIXELS = 32
BAND_HALF_LENGTH_PIXELS = 4096
ZURICH_SEED = (8.5417, 47.3769)
DISTANT_DISCOVERY_SEED = (10.405, 63.44)
# Compatibility name consumed by the released-evidence verifier.
REPPARFJORD_SEED = DISTANT_DISCOVERY_SEED
DISTANT_SEED_DECLARATION = (Path(__file__).resolve().parent.parent
                            / "docs/evidence/grit-d8-live/prepublication/distant-seed.json")
DISTANT_SEED_SCHEMA = "pourpoint.distant-discovery-seed.v1"

FORMER_MANIFEST_BYTES = b'''{
  "format_version": "0.3.0",
  "fabric_name": "grit",
  "fabric_version": "1.0.0",
  "crs": "EPSG:4326",
  "has_up_area": true,
  "topology": "dag",
  "bbox": [
    -180.0,
    -90.0,
    180.0,
    90.0
  ],
  "unit_count": 22337300,
  "created_at": "2026-06-29T20:08:11Z",
  "adapter_version": "grit-global-2.0.0",
  "auxiliary": [
    {
      "schema": "hfx.aux.snap.v2",
      "artifacts": {
        "snap": "aux/snap_segments.parquet"
      },
      "metadata": {
        "name": "segment-stems",
        "description": "Segment-scale stems for level 0 GRIT segment catchments.",
        "references_levels": [
          0
        ],
        "weight_semantics": "drainage_area_km2_partitioned"
      }
    },
    {
      "schema": "hfx.aux.snap.v2",
      "artifacts": {
        "snap": "aux/snap_reaches.parquet"
      },
      "metadata": {
        "name": "reach-stems",
        "description": "Reach-scale stems for level 1 GRIT reach catchments. Weight inherited from parent segment.",
        "references_levels": [
          1
        ],
        "weight_semantics": "drainage_area_km2_partitioned"
      }
    }
  ]
}
'''


POSITIVE_CANDIDATE = b'''{
  "format_version": "0.3.0",
  "fabric_name": "grit",
  "fabric_version": "1.0.0",
  "crs": "EPSG:4326",
  "has_up_area": true,
  "topology": "dag",
  "bbox": [
    -180.0,
    -90.0,
    180.0,
    90.0
  ],
  "unit_count": 22337300,
  "created_at": "2026-07-21T21:05:12Z",
  "adapter_version": "grit-global-2.1.0",
  "auxiliary": [
    {
      "schema": "hfx.aux.snap.v2",
      "artifacts": {
        "snap": "aux/snap_segments.parquet"
      },
      "metadata": {
        "name": "segment-stems",
        "description": "Segment-scale stems for level 0 GRIT segment catchments.",
        "references_levels": [
          0
        ],
        "weight_semantics": "drainage_area_km2_partitioned"
      }
    },
    {
      "schema": "hfx.aux.snap.v2",
      "artifacts": {
        "snap": "aux/snap_reaches.parquet"
      },
      "metadata": {
        "name": "reach-stems",
        "description": "Reach-scale stems for level 1 GRIT reach catchments. Weight inherited from parent segment.",
        "references_levels": [
          1
        ],
        "weight_semantics": "drainage_area_km2_partitioned"
      }
    },
    {
      "schema": "hfx.aux.d8_raster.v2",
      "artifacts": {
        "flow_dir": "aux/d8/flow_dir.tif",
        "flow_acc": "aux/d8/flow_acc.tif"
      },
      "metadata": {
        "crs": "EPSG:8857",
        "flow_dir_encoding": "grass",
        "flow_acc_units": "km2"
      }
    }
  ]
}
'''
POSITIVE_SHA256 = "02339ff92cbfd1d2ea57bb5332cb843b98115cd7a7395f64c14fac78d2ed643c"


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()


def _pairs_no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            fail(FailureCode.EVIDENCE, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def strict_json_bytes(data: bytes, *, canonical: bool = False) -> Any:
    try:
        text = data.decode("utf-8")
        value = json.loads(text, object_pairs_hook=_pairs_no_duplicates,
                           parse_constant=lambda token: fail(FailureCode.EVIDENCE, f"non-finite JSON number: {token}"))
    except ProofFailure:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(FailureCode.EVIDENCE, f"malformed JSON: {exc}")
    if canonical and canonical_json(value) != data:
        fail(FailureCode.EVIDENCE, "JSON is not canonical")
    return value


def candidate_difference(left: Any, right: Any, pointer: str = "") -> list[dict[str, Any]]:
    if type(left) is not type(right):
        return [{"new": right, "old": left, "pointer": pointer or "/"}]
    if isinstance(left, dict):
        differences: list[dict[str, Any]] = []
        for key in sorted(set(left) | set(right)):
            escaped = key.replace("~", "~0").replace("/", "~1")
            child = f"{pointer}/{escaped}"
            if key not in left:
                differences.append({"new": right[key], "old": None, "pointer": child})
            elif key not in right:
                differences.append({"new": None, "old": left[key], "pointer": child})
            else:
                differences.extend(candidate_difference(left[key], right[key], child))
        return differences
    if isinstance(left, list):
        differences = []
        for index in range(max(len(left), len(right))):
            child = f"{pointer}/{index}"
            if index >= len(left):
                differences.append({"new": right[index], "old": None, "pointer": child})
            elif index >= len(right):
                differences.append({"new": None, "old": left[index], "pointer": child})
            else:
                differences.extend(candidate_difference(left[index], right[index], child))
        return differences
    return [] if left == right else [{"new": right, "old": left, "pointer": pointer or "/"}]


def verify_positive_candidate(data: bytes) -> dict[str, Any]:
    if len(data) != 1426 or sha256_bytes(data) != POSITIVE_SHA256 or data != POSITIVE_CANDIDATE:
        fail(FailureCode.IDENTITY, "candidate bytes or digest differ")
    return strict_json_bytes(data)


POSITIVE_VALUE = verify_positive_candidate(POSITIVE_CANDIDATE)
NEGATIVE_VALUE = json.loads(json.dumps(POSITIVE_VALUE))
NEGATIVE_VALUE["auxiliary"][2]["metadata"]["flow_dir_encoding"] = "esri"
NEGATIVE_CANDIDATE = (json.dumps(NEGATIVE_VALUE, indent=2) + "\n").encode()
EXPECTED_NEGATIVE_DIFFERENCE = [{"new": "esri", "old": "grass", "pointer": "/auxiliary/2/metadata/flow_dir_encoding"}]


def verify_negative_candidate(data: bytes) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    try:
        value = strict_json_bytes(data)
    except ProofFailure as exc:
        fail(FailureCode.IDENTITY, str(exc))
    difference = candidate_difference(POSITIVE_VALUE, value)
    if difference != EXPECTED_NEGATIVE_DIFFERENCE:
        fail(FailureCode.IDENTITY, "negative declaration difference is not the exact singleton")
    expected = (json.dumps(value, indent=2) + "\n").encode()
    if data != expected:
        fail(FailureCode.IDENTITY, "negative declaration serialization differs")
    return value, difference


verify_negative_candidate(NEGATIVE_CANDIDATE)


class _RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req: Any, fp: Any, code: int, msg: str,
                         headers: Any, newurl: str) -> None:
        return None


_network_opener: Callable[..., Any] = urllib.request.build_opener(_RejectRedirects).open


def set_network_opener(opener: Callable[..., Any]) -> None:
    global _network_opener
    _network_opener = opener


class DeniedOpener:
    def __init__(self) -> None:
        self.calls = 0

    def __call__(self, *_args: Any, **_kwargs: Any) -> Any:
        self.calls += 1
        raise OfflineNetworkViolation("network opener is unreachable offline")


def validate_live_environment(output_dir: Path, env: dict[str, str]) -> Path:
    if env.get(AUTH_ENV) != AUTHORIZATION:
        fail(FailureCode.CONFIG, f"{AUTH_ENV} must equal the explicit read-only authorization token")
    present = [name for name in AWS_NAMES if env.get(name)]
    if present:
        fail(FailureCode.CONFIG, f"AWS credential environment is forbidden: {present[0]}")
    wheel_text = env.get(WHEEL_ENV, "")
    if not wheel_text:
        fail(FailureCode.CONFIG, f"{WHEEL_ENV} is required")
    wheel = Path(wheel_text)
    if not output_dir.is_absolute():
        fail(FailureCode.OUTPUT, "output path must be absolute")
    if output_dir.exists():
        fail(FailureCode.OUTPUT, "output path must be absent")
    return wheel


def expected_wheel_for_host() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    plat = sysconfig.get_platform().lower()
    if system == "Darwin" and machine == "arm64":
        return "pourpoint-0.3.0-cp39-abi3-macosx_11_0_arm64.whl"
    if system == "Darwin" and machine in {"x86_64", "amd64"}:
        return "pourpoint-0.3.0-cp39-abi3-macosx_11_0_x86_64.whl"
    if system == "Linux" and machine in {"aarch64", "arm64"} and "linux" in plat:
        return "pourpoint-0.3.0-cp39-abi3-manylinux_2_28_aarch64.whl"
    if system == "Linux" and machine in {"x86_64", "amd64"} and "linux" in plat:
        return "pourpoint-0.3.0-cp39-abi3-manylinux_2_28_x86_64.whl"
    if system == "Windows" and machine in {"amd64", "x86_64"}:
        return "pourpoint-0.3.0-cp39-abi3-win_amd64.whl"
    fail(FailureCode.WHEEL, f"unsupported wheel platform: {system}/{machine}/{plat}")


def parse_metadata(data: bytes) -> dict[str, str]:
    try:
        lines = data.decode("utf-8").splitlines()
    except UnicodeDecodeError as exc:
        fail(FailureCode.WHEEL, f"wheel METADATA is not UTF-8: {exc}")
    result: dict[str, str] = {}
    for line in lines:
        if ":" in line:
            key, value = line.split(":", 1)
            if key in {"Name", "Version", "Requires-Python"}:
                if key in result:
                    fail(FailureCode.WHEEL, f"duplicate METADATA field: {key}")
                result[key] = value.strip()
    if result != {"Name": "pourpoint", "Version": "0.3.0", "Requires-Python": ">=3.9"}:
        fail(FailureCode.WHEEL, "wheel METADATA identity differs")
    return result


def verify_wheel(path: Path) -> dict[str, Any]:
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or path.suffix != ".whl":
        fail(FailureCode.WHEEL, "wheel must be an absolute, regular, non-symlink .whl file")
    expected_name = expected_wheel_for_host()
    if path.name != expected_name:
        fail(FailureCode.WHEEL, f"wheel tag does not match host; expected {expected_name}")
    allowed = WHEEL_ALLOWLIST.get(path.name)
    if allowed is None:
        fail(FailureCode.WHEEL, "wheel basename is not allowlisted")
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                size += len(chunk)
                digest.update(chunk)
    except OSError as exc:
        fail(FailureCode.WHEEL, f"cannot read wheel: {exc}")
    if (size, digest.hexdigest()) != allowed:
        fail(FailureCode.WHEEL, "wheel byte count or SHA-256 identity differs")
    try:
        with zipfile.ZipFile(path) as archive:
            names = archive.namelist()
            for name in names:
                pure = Path(name)
                if pure.is_absolute() or ".." in pure.parts or "\\" in name:
                    fail(FailureCode.WHEEL, "wheel contains a path traversal member")
            metadata_names = [name for name in names if re.fullmatch(r"[^/]+\.dist-info/METADATA", name)]
            if len(metadata_names) != 1:
                fail(FailureCode.WHEEL, "wheel must contain exactly one dist-info/METADATA")
            metadata = parse_metadata(archive.read(metadata_names[0]))
    except ProofFailure:
        raise
    except (OSError, zipfile.BadZipFile, KeyError) as exc:
        fail(FailureCode.WHEEL, f"malformed wheel ZIP: {exc}")
    return {"filename": path.name, "metadata_name": metadata["Name"],
            "metadata_requires_python": metadata["Requires-Python"],
            "metadata_version": metadata["Version"], "sha256": allowed[1], "size_bytes": allowed[0]}


def parse_closed_range(header: str | None, size: int) -> ClosedByteRange:
    if header is None or not header.startswith("bytes="):
        fail(FailureCode.BOUNDS, "COG GET requires a closed Range")
    value = header[6:]
    if "," in value or value.startswith("-") or value.endswith("-"):
        fail(FailureCode.BOUNDS, "COG range must be one closed non-suffix range")
    match = re.fullmatch(r"([0-9]+)-([0-9]+)", value)
    if match is None:
        fail(FailureCode.BOUNDS, "malformed COG range")
    start, end = map(int, match.groups())
    if start > end or end >= size or (start == 0 and end == size - 1):
        fail(FailureCode.BOUNDS, "COG range is invalid or full-object")
    byte_range = ClosedByteRange(start, end)
    if byte_range.length >= MAX_COMPRESSED_CHUNK_BYTES:
        fail(FailureCode.BOUNDS, "single compressed chunk lacks positive ceiling margin")
    return byte_range


def observe_ceiling(observed: int, ceiling: int) -> CeilingObservation:
    observation = CeilingObservation(observed, ceiling)
    observation.evidence()
    return observation


def validate_require_d8(status: Any, refined_outlet: Any) -> None:
    if status != "applied":
        fail(FailureCode.REQUIRE_D8, f"RequireD8 refinement status was {status!r}")
    if not coordinate(refined_outlet):
        fail(FailureCode.REQUIRE_D8, "RequireD8 applied result has no finite refined outlet")


def coordinate(value: Any) -> bool:
    return (isinstance(value, (list, tuple)) and len(value) == 2
            and all(type(item) in {int, float} and math.isfinite(item) for item in value)
            and -180 <= value[0] <= 180 and -90 <= value[1] <= 90)


def spherical_distance_metres(left: tuple[float, float] | list[float], right: tuple[float, float] | list[float]) -> float:
    lon1, lat1 = left
    lon2, lat2 = right
    phi1, phi2 = math.radians(lat1), math.radians(lat2)
    delta_phi = phi2 - phi1
    delta_lambda = math.radians(((lon2 - lon1 + 180) % 360) - 180)
    a = math.sin(delta_phi / 2) ** 2 + math.cos(phi1) * math.cos(phi2) * math.sin(delta_lambda / 2) ** 2
    return 2 * 6371008.8 * math.atan2(math.sqrt(a), math.sqrt(1 - a))


def require_distant(horizontal: tuple[float, float], candidate: tuple[float, float], opener_calls: Callable[[], int] | None = None) -> float:
    distance = spherical_distance_metres(horizontal, candidate)
    if distance < 1_000_000:
        if opener_calls is not None and opener_calls() != 0:
            fail(FailureCode.EXHAUSTED, "distant predicate spent a COG read before rejection")
        fail(FailureCode.EXHAUSTED, "distant candidate is below 1000000 metres")
    return distance


# ---- Canonical WKB v1 ----------------------------------------------------

def _read_u32(data: bytes, offset: int, order: str) -> tuple[int, int]:
    if offset + 4 > len(data):
        fail(FailureCode.EVIDENCE, "truncated WKB")
    return struct.unpack_from(order + "I", data, offset)[0], offset + 4


def _read_geometry(data: bytes, offset: int = 0) -> tuple[list[list[list[tuple[float, float]]]], int]:
    if offset >= len(data) or data[offset] not in (0, 1):
        fail(FailureCode.EVIDENCE, "invalid WKB byte order")
    order = "<" if data[offset] == 1 else ">"
    kind, offset = _read_u32(data, offset + 1, order)
    if kind == 3:
        ring_count, offset = _read_u32(data, offset, order)
        polygon: list[list[tuple[float, float]]] = []
        for _ in range(ring_count):
            point_count, offset = _read_u32(data, offset, order)
            if point_count > 10_000_000 or offset + point_count * 16 > len(data):
                fail(FailureCode.EVIDENCE, "invalid WKB ring size")
            ring = [struct.unpack_from(order + "dd", data, offset + index * 16) for index in range(point_count)]
            if any(not math.isfinite(v) for point in ring for v in point):
                fail(FailureCode.EVIDENCE, "non-finite WKB coordinate")
            offset += point_count * 16
            polygon.append(ring)
        return [polygon], offset
    if kind == 6:
        count, offset = _read_u32(data, offset, order)
        polygons: list[list[list[tuple[float, float]]]] = []
        for _ in range(count):
            nested, offset = _read_geometry(data, offset)
            if len(nested) != 1:
                fail(FailureCode.EVIDENCE, "MultiPolygon child is not Polygon")
            polygons.extend(nested)
        return polygons, offset
    fail(FailureCode.EVIDENCE, "only 2D Polygon/MultiPolygon WKB is accepted")


def _signed_area(ring: list[tuple[int, int]]) -> int:
    return sum(x1 * y2 - x2 * y1 for (x1, y1), (x2, y2) in zip(ring, ring[1:] + ring[:1]))


def _normalize_ring(ring: list[tuple[float, float]], exterior: bool) -> list[tuple[int, int]]:
    rounded = [(round(x * 1_000_000), round(y * 1_000_000)) for x, y in ring]
    if len(rounded) > 1 and rounded[0] == rounded[-1]:
        rounded.pop()
    area = _signed_area(rounded)
    if (exterior and area < 0) or (not exterior and area > 0):
        rounded.reverse()
    if rounded:
        variants = [rounded[index:] + rounded[:index] for index in range(len(rounded))]
        rounded = min(variants)
        rounded.append(rounded[0])
    return rounded


def _ring_key(ring: list[tuple[int, int]]) -> tuple[Any, ...]:
    opened = ring[:-1] if ring else ring
    bbox = (min((x for x, _ in opened), default=2**63-1), min((y for _, y in opened), default=2**63-1),
            max((x for x, _ in opened), default=-(2**63)), max((y for _, y in opened), default=-(2**63)))
    return bbox, _signed_area(opened), opened


def canonical_wkb(data: bytes) -> bytes:
    polygons, consumed = _read_geometry(data)
    if consumed != len(data):
        fail(FailureCode.EVIDENCE, "trailing WKB bytes")
    normalized = []
    for polygon in polygons:
        if not polygon:
            fail(FailureCode.EVIDENCE, "polygon has no exterior ring")
        exterior = _normalize_ring(polygon[0], True)
        holes = sorted((_normalize_ring(ring, False) for ring in polygon[1:]), key=_ring_key)
        net_area = abs(_signed_area(exterior[:-1])) - sum(abs(_signed_area(hole[:-1])) for hole in holes)
        normalized.append((exterior, holes, net_area))
    normalized.sort(key=lambda item: (_ring_key(item[0])[0], item[2], len(item[1]), item[0][:-1], tuple(_ring_key(hole) for hole in item[1])))
    output = bytearray(struct.pack("<BII", 1, 6, len(normalized)))
    for exterior, holes, _ in normalized:
        output.extend(struct.pack("<BII", 1, 3, 1 + len(holes)))
        for ring in [exterior, *holes]:
            output.extend(struct.pack("<I", len(ring)))
            for x, y in ring:
                output.extend(struct.pack("<dd", x / 1_000_000, y / 1_000_000))
    return bytes(output)


def validate_canonical_wkb(data: bytes, digest: str, precision: int = 6) -> None:
    if precision != 6 or sha256_bytes(data) != digest:
        fail(FailureCode.EVIDENCE, "canonical WKB digest or precision differs")
    if canonical_wkb(data) != data or canonical_wkb(canonical_wkb(data)) != data:
        fail(FailureCode.EVIDENCE, "canonical WKB is not idempotent")


def simple_multipolygon(polygons: list[list[list[tuple[float, float]]]]) -> bytes:
    output = bytearray(struct.pack("<BII", 1, 6, len(polygons)))
    for polygon in polygons:
        output.extend(struct.pack("<BII", 1, 3, len(polygon)))
        for ring in polygon:
            output.extend(struct.pack("<I", len(ring)))
            for point in ring:
                output.extend(struct.pack("<dd", *point))
    return bytes(output)


def validate_trace(records: list[dict[str, Any]], cache_root: Path, _invocation_id: str) -> tuple[int, int]:
    allowed = {"bytes", "cache_status", "duration_ms", "kind", "matches", "path", "requests", "row_groups", "rows", "stage", "thread", "timestamp"}
    wanted: dict[str, list[int]] = {"raster_localize_flow_dir": [], "raster_localize_flow_acc": []}
    for line_number, record in enumerate(records, 1):
        if (set(record) - allowed or record.get("kind") != "stage"
                or type(record.get("timestamp")) is not int
                or not isinstance(record.get("thread"), str) or not record["thread"]
                or not isinstance(record.get("stage"), str) or not record["stage"]):
            fail(FailureCode.EVIDENCE, "invalid trace key shape")
        duration = record.get("duration_ms")
        if type(duration) not in {int, float} or not math.isfinite(duration) or duration < 0:
            fail(FailureCode.EVIDENCE, "invalid trace duration")
        stage = record.get("stage")
        if stage in wanted:
            if record.get("cache_status") != "fetched":
                fail(FailureCode.REQUIRE_D8, "production localization was not fetched")
            path = Path(record.get("path", ""))
            try:
                path.resolve().relative_to(cache_root.resolve())
            except (OSError, ValueError):
                fail(FailureCode.BOUNDS, "production trace path escapes case cache")
            wanted[stage].append(line_number)
    if any(len(lines) != 1 for lines in wanted.values()):
        fail(FailureCode.REQUIRE_D8, "production localization trace pair is absent or ambiguous")
    return wanted["raster_localize_flow_dir"][0], wanted["raster_localize_flow_acc"][0]


def validate_negative_discriminator(positive: dict[str, Any], negative: dict[str, Any]) -> None:
    exact_keys = ("input", "terminal", "upstream", "status")
    coordinate_keys = ("resolved", "refined")
    coordinates_equal = all(
        coordinate(positive.get(key)) and coordinate(negative.get(key))
        and all(abs(float(left) - float(right)) <= 0.000001
                for left, right in zip(positive[key], negative[key]))
        for key in coordinate_keys)
    if any(positive.get(key) != negative.get(key) for key in exact_keys) or not coordinates_equal:
        fail(FailureCode.EVIDENCE, "negative invariant fields differ")
    if positive.get("geometry_sha256") == negative.get("geometry_sha256"):
        fail(FailureCode.EXHAUSTED, "negative flow-direction geometry did not differ")


def validate_completed_reads(lines: bytes) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for number, raw in enumerate(lines.splitlines(keepends=True), 1):
        if not raw.endswith(b"\n"):
            fail(FailureCode.EVIDENCE, "truncated reads JSONL")
        value = strict_json_bytes(raw, canonical=True)
        required = {"bytes_received", "case_id", "completed", "error", "etag", "key", "method", "origin", "range", "response_content_length", "response_content_range", "seq", "status", "url"}
        if set(value) != required or value["seq"] != number or type(value["completed"]) is not bool:
            fail(FailureCode.EVIDENCE, "reads transcript shape or sequence differs")
        if value["origin"] == "hosted" and value["key"].endswith(".tif") and value["method"] == "GET":
            kind = _cog_kind(value["key"])
            if kind is None or value["url"] != HOSTED_BASE + value["key"]:
                fail(FailureCode.IDENTITY, "completed COG transcript target differs")
            identity = COG_IDENTITIES[kind]
            if (not value["completed"] or value["status"] != 206 or not isinstance(value["range"], dict)
                    or value["etag"] != identity.etag):
                fail(FailureCode.BOUNDS, "completed COG transcript identity differs")
            start, end = value["range"].get("start"), value["range"].get("end_exclusive")
            if (type(start) is not int or type(end) is not int or start < 0 or end <= start
                    or end > identity.content_length or value["bytes_received"] != end - start
                    or value["response_content_length"] != end - start):
                fail(FailureCode.BOUNDS, "completed COG transcript byte count differs")
            expected = f"bytes {start}-{end - 1}/{identity.content_length}"
            if value["response_content_range"] != expected:
                fail(FailureCode.BOUNDS, "completed COG Content-Range differs")
        records.append(value)
    if not records:
        fail(FailureCode.EVIDENCE, "reads transcript is empty")
    return records


RETAINED = ["evidence.json", "flow-acc.window.tif", "flow-dir.window.tif",
            "geometry.canonical.wkb", "install.stderr.txt", "install.stdout.txt",
            "reads.jsonl", "served-manifest.json", "trace.jsonl", "worker.stderr.txt",
            "worker.stdout.txt"]


def build_artifact_index(root: Path) -> dict[str, Any]:
    artifacts = []
    for name in RETAINED:
        path = root / name
        data = path.read_bytes()
        artifacts.append({"path": name, "sha256": sha256_bytes(data), "size_bytes": len(data)})
    return {"artifacts": artifacts, "schema": "pourpoint.released-wheel-proof-artifact-index.v1"}


def verify_artifact_directory(root: Path) -> None:
    expected = set(RETAINED) | {"artifact-index.json"}
    try:
        entries = list(root.iterdir())
    except OSError as exc:
        fail(FailureCode.EVIDENCE, f"cannot read artifact directory: {exc}")
    if {item.name for item in entries} != expected or any(item.is_symlink() or not item.is_file() for item in entries):
        fail(FailureCode.EVIDENCE, "retained artifact path set differs or contains symlink")
    index_data = (root / "artifact-index.json").read_bytes()
    index = strict_json_bytes(index_data, canonical=True)
    if index != build_artifact_index(root):
        fail(FailureCode.EVIDENCE, "artifact index digest, size, or ordering differs")
    if (root / "geometry.canonical.wkb").stat().st_size <= 0:
        fail(FailureCode.EVIDENCE, "canonical geometry is empty")
    validate_completed_reads((root / "reads.jsonl").read_bytes())


def live_preflight_for_test(output: Path, env: dict[str, str], completed_reads: int | None = None) -> None:
    validate_live_environment(output, env)
    if completed_reads is not None and completed_reads == 0:
        fail(FailureCode.ZERO_READS, "authorization present but zero hosted network operations completed")


def _assert_rejected(action: Callable[[], Any], code: FailureCode, contains: str = "") -> None:
    rejection_observed = False
    try:
        action()
    except ProofFailure as exc:
        if exc.code != code or contains not in str(exc):
            raise AssertionError(f"wrong rejection: {exc.code}: {exc}") from exc
        rejection_observed = True
    assert rejection_observed is True


def _synthetic_worker_read_attempt(cache_root: Path) -> WorkerAttempt:
    records = [
        {"bytes": 0, "cache_status": "fetched", "duration_ms": 1.0,
         "kind": "stage", "path": str(cache_root / f"{stage}.tif"),
         "requests": 0, "stage": stage, "thread": "ThreadId(1)",
         "timestamp": 1}
        for stage in ("raster_localize_flow_dir", "raster_localize_flow_acc")
    ]
    return WorkerAttempt({}, [], b"", b"",
                         b"".join(canonical_json(record) for record in records),
                         cache_root)


def self_test_worker_network_reads() -> None:
    manifest = canonical_json({"auxiliary": []})
    replay = ReplayTransport(manifest)
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    identity = ObjectIdentity(len(manifest), sha256=sha256_bytes(manifest))
    preflight_hosted(proxy, identity)
    before_worker = proxy.completed_hosted_reads
    assert before_worker == 3
    with tempfile.TemporaryDirectory() as temporary_text:
        cache = Path(temporary_text) / "hfx-cache" / "attempt-1"
        attempt = _synthetic_worker_read_attempt(cache)
        _assert_rejected(
            lambda: require_completed_worker_reads(proxy, before_worker, attempt),
            FailureCode.ZERO_READS,
            "zero hosted network operations completed by released worker")
    assert len(replay.calls) == 3
    print("PASS: preflight reads cannot satisfy the released-worker read guard")


def self_test_live_hosted_source() -> None:
    for case in CaseMode:
        assert require_hosted_worker_source(HOSTED_BASE) == HOSTED_BASE
        payload = worker_input_payload(case, (8.0, 47.0), "0" * 32, HOSTED_BASE)
        assert payload["dataset_url"] == HOSTED_BASE
        assert "proxy_dataset_url" not in payload
    loopback = "http://127.0.0.1:1234/grit/hfx-v0.3.0/"
    _assert_rejected(
        lambda: worker_input_payload(
            CaseMode.HORIZONTAL, (8.0, 47.0), "0" * 32, loopback),
        FailureCode.CONFIG,
        "exact hosted base",
    )
    # Source validation is the first worker action.  These paths need not exist,
    # which demonstrates that rejection happens before a subprocess can launch.
    _assert_rejected(
        lambda: run_worker(Path("absent"), Path("absent"), loopback,
                           CaseMode.HORIZONTAL, (8.0, 47.0), 1),
        FailureCode.CONFIG,
        "exact hosted base",
    )
    print("PASS: live released-wheel inputs use only the exact hosted dataset base")


def self_test_live_hosted_worker_read_gate() -> None:
    manifest = canonical_json({"auxiliary": []})
    replay = ReplayTransport(manifest)
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    identity = ObjectIdentity(len(manifest), sha256=sha256_bytes(manifest))
    preflight_hosted(proxy, identity)
    before_worker = proxy.completed_hosted_reads
    with tempfile.TemporaryDirectory() as temporary_text:
        cache = Path(temporary_text) / "hfx-cache" / "attempt-1"
        cache.mkdir(parents=True)
        records = [
            {"bytes": 10, "cache_status": "fetched", "duration_ms": 1.0,
             "kind": "stage", "path": str(cache / f"{stage}.tif"),
             "requests": 1, "stage": stage, "thread": "ThreadId(1)",
             "timestamp": 1}
            for stage in ("raster_localize_flow_dir", "raster_localize_flow_acc")
        ]
        attempt = WorkerAttempt({}, [], b"", b"",
                                b"".join(canonical_json(record) for record in records), cache)
        assert require_completed_worker_reads(proxy, before_worker, attempt) == 2
    assert proxy.completed_hosted_reads == before_worker
    assert len(replay.calls) == 3
    print("PASS: hosted worker trace, not idle proxy traffic, satisfies the worker read gate")


def self_test_transport_user_agent() -> None:
    class RecordingDeniedOpener:
        def __init__(self) -> None:
            self.requests: list[urllib.request.Request] = []

        def __call__(self, request: urllib.request.Request, **_kwargs: Any) -> Any:
            self.requests.append(request)
            raise OfflineNetworkViolation("offline transport test stopped before socket open")

    recording = RecordingDeniedOpener()
    set_network_opener(recording)
    try:
        UrllibTransport().request("HEAD", HOSTED_BASE + "manifest.json", {})
    except OfflineNetworkViolation:
        pass
    else:
        raise AssertionError("offline transport test unexpectedly opened a response")
    assert len(recording.requests) == 1
    agent = recording.requests[0].get_header("User-agent")
    assert agent == HOSTED_USER_AGENT and not agent.lower().startswith("python-urllib/")

    denied = DeniedOpener()
    set_network_opener(denied)
    _assert_rejected(
        lambda: UrllibTransport(user_agent="").request(
            "HEAD", HOSTED_BASE + "manifest.json", {}),
        FailureCode.CONFIG,
        "User-Agent",
    )
    assert denied.calls == 0
    print("PASS: hosted transport requires an explicit non-default User-Agent")


def self_test_preflight_published_authority() -> None:
    assert (len(POSITIVE_CANDIDATE), sha256_bytes(POSITIVE_CANDIDATE)) == (
        PUBLISHED_MANIFEST.content_length, PUBLISHED_MANIFEST.sha256)
    assert (len(FORMER_MANIFEST_BYTES), sha256_bytes(FORMER_MANIFEST_BYTES)) == (
        FORMER_MANIFEST.content_length, FORMER_MANIFEST.sha256)
    published = ReplayTransport(POSITIVE_CANDIDATE)
    preflight_hosted(ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, published))
    assert len(published.calls) == 3

    former = ReplayTransport(FORMER_MANIFEST_BYTES)
    _assert_rejected(
        lambda: preflight_hosted(
            ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, former)),
        FailureCode.IDENTITY,
        "published manifest identity",
    )
    assert len(former.calls) == 1
    print("PASS: preflight requires the published D8 declaration authority")


def self_test_authorization() -> None:
    denied = DeniedOpener()
    set_network_opener(denied)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        output = root / "absent"
        base_env = {WHEEL_ENV: str(root / "wheel.whl")}
        _assert_rejected(lambda: live_preflight_for_test(output, base_env),
                         FailureCode.CONFIG, AUTH_ENV)
        assert denied.calls == 0
        authorized = {**base_env, AUTH_ENV: AUTHORIZATION}
        assert validate_live_environment(output, authorized) == root / "wheel.whl"
        zero_replay = ReplayTransport(canonical_json({"auxiliary": []}))
        zero_proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, zero_replay)
        zero_attempt = _synthetic_worker_read_attempt(root)
        _assert_rejected(
            lambda: _build_evidence(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, {},
                                    zero_proxy, zero_attempt, 1, 1,
                                    ZURICH_SEED, None, None, 0),
            FailureCode.ZERO_READS, "zero hosted network operations")
        assert zero_proxy.completed_hosted_reads == 0
        assert zero_replay.calls == []
    print("PASS: authorization failures are loud and typed")


def self_test_bounds() -> None:
    size = COG_IDENTITIES["flow_dir"].content_length
    replay = ReplayTransport(canonical_json({"auxiliary": []}))
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    for header in (None, "bytes=-2", "bytes=2-", f"bytes=0-{size - 1}"):
        _assert_rejected(
            lambda header=header: proxy.hosted(
                "GET", "aux/d8/flow_dir.tif",
                {} if header is None else {"Range": header}),
            FailureCode.BOUNDS)
    response = proxy.hosted("GET", "aux/d8/flow_dir.tif", {"Range": "bytes=10-24"})
    assert response.status == 206 and len(response.body) == 15
    ceilings = (MAX_PLANNED_TILE_COUNT, MAX_COMPRESSED_CHUNK_BYTES,
                MAX_COVERED_CHUNK_BYTES, MAX_DECODED_CHUNK_BYTES,
                MAX_WINDOW_ALLOCATION_BYTES)
    for ceiling in ceilings:
        _assert_rejected(lambda ceiling=ceiling: observe_ceiling(ceiling, ceiling),
                         FailureCode.BOUNDS)
        _assert_rejected(lambda ceiling=ceiling: observe_ceiling(ceiling + 1, ceiling),
                         FailureCode.BOUNDS)
        assert observe_ceiling(ceiling - 1, ceiling).evidence()["margin"] == 1
    print("PASS: hosted ranges and allocation ceilings retain positive margin")


def self_test_candidate_diagnostics() -> None:
    diagnostics = CandidateDiagnostics()
    expected = [
        (CandidateRejectionCode.BOUNDS, (8.0, 47.0)),
        (CandidateRejectionCode.REQUIRE_D8, (9.0, 48.0)),
        (CandidateRejectionCode.BOUNDS, (10.0, 49.0)),
    ]
    for rank, (code, coordinate_value) in enumerate(expected, 1):
        diagnostics.start(rank, coordinate_value)
        try:
            fail(FailureCode[code.value], "injected acceptance predicate rejection")
        except ProofFailure as exc:
            recorded = candidate_rejection_code(exc, CaseMode.HORIZONTAL)
            assert recorded is not None
            diagnostics.reject(recorded)
    try:
        diagnostics.exhausted()
    except ProofFailure as exc:
        assert exc.code == FailureCode.EXHAUSTED
        assert isinstance(exc, CandidateExhaustion)
        message = str(exc)
        ledger_text = message.split("rejection ledger=", 1)[1]
        ledger = json.loads(ledger_text)
        assert ledger == diagnostics.evidence()
        assert [(item["rank"], item["coordinate"], item["rejection_code"])
                for item in ledger] == [
                    (rank, list(coordinate_value), code.value)
                    for rank, (code, coordinate_value) in enumerate(expected, 1)]
        validate_candidate_rejections(ledger, len(expected) + 1,
                                      CaseMode.HORIZONTAL.value)
        _assert_rejected(
            lambda: validate_candidate_rejections(
                ledger, len(expected) + 1, CaseMode.NEGATIVE.value),
            FailureCode.EVIDENCE, "ledger entry differs")
    else:
        raise AssertionError("candidate exhaustion did not fail")

    # Model a faulty controller that empties the reason ledger after rejecting
    # every candidate.  Completeness validation must catch the omission before
    # an exhaustion result can be reported.
    diagnostics._rejections.clear()
    _assert_rejected(diagnostics.exhausted, FailureCode.EVIDENCE,
                     "dropped without a recorded rejection reason")
    print("PASS: candidate exhaustion retains one reason per ordered candidate")


def self_test_seed_probe_predicate() -> None:
    exact_lon = math.degrees(1_000_000 / 6371008.8)
    probe_coordinate = (exact_lon, 0.0)
    assert require_distant((0.0, 0.0), probe_coordinate) >= 1_000_000
    result = {"invocation_id": "0" * 32,
              "refinement_status": "applied",
              "refined_outlet": list(probe_coordinate),
              "resolved_outlet": list(probe_coordinate),
              "terminal_unit_id": "1", "upstream_unit_ids": ["1"]}
    with tempfile.TemporaryDirectory() as temporary:
        attempt = WorkerAttempt(result, [{"id": 1, "outlet": list(probe_coordinate)}],
                                b"", b"", b"", Path(temporary))
        for predicate in (candidate_acceptance_predicate, qualify_seed_probe):
            _assert_rejected(lambda predicate=predicate: predicate(attempt),
                             FailureCode.REQUIRE_D8,
                             "production trace")
    print("PASS: seed probe uses the live measurement predicate")


def self_test_seed_rejection_continues() -> None:
    replay = ReplayTransport(POSITIVE_CANDIDATE)
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    preflight_hosted(proxy)
    candidate_coordinate = (ZURICH_SEED[0], 47.34595845359493)
    result = {"invocation_id": "0" * 32,
              "refinement_status": "applied",
              "refined_outlet": list(ZURICH_SEED),
              "resolved_outlet": list(ZURICH_SEED),
              "terminal_unit_id": "1", "upstream_unit_ids": ["1"]}
    with tempfile.TemporaryDirectory() as temporary:
        discovery = WorkerAttempt(
            result, [{"id": 1, "outlet": list(candidate_coordinate)}],
            b"", b"", b"", Path(temporary))
        candidates, rejection = candidates_after_seed_probe(
            ZURICH_SEED, discovery, CaseMode.HORIZONTAL)
    assert candidates == [candidate_coordinate]
    assert rejection == {"coordinate": list(ZURICH_SEED),
                         "rejection_code": CandidateRejectionCode.REQUIRE_D8.value}
    assert len(replay.calls) == 3 and proxy.completed_hosted_reads == 3
    print("PASS: rejected seed is recorded and ordered candidate search continues")


def self_test_rejected_seed_disqualified() -> None:
    result = {"invocation_id": "0" * 32,
              "refinement_status": "applied",
              "refined_outlet": list(ZURICH_SEED),
              "resolved_outlet": list(ZURICH_SEED),
              "terminal_unit_id": "1", "upstream_unit_ids": ["1"]}
    for units in ([], [{"id": 1, "outlet": list(ZURICH_SEED)}]):
        with tempfile.TemporaryDirectory() as temporary:
            discovery = WorkerAttempt(result, units, b"", b"", b"", Path(temporary))
            candidates, rejection = candidates_after_seed_probe(
                ZURICH_SEED, discovery, CaseMode.HORIZONTAL)
        assert candidates == []
        assert rejection == {"coordinate": list(ZURICH_SEED),
                             "rejection_code": CandidateRejectionCode.REQUIRE_D8.value}
    print("PASS: rejected seed cannot re-enter the ordered candidate loop")


def _self_test_accepted_attempt(
        cache: Path, candidates: list[dict[str, Any]],
) -> WorkerAttempt:
    cache.mkdir(parents=True)
    origin_y = EQUAL_EARTH_Y_MAX - 511.0
    dir_path, acc_path = cache / "dir.tif", cache / "acc.tif"
    _write_fixture_tiff(dir_path, "U8", [1, 2, 4, 8], 2, 2, origin_y, -1.0)
    _write_fixture_tiff(acc_path, "F32", [1.0, 2.0, 3.0, 4.0], 2, 2,
                        origin_y, -1.0)
    records = [
        {"bytes": path.stat().st_size, "cache_status": "fetched",
         "duration_ms": 0.0, "kind": "stage", "path": str(path),
         "requests": 1, "stage": stage, "thread": "ThreadId(1)",
         "timestamp": 1}
        for path, stage in ((dir_path, "raster_localize_flow_dir"),
                            (acc_path, "raster_localize_flow_acc"))
    ]
    return WorkerAttempt(
        {"invocation_id": "0" * 32}, candidates, b"", b"",
        b"".join(canonical_json(record) for record in records), cache)


def self_test_unresolved_status_requires_protocol_marker() -> None:
    original_run = subprocess.run

    def injected_exit(*_args: Any, **_kwargs: Any) -> subprocess.CompletedProcess[bytes]:
        return subprocess.CompletedProcess([], int(FailureCode.UNRESOLVED), b"",
                                           b"injected machinery failure")

    with tempfile.TemporaryDirectory() as temporary_text:
        temporary = Path(temporary_text)
        (temporary / "staging").mkdir()
        subprocess.run = injected_exit  # type: ignore[assignment]
        try:
            _assert_rejected(
                lambda: run_worker(
                    temporary, temporary / "install", HOSTED_BASE,
                    CaseMode.HORIZONTAL, (8.0, 47.0), 1,
                    ambient_environment={},
                ),
                FailureCode.WORKER,
                "unresolved status lacks worker protocol marker",
            )
        finally:
            subprocess.run = original_run  # type: ignore[assignment]
    print("PASS: unresolved status requires the worker protocol marker")


def self_test_unresolved_candidate_recorded(candidate_budget: int = CANDIDATE_BUDGET) -> None:
    if candidate_budget < 2:
        fail(FailureCode.CONFIG, "candidate budget must exercise continuation")
    replay = ReplayTransport(POSITIVE_CANDIDATE)
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    preflight_hosted(proxy)
    candidates = [(8.0, 47.0), (8.1, 47.1), (8.2, 47.2)]
    with tempfile.TemporaryDirectory() as temporary_text:
        accepted_attempt = _self_test_accepted_attempt(
            Path(temporary_text) / "hfx-cache" / "attempt-2", [])
        attempted: list[tuple[float, float]] = []

        def attempt_candidate(candidate_coord: tuple[float, float],
                              rank: int) -> WorkerAttempt:
            attempted.append(candidate_coord)
            if rank == 1:
                fail(FailureCode.UNRESOLVED, "injected outlet resolution rejection")
            return accepted_attempt

        selected, selected_rank, _, diagnostics = select_ordered_candidate(
            candidates, candidate_budget, CaseMode.HORIZONTAL, None,
            attempt_candidate)
    assert selected is accepted_attempt and selected_rank == 2
    assert attempted == candidates[:2]
    ledger = diagnostics.evidence()
    assert ledger == [{"coordinate": [8.0, 47.0], "rank": 1,
                       "rejection_code": CandidateRejectionCode.UNRESOLVED.value}]
    validate_candidate_rejections(ledger, 2, CaseMode.HORIZONTAL.value)
    assert len(replay.calls) == 3 and proxy.completed_hosted_reads == 3
    assert len({item.value for item in CandidateRejectionCode}) == len(CandidateRejectionCode)
    for machinery_code in (FailureCode.WHEEL, FailureCode.WORKER,
                           FailureCode.EVIDENCE, FailureCode.INTERNAL):
        failure = ProofFailure(machinery_code, "injected machinery failure")
        assert candidate_rejection_code(failure, CaseMode.HORIZONTAL) is None
    print("PASS: an unresolvable candidate is recorded and selection continues")


def self_test_accepted_seed_is_ranked(candidate_budget: int = CANDIDATE_BUDGET) -> None:
    if candidate_budget < 1:
        fail(FailureCode.CONFIG, "candidate budget must be positive")
    discovered_coordinate = (ZURICH_SEED[0], 47.34595845359493)
    with tempfile.TemporaryDirectory() as temporary_text:
        discovery = _self_test_accepted_attempt(
            Path(temporary_text) / "hfx-cache" / "attempt-0",
            [{"id": 1, "outlet": list(discovered_coordinate)}])
        ranked, rejection = candidates_after_seed_probe(
            ZURICH_SEED, discovery, CaseMode.HORIZONTAL)
        attempted: list[tuple[float, float]] = []

        def attempt_candidate(candidate_coord: tuple[float, float],
                              _rank: int) -> WorkerAttempt:
            attempted.append(candidate_coord)
            return discovery

        selected, selected_rank, _, _ = select_ordered_candidate(
            ranked, candidate_budget, CaseMode.HORIZONTAL, None,
            attempt_candidate)
    assert rejection is None
    assert discovered_coordinate in ranked
    assert ranked[0] == ZURICH_SEED
    assert ranked.count(ZURICH_SEED) == 1
    assert selected is discovery and selected_rank == 1
    assert attempted == [ZURICH_SEED]
    print("PASS: an accepted seed is ranked and attempted within the supplied budget")


def _synthetic_distant_seed_declaration(seed: tuple[float, float]) -> dict[str, Any]:
    wheel_name = sorted(WHEEL_ALLOWLIST)[0]
    wheel_size, wheel_sha = WHEEL_ALLOWLIST[wheel_name]
    geometry = simple_multipolygon([[[(seed[0], seed[1]),
                                      (seed[0] + 0.001, seed[1]),
                                      (seed[0], seed[1] + 0.001),
                                      (seed[0], seed[1])]]])
    result = {
        "area_km2": "1.0", "geometry_wkb_hex": geometry.hex(),
        "input_outlet": [repr(seed[0]), repr(seed[1])],
        "invocation_id": "0" * 32, "refined_outlet": [repr(seed[0]), repr(seed[1])],
        "refinement_status": "applied", "resolution_method": "Snap",
        "resolved_outlet": [repr(seed[0]), repr(seed[1])],
        "schema": "pourpoint.released-wheel-proof-worker-result.v1",
        "terminal_unit_id": "1", "upstream_unit_ids": ["1"],
    }
    return {
        "dataset": {"base": HOSTED_BASE,
                    "manifest": {"byte_count": PUBLISHED_MANIFEST.content_length,
                                 "sha256": PUBLISHED_MANIFEST.sha256}},
        "discovery_seed": list(seed), "resolution": result,
        "schema": DISTANT_SEED_SCHEMA,
        "wheel": {"filename": wheel_name, "metadata_name": "pourpoint",
                  "metadata_requires_python": ">=3.9", "metadata_version": "0.3.0",
                  "sha256": wheel_sha, "size_bytes": wheel_size},
    }


def self_test_unresolvable_seed_named() -> None:
    with tempfile.TemporaryDirectory() as temporary_text:
        temporary = Path(temporary_text)
        declaration = temporary / "distant-seed.json"
        declaration.write_bytes(canonical_json(
            _synthetic_distant_seed_declaration(DISTANT_DISCOVERY_SEED)))

        def unresolved(*_args: Any, **_kwargs: Any) -> WorkerAttempt:
            fail(FailureCode.UNRESOLVED, "UNRESOLVED:injected seed resolution")

        _assert_rejected(
            lambda: run_distant_seed_discovery(
                declaration, temporary, temporary / "install", 0, unresolved),
            FailureCode.UNRESOLVED,
            f"distant discovery seed {list(DISTANT_DISCOVERY_SEED)} from sealed declaration {declaration} is unresolvable",
        )
        accepted = _self_test_accepted_attempt(
            temporary / "hfx-cache" / "attempt-2", [])

        def ranked_attempt(_coordinate: tuple[float, float], rank: int) -> WorkerAttempt:
            if rank == 1:
                fail(FailureCode.UNRESOLVED,
                     "UNRESOLVED:injected ranked candidate rejection")
            return accepted

        selected, rank, _, diagnostics = select_ordered_candidate(
            [(20.0, 65.0), (21.0, 66.0)], 2, CaseMode.HORIZONTAL,
            None, ranked_attempt)
        assert selected is accepted and rank == 2
        assert diagnostics.evidence() == [
            {"coordinate": [20.0, 65.0], "rank": 1,
             "rejection_code": CandidateRejectionCode.UNRESOLVED.value}]
    print("PASS: unresolvable discovery seed is named while ranked rejection continues")


CORE_SELF_TEST_SECTIONS: dict[str, Callable[[], None]] = {
    "accepted-seed-is-ranked": self_test_accepted_seed_is_ranked,
    "authorization": self_test_authorization,
    "live-hosted-source": self_test_live_hosted_source,
    "live-hosted-worker-read-gate": self_test_live_hosted_worker_read_gate,
    "worker-network-reads": self_test_worker_network_reads,
    "bounds": self_test_bounds,
    "candidate-diagnostics": self_test_candidate_diagnostics,
    "preflight-published-authority": self_test_preflight_published_authority,
    "seed-probe-predicate": self_test_seed_probe_predicate,
    "seed-rejection-continues": self_test_seed_rejection_continues,
    "rejected-seed-disqualified": self_test_rejected_seed_disqualified,
    "transport-user-agent": self_test_transport_user_agent,
    "unresolved-candidate-recorded": self_test_unresolved_candidate_recorded,
    "unresolvable-seed-named": self_test_unresolvable_seed_named,
    "unresolved-status-authenticated": self_test_unresolved_status_requires_protocol_marker,
}


class EnvironmentProbe(dict[str, str]):
    """Records reads of the two ambient values that must not affect self-tests."""

    def __init__(self, values: dict[str, str]) -> None:
        super().__init__(values)
        self.live_input_reads: list[str] = []

    def _record(self, key: object) -> None:
        if key in (AUTH_ENV, WHEEL_ENV):
            self.live_input_reads.append(str(key))

    def __getitem__(self, key: str) -> str:
        self._record(key)
        return super().__getitem__(key)

    def get(self, key: str, default: Any = None) -> Any:
        self._record(key)
        return super().get(key, default)

    def pop(self, key: str, *default: Any) -> Any:
        self._record(key)
        return super().pop(key, *default)

    def __contains__(self, key: object) -> bool:
        self._record(key)
        return super().__contains__(key)

    def _record_present_live_inputs(self) -> None:
        for key in (AUTH_ENV, WHEEL_ENV):
            if super().__contains__(key):
                self.live_input_reads.append(key)

    def __iter__(self) -> Any:
        self._record_present_live_inputs()
        return super().__iter__()

    def keys(self) -> Any:
        self._record_present_live_inputs()
        return super().keys()

    def items(self) -> Any:
        self._record_present_live_inputs()
        return super().items()

    def values(self) -> Any:
        self._record_present_live_inputs()
        return super().values()

    def copy(self) -> dict[str, str]:
        self._record_present_live_inputs()
        return super().copy()


def _run_with_environment_probe(
        action: Callable[[], None],
        values: dict[str, str]) -> tuple[str, str, list[str]]:
    probe = EnvironmentProbe(values)
    stdout = io.StringIO()
    stderr = io.StringIO()
    previous_environment = os.environ
    previous_opener = _network_opener
    try:
        os.environ = probe  # type: ignore[assignment]
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            action()
    finally:
        os.environ = previous_environment
        set_network_opener(previous_opener)
    return stdout.getvalue(), stderr.getvalue(), probe.live_input_reads


def _hermetic_self_test_actions() -> dict[str, Callable[[], None]]:
    import verify_released_wheel_evidence as verifier

    return {**CORE_SELF_TEST_SECTIONS, "evidence-verifier": verifier.self_test}


def self_test_hermetic_under_live_inputs() -> None:
    live_inputs = {
        AUTH_ENV: "invalid-ambient-authorization",
        WHEEL_ENV: "/invalid/ambient/released-wheel.whl",
    }
    for name, action in _hermetic_self_test_actions().items():
        without_live_inputs = _run_with_environment_probe(action, {})[:2]
        with_live_inputs = _run_with_environment_probe(action, live_inputs)[:2]
        if with_live_inputs != without_live_inputs:
            fail(FailureCode.EVIDENCE,
                 f"self-test result differs under ambient live inputs: {name}")
    print("PASS: every self-test result is unchanged under ambient live inputs")


def self_test_ambient_input_isolation() -> None:
    invalid_live_inputs = {
        AUTH_ENV: "invalid-if-read",
        WHEEL_ENV: "/invalid-if-read/released-wheel.whl",
    }

    def probe_control() -> None:
        os.environ.get(AUTH_ENV)
        os.environ[WHEEL_ENV]

    _, _, control_reads = _run_with_environment_probe(
        probe_control, invalid_live_inputs)
    if control_reads != [AUTH_ENV, WHEEL_ENV]:
        fail(FailureCode.INTERNAL, "ambient live-input probe negative control failed")

    reads: dict[str, list[str]] = {}
    for name, action in _hermetic_self_test_actions().items():
        _, _, observed = _run_with_environment_probe(action, invalid_live_inputs)
        if observed:
            reads[name] = observed
    if reads:
        fail(FailureCode.EVIDENCE,
             f"self-test read ambient live inputs: {json.dumps(reads, sort_keys=True)}")
    print("PASS: ambient live-input probe recorded zero reads across all self-tests")


SELF_TEST_SECTIONS: dict[str, Callable[[], None]] = {
    **CORE_SELF_TEST_SECTIONS,
    "ambient-input-isolation": self_test_ambient_input_isolation,
    "hermetic-under-live-inputs": self_test_hermetic_under_live_inputs,
}


PASS_LINES = [
    "PASS: network opener is unreachable offline",
    "PASS: missing authorization rejected loudly",
    "PASS: authorized zero completed reads rejected loudly",
    "PASS: candidate byte drift rejected",
    "PASS: candidate trailing newline drift rejected",
    "PASS: negative declaration extra drift rejected",
    "PASS: wheel size digest and metadata drift rejected",
    "PASS: mutation methods rejected",
    "PASS: unbounded COG reads rejected",
    "PASS: range ceilings rejected",
    "PASS: completed range transcript corruption rejected",
    "PASS: RequireD8 degraded status rejected",
    "PASS: missing production trace rejected",
    "PASS: production window corruption rejected",
    "PASS: canonical WKB corruption rejected",
    "PASS: negative discriminator equality rejected",
    "PASS: distant-region threshold rejected",
    "PASS: synthetic evidence cannot satisfy live proof",
    "PASS: artifact index tampering rejected",
]


BUDGETED_SELF_TEST_SECTIONS = {
    "accepted-seed-is-ranked",
    "unresolved-candidate-recorded",
}


def self_test(section: str | None = None,
              candidate_budget: int = CANDIDATE_BUDGET) -> None:
    if section is not None:
        action = SELF_TEST_SECTIONS[section]
        if section in BUDGETED_SELF_TEST_SECTIONS:
            action(candidate_budget)  # type: ignore[call-arg]
        else:
            action()
        return
    for section_test in SELF_TEST_SECTIONS.values():
        section_test()
    denied = DeniedOpener()
    set_network_opener(denied)

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        output = root / "absent"
        base_env = {WHEEL_ENV: str(root / "wheel.whl")}
        probe = DeniedOpener()
        set_network_opener(probe)
        try:
            _network_opener(urllib.request.Request(HOSTED_BASE + "manifest.json"), timeout=1)
        except OfflineNetworkViolation:
            pass
        else:
            raise AssertionError("installed network opener did not deny the request")
        assert probe.calls == 1
        set_network_opener(denied)
        _assert_rejected(lambda: live_preflight_for_test(output, base_env), FailureCode.CONFIG, AUTH_ENV)
        assert denied.calls == 0
        print(PASS_LINES[0])
        print(PASS_LINES[1])

        authorized = {**base_env, AUTH_ENV: AUTHORIZATION}
        assert validate_live_environment(output, authorized) == root / "wheel.whl"
        zero_replay = ReplayTransport(canonical_json({"auxiliary": []}))
        zero_proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, zero_replay)
        zero_attempt = _synthetic_worker_read_attempt(root)
        _assert_rejected(
            lambda: _build_evidence(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, {}, zero_proxy,
                                    zero_attempt, 1, 1, ZURICH_SEED, None, None, 0),
            FailureCode.ZERO_READS)
        print(PASS_LINES[2])

        drift = bytearray(POSITIVE_CANDIDATE); drift[10] ^= 1
        _assert_rejected(lambda: verify_positive_candidate(bytes(drift)), FailureCode.IDENTITY)
        print(PASS_LINES[3])
        for data in (POSITIVE_CANDIDATE[:-1], POSITIVE_CANDIDATE + b"\n"):
            _assert_rejected(lambda data=data: verify_positive_candidate(data), FailureCode.IDENTITY)
        print(PASS_LINES[4])
        extra = json.loads(NEGATIVE_CANDIDATE); extra["fabric_name"] = "other"
        extra_bytes = (json.dumps(extra, indent=2) + "\n").encode()
        _assert_rejected(lambda: verify_negative_candidate(extra_bytes), FailureCode.IDENTITY)
        print(PASS_LINES[5])

        # Synthetic wheels can only exercise the rejection path: no generated
        # archive can possess an allowlisted digest.
        wheel = root / expected_wheel_for_host()
        metadata_variants = [
            b"Name: wrong\nVersion: 0.3.0\nRequires-Python: >=3.9\n",
            b"Name: pourpoint\nVersion: 9\nRequires-Python: >=3.9\n",
            b"Name: pourpoint\nVersion: 0.3.0\nRequires-Python: >=3.8\n",
        ]
        for metadata in metadata_variants:
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("pourpoint-0.3.0.dist-info/METADATA", metadata)
            _assert_rejected(lambda: verify_wheel(wheel), FailureCode.WHEEL)
        for name in ("duplicate metadata", "traversal"):
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("pourpoint-0.3.0.dist-info/METADATA", b"Name: pourpoint\nVersion: 0.3.0\nRequires-Python: >=3.9\n")
                archive.writestr("other.dist-info/METADATA" if name == "duplicate metadata" else "../escape", b"x")
            _assert_rejected(lambda: verify_wheel(wheel), FailureCode.WHEEL)
        wrong_tag = root / "pourpoint-0.3.0-cp39-abi3-win_amd64.whl"
        wrong_tag.write_bytes(b"bad")
        _assert_rejected(lambda: verify_wheel(wrong_tag), FailureCode.WHEEL)
        print(PASS_LINES[6])

        prohibited = {"POST", "PUT", "PATCH", "DELETE", "COPY", "MOVE", "PROPFIND", "MKCOL", "LOCK", "UNLOCK", "OPTIONS", "CONNECT", "TRACE"}
        mutation_replay = ReplayTransport(canonical_json({"auxiliary": []}))
        mutation_proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, mutation_replay)
        for method in prohibited:
            _assert_rejected(
                lambda method=method: mutation_proxy.hosted(method, "aux/d8/flow_dir.tif", {"Range": "bytes=10-24"}),
                FailureCode.IDENTITY, "405")
        assert mutation_proxy.mutation_attempt_count == len(prohibited)
        assert mutation_replay.calls == []
        assert mutation_proxy.completed_hosted_reads == 0

        handler_replay = ReplayTransport(canonical_json({"auxiliary": []}))
        handler_proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, handler_replay)
        with running_proxy(handler_proxy) as proxy_url:
            parsed_proxy = urllib.parse.urlsplit(proxy_url)
            assert parsed_proxy.hostname is not None and parsed_proxy.port is not None
            for method in prohibited:
                before = handler_proxy.mutation_attempt_count
                connection = http.client.HTTPConnection(parsed_proxy.hostname, parsed_proxy.port, timeout=1)
                connection.request(method, parsed_proxy.path + "aux/d8/flow_dir.tif")
                response = connection.getresponse()
                response.read()
                connection.close()
                assert response.status == 405
                assert handler_proxy.mutation_attempt_count == before + 1
        assert handler_proxy.mutation_attempt_count == len(prohibited)
        assert handler_replay.calls == []
        assert handler_proxy.completed_hosted_reads == 0
        print(PASS_LINES[7])

        size = COG_IDENTITIES["flow_dir"].content_length
        for header in (None, "bytes=-2", "bytes=2-", "bytes=1-2,4-5", f"bytes=0-{size-1}"):
            _assert_rejected(lambda header=header: parse_closed_range(header, size), FailureCode.BOUNDS)
        print(PASS_LINES[8])
        for ceiling in (MAX_PLANNED_TILE_COUNT, MAX_COMPRESSED_CHUNK_BYTES, MAX_COVERED_CHUNK_BYTES, MAX_DECODED_CHUNK_BYTES, MAX_WINDOW_ALLOCATION_BYTES):
            for _kind in COG_IDENTITIES:
                _assert_rejected(lambda ceiling=ceiling: observe_ceiling(ceiling + 1, ceiling), FailureCode.BOUNDS)
                _assert_rejected(lambda ceiling=ceiling: observe_ceiling(ceiling, ceiling), FailureCode.BOUNDS)
                assert observe_ceiling(ceiling - 1, ceiling).evidence()["margin"] == 1
        print(PASS_LINES[9])

        valid_read = {"bytes_received": 10, "case_id": "horizontal-boundary", "completed": True, "error": None,
                      "etag": COG_IDENTITIES["flow_dir"].etag, "key": "aux/d8/flow_dir.tif", "method": "GET", "origin": "hosted",
                      "range": {"end_exclusive": 20, "start": 10}, "response_content_length": 10,
                      "response_content_range": f"bytes 10-19/{size}", "seq": 1, "status": 206,
                      "url": HOSTED_BASE + "aux/d8/flow_dir.tif"}
        valid_line = canonical_json(valid_read)
        validate_completed_reads(valid_line)
        corruptions = [b"", valid_line + valid_line, valid_line[:-1] + b"x", canonical_json({**valid_read, "seq": 2}),
                       canonical_json({**valid_read, "bytes_received": 9}), canonical_json({**valid_read, "status": 200}),
                       canonical_json({**valid_read, "response_content_range": "wrong"}),
                       canonical_json({**valid_read, "etag": '"forged-etag"'}),
                       canonical_json({**valid_read, "response_content_length": 9}),
                       canonical_json({**valid_read, "completed": False}), valid_line + b"{"]
        for corrupted in corruptions:
            _assert_rejected(lambda corrupted=corrupted: validate_completed_reads(corrupted),
                             FailureCode.EVIDENCE if corrupted in (b"", valid_line + valid_line, valid_line[:-1] + b"x", canonical_json({**valid_read, "seq": 2}), valid_line + b"{") else FailureCode.BOUNDS)
        print(PASS_LINES[10])

        validate_require_d8("applied", [1.0, 2.0])
        for status, refined in (("best_effort_skipped", [1, 2]), ("disabled", [1, 2]), (None, [1, 2]), ("applied", None)):
            _assert_rejected(lambda status=status, refined=refined: validate_require_d8(status, refined), FailureCode.REQUIRE_D8)
        print(PASS_LINES[11])

        cache = root / "cache"; cache.mkdir()
        invocation = "0" * 32
        trace = [{"kind": "stage", "stage": stage, "timestamp": 1, "duration_ms": 0.0, "thread": "t",
                  "cache_status": "fetched", "path": str(cache / f"{stage}.tif"), "bytes": 1, "requests": 1}
                 for stage in ("raster_localize_flow_dir", "raster_localize_flow_acc")]
        validate_trace(trace, cache, invocation)
        trace_mutations = [trace[:1], trace + [dict(trace[0])], [{**trace[0], "stage": "other"}, trace[1]],
                           [{**trace[0], "cache_status": "hit"}, trace[1]], [{**trace[0], "path": str(root / "outside")}, trace[1]],
                           [{**trace[0], "invocation_id": "1" * 32}, trace[1]]]
        for mutation in trace_mutations:
            try:
                validate_trace(mutation, cache, invocation)
            except ProofFailure as exc:
                assert exc.code in {FailureCode.REQUIRE_D8, FailureCode.BOUNDS, FailureCode.EVIDENCE}
            else:
                raise AssertionError("trace corruption accepted")
        print(PASS_LINES[12])

        validate_window({"width": 2, "height": 2, "sample_type": "U8", "samples": [1, 2, 4, 8], "seam": 1})
        window_corruptions = [
            {"width": 0, "height": 2, "sample_type": "U8", "samples": [], "seam": 1},
            {"width": 2, "height": 2, "sample_type": "I16", "samples": [1]*4, "seam": 1},
            {"width": 2, "height": 2, "sample_type": "U8", "samples": [1]*3, "seam": 1},
            {"width": 2, "height": 2, "sample_type": "U8", "samples": [255]*4, "seam": 1},
            {"width": 2, "height": 2, "sample_type": "U8", "samples": [1]*4, "seam": 0},
            {"width": 2, "height": 2, "sample_type": "U8", "samples": [255,255,1,2], "seam": 1},
        ]
        for value in window_corruptions:
            _assert_rejected(lambda value=value: validate_window(value), FailureCode.BOUNDS)
        print(PASS_LINES[13])

        square = [[[(1, 0), (0, 0), (0, 1), (1, 1), (1, 0)]]]
        reordered = [[[(0, 1), (1, 1), (1, 0), (0, 0), (0, 1)]]]
        first = canonical_wkb(simple_multipolygon(square)); second = canonical_wkb(simple_multipolygon(reordered))
        assert first == second
        validate_canonical_wkb(first, sha256_bytes(first))
        broken = bytearray(first); broken[-1] ^= 1
        for action in (lambda: validate_canonical_wkb(bytes(broken), sha256_bytes(first)),
                       lambda: validate_canonical_wkb(first, "0" * 64), lambda: validate_canonical_wkb(first, sha256_bytes(first), 5)):
            _assert_rejected(action, FailureCode.EVIDENCE)
        print(PASS_LINES[14])

        positive = {"input": [1, 2], "terminal": 1, "upstream": [1], "resolved": [1, 2], "refined": [1, 2], "status": "applied", "geometry_sha256": "a"}
        validate_negative_discriminator(positive, {**positive, "geometry_sha256": "b"})
        _assert_rejected(lambda: validate_negative_discriminator(positive, dict(positive)), FailureCode.EXHAUSTED)
        print(PASS_LINES[15])

        # Construct exact target distances along the equator, where the inverse
        # reduces without introducing a case-specific approximation.
        exact_lon = math.degrees(1_000_000 / 6371008.8)
        below_lon = math.degrees(999_999.999999 / 6371008.8)
        _assert_rejected(lambda: require_distant((0.0, 0.0), (below_lon, 0.0), lambda: denied.calls), FailureCode.EXHAUSTED)
        exact_distance = require_distant((0.0, 0.0), (exact_lon, 0.0))
        assert exact_distance >= 1_000_000 and abs(exact_distance - 1_000_000) < 1e-8
        _assert_rejected(lambda: verify_recorded_distance((0.0, 0.0), (exact_lon, 0.0), exact_distance + 1), FailureCode.EVIDENCE)
        print(PASS_LINES[16])

        exercise_injected_controller(root)
        _assert_rejected(lambda: validate_live_evidence({"fixture_kind": "synthetic"}), FailureCode.EVIDENCE)
        _assert_rejected(lambda: validate_live_evidence({"hosted": {"completed_network_reads": 1}}), FailureCode.EVIDENCE)
        print(PASS_LINES[17])

        artifact = root / "artifact"; artifact.mkdir()
        for name in RETAINED:
            data = b"x"
            if name == "geometry.canonical.wkb": data = first
            elif name == "reads.jsonl": data = valid_line
            elif name.endswith(".json"): data = canonical_json({})
            (artifact / name).write_bytes(data)
        (artifact / "artifact-index.json").write_bytes(canonical_json(build_artifact_index(artifact)))
        verify_artifact_directory(artifact)
        for name in RETAINED:
            copy = root / ("copy-" + name.replace("/", "_")); shutil.copytree(artifact, copy)
            (copy / name).write_bytes((copy / name).read_bytes() + b"x")
            _assert_rejected(lambda copy=copy: verify_artifact_directory(copy), FailureCode.EVIDENCE)
        for label, mutate in (
            ("extra", lambda p: (p / "extra").write_bytes(b"x")),
            ("missing", lambda p: (p / RETAINED[0]).unlink()),
            ("index", lambda p: (p / "artifact-index.json").write_bytes(canonical_json({"artifacts": [], "schema": "pourpoint.released-wheel-proof-artifact-index.v1"}))),
        ):
            copy = root / label; shutil.copytree(artifact, copy); mutate(copy)
            _assert_rejected(lambda copy=copy: verify_artifact_directory(copy), FailureCode.EVIDENCE)
        symlink_copy = root / "symlink"; shutil.copytree(artifact, symlink_copy); (symlink_copy / RETAINED[0]).unlink(); (symlink_copy / RETAINED[0]).symlink_to(artifact / RETAINED[0])
        _assert_rejected(lambda: verify_artifact_directory(symlink_copy), FailureCode.EVIDENCE)
        print(PASS_LINES[18])

    assert denied.calls == 0


def validate_window(value: dict[str, Any]) -> None:
    width, height = value.get("width"), value.get("height")
    sample_type, samples, seam = value.get("sample_type"), value.get("samples"), value.get("seam")
    if type(width) is not int or type(height) is not int or width <= 0 or height <= 0 or sample_type not in {"U8", "F32"}:
        fail(FailureCode.BOUNDS, "production window dimensions or sample type differ")
    sample_width = 1 if sample_type == "U8" else 4
    observe_ceiling(width * height * sample_width, MAX_WINDOW_ALLOCATION_BYTES)
    if not isinstance(samples, list) or len(samples) != width * height or type(seam) is not int or not 0 < seam < height:
        fail(FailureCode.BOUNDS, "production window decoded size or seam differs")
    valid = [(sample != 255) if sample_type == "U8" else (type(sample) in {int, float} and math.isfinite(sample)) for sample in samples]
    if not any(valid) or not any(valid[:seam * width]) or not any(valid[seam * width:]):
        fail(FailureCode.BOUNDS, "production window lacks real samples on both seam sides")


def verify_recorded_distance(horizontal: tuple[float, float], distant: tuple[float, float], recorded: float) -> None:
    computed = spherical_distance_metres(horizontal, distant)
    if type(recorded) not in {int, float} or not math.isfinite(recorded) or recorded != computed:
        fail(FailureCode.EVIDENCE, "recorded distant distance differs from independent computation")


@dataclasses.dataclass(frozen=True)
class TransportResponse:
    status: int
    headers: dict[str, str]
    body: bytes


class ReadOnlyTransport(Protocol):
    def request(self, method: str, url: str, headers: dict[str, str]) -> TransportResponse: ...


class UrllibTransport:
    """Real, redirect-rejecting transport used only by authorized live mode."""

    def __init__(self, user_agent: str = HOSTED_USER_AGENT) -> None:
        self.user_agent = user_agent

    def request(self, method: str, url: str, headers: dict[str, str]) -> TransportResponse:
        if not isinstance(self.user_agent, str):
            fail(FailureCode.CONFIG, "hosted transport requires an explicit non-default User-Agent")
        user_agent = self.user_agent.strip()
        if not user_agent or user_agent.lower().startswith("python-urllib/"):
            fail(FailureCode.CONFIG, "hosted transport requires an explicit non-default User-Agent")
        identified_headers = {key: value for key, value in headers.items()
                              if key.lower() != "user-agent"}
        identified_headers["User-Agent"] = user_agent
        request = urllib.request.Request(url, headers=identified_headers, method=method)
        try:
            response = _network_opener(request, timeout=120)
        except urllib.error.HTTPError as exc:
            if 300 <= exc.code < 400:
                fail(FailureCode.IDENTITY, "hosted redirect was rejected")
            fail(FailureCode.IDENTITY, f"hosted HTTP failure {exc.code}")
        except (OSError, urllib.error.URLError) as exc:
            fail(FailureCode.IDENTITY, f"hosted transport failure: {exc}")
        with contextlib.closing(response):
            final_url = response.geturl()
            if final_url != url:
                fail(FailureCode.IDENTITY, "hosted response changed the fixed URL")
            raw_headers = {key.lower(): value.strip() for key, value in response.headers.items()}
            limit = _response_limit(url, method, headers)
            body = _read_limited(response, limit) if method == "GET" else b""
            return TransportResponse(int(response.status), raw_headers, body)


def _response_limit(url: str, method: str, headers: dict[str, str]) -> int:
    if method != "GET":
        return 0
    if url.endswith(("flow_dir.tif", "flow_acc.tif")):
        value = headers.get("Range") or headers.get("range")
        kind = "flow_dir" if url.endswith("flow_dir.tif") else "flow_acc"
        return parse_closed_range(value, COG_IDENTITIES[kind].content_length).length
    return 536870912


def _read_limited(source: BinaryIO, limit: int) -> bytes:
    output = bytearray()
    while True:
        chunk = source.read(min(1024 * 1024, limit + 1 - len(output)))
        if not chunk:
            return bytes(output)
        output.extend(chunk)
        if len(output) > limit:
            fail(FailureCode.BOUNDS, "hosted response exceeded the bounded body limit")


def _header(headers: dict[str, str], name: str) -> str | None:
    lowered = name.lower()
    return next((value for key, value in headers.items() if key.lower() == lowered), None)


class ProxyController:
    """Candidate substitution plus bounded forwarding for one proof case."""

    def __init__(self, case: CaseMode, candidate: bytes, transport: ReadOnlyTransport) -> None:
        self.case = case
        self.candidate = candidate
        self.transport = transport
        self.records: list[dict[str, Any]] = []
        self.mutation_attempt_count = 0
        self.fatal: ProofFailure | None = None
        self.covered = {kind: 0 for kind in COG_IDENTITIES}
        self.range_count = {kind: 0 for kind in COG_IDENTITIES}
        self.max_range = {kind: 0 for kind in COG_IDENTITIES}
        self._lock = threading.Lock()

    @property
    def completed_hosted_reads(self) -> int:
        return sum(record["origin"] == "hosted" and record["completed"] for record in self.records)

    def _record(self, *, method: str, key: str, origin: str, url: str,
                response: TransportResponse | None, byte_range: ClosedByteRange | None,
                error: str | None) -> None:
        with self._lock:
            self.records.append({
                "bytes_received": len(response.body) if response is not None else 0,
                "case_id": self.case.value,
                "completed": response is not None and error is None,
                "error": error,
                "etag": _header(response.headers, "etag") if response is not None else None,
                "key": key,
                "method": method,
                "origin": origin,
                "range": None if byte_range is None else {
                    "end_exclusive": byte_range.end_inclusive + 1, "start": byte_range.start},
                "response_content_length": _int_header(response, "content-length") if response is not None else None,
                "response_content_range": _header(response.headers, "content-range") if response is not None else None,
                "seq": len(self.records) + 1,
                "status": response.status if response is not None else None,
                "url": url,
            })

    def local_candidate(self, method: str, url: str) -> TransportResponse:
        if self.case is CaseMode.NEGATIVE:
            verify_negative_candidate(self.candidate)
        else:
            verify_positive_candidate(self.candidate)
        body = self.candidate if method == "GET" else b""
        response = TransportResponse(200, {"content-length": str(len(self.candidate)),
                                           "etag": f'"{sha256_bytes(self.candidate)}"'}, body)
        self._record(method=method, key="manifest.json", origin="local_candidate", url=url,
                     response=response, byte_range=None, error=None)
        return response

    def hosted(self, method: str, key: str, request_headers: dict[str, str]) -> TransportResponse:
        if method not in {"HEAD", "GET"}:
            self.mutation_attempt_count += 1
            failure = ProofFailure(FailureCode.IDENTITY, f"mutation method {method} rejected with HTTP 405")
            self.fatal = failure
            raise failure
        _validate_relative_key(key)
        url = HOSTED_BASE + key
        byte_range = None
        kind = _cog_kind(key)
        headers: dict[str, str] = {}
        if kind is not None and method == "GET":
            byte_range = parse_closed_range(_header(request_headers, "range"), COG_IDENTITIES[kind].content_length)
            prospective_count = self.range_count[kind] + 1
            prospective_covered = self.covered[kind] + byte_range.length
            observe_ceiling(prospective_count, MAX_PLANNED_TILE_COUNT)
            observe_ceiling(prospective_covered, MAX_COVERED_CHUNK_BYTES)
            headers["Range"] = f"bytes={byte_range.start}-{byte_range.end_inclusive}"
        try:
            response = self.transport.request(method, url, headers)
            self._validate_response(kind, method, byte_range, response)
        except ProofFailure as exc:
            self._record(method=method, key=key, origin="hosted", url=url, response=None,
                         byte_range=byte_range, error=str(exc))
            self.fatal = exc
            raise
        self._record(method=method, key=key, origin="hosted", url=url, response=response,
                     byte_range=byte_range, error=None)
        if kind is not None and method == "GET" and byte_range is not None:
            self.range_count[kind] += 1
            self.covered[kind] += byte_range.length
            self.max_range[kind] = max(self.max_range[kind], byte_range.length)
        return response

    def _validate_response(self, kind: str | None, method: str,
                           byte_range: ClosedByteRange | None, response: TransportResponse) -> None:
        if kind is None:
            if response.status != 200:
                fail(FailureCode.IDENTITY, "hosted non-COG response was not HTTP 200")
            return
        identity = COG_IDENTITIES[kind]
        if _header(response.headers, "etag") != identity.etag:
            fail(FailureCode.BOUNDS, f"{kind} ETag differs")
        if method == "HEAD":
            if response.status != 200 or _int_header(response, "content-length") != identity.content_length or response.body:
                fail(FailureCode.BOUNDS, f"{kind} HEAD identity differs")
            return
        if byte_range is None:
            fail(FailureCode.BOUNDS, "COG GET lacked its validated range")
        expected_range = f"bytes {byte_range.start}-{byte_range.end_inclusive}/{identity.content_length}"
        if (response.status != 206 or _header(response.headers, "content-range") != expected_range
                or len(response.body) != byte_range.length
                or _int_header(response, "content-length") != byte_range.length):
            fail(FailureCode.BOUNDS, f"{kind} ranged response identity differs")


def _int_header(response: TransportResponse, name: str) -> int | None:
    value = _header(response.headers, name)
    try:
        return int(value) if value is not None else None
    except ValueError:
        return None


def _validate_relative_key(key: str) -> None:
    parsed = urllib.parse.urlsplit(key)
    decoded = urllib.parse.unquote(key)
    if (parsed.scheme or parsed.netloc or parsed.query or parsed.fragment or key.startswith("/")
            or ".." in Path(decoded).parts or "%2f" in key.lower() or "%5c" in key.lower()
            or "\\" in decoded):
        fail(FailureCode.IDENTITY, "proxy routing escaped the fixed hosted prefix")


def _cog_kind(key: str) -> str | None:
    return {"aux/d8/flow_dir.tif": "flow_dir", "aux/d8/flow_acc.tif": "flow_acc"}.get(key)


class _ProxyServer(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, controller: ProxyController) -> None:
        self.controller = controller
        super().__init__(("127.0.0.1", 0), _ProxyHandler)


class _ProxyHandler(http.server.BaseHTTPRequestHandler):
    server: _ProxyServer

    def log_message(self, _format: str, *_args: Any) -> None:
        return

    def _serve(self) -> None:
        method = self.command.upper()
        parsed = urllib.parse.urlsplit(self.path)
        prefix = "/grit/hfx-v0.3.0/"
        if parsed.query or parsed.fragment or not parsed.path.startswith(prefix):
            self.send_error(400)
            return
        key = parsed.path[len(prefix):]
        try:
            if method not in {"HEAD", "GET"}:
                self.server.controller.mutation_attempt_count += 1
                self.server.controller.fatal = ProofFailure(
                    FailureCode.IDENTITY, f"mutation method {method} rejected with HTTP 405")
                self.send_error(405)
                return
            if key == "manifest.json":
                response = self.server.controller.local_candidate(method, self._loopback_url())
            else:
                response = self.server.controller.hosted(method, key, dict(self.headers.items()))
            self.send_response(response.status)
            for name in ("content-length", "content-range", "etag"):
                value = _header(response.headers, name)
                if value is not None:
                    self.send_header(name.title(), value)
            self.end_headers()
            if method == "GET":
                self.wfile.write(response.body)
        except ProofFailure as exc:
            self.server.controller.fatal = exc
            self.send_error(502)

    def _loopback_url(self) -> str:
        host, port = self.server.server_address
        return f"http://{host}:{port}{self.path}"

    do_HEAD = _serve
    do_GET = _serve
    do_POST = _serve
    do_PUT = _serve
    do_PATCH = _serve
    do_DELETE = _serve
    do_COPY = _serve
    do_MOVE = _serve
    do_PROPFIND = _serve
    do_MKCOL = _serve
    do_LOCK = _serve
    do_UNLOCK = _serve
    do_OPTIONS = _serve
    do_CONNECT = _serve
    do_TRACE = _serve


@contextlib.contextmanager
def running_proxy(controller: ProxyController) -> Iterable[str]:
    server = _ProxyServer(controller)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address
        yield f"http://{host}:{port}/grit/hfx-v0.3.0/"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def preflight_hosted(controller: ProxyController,
                     expected_identity: ObjectIdentity = PUBLISHED_MANIFEST) -> None:
    response = controller.hosted("GET", "manifest.json", {})
    if (len(response.body) != expected_identity.content_length
            or sha256_bytes(response.body) != expected_identity.sha256):
        fail(FailureCode.IDENTITY, "hosted published manifest identity differs")
    value = strict_json_bytes(response.body)
    auxiliary = value.get("auxiliary", []) if isinstance(value, dict) else []
    count = sum(isinstance(item, dict) and item.get("schema") == "hfx.aux.d8_raster.v2" for item in auxiliary)
    required_count = 1 if expected_identity == PUBLISHED_MANIFEST else 0
    if count != required_count:
        fail(FailureCode.IDENTITY, "hosted manifest D8 declaration count differs")
    for kind in ("flow_dir", "flow_acc"):
        controller.hosted("HEAD", f"aux/d8/{kind}.tif", {})


EARTH_RADIUS = 6371007.180918475
EQUAL_EARTH_X_MIN = -17243959.062216
EQUAL_EARTH_Y_MAX = 8392927.598466
EQUAL_EARTH_X_MAX = 17243959.062216
EQUAL_EARTH_Y_MIN = -8392927.598466


def equal_earth_forward(coord: tuple[float, float]) -> tuple[float, float]:
    lon, lat = map(math.radians, coord)
    theta = math.asin(math.sqrt(3) * math.sin(lat) / 2)
    theta2 = theta * theta
    denominator = 3 * (1.340264 + 3 * -0.081106 * theta2
                       + theta2 * theta2 * (7 * 0.000893 + 9 * 0.003796 * theta2))
    x = 2 * math.sqrt(3) * EARTH_RADIUS * lon * math.cos(theta) / denominator
    y = EARTH_RADIUS * theta * (1.340264 + theta2 * (-0.081106 + theta2 * (0.000893 + 0.003796 * theta2)))
    return x, y


def ordered_candidates(seed: tuple[float, float], units: list[dict[str, Any]]) -> list[tuple[float, float]]:
    seed_x, seed_y = equal_earth_forward(seed)
    px = (EQUAL_EARTH_X_MAX - EQUAL_EARTH_X_MIN) / 1070000
    py = (EQUAL_EARTH_Y_MAX - EQUAL_EARTH_Y_MIN) / 500000
    seed_col = (seed_x - EQUAL_EARTH_X_MIN) / px
    seed_row = (EQUAL_EARTH_Y_MAX - seed_y) / py
    x_seams = sorted(range(TILE_SIZE, 1070000, TILE_SIZE), key=lambda value: (abs(value - seed_col), value))[:4]
    y_seams = sorted(range(TILE_SIZE, 500000, TILE_SIZE), key=lambda value: (abs(value - seed_row), value))[:4]
    ranked: dict[int, tuple[tuple[Any, ...], tuple[float, float]]] = {}
    for unit in units:
        terminal = unit.get("id")
        outlet = unit.get("outlet")
        if type(terminal) is not int or not coordinate(outlet):
            continue
        x, y = equal_earth_forward((float(outlet[0]), float(outlet[1])))
        col = (x - EQUAL_EARTH_X_MIN) / px
        row = (EQUAL_EARTH_Y_MAX - y) / py
        dx = min(abs(col - seam) for seam in x_seams)
        dy = min(abs(row - seam) for seam in y_seams)
        in_x = dx <= BAND_HALF_WIDTH_PIXELS and abs(row - seed_row) <= BAND_HALF_LENGTH_PIXELS
        in_y = dy <= BAND_HALF_WIDTH_PIXELS and abs(col - seed_col) <= BAND_HALF_LENGTH_PIXELS
        if in_x or in_y:
            ranked[terminal] = ((not (in_x and in_y), min(dx, dy), terminal),
                                (float(outlet[0]), float(outlet[1])))
    return [item[1] for item in sorted(ranked.values(), key=lambda item: item[0])[:CANDIDATE_BUDGET]]


WORKER_SOURCE = r'''import json, math, os, pathlib, sys
def die(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)
source = pathlib.Path(os.environ["POURPOINT_WORKER_INPUT"])
target = pathlib.Path(os.environ["POURPOINT_INSTALL_TARGET"]).resolve()
result_path = pathlib.Path(os.environ["POURPOINT_WORKER_RESULT"])
value = json.loads(source.read_text(encoding="utf-8"))
sys.path.insert(0, str(target))
import pourpoint
module_path = pathlib.Path(pourpoint.__file__).resolve()
if target not in module_path.parents:
    die("released pourpoint import escaped verified install target")
input_lon, input_lat = map(float, value["input_outlet"])
class RequireD8Failure(Exception):
    pass
try:
    engine = pourpoint.Engine(value["dataset_url"], refine=True)
    level = engine.select_level(selection=pourpoint.LevelSelection.FINEST)
    try:
        outlet = engine.resolve_outlet(level, lat=input_lat, lon=input_lon)
    except pourpoint.ResolutionError as error:
        message = str(error)
        unresolved = ("no snap candidates within " in message
                      or " is outside all catchment polygons" in message)
        if not unresolved:
            raise
        print("UNRESOLVED:" + repr(error), file=sys.stderr)
        raise SystemExit(12)
    upstream = engine.traverse(outlet)
    units = engine.pre_merge_units(upstream)
    refinement = engine.refine(outlet, units)
    if refinement.status != "applied":
        raise RequireD8Failure(refinement.status)
    dissolved = engine.dissolve(units, refinement)
    result = engine.compose_result(outlet, upstream, units, refinement, dissolved)
except (RequireD8Failure, AttributeError) as error:
    print("REQUIRE_D8:" + repr(error), file=sys.stderr)
    raise SystemExit(8)
refined = getattr(result, "refined_outlet", None)
if refined is None:
    print("REQUIRE_D8:null refined outlet", file=sys.stderr)
    raise SystemExit(8)
geometry = bytes(result.geometry_wkb)
payload = {"area_km2":repr(result.area_km2),"geometry_wkb_hex":geometry.hex(),
 "input_outlet":[repr(input_lon),repr(input_lat)],"invocation_id":value["invocation_id"],
 "refined_outlet":[repr(refined[0]),repr(refined[1])],"refinement_status":"applied",
 "resolution_method":result.resolution_method,"resolved_outlet":[repr(result.resolved_outlet[0]),repr(result.resolved_outlet[1])],
 "schema":"pourpoint.released-wheel-proof-worker-result.v1","terminal_unit_id":str(result.terminal_unit_id),
 "upstream_unit_ids":[str(item) for item in result.upstream_unit_ids]}
result_path.write_text(json.dumps(payload,sort_keys=True,separators=(",",":"),allow_nan=False)+"\n",encoding="utf-8")
candidate_units=[{"id":unit.id,"outlet":[unit.outlet[0],unit.outlet[1]]} for unit in units.units]
print("POURPOINT_CANDIDATES="+json.dumps(candidate_units,sort_keys=True,separators=(",",":"),allow_nan=False))
'''


@dataclasses.dataclass
class WorkerAttempt:
    result: dict[str, Any]
    candidates: list[dict[str, Any]]
    stdout: bytes
    stderr: bytes
    trace: bytes
    cache_root: Path


class ReplayTransport:
    """In-process hosted-response replay used only by offline controller tests."""

    def __init__(self, manifest: bytes) -> None:
        self.manifest = manifest
        self.calls: list[tuple[str, str, dict[str, str]]] = []

    def request(self, method: str, url: str, headers: dict[str, str]) -> TransportResponse:
        self.calls.append((method, url, dict(headers)))
        if url == HOSTED_BASE + "manifest.json" and method == "GET":
            return TransportResponse(200, {"content-length": str(len(self.manifest))}, self.manifest)
        kind = "flow_dir" if url.endswith("flow_dir.tif") else "flow_acc" if url.endswith("flow_acc.tif") else None
        if kind is None:
            fail(FailureCode.IDENTITY, "replay request escaped its recorded response set")
        identity = COG_IDENTITIES[kind]
        if method == "HEAD":
            return TransportResponse(200, {"content-length": str(identity.content_length),
                                           "etag": identity.etag or ""}, b"")
        byte_range = parse_closed_range(headers.get("Range"), identity.content_length)
        body = bytes(byte_range.length)
        return TransportResponse(206, {"content-length": str(len(body)), "etag": identity.etag or "",
                                       "content-range": f"bytes {byte_range.start}-{byte_range.end_inclusive}/{identity.content_length}"}, body)


def _write_fixture_tiff(path: Path, sample_type: str, samples: list[int | float],
                        width: int, height: int, origin_y: float, pixel_height: float) -> None:
    if width * height != len(samples) or sample_type not in {"U8", "F32"}:
        raise AssertionError("invalid TIFF fixture")
    pixel_data = bytes(samples) if sample_type == "U8" else struct.pack("<" + "f" * len(samples), *samples)
    entries: list[tuple[int, int, int, int]] = []
    entry_count = 13
    values_start = 8 + 2 + entry_count * 12 + 4
    scale_offset = values_start
    tie_offset = scale_offset + 24
    pixels_offset = tie_offset + 48
    def add(tag: int, field_type: int, count: int, value: int) -> None:
        entries.append((tag, field_type, count, value))
    add(256, 4, 1, width); add(257, 4, 1, height)
    add(258, 3, 1, 8 if sample_type == "U8" else 32); add(259, 3, 1, 1)
    add(262, 3, 1, 1); add(273, 4, 1, pixels_offset); add(277, 3, 1, 1)
    add(278, 4, 1, height); add(279, 4, 1, len(pixel_data)); add(284, 3, 1, 1)
    add(339, 3, 1, 1 if sample_type == "U8" else 3)
    add(33550, 12, 3, scale_offset); add(33922, 12, 6, tie_offset)
    output = bytearray(b"II" + struct.pack("<HIH", 42, 8, entry_count))
    for tag, field_type, count, value in sorted(entries):
        output.extend(struct.pack("<HHI", tag, field_type, count))
        if field_type == 3 and count == 1:
            output.extend(struct.pack("<H", value) + b"\0\0")
        else:
            output.extend(struct.pack("<I", value))
    output.extend(struct.pack("<I", 0))
    output.extend(struct.pack("<ddd", 1.0, abs(pixel_height), 0.0))
    output.extend(struct.pack("<dddddd", 0.0, 0.0, 0.0, 0.0, origin_y, 0.0))
    output.extend(pixel_data)
    path.write_bytes(output)


def exercise_injected_controller(root: Path) -> None:
    """Drive preflight, ranges, production decode, telemetry, and evidence guards offline."""
    manifest = canonical_json({"auxiliary": []})
    replay = ReplayTransport(manifest)
    proxy = ProxyController(CaseMode.HORIZONTAL, POSITIVE_CANDIDATE, replay)
    preflight_hosted(proxy, ObjectIdentity(len(manifest), sha256=sha256_bytes(manifest)))
    for kind in ("flow_dir", "flow_acc"):
        proxy.hosted("GET", f"aux/d8/{kind}.tif", {"Range": "bytes=10-24"})
    cache = root / "hfx-cache" / "attempt-1"
    cache.mkdir(parents=True)
    origin_y = EQUAL_EARTH_Y_MAX - 511.0
    dir_path, acc_path = cache / "dir.tif", cache / "acc.tif"
    _write_fixture_tiff(dir_path, "U8", [1, 2, 4, 8], 2, 2, origin_y, -1.0)
    _write_fixture_tiff(acc_path, "F32", [1.0, 2.0, 3.0, 4.0], 2, 2, origin_y, -1.0)
    records = [{"bytes": path.stat().st_size, "cache_status": "fetched", "duration_ms": 0.0,
                "kind": "stage", "path": str(path), "requests": 1, "stage": stage,
                "thread": "ThreadId(1)", "timestamp": 1}
               for path, stage in ((dir_path, "raster_localize_flow_dir"),
                                   (acc_path, "raster_localize_flow_acc"))]
    trace = b"".join(canonical_json(record) for record in records)
    square = simple_multipolygon([[[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0),
                                    (0.0, 1.0), (0.0, 0.0)]]])
    result = {"area_km2": "1.0", "geometry_wkb_hex": square.hex(),
              "input_outlet": [repr(ZURICH_SEED[0]), repr(ZURICH_SEED[1])],
              "invocation_id": "0" * 32, "refined_outlet": ["8.5", "47.3"],
              "refinement_status": "applied", "resolution_method": "Snap",
              "resolved_outlet": ["8.5", "47.3"],
              "schema": "pourpoint.released-wheel-proof-worker-result.v1",
              "terminal_unit_id": "1", "upstream_unit_ids": ["1"]}
    attempt = WorkerAttempt(result, [{"id": 1, "outlet": list(ZURICH_SEED)}], b"", b"", trace, cache)
    telemetry, windows, decoded = candidate_acceptance_predicate(attempt)
    ceilings = _ceiling_evidence(windows, decoded, _worker_raster_observations(attempt))
    assert telemetry["accepted_trace_line_numbers"] == [1, 2]
    assert all(window["horizontal_seam_row"] == 1 for window in windows.values())
    assert all(observation["margin"] >= 1 for values in ceilings.values() for observation in values.values())
    assert proxy.completed_hosted_reads == 5 and len(replay.calls) == 5
    fixture = {"fixture_kind": "synthetic", "telemetry": telemetry, "windows": windows,
               "ceilings": ceilings, "ordered_candidates": ordered_candidates(ZURICH_SEED, attempt.candidates)}
    _assert_rejected(lambda: validate_live_evidence(fixture), FailureCode.EVIDENCE)


def install_wheel(wheel: Path, target: Path, stdout_path: Path, stderr_path: Path) -> None:
    command = [sys.executable, "-m", "pip", "install", "--no-index", "--no-deps",
               "--only-binary=:all:", "--disable-pip-version-check", "--target", str(target), str(wheel)]
    try:
        completed = subprocess.run(command, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=_sanitized_environment({"PYTHONNOUSERSITE": "1", "PIP_NO_INDEX": "1",
                                                               "PIP_DISABLE_PIP_VERSION_CHECK": "1"}))
    except OSError as exc:
        fail(FailureCode.WORKER, f"offline wheel installation failed to launch: {exc}")
    stdout_path.write_bytes(completed.stdout)
    stderr_path.write_bytes(completed.stderr)
    if completed.returncode != 0:
        fail(FailureCode.WORKER, f"offline wheel installation failed with status {completed.returncode}")


def _sanitized_environment(
        updates: dict[str, str], ambient_environment: dict[str, str] | None = None,
) -> dict[str, str]:
    source = os.environ if ambient_environment is None else ambient_environment
    env = {key: value for key, value in source.items() if not key.startswith("AWS_")}
    env.update(updates)
    env["AWS_EC2_METADATA_DISABLED"] = "true"
    env["PYTHONNOUSERSITE"] = "1"
    env["PIP_NO_INDEX"] = "1"
    env["PIP_DISABLE_PIP_VERSION_CHECK"] = "1"
    return env


def require_hosted_worker_source(dataset_url: str) -> str:
    if dataset_url != HOSTED_BASE:
        fail(FailureCode.CONFIG, "released wheel requires the exact hosted base")
    return dataset_url


def worker_input_payload(case: CaseMode, input_outlet: tuple[float, float],
                         invocation_id: str, dataset_url: str) -> dict[str, Any]:
    source = require_hosted_worker_source(dataset_url)
    return {"case": case.value,
            "dataset_url": source,
            "input_outlet": [repr(input_outlet[0]), repr(input_outlet[1])],
            "invocation_id": invocation_id,
            "schema": "pourpoint.released-wheel-proof-worker-input.v1"}


def run_worker(temporary: Path, install_target: Path, dataset_url: str, case: CaseMode,
               input_outlet: tuple[float, float], attempt_number: int, *,
               ambient_environment: dict[str, str] | None = None) -> WorkerAttempt:
    require_hosted_worker_source(dataset_url)
    invocation_id = uuid.uuid4().hex
    worker_input = temporary / "worker-input.json"
    worker_result = temporary / "worker-result.json"
    trace_path = temporary / "staging" / "trace.jsonl"
    cache_root = temporary / "hfx-cache" / f"attempt-{attempt_number}"
    xdg_root = temporary / "xdg-cache" / f"attempt-{attempt_number}"
    cache_root.mkdir(parents=True)
    xdg_root.mkdir(parents=True)
    if worker_result.exists():
        worker_result.unlink()
    trace_path.write_bytes(b"")
    payload = worker_input_payload(case, input_outlet, invocation_id, dataset_url)
    worker_input.write_bytes(canonical_json(payload))
    env = _sanitized_environment({"HFX_CACHE_DIR": str(cache_root.resolve()),
                                  "XDG_CACHE_HOME": str(xdg_root.resolve()),
                                  "POURPOINT_BENCH_TRACE": str(trace_path.resolve()),
                                  "POURPOINT_INSTALL_TARGET": str(install_target.resolve()),
                                  "POURPOINT_WORKER_INPUT": str(worker_input.resolve()),
                                  "POURPOINT_WORKER_RESULT": str(worker_result.resolve()),
                                  "PYTHONPATH": str(install_target.resolve())},
                                 ambient_environment)
    try:
        completed = subprocess.run([sys.executable, "-I", "-c", WORKER_SOURCE], check=False,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    except OSError as exc:
        fail(FailureCode.WORKER, f"released worker failed to launch: {exc}")
    if completed.returncode == int(FailureCode.REQUIRE_D8):
        fail(FailureCode.REQUIRE_D8, completed.stderr.decode("utf-8", "replace").strip())
    if completed.returncode == int(FailureCode.UNRESOLVED):
        stderr = completed.stderr.decode("utf-8", "replace").strip()
        if not any(line.startswith("UNRESOLVED:") for line in stderr.splitlines()):
            fail(FailureCode.WORKER,
                 f"unresolved status lacks worker protocol marker: {stderr}")
        fail(FailureCode.UNRESOLVED, stderr)
    if completed.returncode != 0:
        fail(FailureCode.WORKER, f"released worker failed with status {completed.returncode}: "
             + completed.stderr.decode("utf-8", "replace").strip())
    try:
        result = strict_json_bytes(worker_result.read_bytes(), canonical=True)
    except OSError as exc:
        fail(FailureCode.WORKER, f"worker protocol result is absent: {exc}")
    _validate_worker_result(result, invocation_id, input_outlet)
    marker = b"POURPOINT_CANDIDATES="
    lines = [line for line in completed.stdout.splitlines() if line.startswith(marker)]
    if len(lines) != 1:
        fail(FailureCode.WORKER, "worker candidate protocol is absent or ambiguous")
    try:
        candidates = json.loads(lines[0][len(marker):])
    except json.JSONDecodeError as exc:
        fail(FailureCode.WORKER, f"worker candidate protocol is malformed: {exc}")
    return WorkerAttempt(result, candidates, completed.stdout, completed.stderr,
                         trace_path.read_bytes(), cache_root)


def _validate_worker_result(value: Any, invocation_id: str, expected_input: tuple[float, float]) -> None:
    keys = {"area_km2", "geometry_wkb_hex", "input_outlet", "invocation_id", "refined_outlet",
            "refinement_status", "resolution_method", "resolved_outlet", "schema", "terminal_unit_id",
            "upstream_unit_ids"}
    if not isinstance(value, dict) or set(value) != keys or value.get("schema") != "pourpoint.released-wheel-proof-worker-result.v1":
        fail(FailureCode.WORKER, "worker result schema differs")
    if value.get("invocation_id") != invocation_id or tuple(map(float, value.get("input_outlet", []))) != expected_input:
        fail(FailureCode.WORKER, "worker result invocation identity differs")
    validate_require_d8(value.get("refinement_status"), list(map(float, value.get("refined_outlet", []))))
    geometry = value.get("geometry_wkb_hex")
    if not isinstance(geometry, str) or not re.fullmatch(r"(?:[0-9a-f]{2})+", geometry):
        fail(FailureCode.WORKER, "worker geometry protocol differs")


def validate_distant_seed_declaration(value: Any) -> tuple[float, float]:
    """Parse one sealed discovery seed and its released-wheel resolution proof."""
    required = {"dataset", "discovery_seed", "resolution", "schema", "wheel"}
    if (not isinstance(value, dict) or set(value) != required
            or value.get("schema") != DISTANT_SEED_SCHEMA):
        fail(FailureCode.EVIDENCE, "distant seed declaration schema differs")
    seed = value.get("discovery_seed")
    if not coordinate(seed):
        fail(FailureCode.EVIDENCE, "distant discovery seed coordinate differs")
    dataset = value.get("dataset")
    expected_dataset = {
        "base": HOSTED_BASE,
        "manifest": {"byte_count": PUBLISHED_MANIFEST.content_length,
                     "sha256": PUBLISHED_MANIFEST.sha256},
    }
    if dataset != expected_dataset:
        fail(FailureCode.EVIDENCE, "distant seed hosted dataset identity differs")
    wheel = value.get("wheel")
    wheel_keys = {"filename", "metadata_name", "metadata_requires_python",
                  "metadata_version", "sha256", "size_bytes"}
    if not isinstance(wheel, dict) or set(wheel) != wheel_keys:
        fail(FailureCode.EVIDENCE, "distant seed wheel identity schema differs")
    allowed = WHEEL_ALLOWLIST.get(wheel.get("filename"))
    if (allowed is None or (wheel.get("size_bytes"), wheel.get("sha256")) != allowed
            or wheel.get("metadata_name") != "pourpoint"
            or wheel.get("metadata_version") != "0.3.0"
            or wheel.get("metadata_requires_python") != ">=3.9"):
        fail(FailureCode.EVIDENCE, "distant seed released wheel identity differs")
    result = value.get("resolution")
    if not isinstance(result, dict):
        fail(FailureCode.EVIDENCE, "distant seed retained resolution record is absent")
    invocation_id = result.get("invocation_id")
    if not isinstance(invocation_id, str) or not re.fullmatch(r"[0-9a-f]{32}", invocation_id):
        fail(FailureCode.EVIDENCE, "distant seed resolution invocation identity differs")
    try:
        _validate_worker_result(result, invocation_id,
                                (float(seed[0]), float(seed[1])))
    except (TypeError, ValueError) as exc:
        fail(FailureCode.EVIDENCE,
             f"distant seed worker result value differs: {exc}")
    area = result.get("area_km2")
    try:
        parsed_area = float(area)
    except (TypeError, ValueError) as exc:
        fail(FailureCode.EVIDENCE,
             f"distant seed resolved area differs: {exc}")
    if not isinstance(area, str) or not math.isfinite(parsed_area) or parsed_area <= 0:
        fail(FailureCode.EVIDENCE, "distant seed resolved area differs")
    method = result.get("resolution_method")
    if not isinstance(method, str) or not method:
        fail(FailureCode.EVIDENCE, "distant seed resolution method differs")
    try:
        resolved_outlet = list(map(float, result.get("resolved_outlet", [])))
    except (TypeError, ValueError) as exc:
        fail(FailureCode.EVIDENCE,
             f"distant seed resolved outlet differs: {exc}")
    if not coordinate(resolved_outlet):
        fail(FailureCode.EVIDENCE, "distant seed resolved outlet differs")
    terminal = result.get("terminal_unit_id")
    upstream = result.get("upstream_unit_ids")
    if (not isinstance(terminal, str) or not terminal
            or not isinstance(upstream, list) or not upstream
            or any(not isinstance(item, str) or not item for item in upstream)
            or len(set(upstream)) != len(upstream) or terminal not in upstream):
        fail(FailureCode.EVIDENCE,
             "distant seed resolved unit identities differ")
    geometry = result.get("geometry_wkb_hex")
    if not isinstance(geometry, str):
        fail(FailureCode.EVIDENCE, "distant seed resolved geometry differs")
    canonical_wkb(bytes.fromhex(geometry))
    return float(seed[0]), float(seed[1])


def read_distant_seed_declaration(path: Path = DISTANT_SEED_DECLARATION) -> tuple[dict[str, Any], tuple[float, float]]:
    try:
        value = strict_json_bytes(path.read_bytes(), canonical=True)
    except OSError as exc:
        fail(FailureCode.EVIDENCE, f"sealed distant discovery seed declaration is unreadable at {path}: {exc}")
    seed = validate_distant_seed_declaration(value)
    return value, seed


def run_distant_seed_discovery(
        declaration_path: Path, temporary: Path, install_target: Path,
        attempt_number: int,
        worker: Callable[[Path, Path, str, CaseMode, tuple[float, float], int], WorkerAttempt] = run_worker,
) -> WorkerAttempt:
    """Run discovery from the sealed seed and name that authority on failure."""
    _, seed = read_distant_seed_declaration(declaration_path)
    try:
        return worker(temporary, install_target, HOSTED_BASE, CaseMode.DISTANT,
                      seed, attempt_number)
    except ProofFailure as exc:
        if exc.code is FailureCode.UNRESOLVED:
            fail(FailureCode.UNRESOLVED,
                 f"distant discovery seed {list(seed)} from sealed declaration "
                 f"{declaration_path} is unresolvable: {exc}")
        raise


def validate_candidate_rejections(value: Any, selected_rank: Any,
                                  case_value: Any) -> None:
    if not isinstance(value, list) or type(selected_rank) is not int or selected_rank < 1:
        fail(FailureCode.EVIDENCE, "candidate rejection ledger shape differs")
    if len(value) != selected_rank - 1:
        fail(FailureCode.EVIDENCE, "candidate dropped without a recorded rejection reason")
    allowed_codes = {
        CaseMode.HORIZONTAL.value: {CandidateRejectionCode.BOUNDS.value,
                                    CandidateRejectionCode.REQUIRE_D8.value,
                                    CandidateRejectionCode.UNRESOLVED.value},
        CaseMode.DISTANT.value: {item.value for item in CandidateRejectionCode},
        CaseMode.NEGATIVE.value: set(),
    }.get(case_value)
    if allowed_codes is None:
        fail(FailureCode.EVIDENCE, "candidate rejection case differs")
    for expected_rank, entry in enumerate(value, 1):
        if (not isinstance(entry, dict)
                or set(entry) != {"coordinate", "rank", "rejection_code"}
                or entry.get("rank") != expected_rank
                or not coordinate(entry.get("coordinate"))
                or entry.get("rejection_code") not in allowed_codes):
            fail(FailureCode.EVIDENCE, "candidate rejection ledger entry differs")


def validate_live_evidence(value: Any) -> None:
    if not isinstance(value, dict) or "fixture_kind" in value:
        fail(FailureCode.EVIDENCE, "synthetic evidence cannot satisfy live proof")
    required = {"case", "candidate", "ceilings", "geometry", "hosted", "invocation", "mutation_attempt_count", "refinement", "result", "schema", "selection", "telemetry", "wheel", "windows"}
    if set(value) != required or value.get("schema") != "pourpoint.released-wheel-proof-evidence.v1":
        fail(FailureCode.EVIDENCE, "live evidence schema or key set differs")
    hosted = value.get("hosted", {})
    legacy_reads = hosted.get("completed_network_reads")
    preflight_reads = hosted.get("completed_preflight_reads")
    worker_reads = hosted.get("released_worker_raster_reads")
    legacy_shape = type(legacy_reads) is int and legacy_reads > 0
    traced_shape = (type(preflight_reads) is int and preflight_reads > 0
                    and type(worker_reads) is int and worker_reads > 0
                    and "completed_network_reads" not in hosted)
    if legacy_shape == traced_shape:
        fail(FailureCode.EVIDENCE, "live evidence hosted-read scope differs")
    invocation = value.get("invocation", {})
    selection = value.get("selection")
    selection_keys = {"candidate_budget", "candidate_rejections",
                      "horizontal_seam_crossed", "minimum_distant_metres",
                      "mode", "ordered_candidates_tried",
                      "selected_distance_from_horizontal_metres"}
    if (not isinstance(selection, dict)
            or set(selection) not in {frozenset(selection_keys),
                                      frozenset(selection_keys | {"seed_probe_rejection"})}):
        fail(FailureCode.EVIDENCE, "selection schema or key set differs")
    seed_rejection = selection.get("seed_probe_rejection")
    if seed_rejection is not None:
        allowed = {CandidateRejectionCode.BOUNDS.value,
                   CandidateRejectionCode.REQUIRE_D8.value}
        if (not isinstance(seed_rejection, dict)
                or set(seed_rejection) != {"coordinate", "rejection_code"}
                or seed_rejection.get("coordinate") != invocation.get("seed")
                or seed_rejection.get("rejection_code") not in allowed
                or value.get("case") == CaseMode.NEGATIVE.value):
            fail(FailureCode.EVIDENCE, "seed probe rejection differs")
    selected_rank = invocation.get("candidate_rank")
    if selection.get("ordered_candidates_tried") != selected_rank:
        fail(FailureCode.EVIDENCE, "selection tried count differs from selected rank")
    validate_candidate_rejections(selection.get("candidate_rejections"),
                                  selected_rank, value.get("case"))


@dataclasses.dataclass(frozen=True)
class DecodedWindow:
    width: int
    height: int
    sample_type: str
    samples: list[int | float]
    origin_x: float
    origin_y: float
    pixel_width: float
    pixel_height: float
    decoded_chunk_bytes: int


def _tiff_values(data: bytes, entry: tuple[int, int, int, int], order: str,
                 big: bool) -> list[int | float]:
    _tag, field_type, count, value_offset = entry
    widths = {1: 1, 2: 1, 3: 2, 4: 4, 8: 2, 11: 4, 12: 8, 16: 8, 17: 8, 18: 8}
    width = widths.get(field_type)
    if width is None or count > 10_000_000:
        fail(FailureCode.BOUNDS, "localized TIFF field type or count is unsupported")
    byte_count = width * count
    inline_width = 8 if big else 4
    entry_width = 20 if big else 12
    value_field_offset = value_offset + entry_width - inline_width
    if byte_count <= inline_width:
        raw = data[value_field_offset:value_field_offset + byte_count]
    else:
        pointer_format = order + ("Q" if big else "I")
        pointer = struct.unpack_from(pointer_format, data, value_field_offset)[0]
        raw = data[pointer:pointer + byte_count]
    if len(raw) != byte_count:
        fail(FailureCode.BOUNDS, "localized TIFF field is truncated")
    formats = {1: "B", 2: "B", 3: "H", 4: "I", 8: "h", 11: "f", 12: "d",
               16: "Q", 17: "q", 18: "Q"}
    return list(struct.unpack(order + formats[field_type] * count, raw))


def decode_local_tiff(path: Path, expected_kind: str) -> DecodedWindow:
    try:
        data = path.read_bytes()
    except OSError as exc:
        fail(FailureCode.BOUNDS, f"production TIFF is unreadable: {exc}")
    if len(data) < 8 or data[:2] not in {b"II", b"MM"}:
        fail(FailureCode.BOUNDS, "localized TIFF byte order is invalid")
    order = "<" if data[:2] == b"II" else ">"
    magic = struct.unpack_from(order + "H", data, 2)[0]
    big = magic == 43
    if magic not in {42, 43}:
        fail(FailureCode.BOUNDS, "localized TIFF magic is unsupported")
    if big:
        if len(data) < 16 or struct.unpack_from(order + "HH", data, 4) != (8, 0):
            fail(FailureCode.BOUNDS, "localized BigTIFF header is invalid")
        ifd = struct.unpack_from(order + "Q", data, 8)[0]
        count_width, entry_width = 8, 20
        if ifd + 8 > len(data):
            fail(FailureCode.BOUNDS, "localized BigTIFF IFD is truncated")
        count = struct.unpack_from(order + "Q", data, ifd)[0]
    else:
        ifd = struct.unpack_from(order + "I", data, 4)[0]
        count_width, entry_width = 2, 12
        if ifd + 2 > len(data):
            fail(FailureCode.BOUNDS, "localized TIFF IFD is truncated")
        count = struct.unpack_from(order + "H", data, ifd)[0]
    if count == 0 or count > 4096 or ifd + count_width + count * entry_width > len(data):
        fail(FailureCode.BOUNDS, "localized TIFF IFD count is invalid")
    entries: dict[int, tuple[int, int, int, int]] = {}
    count_format = "Q" if big else "I"
    for index in range(count):
        offset = ifd + count_width + index * entry_width
        tag, field_type = struct.unpack_from(order + "HH", data, offset)
        item_count = struct.unpack_from(order + count_format, data, offset + 4)[0]
        if tag in entries:
            fail(FailureCode.BOUNDS, "localized TIFF contains duplicate tags")
        entries[tag] = (tag, field_type, item_count, offset)

    def scalar(tag: int, default: int | None = None) -> int:
        entry = entries.get(tag)
        if entry is None:
            if default is None:
                fail(FailureCode.BOUNDS, f"localized TIFF is missing tag {tag}")
            return default
        values = _tiff_values(data, entry, order, big)
        if len(values) != 1 or type(values[0]) not in {int, float}:
            fail(FailureCode.BOUNDS, f"localized TIFF scalar tag {tag} differs")
        return int(values[0])

    width, height = scalar(256), scalar(257)
    bits = scalar(258)
    compression = scalar(259, 1)
    samples_per_pixel = scalar(277, 1)
    sample_format = scalar(339, 1)
    if width <= 0 or height <= 0 or compression != 1 or samples_per_pixel != 1:
        fail(FailureCode.BOUNDS, "localized TIFF dimensions/compression/layout differ")
    sample_type = "U8" if (bits, sample_format) == (8, 1) else "F32" if (bits, sample_format) == (32, 3) else ""
    expected_type = "U8" if expected_kind == "flow_dir" else "F32"
    if sample_type != expected_type:
        fail(FailureCode.BOUNDS, f"localized {expected_kind} TIFF sample type differs")
    offsets_tag, counts_tag = (273, 279) if 273 in entries else (324, 325)
    offsets = [int(value) for value in _tiff_values(data, entries[offsets_tag], order, big)]
    counts = [int(value) for value in _tiff_values(data, entries[counts_tag], order, big)]
    if len(offsets) != len(counts) or not offsets:
        fail(FailureCode.BOUNDS, "localized TIFF chunk indexes differ")
    sample_width = bits // 8
    allocation = width * height * sample_width
    observe_ceiling(allocation, MAX_WINDOW_ALLOCATION_BYTES)
    chunks = bytearray()
    max_decoded = 0
    for offset, byte_count in zip(offsets, counts):
        observe_ceiling(byte_count, MAX_DECODED_CHUNK_BYTES)
        if offset < 0 or byte_count < 0 or offset + byte_count > len(data):
            fail(FailureCode.BOUNDS, "localized TIFF chunk is truncated")
        chunks.extend(data[offset:offset + byte_count])
        max_decoded = max(max_decoded, byte_count)
    if len(chunks) != allocation:
        fail(FailureCode.BOUNDS, "localized TIFF decoded size differs")
    fmt = "B" if sample_type == "U8" else "f"
    samples = list(struct.unpack(order + fmt * (width * height), chunks))
    scale = _tiff_values(data, entries.get(33550, (33550, 12, 3, -1)), order, big) if 33550 in entries else []
    tie = _tiff_values(data, entries.get(33922, (33922, 12, 6, -1)), order, big) if 33922 in entries else []
    if (len(scale) < 2 or len(tie) < 6
            or float(scale[0]) <= 0 or float(scale[1]) <= 0):
        fail(FailureCode.BOUNDS, "localized TIFF geotransform differs")
    origin_x = float(tie[3]) - float(tie[0]) * float(scale[0])
    origin_y = float(tie[4]) + float(tie[1]) * float(scale[1])
    return DecodedWindow(width, height, sample_type, samples, origin_x, origin_y,
                         float(scale[0]), -float(scale[1]), max_decoded)


def _window_evidence(window: DecodedWindow, kind: str) -> tuple[dict[str, Any], int]:
    global_row = (EQUAL_EARTH_Y_MAX - window.origin_y) / abs(window.pixel_height)
    first_seam = math.floor(global_row / TILE_SIZE + 1) * TILE_SIZE
    seam = round(first_seam - global_row)
    validate_window({"width": window.width, "height": window.height,
                     "sample_type": window.sample_type, "samples": window.samples, "seam": seam})
    if kind == "flow_dir":
        values = [int(value) for value in window.samples]
        legal = sum(value in {1, 2, 3, 4, 5, 6, 7, 8} for value in values)
        if legal <= 0:
            fail(FailureCode.BOUNDS, "production flow-direction window has no legal GRASS sample")
        evidence = {"distinct_values": sorted(set(values)), "height": window.height,
                    "horizontal_seam_row": seam, "legal_grass_non_nodata_count": legal,
                    "nodata_255_count": values.count(255), "sample_type": "U8",
                    "source": "production_localization_trace_path", "width": window.width}
    else:
        real = [float(value) for value in window.samples if math.isfinite(float(value))]
        if not real or max(real) <= 0:
            fail(FailureCode.BOUNDS, "production accumulation window has no positive real sample")
        evidence = {"height": window.height, "horizontal_seam_row": seam,
                    "nan_count": len(window.samples) - len(real), "non_nan_count": len(real),
                    "non_nan_max": max(real), "non_nan_min": min(real), "sample_type": "F32",
                    "source": "production_localization_trace_path", "width": window.width}
    return evidence, window.decoded_chunk_bytes


def parse_trace_jsonl(data: bytes) -> list[dict[str, Any]]:
    records = []
    for raw in data.splitlines(keepends=True):
        if not raw.endswith(b"\n"):
            fail(FailureCode.EVIDENCE, "truncated trace JSONL")
        value = strict_json_bytes(raw)
        if not isinstance(value, dict):
            fail(FailureCode.EVIDENCE, "trace line is not an object")
        records.append(value)
    if not records:
        fail(FailureCode.REQUIRE_D8, "production trace is empty")
    return records


def trace_cache_relative(raw: Any, label: str) -> tuple[Path, str]:
    if not isinstance(raw, str):
        fail(FailureCode.EVIDENCE, f"{label} trace path is not a string")
    normalized = raw.replace("\\", "/")
    path = Path(normalized)
    if not path.is_absolute() or ".." in path.parts or path.parts.count("hfx-cache") != 1:
        fail(FailureCode.EVIDENCE, f"{label} trace path is not an absolute case cache path")
    cache_index = path.parts.index("hfx-cache")
    relative_parts = path.parts[cache_index + 1:]
    if not relative_parts:
        fail(FailureCode.EVIDENCE, f"{label} trace path does not name a localized file")
    cache_root = Path(*path.parts[:cache_index + 1])
    return cache_root, "hfx-cache/" + "/".join(relative_parts)


def _attempt_measurements(attempt: WorkerAttempt) -> tuple[dict[str, Any], dict[str, Any], dict[str, int]]:
    records = parse_trace_jsonl(attempt.trace)
    line_dir, line_acc = validate_trace(records, attempt.cache_root, attempt.result["invocation_id"])
    telemetry: dict[str, Any] = {"accepted_trace_line_numbers": [line_dir, line_acc]}
    windows: dict[str, Any] = {}
    decoded: dict[str, int] = {}
    for kind, line in (("flow_dir", line_dir), ("flow_acc", line_acc)):
        record = records[line - 1]
        path = Path(record["path"])
        window = decode_local_tiff(path, kind)
        windows[kind], decoded[kind] = _window_evidence(window, kind)
        try:
            path.resolve().relative_to(attempt.cache_root.resolve())
        except (OSError, ValueError):
            fail(FailureCode.BOUNDS, "production trace path escapes accepted cache")
        _, relative = trace_cache_relative(str(path.resolve()), kind)
        bytes_value, requests = record.get("bytes"), record.get("requests")
        if type(bytes_value) is not int or bytes_value <= 0 or type(requests) is not int or requests <= 0:
            fail(FailureCode.BOUNDS, "production trace measurements are not positive integers")
        telemetry[kind] = {"bytes": bytes_value, "cache_status": "fetched",
                           "path": relative, "requests": requests}
    return telemetry, windows, decoded


def candidate_acceptance_predicate(
        attempt: WorkerAttempt,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, int]]:
    """Apply the production bounds and RequireD8 measurement predicate."""
    return _attempt_measurements(attempt)


def qualify_seed_probe(
        attempt: WorkerAttempt,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, int]]:
    """Qualify discovery only when it passes the live candidate predicate."""
    return candidate_acceptance_predicate(attempt)


def candidates_after_seed_probe(
        seed: tuple[float, float], discovery: WorkerAttempt, case: CaseMode,
) -> tuple[list[tuple[float, float]], dict[str, Any] | None]:
    """Apply the live predicate to the seed, then order discovery candidates.

    A bounds or RequireD8 rejection disqualifies the seed as a result.  It does
    not discard the discovery response, whose units still define the ordered
    candidate search.  A qualified seed is ranked first because the live
    predicate has already established it as the strongest candidate.
    """
    rejection: dict[str, Any] | None = None
    try:
        qualify_seed_probe(discovery)
    except ProofFailure as exc:
        code = candidate_rejection_code(exc, case)
        if code is None:
            raise
        rejection = {"coordinate": list(map(float, seed)),
                     "rejection_code": code.value}
    candidates = ordered_candidates(seed, discovery.candidates)
    if rejection is not None:
        candidates = [candidate for candidate in candidates if candidate != seed]
    else:
        candidates = [seed, *(candidate for candidate in candidates if candidate != seed)]
    return candidates, rejection


def select_ordered_candidate(
        candidates: list[tuple[float, float]], candidate_budget: int,
        case: CaseMode, horizontal: tuple[float, float] | None,
        attempt_candidate: Callable[[tuple[float, float], int], WorkerAttempt],
) -> tuple[WorkerAttempt, int, float | None, CandidateDiagnostics]:
    """Attempt ranked candidates while isolating only declared rejections."""
    diagnostics = CandidateDiagnostics()
    for rank, candidate_outlet in enumerate(candidates[:candidate_budget], 1):
        candidate_coord = (float(candidate_outlet[0]), float(candidate_outlet[1]))
        diagnostics.start(rank, candidate_coord)
        candidate_distance = None
        if case is CaseMode.DISTANT:
            if horizontal is None:
                fail(FailureCode.EVIDENCE, "distant horizontal reference is absent")
            try:
                candidate_distance = require_distant(horizontal, candidate_coord)
            except ProofFailure as exc:
                if exc.code == FailureCode.EXHAUSTED:
                    diagnostics.reject(CandidateRejectionCode.DISTANCE)
                    continue
                raise
        try:
            attempt = attempt_candidate(candidate_coord, rank)
            candidate_acceptance_predicate(attempt)
        except ProofFailure as exc:
            rejection_code = candidate_rejection_code(exc, case)
            if rejection_code is not None:
                diagnostics.reject(rejection_code)
                continue
            raise
        diagnostics.accept()
        return attempt, rank, candidate_distance, diagnostics
    diagnostics.exhausted()


def _worker_comparison(result: dict[str, Any], geometry_sha: str) -> dict[str, Any]:
    return {"input": list(map(float, result["input_outlet"])), "terminal": int(result["terminal_unit_id"]),
            "upstream": sorted(set(map(int, result["upstream_unit_ids"]))),
            "resolved": list(map(float, result["resolved_outlet"])),
            "refined": list(map(float, result["refined_outlet"])), "status": result["refinement_status"],
            "geometry_sha256": geometry_sha}


def _candidate_evidence(case: CaseMode, candidate: bytes) -> dict[str, Any]:
    if case is CaseMode.NEGATIVE:
        value, difference = verify_negative_candidate(candidate)
    else:
        value, difference = verify_positive_candidate(candidate), []
    return {"byte_count": len(candidate), "difference_from_positive": difference,
            "flow_dir_encoding": value["auxiliary"][2]["metadata"]["flow_dir_encoding"],
            "sha256": sha256_bytes(candidate)}


def _worker_raster_observations(attempt: WorkerAttempt) -> dict[str, dict[str, int]]:
    records = parse_trace_jsonl(attempt.trace)
    line_dir, line_acc = validate_trace(
        records, attempt.cache_root, attempt.result.get("invocation_id"))
    output = {}
    for kind, line in (("flow_dir", line_dir), ("flow_acc", line_acc)):
        record = records[line - 1]
        requests, bytes_value = record.get("requests"), record.get("bytes")
        if (type(requests) is not int or requests <= 0
                or type(bytes_value) is not int or bytes_value <= 0):
            fail(FailureCode.ZERO_READS,
                 "authorization present but zero hosted network operations completed by released worker")
        output[kind] = {"bytes": bytes_value, "requests": requests}
    return output


def _ceiling_evidence(windows: dict[str, Any], decoded: dict[str, int],
                      worker: dict[str, dict[str, int]]) -> dict[str, Any]:
    output = {}
    for kind in ("flow_dir", "flow_acc"):
        sample_width = 1 if kind == "flow_dir" else 4
        allocation = windows[kind]["width"] * windows[kind]["height"] * sample_width
        fetched_bytes = worker[kind]["bytes"]
        output[kind] = {
            "covered_chunk_bytes": observe_ceiling(fetched_bytes, MAX_COVERED_CHUNK_BYTES).evidence(),
            "decoded_chunk_bytes": observe_ceiling(decoded[kind], MAX_DECODED_CHUNK_BYTES).evidence(),
            "planned_tile_count": observe_ceiling(worker[kind]["requests"], MAX_PLANNED_TILE_COUNT).evidence(),
            # The trace exposes total fetched bytes, not each range length. Treating the total as
            # the largest range is a conservative upper bound for every compressed chunk request.
            "single_compressed_chunk_bytes": observe_ceiling(fetched_bytes, MAX_COMPRESSED_CHUNK_BYTES).evidence(),
            "window_allocation_bytes": observe_ceiling(allocation, MAX_WINDOW_ALLOCATION_BYTES).evidence(),
        }
    return output


def require_completed_worker_reads(proxy: ProxyController,
                                   completed_before_worker: int,
                                   attempt: WorkerAttempt | None = None) -> int:
    completed = proxy.completed_hosted_reads
    if (type(completed_before_worker) is not int or completed_before_worker < 0
            or completed < completed_before_worker):
        fail(FailureCode.EVIDENCE, "released-worker read baseline differs")
    controller_reads = completed - completed_before_worker
    if attempt is None:
        if controller_reads == 0:
            fail(FailureCode.ZERO_READS,
                 "authorization present but zero hosted network operations completed by released worker")
        return controller_reads
    if controller_reads != 0:
        fail(FailureCode.EVIDENCE, "hosted worker unexpectedly used the controller proxy")
    observations = _worker_raster_observations(attempt)
    return sum(observation["requests"] for observation in observations.values())


def _build_evidence(case: CaseMode, candidate: bytes, wheel: dict[str, Any], proxy: ProxyController,
                    attempt: WorkerAttempt, rank: int, tried: int, seed: tuple[float, float],
                    distance: float | None, positive: dict[str, Any] | None,
                    completed_reads_before_worker: int,
                    rejections: list[dict[str, Any]] | None = None,
                    seed_probe_rejection: dict[str, Any] | None = None) -> tuple[dict[str, Any], bytes]:
    worker_reads = require_completed_worker_reads(proxy, completed_reads_before_worker, attempt)
    worker_observations = _worker_raster_observations(attempt)
    telemetry, windows, decoded = candidate_acceptance_predicate(attempt)
    raw_wkb = bytes.fromhex(attempt.result["geometry_wkb_hex"])
    geometry = canonical_wkb(raw_wkb)
    validate_canonical_wkb(geometry, sha256_bytes(geometry))
    result_ids = sorted(set(map(int, attempt.result["upstream_unit_ids"])))
    terminal = int(attempt.result["terminal_unit_id"])
    if terminal not in result_ids:
        fail(FailureCode.EVIDENCE, "worker upstream IDs omit terminal")
    if case is CaseMode.NEGATIVE and positive is not None:
        validate_negative_discriminator(
            {"input": positive["invocation"]["input_outlet"], "terminal": positive["result"]["terminal_unit_id"],
             "upstream": positive["result"]["upstream_unit_ids"], "resolved": positive["result"]["resolved_outlet"],
             "refined": positive["refinement"]["refined_outlet"], "status": positive["refinement"]["status"],
             "geometry_sha256": positive["geometry"]["sha256"]},
            _worker_comparison(attempt.result, sha256_bytes(geometry)))
    evidence = {
        "case": case.value, "candidate": _candidate_evidence(case, candidate),
        "ceilings": _ceiling_evidence(windows, decoded, worker_observations),
        "geometry": {"canonicalizer": "pourpoint-canonical-wkb-v1", "decimal_precision": 6,
                     "sha256": sha256_bytes(geometry), "size_bytes": len(geometry)},
        "hosted": {"base": HOSTED_BASE,
                   "completed_preflight_reads": proxy.completed_hosted_reads,
                   "released_worker_raster_reads": worker_reads,
                   "flow_acc": _hosted_identity("flow_acc"), "flow_dir": _hosted_identity("flow_dir"),
                   "former_manifest": {"byte_count": FORMER_MANIFEST.content_length,
                                       "d8_declaration_count": 0, "sha256": FORMER_MANIFEST.sha256}},
        "invocation": {"candidate_rank": rank, "input_outlet": list(map(float, attempt.result["input_outlet"])),
                       "invocation_id": attempt.result["invocation_id"], "seed": list(seed)},
        "mutation_attempt_count": proxy.mutation_attempt_count,
        "refinement": {"provenance": {"basis": "identity_derived_from_pinned_wheel_shipped_Engine_path",
                                      "declaration_index": 2, "strategy": "BuiltInD8"},
                       "refined_outlet": list(map(float, attempt.result["refined_outlet"])), "status": "applied"},
        "result": {"area_km2": float(attempt.result["area_km2"]),
                   "resolution_method": attempt.result["resolution_method"],
                   "resolved_outlet": list(map(float, attempt.result["resolved_outlet"])),
                   "terminal_unit_id": terminal, "upstream_unit_ids": result_ids},
        "schema": "pourpoint.released-wheel-proof-evidence.v1",
        "selection": {"candidate_budget": CANDIDATE_BUDGET,
                      "candidate_rejections": rejections if rejections is not None else [],
                      "horizontal_seam_crossed": all(0 < windows[k]["horizontal_seam_row"] < windows[k]["height"] for k in windows),
                      "minimum_distant_metres": 1000000,
                      "mode": {CaseMode.HORIZONTAL: "horizontal-row-seam", CaseMode.DISTANT: "distant-region",
                               CaseMode.NEGATIVE: "negative-control"}[case],
                      "ordered_candidates_tried": tried,
                      "seed_probe_rejection": seed_probe_rejection,
                      "selected_distance_from_horizontal_metres": distance},
        "telemetry": telemetry, "wheel": wheel, "windows": windows,
    }
    validate_live_evidence(evidence)
    return evidence, geometry


def _hosted_identity(kind: str) -> dict[str, Any]:
    identity = COG_IDENTITIES[kind]
    return {"body_sha256": None, "claim": "content_length_and_etag_only",
            "content_length": identity.content_length, "etag": identity.etag,
            "matches_recorded_sha256": None, "recorded_historical_sha256": HISTORICAL_SHA256[kind]}


def _load_positive(case: CaseMode, path_text: str | None) -> tuple[dict[str, Any], tuple[float, float]]:
    if not path_text:
        fail(FailureCode.CONFIG, "dependent case requires --positive-evidence")
    path = Path(path_text)
    outlet = read_positive_outlet(path)
    value = strict_json_bytes(path.read_bytes(), canonical=True)
    if case is CaseMode.NEGATIVE:
        try:
            import verify_released_wheel_evidence as verifier
            verifier.verify_case(path.parent.resolve(), CaseMode.HORIZONTAL)
        except ImportError as exc:
            fail(FailureCode.EVIDENCE, f"positive evidence verifier import failed: {exc}")
    return value, outlet


def _write_artifacts(staging: Path, candidate: bytes, proxy: ProxyController,
                     attempt: WorkerAttempt, evidence: dict[str, Any], geometry: bytes,
                     install_stdout: Path, install_stderr: Path) -> None:
    records = parse_trace_jsonl(attempt.trace)
    line_dir, line_acc = validate_trace(
        records, attempt.cache_root, attempt.result["invocation_id"])
    for kind, line in (("flow-dir", line_dir), ("flow-acc", line_acc)):
        source = Path(records[line - 1]["path"])
        shutil.copyfile(source, staging / f"{kind}.window.tif")
    (staging / "served-manifest.json").write_bytes(candidate)
    (staging / "reads.jsonl").write_bytes(b"".join(canonical_json(record) for record in proxy.records))
    (staging / "trace.jsonl").write_bytes(attempt.trace)
    (staging / "install.stdout.txt").write_bytes(install_stdout.read_bytes())
    (staging / "install.stderr.txt").write_bytes(install_stderr.read_bytes())
    (staging / "worker.stdout.txt").write_bytes(attempt.stdout)
    (staging / "worker.stderr.txt").write_bytes(attempt.stderr)
    (staging / "geometry.canonical.wkb").write_bytes(geometry)
    (staging / "evidence.json").write_bytes(canonical_json(evidence))
    for name in RETAINED:
        with (staging / name).open("rb") as source:
            os.fsync(source.fileno())
    (staging / "artifact-index.json").write_bytes(canonical_json(build_artifact_index(staging)))
    with (staging / "artifact-index.json").open("rb") as source:
        os.fsync(source.fileno())


def run_live(args: argparse.Namespace, transport: ReadOnlyTransport | None = None) -> None:
    output = Path(args.output_dir)
    wheel_path = validate_live_environment(output, dict(os.environ))
    case = CaseMode(args.case)
    positive: dict[str, Any] | None = None
    horizontal: tuple[float, float] | None = None
    if case is not CaseMode.HORIZONTAL:
        positive, horizontal = _load_positive(case, args.positive_evidence)
    wheel = verify_wheel(wheel_path)
    candidate = NEGATIVE_CANDIDATE if case is CaseMode.NEGATIVE else POSITIVE_CANDIDATE
    if not output.parent.is_dir():
        fail(FailureCode.OUTPUT, "output parent must already exist")
    selected: WorkerAttempt | None = None
    selected_rank = 0
    tried = 0
    distance: float | None = None
    seed_probe_rejection: dict[str, Any] | None = None
    active_transport = transport if transport is not None else UrllibTransport()
    with tempfile.TemporaryDirectory(dir=output.parent, prefix=".pourpoint-proof-") as temporary_text:
        temporary = Path(temporary_text)
        install_target = temporary / "install-target"
        hfx_cache = temporary / "hfx-cache"
        xdg_cache = temporary / "xdg-cache"
        staging = temporary / "staging"
        for directory in (install_target, hfx_cache, xdg_cache, staging):
            directory.mkdir()
        install_stdout = temporary / "staging" / "install.stdout.txt"
        install_stderr = temporary / "staging" / "install.stderr.txt"
        install_wheel(wheel_path, install_target, install_stdout, install_stderr)
        proxy = ProxyController(case, candidate, active_transport)
        preflight_hosted(proxy)
        completed_reads_before_worker = proxy.completed_hosted_reads
        with running_proxy(proxy) as proxy_url:
            if case in {CaseMode.HORIZONTAL, CaseMode.NEGATIVE}:
                seed = ZURICH_SEED
            else:
                _, seed = read_distant_seed_declaration()
            if case is CaseMode.NEGATIVE:
                candidates = [horizontal] if horizontal is not None else []
            else:
                discovery = (run_distant_seed_discovery(
                    DISTANT_SEED_DECLARATION, temporary, install_target, 0)
                    if case is CaseMode.DISTANT else
                    run_worker(temporary, install_target, HOSTED_BASE, case, seed, 0))
                candidates, seed_probe_rejection = candidates_after_seed_probe(
                    seed, discovery, case)
            def attempt_candidate(candidate_coord: tuple[float, float],
                                  rank: int) -> WorkerAttempt:
                return run_worker(temporary, install_target, HOSTED_BASE, case,
                                  candidate_coord, rank)

            selected, selected_rank, distance, diagnostics = select_ordered_candidate(
                candidates, CANDIDATE_BUDGET, case, horizontal, attempt_candidate)
        if proxy.fatal is not None:
            raise proxy.fatal
        tried = diagnostics.attempted_count
        rejection_evidence = diagnostics.evidence()
        evidence, geometry = _build_evidence(
            case, candidate, wheel, proxy, selected, selected_rank, tried, seed,
            distance, positive, completed_reads_before_worker,
            rejections=rejection_evidence,
            seed_probe_rejection=seed_probe_rejection)
        _write_artifacts(staging, candidate, proxy, selected, evidence, geometry,
                         install_stdout, install_stderr)
        try:
            import verify_released_wheel_evidence as verifier
            verifier.verify_case(staging.resolve(), case)
        except ImportError as exc:
            fail(FailureCode.EVIDENCE, f"evidence verifier import failed: {exc}")
        try:
            os.rename(staging, output)
        except OSError as exc:
            fail(FailureCode.OUTPUT, f"atomic evidence publication failed: {exc}")


def read_positive_outlet(path: Path) -> tuple[float, float]:
    try:
        value = strict_json_bytes(path.read_bytes(), canonical=True)
    except OSError as exc:
        fail(FailureCode.EVIDENCE, f"positive evidence is unreadable: {exc}")
    invocation = value.get("invocation") if isinstance(value, dict) else None
    outlet = invocation.get("input_outlet") if isinstance(invocation, dict) else None
    if not coordinate(outlet):
        fail(FailureCode.EVIDENCE, "positive evidence input outlet differs")
    return float(outlet[0]), float(outlet[1])


def positive_candidate_budget(value: str) -> int:
    try:
        parsed = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("candidate budget must be an integer") from exc
    if parsed <= 0:
        raise argparse.ArgumentTypeError("candidate budget must be positive")
    return parsed


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    sub = result.add_subparsers(dest="command", required=True)
    self_test_parser = sub.add_parser("self-test")
    self_test_parser.add_argument("--section", choices=sorted(SELF_TEST_SECTIONS))
    self_test_parser.add_argument("--candidate-budget", type=positive_candidate_budget,
                                  default=CANDIDATE_BUDGET)
    live = sub.add_parser("live")
    live.add_argument("--case", required=True, choices=[mode.value for mode in CaseMode])
    live.add_argument("--positive-evidence")
    live.add_argument("--output-dir", required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        args = parser().parse_args(argv)
        if args.command == "self-test":
            self_test(args.section, args.candidate_budget)
        else:
            run_live(args)
        return 0
    except ProofFailure as exc:
        print(f"ERROR[{int(exc.code)}]: {exc}", file=sys.stderr)
        return int(exc.code)
    except SystemExit as exc:
        return int(exc.code)
    except Exception as exc:  # stable boundary for unexpected faults
        print(f"ERROR[70]: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 70


if __name__ == "__main__":
    raise SystemExit(main())
