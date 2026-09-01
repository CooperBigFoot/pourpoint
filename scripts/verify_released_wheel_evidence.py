#!/usr/bin/env python3
"""verify : HorizontalEvidence × DistantEvidence × NegativeEvidence → Verified | Error."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any

import released_wheel_proof as proof


HISTORICAL_PATHS = (
    "crates/core/tests/fixtures/parity/goldens/v01_grit_nonrefined/oracle_a_grit_nonrefined.json",
    "crates/core/tests/fixtures/parity/goldens/v01_merit_refined/oracle_c_merit_refined.json",
    "crates/core/tests/fixtures/parity/goldens/v021_synthetic_nonrefined/v021_synthetic_nonrefined.json",
    "docs/evidence/2026-08-06-released-reader-mutation-control.json",
)
PASS_LINE = "PASS: historical artifacts are immutable"

_FORGED_WKB = {
    proof.CaseMode.HORIZONTAL: bytes.fromhex("0106000000010000000103000000010000000500000000000000000000000000000000000000000000000000f03f0000000000000000000000000000f03f000000000000f03f0000000000000000000000000000f03f00000000000000000000000000000000"),
    proof.CaseMode.DISTANT: bytes.fromhex("010600000001000000010300000001000000050000000000000000000000000000000000000000000000000008400000000000000000000000000000084000000000000008400000000000000000000000000000084000000000000000000000000000000000"),
    proof.CaseMode.NEGATIVE: bytes.fromhex("010600000001000000010300000001000000050000000000000000000000000000000000000000000000000000400000000000000000000000000000004000000000000000400000000000000000000000000000004000000000000000000000000000000000"),
}


def _hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _object(value: Any, keys: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != keys:
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} key set differs")
    return value


def _integer(value: Any, label: str, minimum: int | None = None) -> int:
    if type(value) is not int or minimum is not None and value < minimum:
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} is not an integer in range")
    return value


def _number(value: Any, label: str, minimum: float | None = None) -> float:
    if type(value) not in {int, float} or not math.isfinite(value) or minimum is not None and value < minimum:
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} is not a finite number in range")
    return float(value)


def _write_hand_authored_forgery(root: Path, case: proof.CaseMode) -> None:
    """Assemble a self-consistent retained tree without running a wheel or opening a socket."""
    root.mkdir(parents=True)
    candidate = proof.NEGATIVE_CANDIDATE if case is proof.CaseMode.NEGATIVE else proof.POSITIVE_CANDIDATE
    geometry = _FORGED_WKB[case]
    horizontal = [8.5, 47.3]
    input_outlet = [8.5, 57.19252400096991] if case is proof.CaseMode.DISTANT else horizontal
    mode = {proof.CaseMode.HORIZONTAL: "horizontal-row-seam", proof.CaseMode.DISTANT: "distant-region",
            proof.CaseMode.NEGATIVE: "negative-control"}[case]
    seed = list(proof.REPPARFJORD_SEED if case is proof.CaseMode.DISTANT else proof.ZURICH_SEED)
    identity = proof.WHEEL_ALLOWLIST[proof.expected_wheel_for_host()]
    ceiling_values = {
        "covered_chunk_bytes": proof.MAX_COVERED_CHUNK_BYTES,
        "decoded_chunk_bytes": proof.MAX_DECODED_CHUNK_BYTES,
        "planned_tile_count": proof.MAX_PLANNED_TILE_COUNT,
        "single_compressed_chunk_bytes": proof.MAX_COMPRESSED_CHUNK_BYTES,
        "window_allocation_bytes": proof.MAX_WINDOW_ALLOCATION_BYTES,
    }
    ceilings = {kind: {name: {"ceiling": ceiling, "margin": 1, "observed": ceiling - 1}
                       for name, ceiling in ceiling_values.items()} for kind in ("flow_acc", "flow_dir")}
    candidate_value, difference = (proof.verify_negative_candidate(candidate) if case is proof.CaseMode.NEGATIVE
                                   else (proof.verify_positive_candidate(candidate), []))
    evidence = {
        "candidate": {"byte_count": len(candidate), "difference_from_positive": difference,
                      "flow_dir_encoding": candidate_value["auxiliary"][2]["metadata"]["flow_dir_encoding"],
                      "sha256": proof.sha256_bytes(candidate)},
        "case": case.value, "ceilings": ceilings,
        "geometry": {"canonicalizer": "pourpoint-canonical-wkb-v1", "decimal_precision": 6,
                     "sha256": proof.sha256_bytes(geometry), "size_bytes": len(geometry)},
        "hosted": {"base": proof.HOSTED_BASE, "completed_network_reads": 1,
                   "flow_acc": proof._hosted_identity("flow_acc"), "flow_dir": proof._hosted_identity("flow_dir"),
                   "former_manifest": {"byte_count": 1132, "d8_declaration_count": 0,
                                       "sha256": proof.FORMER_MANIFEST.sha256}},
        "invocation": {"candidate_rank": 1, "input_outlet": input_outlet,
                       "invocation_id": "0" * 32, "seed": seed},
        "mutation_attempt_count": 0,
        "refinement": {"provenance": {"basis": "identity_derived_from_pinned_wheel_shipped_Engine_path",
                                        "declaration_index": 2, "strategy": "BuiltInD8"},
                       "refined_outlet": horizontal, "status": "applied"},
        "result": {"area_km2": 1.0, "resolution_method": "forged", "resolved_outlet": horizontal,
                   "terminal_unit_id": 1, "upstream_unit_ids": [1]},
        "schema": "pourpoint.released-wheel-proof-evidence.v1",
        "selection": {"candidate_budget": 128, "candidate_rejections": [],
                      "horizontal_seam_crossed": True,
                      "minimum_distant_metres": 1_000_000, "mode": mode, "ordered_candidates_tried": 1,
                      "selected_distance_from_horizontal_metres": 1_100_000.0000000002 if case is proof.CaseMode.DISTANT else None},
        "telemetry": {"accepted_trace_line_numbers": [2, 4],
                      "flow_acc": {"bytes": 1, "cache_status": "fetched", "path": "hfx-cache/attempt-1/a.tif", "requests": 1},
                      "flow_dir": {"bytes": 1, "cache_status": "fetched", "path": "hfx-cache/attempt-1/d.tif", "requests": 1}},
        "wheel": {"filename": proof.expected_wheel_for_host(), "metadata_name": "pourpoint",
                  "metadata_requires_python": ">=3.9", "metadata_version": "0.3.0",
                  "sha256": identity[1], "size_bytes": identity[0]},
        "windows": {
            "flow_acc": {"height": 2, "horizontal_seam_row": 1, "nan_count": 0, "non_nan_count": 4,
                         "non_nan_max": 2.0, "non_nan_min": 1.0, "sample_type": "F32",
                         "source": "production_localization_trace_path", "width": 2},
            "flow_dir": {"distinct_values": [1, 2], "height": 2, "horizontal_seam_row": 1,
                         "legal_grass_non_nodata_count": 4, "nodata_255_count": 0, "sample_type": "U8",
                         "source": "production_localization_trace_path", "width": 2}},
    }
    read = {"bytes_received": 10, "case_id": case.value, "completed": True, "error": None,
            "etag": proof.COG_IDENTITIES["flow_dir"].etag, "key": "aux/d8/flow_dir.tif", "method": "GET",
            "origin": "hosted", "range": {"end_exclusive": 20, "start": 10},
            "response_content_length": 10,
            "response_content_range": f"bytes 10-19/{proof.COG_IDENTITIES['flow_dir'].content_length}",
            "seq": 1, "status": 206, "url": proof.HOSTED_BASE + "aux/d8/flow_dir.tif"}
    files = {"evidence.json": proof.canonical_json(evidence), "flow-acc.window.tif": b"forged",
             "flow-dir.window.tif": b"forged", "geometry.canonical.wkb": geometry,
             "install.stderr.txt": b"", "install.stdout.txt": b"", "reads.jsonl": proof.canonical_json(read),
             "served-manifest.json": candidate,
             "trace.jsonl": proof.canonical_json({"totally": "unrelated hand-authored content"}),
             "worker.stderr.txt": b"",
             "worker.stdout.txt": b"POURPOINT_CANDIDATES=" + proof.canonical_json(
                 [{"id": 1, "outlet": horizontal}])}
    for name, data in files.items():
        (root / name).write_bytes(data)
    origin_y = proof.EQUAL_EARTH_Y_MAX - 511.0
    proof._write_fixture_tiff(root / "flow-dir.window.tif", "U8", [1, 2, 1, 2],
                              2, 2, origin_y, -1.0)
    proof._write_fixture_tiff(root / "flow-acc.window.tif", "F32", [1.0, 2.0, 1.0, 2.0],
                              2, 2, origin_y, -1.0)
    (root / "artifact-index.json").write_bytes(proof.canonical_json(proof.build_artifact_index(root)))


def _rewrite_case(root: Path, evidence: dict[str, Any], trace: bytes) -> None:
    (root / "evidence.json").write_bytes(proof.canonical_json(evidence))
    (root / "trace.jsonl").write_bytes(trace)
    (root / "artifact-index.json").write_bytes(proof.canonical_json(proof.build_artifact_index(root)))


def _expected_trace(root: Path) -> tuple[list[dict[str, Any]], bytes]:
    records = []
    for name, stage in (("d.tif", "raster_localize_flow_dir"),
                        ("a.tif", "raster_localize_flow_acc")):
        path = str(root / "hfx-cache" / "attempt-1" / name)
        records.extend([
            {"bytes": 1, "duration_ms": 0.0, "kind": "stage", "path": path,
             "requests": 1, "stage": "cog_fetch_tiles", "thread": "ThreadId(1)",
             "timestamp": 1},
            {"bytes": 1, "cache_status": "fetched", "duration_ms": 0.0, "kind": "stage",
             "path": path, "requests": 1, "stage": stage, "thread": "ThreadId(1)",
             "timestamp": 1},
        ])
    return records, b"".join(proof.canonical_json(record) for record in records)


def _assert_rejected(action: Any, label: str) -> None:
    observed = False
    try:
        action()
    except proof.ProofFailure as exc:
        if exc.code not in {proof.FailureCode.EVIDENCE, proof.FailureCode.REQUIRE_D8, proof.FailureCode.BOUNDS}:
            raise AssertionError(f"{label} rejection used wrong exit code: {exc.code}") from exc
        observed = True
    if observed is not True:
        raise AssertionError(f"{label} data was accepted")


def _producer_verifier_round_trip(temporary: Path) -> None:
    cache_root = temporary / "hfx-cache" / "attempt-1"
    cache_root.mkdir(parents=True)
    direction_path = cache_root / "direction.tif"
    accumulation_path = cache_root / "accumulation.tif"
    origin_y = proof.EQUAL_EARTH_Y_MAX - 511.0
    proof._write_fixture_tiff(direction_path, "U8", [1, 2, 4, 8], 2, 2, origin_y, -1.0)
    proof._write_fixture_tiff(accumulation_path, "F32", [1.0, 2.0, 3.0, 4.0], 2, 2, origin_y, -1.0)
    records = []
    for path, stage in (
            (direction_path, "raster_localize_flow_dir"),
            (accumulation_path, "raster_localize_flow_acc")):
        byte_count = path.stat().st_size
        records.extend([
            {"bytes": byte_count, "duration_ms": 0.0, "kind": "stage", "path": str(path),
             "requests": 1, "stage": "cog_fetch_tiles", "thread": "ThreadId(1)",
             "timestamp": 1},
            {"bytes": byte_count, "cache_status": "fetched", "duration_ms": 0.0,
             "kind": "stage", "path": str(path), "requests": 1, "stage": stage,
             "thread": "ThreadId(1)", "timestamp": 1},
        ])
    trace = b"".join(proof.canonical_json(record) for record in records)
    geometry = proof.simple_multipolygon([[[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0),
                                               (0.0, 1.0), (0.0, 0.0)]]])
    invocation_id = "0" * 32
    result = {
        "area_km2": "1.0", "geometry_wkb_hex": geometry.hex(),
        "input_outlet": [repr(proof.ZURICH_SEED[0]), repr(proof.ZURICH_SEED[1])],
        "invocation_id": invocation_id, "refined_outlet": ["8.5", "47.3"],
        "refinement_status": "applied", "resolution_method": "Snap",
        "resolved_outlet": ["8.5", "47.3"],
        "schema": "pourpoint.released-wheel-proof-worker-result.v1",
        "terminal_unit_id": "1", "upstream_unit_ids": ["1"],
    }
    worker_stdout = b"POURPOINT_CANDIDATES=" + proof.canonical_json(
        [{"id": 1, "outlet": [8.5, 47.3]}])
    attempt = proof.WorkerAttempt(result, [], worker_stdout, b"", trace, cache_root)

    former = proof.canonical_json({"auxiliary": []})
    proxy = proof.ProxyController(proof.CaseMode.HORIZONTAL, proof.POSITIVE_CANDIDATE,
                                  proof.ReplayTransport(former))
    proof.preflight_hosted(proxy, proof.ObjectIdentity(len(former), sha256=proof.sha256_bytes(former)))
    completed_reads_before_worker = proxy.completed_hosted_reads

    filename = proof.expected_wheel_for_host()
    wheel_identity = proof.WHEEL_ALLOWLIST[filename]
    wheel = {"filename": filename, "metadata_name": "pourpoint", "metadata_requires_python": ">=3.9",
             "metadata_version": "0.3.0", "sha256": wheel_identity[1], "size_bytes": wheel_identity[0]}
    evidence, canonical_geometry = proof._build_evidence(
        proof.CaseMode.HORIZONTAL, proof.POSITIVE_CANDIDATE, wheel, proxy, attempt,
        1, 1, proof.ZURICH_SEED, None, None, completed_reads_before_worker,
    )
    expected_paths = {
        "flow_dir": "hfx-cache/attempt-1/direction.tif",
        "flow_acc": "hfx-cache/attempt-1/accumulation.tif",
    }
    for kind, expected_path in expected_paths.items():
        if evidence["telemetry"][kind]["path"] != expected_path:
            raise AssertionError(f"{kind} producer cache redaction differs")
    staging = temporary / "staging"
    staging.mkdir()
    install_stdout = temporary / "install.stdout.txt"
    install_stderr = temporary / "install.stderr.txt"
    install_stdout.write_bytes(b"")
    install_stderr.write_bytes(b"")
    proof._write_artifacts(staging, proof.POSITIVE_CANDIDATE, proxy, attempt, evidence,
                           canonical_geometry, install_stdout, install_stderr)
    verify_case(staging.resolve(), proof.CaseMode.HORIZONTAL)

    mismatched_count = json.loads(json.dumps(evidence))
    mismatched_count["hosted"]["completed_network_reads"] += 1
    _rewrite_case(staging, mismatched_count, trace)
    _assert_rejected(lambda: verify_case(staging.resolve(), proof.CaseMode.HORIZONTAL),
                     "completed hosted read count mismatch")
    _rewrite_case(staging, evidence, trace)

    reads = proof.validate_completed_reads((staging / "reads.jsonl").read_bytes())
    mismatched_case = [dict(record) for record in reads]
    mismatched_case[0]["case_id"] = proof.CaseMode.DISTANT.value
    (staging / "reads.jsonl").write_bytes(b"".join(proof.canonical_json(record) for record in mismatched_case))
    (staging / "artifact-index.json").write_bytes(proof.canonical_json(proof.build_artifact_index(staging)))
    _assert_rejected(lambda: verify_case(staging.resolve(), proof.CaseMode.HORIZONTAL),
                     "reads transcript case mismatch")


def _regression_trace_binding_tests(temporary: Path) -> None:
    _producer_verifier_round_trip(temporary / "round-trip")
    roots = {case: temporary / case.value for case in proof.CaseMode}
    for case, root in roots.items():
        _write_hand_authored_forgery(root, case)
    _assert_rejected(
        lambda: verify_all(roots[proof.CaseMode.HORIZONTAL], roots[proof.CaseMode.DISTANT],
                           roots[proof.CaseMode.NEGATIVE]),
        "hand-authored forgery")

    for root in roots.values():
        evidence = proof.strict_json_bytes((root / "evidence.json").read_bytes(), canonical=True)
        for kind, allocation in (("flow_dir", 4), ("flow_acc", 16)):
            observation = evidence["ceilings"][kind]["window_allocation_bytes"]
            observation["observed"] = allocation
            observation["margin"] = observation["ceiling"] - allocation
        _, trace = _expected_trace(root)
        _rewrite_case(root, evidence, trace)
    verify_all(roots[proof.CaseMode.HORIZONTAL], roots[proof.CaseMode.DISTANT], roots[proof.CaseMode.NEGATIVE])

    distant_root = roots[proof.CaseMode.DISTANT]
    distant_evidence = proof.strict_json_bytes((distant_root / "evidence.json").read_bytes(), canonical=True)
    distant_trace = (distant_root / "trace.jsonl").read_bytes()
    extra_invocation_field = json.loads(json.dumps(distant_evidence))
    extra_invocation_field["invocation"]["horizontal_reference"] = [8.5, 47.3]
    _rewrite_case(distant_root, extra_invocation_field, distant_trace)
    _assert_rejected(
        lambda: verify_case(distant_root, proof.CaseMode.DISTANT),
        "extra invocation field",
    )
    _rewrite_case(distant_root, distant_evidence, distant_trace)

    root = roots[proof.CaseMode.HORIZONTAL]
    evidence = proof.strict_json_bytes((root / "evidence.json").read_bytes(), canonical=True)
    records, valid_trace = _expected_trace(root)
    contradictory_selection = json.loads(json.dumps(evidence))
    contradictory_selection["selection"]["ordered_candidates_tried"] = 2
    _rewrite_case(root, contradictory_selection, valid_trace)
    _assert_rejected(lambda: verify_case(root, proof.CaseMode.HORIZONTAL),
                     "candidate tried/rank contradiction")
    extra_selection_field = json.loads(json.dumps(evidence))
    extra_selection_field["selection"]["unreviewed"] = True
    _rewrite_case(root, extra_selection_field, valid_trace)
    _assert_rejected(lambda: verify_case(root, proof.CaseMode.HORIZONTAL),
                     "extra selection field")
    _rewrite_case(root, evidence, valid_trace)
    _verify_trace_binding(evidence, valid_trace)
    without_fetches = [record for record in records if record.get("stage") != "cog_fetch_tiles"]
    _assert_rejected(
        lambda: _verify_trace_binding(
            evidence, b"".join(proof.canonical_json(record) for record in without_fetches)),
        "missing completed worker raster read",
    )
    after_localizations = [records[1], records[0], records[3], records[2]]
    _assert_rejected(
        lambda: _verify_trace_binding(
            evidence, b"".join(proof.canonical_json(record) for record in after_localizations)),
        "worker raster read after localization",
    )
    for field, value in (("path", str(root / "hfx-cache" / "attempt-1" / "other.tif")),
                         ("bytes", 2), ("requests", 2)):
        mismatched_reads = [dict(record) for record in records]
        for record in mismatched_reads:
            if record.get("stage") == "cog_fetch_tiles":
                record[field] = value
        _assert_rejected(
            lambda mismatched=mismatched_reads: _verify_trace_binding(
                evidence, b"".join(proof.canonical_json(record) for record in mismatched)),
            f"worker raster read {field} mismatch",
        )
    _assert_rejected(lambda: _verify_trace_binding(evidence, proof.canonical_json(records[0])), "missing")
    _assert_rejected(lambda: _verify_trace_binding(evidence, b"{malformed\n"), "malformed")
    duplicate = valid_trace + proof.canonical_json(records[1])
    _assert_rejected(lambda: _verify_trace_binding(evidence, duplicate), "duplicate")
    inconsistent = [dict(records[0]), dict(records[1])]
    inconsistent[0]["bytes"] = 2
    _assert_rejected(
        lambda: _verify_trace_binding(evidence, b"".join(proof.canonical_json(item) for item in inconsistent)),
        "inconsistent")
    unreferenced = json.loads(json.dumps(evidence))
    unreferenced["telemetry"]["accepted_trace_line_numbers"] = [2, 1]
    _assert_rejected(lambda: _verify_trace_binding(unreferenced, valid_trace), "unreferenced")


def _trace_cache_relative(raw: Any, label: str) -> tuple[Path, str]:
    return proof.trace_cache_relative(raw, label)


def _validate_window_binding(evidence: dict[str, Any], kind: str) -> None:
    window = evidence["windows"].get(kind)
    if not isinstance(window, dict):
        proof.fail(proof.FailureCode.EVIDENCE, f"{kind} production window is absent")
    common = {"height", "horizontal_seam_row", "sample_type", "source", "width"}
    kind_keys = ({"distinct_values", "legal_grass_non_nodata_count", "nodata_255_count"} if kind == "flow_dir"
                 else {"nan_count", "non_nan_count", "non_nan_max", "non_nan_min"})
    if set(window) != common | kind_keys or window.get("source") != "production_localization_trace_path":
        proof.fail(proof.FailureCode.EVIDENCE, f"{kind} production window shape differs")
    width = _integer(window.get("width"), f"{kind} window width", 1)
    height = _integer(window.get("height"), f"{kind} window height", 1)
    seam = _integer(window.get("horizontal_seam_row"), f"{kind} seam row", 1)
    expected_type = "U8" if kind == "flow_dir" else "F32"
    if window.get("sample_type") != expected_type or seam >= height:
        proof.fail(proof.FailureCode.EVIDENCE, f"{kind} window type or seam differs")
    total = width * height
    if kind == "flow_dir":
        distinct = window.get("distinct_values")
        legal = _integer(window.get("legal_grass_non_nodata_count"), "legal GRASS sample count", 1)
        nodata = _integer(window.get("nodata_255_count"), "flow-direction nodata count", 0)
        if (not isinstance(distinct, list) or any(type(item) is not int or not 0 <= item <= 255 for item in distinct)
                or distinct != sorted(set(distinct)) or legal + nodata > total):
            proof.fail(proof.FailureCode.EVIDENCE, "flow-direction window samples differ")
    else:
        nan_count = _integer(window.get("nan_count"), "accumulation NaN count", 0)
        non_nan = _integer(window.get("non_nan_count"), "accumulation real count", 1)
        minimum = _number(window.get("non_nan_min"), "accumulation minimum")
        maximum = _number(window.get("non_nan_max"), "accumulation maximum")
        if nan_count + non_nan != total or maximum <= 0 or minimum > maximum:
            proof.fail(proof.FailureCode.EVIDENCE, "flow-accumulation window samples differ")
    allocation = evidence["ceilings"][kind]["window_allocation_bytes"].get("observed")
    if allocation != total * (1 if kind == "flow_dir" else 4):
        proof.fail(proof.FailureCode.EVIDENCE, f"{kind} window allocation contradicts its dimensions")


def _verify_completed_worker_raster_read(
        records: list[dict[str, Any]], localizations: list[tuple[int, dict[str, Any]]]) -> None:
    localized_by_path = {record.get("path"): (line, record) for line, record in localizations}
    matched_after_localization = False
    for line, record in enumerate(records, 1):
        if record.get("stage") != "cog_fetch_tiles" or record.get("path") not in localized_by_path:
            continue
        localized_line, localized = localized_by_path[record["path"]]
        counters_match = all(
            type(record.get(field)) is int
            and record[field] > 0
            and record[field] == localized.get(field)
            for field in ("bytes", "requests")
        )
        if counters_match and line < localized_line:
            return
        if counters_match:
            matched_after_localization = True
    if matched_after_localization:
        proof.fail(
            proof.FailureCode.EVIDENCE,
            "completed released-worker D8 raster-window read never precedes its localization",
        )
    proof.fail(
        proof.FailureCode.EVIDENCE,
        "retained trace records no completed released-worker D8 raster-window read",
    )


def _verify_trace_binding(evidence: dict[str, Any], trace_bytes: bytes) -> None:
    records = proof.parse_trace_jsonl(trace_bytes)
    allowed = {"bytes", "cache_status", "duration_ms", "kind", "matches", "path", "requests",
               "row_groups", "rows", "stage", "thread", "timestamp"}
    integer_fields = {"bytes", "matches", "requests", "row_groups", "rows"}
    string_fields = {"cache_status", "path"}
    for record in records:
        if set(record) - allowed:
            proof.fail(proof.FailureCode.EVIDENCE, "trace contains unknown data")
        for field in integer_fields & set(record):
            if type(record[field]) is not int or record[field] < 0:
                proof.fail(proof.FailureCode.EVIDENCE, f"trace {field} type differs")
        for field in string_fields & set(record):
            if not isinstance(record[field], str) or not record[field]:
                proof.fail(proof.FailureCode.EVIDENCE, f"trace {field} type differs")

    localizations = [(number, record) for number, record in enumerate(records, 1)
                     if record.get("stage") in {"raster_localize_flow_dir", "raster_localize_flow_acc"}]
    if len(localizations) != 2:
        proof.fail(proof.FailureCode.REQUIRE_D8, "production localization trace pair is absent or ambiguous")
    cache_roots = {_trace_cache_relative(record.get("path"), "production localization")[0]
                   for _, record in localizations}
    if len(cache_roots) != 1:
        proof.fail(proof.FailureCode.EVIDENCE, "production localization traces use different case caches")
    line_dir, line_acc = proof.validate_trace(records, next(iter(cache_roots)), evidence["invocation"]["invocation_id"])
    _verify_completed_worker_raster_read(records, localizations)

    telemetry = evidence.get("telemetry")
    if not isinstance(telemetry, dict) or set(telemetry) != {"accepted_trace_line_numbers", "flow_dir", "flow_acc"}:
        proof.fail(proof.FailureCode.EVIDENCE, "telemetry shape differs")
    accepted = telemetry.get("accepted_trace_line_numbers")
    if (not isinstance(accepted, list) or len(accepted) != 2
            or any(type(item) is not int or not 1 <= item <= len(records) for item in accepted)
            or accepted != [line_dir, line_acc] or len(set(accepted)) != 2):
        proof.fail(proof.FailureCode.EVIDENCE, "accepted trace line numbers do not reference the localization pair")
    for kind, stage, line in (("flow_dir", "raster_localize_flow_dir", line_dir),
                              ("flow_acc", "raster_localize_flow_acc", line_acc)):
        record = records[line - 1]
        recorded = telemetry.get(kind)
        if not isinstance(recorded, dict) or set(recorded) != {"bytes", "cache_status", "path", "requests"}:
            proof.fail(proof.FailureCode.EVIDENCE, f"{kind} telemetry shape differs")
        _, relative = _trace_cache_relative(record.get("path"), kind)
        if (record.get("kind") != "stage" or record.get("stage") != stage
                or record.get("cache_status") != "fetched" or recorded.get("cache_status") != "fetched"
                or type(record.get("bytes")) is not int or record["bytes"] <= 0
                or type(record.get("requests")) is not int or record["requests"] <= 0
                or record["bytes"] != recorded.get("bytes") or record["requests"] != recorded.get("requests")
                or recorded.get("path") != relative):
            proof.fail(proof.FailureCode.EVIDENCE, f"{kind} evidence contradicts its accepted trace event")
        _validate_window_binding(evidence, kind)


def _verify_worker_unit_transcript(root: Path, evidence: dict[str, Any]) -> None:
    marker = b"POURPOINT_CANDIDATES="
    lines = [(number, line) for number, line in enumerate(
        (root / "worker.stdout.txt").read_bytes().splitlines(), 1)
        if line.startswith(marker)]
    if len(lines) != 1:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "released-worker unit transcript is absent or ambiguous")
    _number, line = lines[0]
    payload = line[len(marker):]
    try:
        candidates = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        proof.fail(proof.FailureCode.EVIDENCE,
                   f"released-worker unit transcript is malformed: {exc}")
    if (not isinstance(candidates, list) or not candidates
            or proof.canonical_json(candidates).rstrip(b"\n") != payload):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "released-worker unit transcript shape differs")
    unit_ids = []
    for candidate in candidates:
        if (not isinstance(candidate, dict) or set(candidate) != {"id", "outlet"}
                or type(candidate["id"]) is not int
                or not proof.coordinate(candidate["outlet"])):
            proof.fail(proof.FailureCode.EVIDENCE,
                       "released-worker unit transcript entry differs")
        unit_ids.append(candidate["id"])
    if (len(set(unit_ids)) != len(unit_ids)
            or set(unit_ids) != set(evidence["result"]["upstream_unit_ids"])):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "released-worker units differ from recorded upstream IDs")


def _retained_window(root: Path, evidence: dict[str, Any], kind: str) -> proof.DecodedWindow:
    filename = {"flow_dir": "flow-dir.window.tif", "flow_acc": "flow-acc.window.tif"}[kind]
    window = proof.decode_local_tiff(root / filename, kind)
    recomputed, _ = proof._window_evidence(window, kind)
    if recomputed != evidence["windows"][kind]:
        proof.fail(proof.FailureCode.EVIDENCE,
                   f"retained {kind} window differs from production measurements")
    seam = recomputed["horizontal_seam_row"]
    if not 0 < seam < window.height:
        proof.fail(proof.FailureCode.BOUNDS,
                   f"retained {kind} window does not cross a horizontal tile boundary")
    above = window.samples[(seam - 1) * window.width:seam * window.width]
    below = window.samples[seam * window.width:(seam + 1) * window.width]
    if kind == "flow_dir":
        if not any(int(value) in range(1, 9) for value in above) or not any(
                int(value) in range(1, 9) for value in below):
            proof.fail(proof.FailureCode.REQUIRE_D8,
                       "retained direction boundary rows lack real D8 samples")
    elif not any(math.isfinite(float(value)) for value in above) or not any(
            math.isfinite(float(value)) for value in below):
        proof.fail(proof.FailureCode.REQUIRE_D8,
                   "retained accumulation boundary rows lack real samples")
    return window


def _window_pin(root: Path, evidence: dict[str, Any], kind: str) -> dict[str, Any]:
    window = _retained_window(root, evidence, kind)
    seam = evidence["windows"][kind]["horizontal_seam_row"]
    global_row = (proof.EQUAL_EARTH_Y_MAX - window.origin_y) / abs(window.pixel_height)
    first_tile_row = math.floor(global_row / proof.TILE_SIZE)
    row_digests = []
    for row in (seam - 1, seam):
        samples = window.samples[row * window.width:(row + 1) * window.width]
        row_digests.append(proof.sha256_bytes(proof.canonical_json(samples)))
    return {"boundary_row_sha256": row_digests, "height": window.height,
            "horizontal_seam_row": seam, "origin_x": window.origin_x,
            "origin_y": window.origin_y, "pixel_height": window.pixel_height,
            "pixel_width": window.pixel_width,
            "tile_row_pair": [first_tile_row, first_tile_row + 1],
            "width": window.width}


def verify_case(root: Path, expected_case: proof.CaseMode) -> dict[str, Any]:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        proof.fail(proof.FailureCode.EVIDENCE, f"{expected_case.value} artifact directory is invalid")
    proof.verify_artifact_directory(root)
    evidence_bytes = (root / "evidence.json").read_bytes()
    evidence = proof.strict_json_bytes(evidence_bytes, canonical=True)
    proof.validate_live_evidence(evidence)
    for kind in ("flow_dir", "flow_acc"):
        _retained_window(root, evidence, kind)
    reads = proof.validate_completed_reads((root / "reads.jsonl").read_bytes())
    completed_hosted_reads = sum(record["origin"] == "hosted" and record["completed"] for record in reads)
    if (any(record["case_id"] != expected_case.value for record in reads)
            or completed_hosted_reads != evidence["hosted"]["completed_network_reads"]):
        proof.fail(proof.FailureCode.EVIDENCE, "reads transcript case or completed count differs")
    if evidence["case"] != expected_case.value or evidence["mutation_attempt_count"] != 0:
        proof.fail(proof.FailureCode.EVIDENCE, "case identity or mutation count differs")

    served = (root / "served-manifest.json").read_bytes()
    candidate = evidence["candidate"]
    if expected_case is proof.CaseMode.NEGATIVE:
        _, difference = proof.verify_negative_candidate(served)
        if candidate.get("difference_from_positive") != difference:
            proof.fail(proof.FailureCode.EVIDENCE, "negative candidate difference differs")
    else:
        proof.verify_positive_candidate(served)
        if candidate.get("difference_from_positive") != []:
            proof.fail(proof.FailureCode.EVIDENCE, "positive candidate has structural differences")
    if candidate.get("byte_count") != len(served) or candidate.get("sha256") != proof.sha256_bytes(served):
        proof.fail(proof.FailureCode.EVIDENCE, "candidate evidence identity differs")

    wheel = evidence["wheel"]
    expected_wheel = proof.expected_wheel_for_host()
    allowed = proof.WHEEL_ALLOWLIST.get(expected_wheel)
    if allowed is None or wheel != {"filename": expected_wheel, "metadata_name": "pourpoint", "metadata_requires_python": ">=3.9",
                                    "metadata_version": "0.3.0", "sha256": allowed[1], "size_bytes": allowed[0]}:
        proof.fail(proof.FailureCode.EVIDENCE, "wheel identity differs")

    hosted = evidence["hosted"]
    if hosted.get("base") != proof.HOSTED_BASE or _integer(hosted.get("completed_network_reads"), "completed reads", 1) < 1:
        proof.fail(proof.FailureCode.EVIDENCE, "hosted base or read count differs")
    former = hosted.get("former_manifest")
    if former != {"byte_count": 1132, "d8_declaration_count": 0, "sha256": proof.FORMER_MANIFEST.sha256}:
        proof.fail(proof.FailureCode.EVIDENCE, "former manifest identity differs")
    for kind in ("flow_dir", "flow_acc"):
        identity = proof.COG_IDENTITIES[kind]
        expected = {"body_sha256": None, "claim": "content_length_and_etag_only", "content_length": identity.content_length,
                    "etag": identity.etag, "matches_recorded_sha256": None,
                    "recorded_historical_sha256": proof.HISTORICAL_SHA256[kind]}
        if hosted.get(kind) != expected:
            proof.fail(proof.FailureCode.EVIDENCE, f"{kind} hosted identity differs")

    expected_ceilings = {
        "planned_tile_count": proof.MAX_PLANNED_TILE_COUNT,
        "single_compressed_chunk_bytes": proof.MAX_COMPRESSED_CHUNK_BYTES,
        "covered_chunk_bytes": proof.MAX_COVERED_CHUNK_BYTES,
        "decoded_chunk_bytes": proof.MAX_DECODED_CHUNK_BYTES,
        "window_allocation_bytes": proof.MAX_WINDOW_ALLOCATION_BYTES,
    }
    for kind in ("flow_dir", "flow_acc"):
        observations = evidence["ceilings"].get(kind, {})
        if set(observations) != set(expected_ceilings):
            proof.fail(proof.FailureCode.EVIDENCE, f"{kind} ceiling keys differ")
        for name, ceiling in expected_ceilings.items():
            observation = observations[name]
            if set(observation) != {"ceiling", "margin", "observed"} or observation["ceiling"] != ceiling:
                proof.fail(proof.FailureCode.EVIDENCE, f"{kind} {name} ceiling differs")
            observed = _integer(observation["observed"], "ceiling observation", 0)
            margin = _integer(observation["margin"], "ceiling margin", 1)
            if margin != ceiling - observed:
                proof.fail(proof.FailureCode.EVIDENCE, "ceiling margin arithmetic differs")

    invocation = evidence["invocation"]
    if (set(invocation) != {"candidate_rank", "input_outlet", "invocation_id", "seed"}
            or not proof.coordinate(invocation.get("input_outlet"))
            or not proof.coordinate(invocation.get("seed"))):
        proof.fail(proof.FailureCode.EVIDENCE, "invocation shape or coordinates differ")
    invocation_id = invocation.get("invocation_id")
    if not isinstance(invocation_id, str) or not __import__("re").fullmatch(r"[0-9a-f]{32}", invocation_id):
        proof.fail(proof.FailureCode.EVIDENCE, "invocation ID differs")
    rank = _integer(invocation.get("candidate_rank"), "candidate rank", 1)
    if rank > proof.CANDIDATE_BUDGET:
        proof.fail(proof.FailureCode.EVIDENCE, "candidate rank exceeds budget")

    refinement = evidence["refinement"]
    proof.validate_require_d8(refinement.get("status"), refinement.get("refined_outlet"))
    if refinement.get("provenance") != {"basis": "identity_derived_from_pinned_wheel_shipped_Engine_path", "declaration_index": 2, "strategy": "BuiltInD8"}:
        proof.fail(proof.FailureCode.EVIDENCE, "refinement provenance differs")
    result = evidence["result"]
    ids = result.get("upstream_unit_ids")
    terminal = result.get("terminal_unit_id")
    if not isinstance(ids, list) or any(type(item) is not int for item in ids) or ids != sorted(set(ids)) or terminal not in ids:
        proof.fail(proof.FailureCode.EVIDENCE, "upstream IDs are not normalized or inclusive")
    _number(result.get("area_km2"), "area", 0.0)
    if result["area_km2"] <= 0 or not isinstance(result.get("resolution_method"), str) or not result["resolution_method"]:
        proof.fail(proof.FailureCode.EVIDENCE, "result area or method differs")
    _verify_worker_unit_transcript(root, evidence)

    geometry = evidence["geometry"]
    wkb = (root / "geometry.canonical.wkb").read_bytes()
    if geometry.get("canonicalizer") != "pourpoint-canonical-wkb-v1" or geometry.get("size_bytes") != len(wkb):
        proof.fail(proof.FailureCode.EVIDENCE, "canonical geometry metadata differs")
    proof.validate_canonical_wkb(wkb, geometry.get("sha256", ""), geometry.get("decimal_precision"))

    selection = evidence["selection"]
    if selection.get("candidate_budget") != 128 or not 1 <= _integer(selection.get("ordered_candidates_tried"), "tried", 1) <= 128:
        proof.fail(proof.FailureCode.EVIDENCE, "selection bounds differ")
    if expected_case in {proof.CaseMode.HORIZONTAL, proof.CaseMode.NEGATIVE} and selection.get("horizontal_seam_crossed") is not True:
        proof.fail(proof.FailureCode.EVIDENCE, "horizontal seam was not crossed")
    if expected_case is proof.CaseMode.HORIZONTAL and invocation["seed"] != list(proof.ZURICH_SEED):
        proof.fail(proof.FailureCode.EVIDENCE, "Zurich seed differs")
    if expected_case is proof.CaseMode.DISTANT and invocation["seed"] != list(proof.REPPARFJORD_SEED):
        proof.fail(proof.FailureCode.EVIDENCE, "Repparfjord seed differs")
    _verify_trace_binding(evidence, (root / "trace.jsonl").read_bytes())
    return evidence


def _coordinate_equal(left: list[float], right: list[float]) -> bool:
    return all(abs(float(a) - float(b)) <= 0.000001 for a, b in zip(left, right))


def verify_all(horizontal_root: Path, distant_root: Path, negative_root: Path) -> None:
    horizontal = verify_case(horizontal_root, proof.CaseMode.HORIZONTAL)
    distant = verify_case(distant_root, proof.CaseMode.DISTANT)
    negative = verify_case(negative_root, proof.CaseMode.NEGATIVE)
    proof.verify_recorded_distance(tuple(horizontal["invocation"]["input_outlet"]), tuple(distant["invocation"]["input_outlet"]),
                                   distant["selection"]["selected_distance_from_horizontal_metres"])
    if distant["selection"]["selected_distance_from_horizontal_metres"] < 1_000_000:
        proof.fail(proof.FailureCode.EVIDENCE, "distant selection is below threshold")
    if horizontal["invocation"]["input_outlet"] != negative["invocation"]["input_outlet"]:
        proof.fail(proof.FailureCode.EVIDENCE, "negative input differs")
    if horizontal["result"]["terminal_unit_id"] != negative["result"]["terminal_unit_id"] or horizontal["result"]["upstream_unit_ids"] != negative["result"]["upstream_unit_ids"]:
        proof.fail(proof.FailureCode.EVIDENCE, "negative topology differs")
    for key in ("resolved_outlet",):
        if not _coordinate_equal(horizontal["result"][key], negative["result"][key]):
            proof.fail(proof.FailureCode.EVIDENCE, f"negative {key} differs")
    if not _coordinate_equal(horizontal["refinement"]["refined_outlet"], negative["refinement"]["refined_outlet"]):
        proof.fail(proof.FailureCode.EVIDENCE, "negative refined outlet differs")
    if horizontal["geometry"]["sha256"] == negative["geometry"]["sha256"]:
        proof.fail(proof.FailureCode.EXHAUSTED, "negative geometry discriminator did not differ")


FIXED_CASES_SCHEMA = "pourpoint.grit-d8-fixed-cases.v1"


def _fixed_case_projection(root: Path, evidence: dict[str, Any]) -> dict[str, Any]:
    selection = evidence["selection"]
    invocation = evidence["invocation"]
    telemetry = evidence["telemetry"]
    return {
        "cache_identity": {kind: telemetry[kind]["path"] for kind in ("flow_acc", "flow_dir")},
        "candidate_manifest": evidence["candidate"],
        "candidate_ordering": {
            "budget": selection["candidate_budget"],
            "rejected": selection["candidate_rejections"],
            "seed_probe_rejection": selection.get("seed_probe_rejection"),
            "selected": {"coordinate": invocation["input_outlet"],
                         "rank": invocation["candidate_rank"]},
        },
        "coordinate": invocation["input_outlet"],
        "evidence_path": root.name,
        "hosted_objects": {kind: evidence["hosted"][kind]
                           for kind in ("flow_acc", "flow_dir")},
        "process_identity": invocation["invocation_id"],
        "region_identity": {
            "discovery_seed": invocation["seed"],
            "distance_from_horizontal_metres": selection["selected_distance_from_horizontal_metres"],
            "selection_mode": selection["mode"],
        },
        "terminal_identity": evidence["result"]["terminal_unit_id"],
        "wheel_identity": evidence["wheel"],
        "windows": {kind: _window_pin(root, evidence, kind)
                    for kind in ("flow_acc", "flow_dir")},
    }


def verify_fixed_cases(path: Path) -> dict[str, Any]:
    if not path.is_absolute() or path.is_symlink() or not path.is_file():
        proof.fail(proof.FailureCode.EVIDENCE, "fixed-case declaration path is invalid")
    declaration = proof.strict_json_bytes(path.read_bytes(), canonical=True)
    if not isinstance(declaration, dict) or set(declaration) != {"cases", "schema"}:
        proof.fail(proof.FailureCode.EVIDENCE, "fixed-case declaration shape differs")
    if declaration["schema"] != FIXED_CASES_SCHEMA or not isinstance(declaration["cases"], dict):
        proof.fail(proof.FailureCode.EVIDENCE, "fixed-case declaration schema differs")
    expected_names = {proof.CaseMode.HORIZONTAL.value, proof.CaseMode.DISTANT.value}
    if set(declaration["cases"]) != expected_names:
        proof.fail(proof.FailureCode.EVIDENCE, "fixed-case declaration case set differs")
    verified: dict[str, dict[str, Any]] = {}
    for name in sorted(expected_names):
        case = proof.CaseMode(name)
        declared = declaration["cases"][name]
        if not isinstance(declared, dict) or declared.get("evidence_path") != name:
            proof.fail(proof.FailureCode.EVIDENCE, f"{name} evidence path differs")
        case_root = (path.parent / name).resolve()
        try:
            case_root.relative_to(path.parent.resolve())
        except ValueError:
            proof.fail(proof.FailureCode.EVIDENCE, f"{name} evidence path escapes declaration")
        evidence = verify_case(case_root, case)
        expected = _fixed_case_projection(case_root, evidence)
        if declared != expected:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{name} declaration differs from retained selection evidence")
        rejections = declared["candidate_ordering"]["rejected"]
        selected_rank = declared["candidate_ordering"]["selected"]["rank"]
        if [entry["rank"] for entry in rejections] != list(range(1, selected_rank)):
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{name} candidate ordering has an unaccounted rank")
        row_pairs = {tuple(window["tile_row_pair"])
                     for window in declared["windows"].values()}
        transforms = {(window["origin_x"], window["origin_y"],
                       window["pixel_width"], window["pixel_height"],
                       window["width"], window["height"])
                      for window in declared["windows"].values()}
        if len(row_pairs) != 1 or len(transforms) != 1:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{name} production window placements differ")
        verified[name] = evidence
    horizontal = verified[proof.CaseMode.HORIZONTAL.value]
    distant = verified[proof.CaseMode.DISTANT.value]
    distance = distant["selection"]["selected_distance_from_horizontal_metres"]
    proof.verify_recorded_distance(tuple(horizontal["invocation"]["input_outlet"]),
                                   tuple(distant["invocation"]["input_outlet"]), distance)
    if distance < 1_000_000:
        proof.fail(proof.FailureCode.EVIDENCE, "fixed outlets are not distant")
    process_ids = {case["process_identity"] for case in declaration["cases"].values()}
    cache_ids = {tuple(sorted(case["cache_identity"].values()))
                 for case in declaration["cases"].values()}
    if len(process_ids) != 2 or len(cache_ids) != 2:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "fixed cases do not have distinct process and cache identities")
    return declaration


REPRODUCIBILITY_SCHEMA = "pourpoint.grit-d8-reproducibility.v1"
OFFLINE_NEGATIVE_SCHEMA = "pourpoint.grit-d8-offline-negative.v1"
OFFLINE_NEGATIVE_INDEX_SCHEMA = "pourpoint.grit-d8-offline-negative-artifact-index.v1"


def _resolved_child(root: Path, relative: Any, label: str) -> Path:
    if (not isinstance(relative, str) or not relative or Path(relative).is_absolute()
            or ".." in Path(relative).parts):
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} path differs")
    child = (root / relative).resolve()
    try:
        child.relative_to(root.resolve())
    except ValueError:
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} path escapes evidence root")
    return child


def _trace_cache_identity(root: Path, evidence: dict[str, Any]) -> str:
    records = proof.parse_trace_jsonl((root / "trace.jsonl").read_bytes())
    identities = set()
    for kind, line in zip(("flow_dir", "flow_acc"),
                          evidence["telemetry"]["accepted_trace_line_numbers"]):
        cache_root, _relative = proof.trace_cache_relative(
            records[line - 1].get("path"), kind)
        identities.add(str(cache_root))
    if len(identities) != 1:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "one run does not carry one cache identity")
    return identities.pop()


def verify_reproducibility(root: Path) -> dict[str, Any]:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        proof.fail(proof.FailureCode.EVIDENCE,
                   "reproducibility evidence root is invalid")
    declaration_path = root / "reproducibility.json"
    declaration = proof.strict_json_bytes(declaration_path.read_bytes(), canonical=True)
    _object(declaration, {"cases", "schema"}, "reproducibility declaration")
    if declaration["schema"] != REPRODUCIBILITY_SCHEMA:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "reproducibility declaration schema differs")
    expected_cases = {proof.CaseMode.HORIZONTAL.value,
                      proof.CaseMode.DISTANT.value}
    cases = declaration["cases"]
    if not isinstance(cases, dict) or set(cases) != expected_cases:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "reproducibility case set differs")
    fixed = verify_fixed_cases((root / "fixed-cases.json").resolve())
    verified_count = 0
    for case_name in sorted(expected_cases):
        entry = _object(cases[case_name], {"runs"},
                        f"{case_name} reproducibility entry")
        expected_runs = [case_name, f"reproducibility/{case_name}/run-2"]
        if entry["runs"] != expected_runs:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{case_name} run references differ")
        mode = proof.CaseMode(case_name)
        run_roots = [_resolved_child(root, item, f"{case_name} run")
                     for item in entry["runs"]]
        runs = [verify_case(path, mode) for path in run_roots]
        pin = fixed["cases"][case_name]
        for run in runs:
            if (run["invocation"]["input_outlet"] != pin["coordinate"]
                    or run["candidate"] != pin["candidate_manifest"]
                    or run["wheel"] != pin["wheel_identity"]
                    or {kind: run["hosted"][kind] for kind in ("flow_acc", "flow_dir")}
                    != pin["hosted_objects"]):
                proof.fail(proof.FailureCode.EVIDENCE,
                           f"{case_name} run differs from fixed-case authority")
        left, right = runs
        if left["invocation"]["invocation_id"] == right["invocation"]["invocation_id"]:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{case_name} process identities are not distinct")
        cache_identities = [_trace_cache_identity(path, run)
                            for path, run in zip(run_roots, runs)]
        if cache_identities[0] == cache_identities[1]:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{case_name} cache identities are not distinct")
        comparisons = (
            (left["result"]["terminal_unit_id"], right["result"]["terminal_unit_id"], "terminal ID"),
            (left["result"]["upstream_unit_ids"], right["result"]["upstream_unit_ids"], "ordered upstream IDs"),
            (left["refinement"]["refined_outlet"], right["refinement"]["refined_outlet"], "refined outlet"),
            (left["refinement"]["provenance"], right["refinement"]["provenance"], "provenance"),
        )
        for first, second, label in comparisons:
            if first != second:
                proof.fail(proof.FailureCode.EVIDENCE,
                           f"{case_name} does not reproduce {label}")
        if ((run_roots[0] / "geometry.canonical.wkb").read_bytes()
                != (run_roots[1] / "geometry.canonical.wkb").read_bytes()):
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{case_name} canonical geometry bytes differ")
        verified_count += 1
    return {"cases_verified": verified_count, "runs_verified": verified_count * 2,
            "schema": REPRODUCIBILITY_SCHEMA}


def _verify_offline_negative_index(root: Path) -> None:
    index = proof.strict_json_bytes((root / "artifact-index.json").read_bytes(),
                                    canonical=True)
    _object(index, {"artifacts", "schema"}, "offline negative artifact index")
    if index["schema"] != OFFLINE_NEGATIVE_INDEX_SCHEMA:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative artifact index schema differs")
    expected = {"accepted-manifest.json", "evidence.json", "false-manifest.json",
                "flow-acc.window.tif", "flow-dir.window.tif",
                "network-operations.jsonl"}
    artifacts = index["artifacts"]
    if not isinstance(artifacts, dict) or set(artifacts) != expected:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative artifact set differs")
    actual_files = {path.name for path in root.iterdir() if path.is_file()}
    if actual_files != expected | {"artifact-index.json"}:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative directory contains an unindexed artifact")
    for name in sorted(expected):
        record = _object(artifacts[name], {"sha256", "size_bytes"},
                         f"{name} artifact record")
        data = (root / name).read_bytes()
        if record != {"sha256": proof.sha256_bytes(data),
                      "size_bytes": len(data)}:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{name} artifact identity differs")


def verify_offline_negative(root: Path) -> dict[str, Any]:
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative evidence root is invalid")
    _verify_offline_negative_index(root)
    evidence = proof.strict_json_bytes((root / "evidence.json").read_bytes(),
                                       canonical=True)
    _object(evidence, {"case", "declarations", "difference", "network",
                       "recomputation", "schema", "source_positive", "windows"},
            "offline negative evidence")
    if (evidence["schema"] != OFFLINE_NEGATIVE_SCHEMA
            or evidence["case"] != proof.CaseMode.NEGATIVE.value
            or evidence["source_positive"] != "../horizontal-boundary"):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative evidence identity differs")
    source_root = (root / evidence["source_positive"]).resolve()
    if source_root != (root.parent / "horizontal-boundary").resolve():
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative positive-source path differs")
    positive = verify_case(source_root, proof.CaseMode.HORIZONTAL)
    declarations = _object(evidence["declarations"], {"accepted", "false"},
                           "offline negative declarations")
    declaration_bytes = {}
    for label, filename in (("accepted", "accepted-manifest.json"),
                            ("false", "false-manifest.json")):
        record = _object(declarations[label], {"filename", "sha256", "size_bytes"},
                         f"{label} declaration")
        if record["filename"] != filename:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{label} declaration filename differs")
        data = (root / filename).read_bytes()
        if record != {"filename": filename, "sha256": proof.sha256_bytes(data),
                      "size_bytes": len(data)}:
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"{label} declaration identity differs")
        declaration_bytes[label] = data
    proof.verify_positive_candidate(declaration_bytes["accepted"])
    _false_value, difference = proof.verify_negative_candidate(
        declaration_bytes["false"])
    if evidence["difference"] != difference:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative declaration difference differs")
    windows = _object(evidence["windows"], {"flow_acc", "flow_dir"},
                      "offline negative windows")
    decoded = {}
    for kind, filename in (("flow_acc", "flow-acc.window.tif"),
                           ("flow_dir", "flow-dir.window.tif")):
        record = _object(windows[kind], {"filename", "sha256", "size_bytes"},
                         f"offline negative {kind} window")
        data = (root / filename).read_bytes()
        if (record != {"filename": filename, "sha256": proof.sha256_bytes(data),
                       "size_bytes": len(data)}
                or data != (source_root / filename).read_bytes()):
            proof.fail(proof.FailureCode.EVIDENCE,
                       f"offline negative {kind} window differs from accepted bytes")
        decoded[kind] = proof.decode_local_tiff(root / filename, kind)
    network = _object(evidence["network"], {"attempted_reads", "completed_reads",
                                             "policy", "transcript", "writes"},
                      "offline negative network record")
    if (network != {"attempted_reads": 0, "completed_reads": 0,
                    "policy": "deny-all", "transcript": "network-operations.jsonl",
                    "writes": 0}
            or (root / "network-operations.jsonl").read_bytes() != b""):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative performed or recorded a hosted operation")
    recomputation = _object(evidence["recomputation"],
                            {"failure", "flow_dir_interpretation", "outcome"},
                            "offline negative recomputation")
    invalid = sorted({int(value) for value in decoded["flow_dir"].samples
                      if int(value) not in {1, 2, 4, 8, 16, 32, 64, 128, 255}})
    if (recomputation["flow_dir_interpretation"] != "esri"
            or recomputation["outcome"] != "failed"
            or recomputation["failure"] != {
                "code": "INVALID_FLOW_DIRECTION", "invalid_values": invalid}
            or not invalid):
        proof.fail(proof.FailureCode.EXHAUSTED,
                   "false flow-direction declaration reproduced the accepted watershed")
    if positive["candidate"]["sha256"] != declarations["accepted"]["sha256"]:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "offline negative accepted declaration is not the fixed positive")
    return {"case_verified": proof.CaseMode.NEGATIVE.value,
            "schema": OFFLINE_NEGATIVE_SCHEMA}


def verify_public_case(root: Path, case: proof.CaseMode) -> dict[str, Any]:
    """Verify one retained run against the fixed prepublication authority."""
    if case not in {proof.CaseMode.HORIZONTAL, proof.CaseMode.DISTANT}:
        proof.fail(proof.FailureCode.CONFIG, "public verification requires a positive fixed case")
    evidence = verify_case(root, case)
    repository = Path(__file__).resolve().parent.parent
    prepublication = repository / "docs/evidence/grit-d8-live/prepublication"
    fixed = verify_fixed_cases((prepublication / "fixed-cases.json").resolve())
    accepted_root = (prepublication / case.value).resolve()
    accepted = verify_case(accepted_root, case)
    pin = fixed["cases"][case.value]
    if evidence["invocation"]["input_outlet"] != pin["coordinate"]:
        proof.fail(proof.FailureCode.EVIDENCE, "public run outlet differs from fixed case")
    comparisons = (
        (evidence["result"]["terminal_unit_id"], accepted["result"]["terminal_unit_id"], "terminal identity"),
        (evidence["result"]["upstream_unit_ids"], accepted["result"]["upstream_unit_ids"], "upstream identities"),
        (evidence["refinement"]["refined_outlet"], accepted["refinement"]["refined_outlet"], "refined outlet"),
        (evidence["refinement"]["provenance"], accepted["refinement"]["provenance"], "refinement provenance"),
    )
    for observed, expected, label in comparisons:
        if observed != expected:
            proof.fail(proof.FailureCode.EVIDENCE, f"public run {label} differs from accepted case")
    if ((root / "geometry.canonical.wkb").read_bytes()
            != (accepted_root / "geometry.canonical.wkb").read_bytes()):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "public run canonical geometry differs from accepted case")
    if evidence["invocation"]["invocation_id"] == accepted["invocation"]["invocation_id"]:
        proof.fail(proof.FailureCode.EVIDENCE, "public run did not use a fresh process identity")
    if _trace_cache_identity(root, evidence) == _trace_cache_identity(accepted_root, accepted):
        proof.fail(proof.FailureCode.EVIDENCE, "public run did not use a fresh cache identity")
    trace = proof.parse_trace_jsonl((root / "trace.jsonl").read_bytes())
    remote_open = [record for record in trace if record.get("stage") == "remote_open"]
    manifest_fetch = [record for record in trace if record.get("stage") == "manifest_fetch"]
    if (len(remote_open) != 1 or remote_open[0].get("path") != proof.HOSTED_BASE
            or len(manifest_fetch) != 1
            or manifest_fetch[0].get("path") != "grit/hfx-v0.3.0/manifest.json"
            or manifest_fetch[0].get("bytes") != len(proof.POSITIVE_CANDIDATE)):
        proof.fail(proof.FailureCode.EVIDENCE,
                   "released wheel did not record the hosted public declaration source")
    if (root / "served-manifest.json").read_bytes() != proof.POSITIVE_CANDIDATE:
        proof.fail(proof.FailureCode.IDENTITY, "served public manifest identity differs")
    if case is proof.CaseMode.DISTANT:
        distance = evidence["selection"]["selected_distance_from_horizontal_metres"]
        if distance != pin["region_identity"]["distance_from_horizontal_metres"] or distance < 1_000_000:
            proof.fail(proof.FailureCode.EVIDENCE, "public distant-region rule differs")
    return evidence


def verify_bounded_public_reads(root: Path) -> dict[str, Any]:
    """Verify retained public COG reads and fixed allocation margins offline."""
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        proof.fail(proof.FailureCode.EVIDENCE, "bounded-read evidence root is invalid")
    verified = 0
    for case in (proof.CaseMode.HORIZONTAL, proof.CaseMode.DISTANT):
        case_root = (root / case.value).resolve()
        try:
            case_root.relative_to(root.resolve())
        except ValueError:
            proof.fail(proof.FailureCode.EVIDENCE, "bounded-read case escapes evidence root")
        evidence = verify_public_case(case_root, case)
        trace = proof.parse_trace_jsonl((case_root / "trace.jsonl").read_bytes())
        for kind, stage_name in (("flow_dir", "raster_localize_flow_dir"),
                                 ("flow_acc", "raster_localize_flow_acc")):
            telemetry = evidence["telemetry"][kind]
            localized = [record for record in trace if record.get("stage") == stage_name]
            identity = proof.COG_IDENTITIES[kind]
            if (len(localized) != 1 or localized[0].get("cache_status") != "fetched"
                    or localized[0].get("bytes") != telemetry.get("bytes")
                    or localized[0].get("requests") != telemetry.get("requests")
                    or type(telemetry.get("requests")) is not int or telemetry["requests"] <= 0
                    or type(telemetry.get("bytes")) is not int or telemetry["bytes"] <= 0
                    or telemetry["bytes"] >= identity.content_length):
                proof.fail(proof.FailureCode.BOUNDS,
                           f"{case.value} {kind} did not retain bounded sub-object reads")
            window = evidence["windows"][kind]
            match = __import__("re").search(
                r"\.x[0-9]+-y[0-9]+-w([0-9]+)-h([0-9]+)\.tif$",
                telemetry.get("path", ""))
            if (match is None or int(match.group(1)) != window["width"]
                    or int(match.group(2)) != window["height"]):
                proof.fail(proof.FailureCode.BOUNDS,
                           f"{case.value} {kind} reads are not tied to the selected window")
            for guard in evidence["ceilings"][kind].values():
                if type(guard.get("margin")) is not int or guard["margin"] <= 0:
                    proof.fail(proof.FailureCode.BOUNDS,
                               f"{case.value} {kind} allocation margin is not positive")
        verified += 1
    return {"cases_verified": verified,
            "schema": "pourpoint.public-bounded-read-verification.v1"}

def self_test() -> None:
    denied = proof.DeniedOpener()
    proof.set_network_opener(denied)
    repository = Path(__file__).resolve().parent.parent
    before = {relative: _hash(repository / relative) for relative in HISTORICAL_PATHS}
    fixed_path = repository / "docs/evidence/grit-d8-live/prepublication/fixed-cases.json"
    verify_fixed_cases(fixed_path.resolve())
    with tempfile.TemporaryDirectory() as temporary:
        temporary_root = Path(temporary)
        _regression_trace_binding_tests(temporary_root / "forged")
        copied = temporary_root / "copy.json"
        copied.write_bytes((repository / HISTORICAL_PATHS[0]).read_bytes())
        expected = _hash(copied)
        data = bytearray(copied.read_bytes()); data[len(data) // 2] ^= 1; copied.write_bytes(data)
        rejected = _hash(copied) != expected
        assert rejected is True
        fixed_copy = temporary_root / "fixed"
        fixed_copy.mkdir()
        for case in (proof.CaseMode.HORIZONTAL, proof.CaseMode.DISTANT):
            shutil.copytree(fixed_path.parent / case.value, fixed_copy / case.value)
        altered = proof.strict_json_bytes(fixed_path.read_bytes(), canonical=True)
        altered["cases"][proof.CaseMode.HORIZONTAL.value]["terminal_identity"] += 1
        altered_path = fixed_copy / "fixed-cases.json"
        altered_path.write_bytes(proof.canonical_json(altered))
        _assert_rejected(lambda: verify_fixed_cases(altered_path.resolve()),
                         "unsupported fixed-case declaration")
    after = {relative: _hash(repository / relative) for relative in HISTORICAL_PATHS}
    if before != after or denied.calls != 0:
        proof.fail(proof.FailureCode.EVIDENCE, "historical artifact changed or network opener was reached")
    print(PASS_LINE)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--self-test", action="store_true")
    result.add_argument("--case", choices=[case.value for case in proof.CaseMode])
    result.add_argument("--evidence")
    result.add_argument("--public", action="store_true")
    result.add_argument("--bounded-reads")
    result.add_argument("--cases")
    result.add_argument("--horizontal")
    result.add_argument("--distant")
    result.add_argument("--negative")
    result.add_argument("--reproducibility")
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        args = parser().parse_args(argv)
        legacy = (args.horizontal, args.distant, args.negative)
        if args.self_test:
            if any((*legacy, args.case, args.evidence, args.public,
                    args.bounded_reads, args.cases, args.reproducibility)):
                proof.fail(proof.FailureCode.CONFIG, "--self-test cannot be combined with evidence")
            self_test()
        elif args.bounded_reads:
            if any((*legacy, args.case, args.evidence, args.public,
                    args.cases, args.reproducibility)):
                proof.fail(proof.FailureCode.CONFIG,
                           "--bounded-reads cannot be combined with other evidence modes")
            result = verify_bounded_public_reads(Path(args.bounded_reads).resolve())
            print(json.dumps({**result, "status": "verified"},
                             sort_keys=True, separators=(",", ":")))
        elif args.reproducibility:
            if any((*legacy, args.case, args.evidence, args.public, args.cases)):
                proof.fail(proof.FailureCode.CONFIG,
                           "--reproducibility cannot be combined with other evidence modes")
            result = verify_reproducibility(Path(args.reproducibility).resolve())
            print(json.dumps({**result, "status": "verified"},
                             sort_keys=True, separators=(",", ":")))
        elif args.cases:
            if any((*legacy, args.case, args.evidence, args.public)):
                proof.fail(proof.FailureCode.CONFIG, "--cases cannot be combined with other evidence modes")
            declaration = verify_fixed_cases(Path(args.cases).resolve())
            print(json.dumps({"cases_verified": len(declaration["cases"]),
                              "schema": FIXED_CASES_SCHEMA, "status": "verified"},
                             sort_keys=True, separators=(",", ":")))
        elif args.case or args.evidence or args.public:
            if not args.case or not args.evidence or any(legacy):
                proof.fail(proof.FailureCode.CONFIG,
                           "--case and --evidence are required together and cannot use legacy inputs")
            evidence_root = Path(args.evidence).resolve()
            if args.public:
                verify_public_case(evidence_root, proof.CaseMode(args.case))
                schema = "pourpoint.public-released-wheel-verification.v1"
            elif args.case == proof.CaseMode.NEGATIVE.value:
                result = verify_offline_negative(evidence_root)
                schema = result["schema"]
            else:
                verify_case(evidence_root, proof.CaseMode(args.case))
                schema = "pourpoint.released-wheel-proof-verification.v1"
            print(json.dumps({"case_verified": args.case, "schema": schema,
                              "status": "verified"}, sort_keys=True, separators=(",", ":")))
        else:
            if not all(legacy):
                proof.fail(proof.FailureCode.CONFIG, "--horizontal, --distant, and --negative are required")
            verify_all(Path(args.horizontal), Path(args.distant), Path(args.negative))
            print(json.dumps({"cases_verified": 3, "schema": "pourpoint.released-wheel-proof-verification.v1", "status": "verified"},
                             sort_keys=True, separators=(",", ":")))
        return 0
    except proof.ProofFailure as exc:
        print(f"ERROR[{int(exc.code)}]: {exc}", file=sys.stderr)
        return int(exc.code)
    except SystemExit as exc:
        return int(exc.code)
    except Exception as exc:
        print(f"ERROR[70]: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 70


if __name__ == "__main__":
    raise SystemExit(main())
