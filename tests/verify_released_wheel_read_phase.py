#!/usr/bin/env python3
"""Falsify offline verification with a transcript containing preflight traffic only."""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent / "scripts"
sys.path.insert(0, str(SCRIPTS))

import released_wheel_proof as proof  # noqa: E402
import verify_released_wheel_evidence as verifier  # noqa: E402


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / "horizontal"
        verifier._write_hand_authored_forgery(root, proof.CaseMode.HORIZONTAL)
        evidence = proof.strict_json_bytes((root / "evidence.json").read_bytes(), canonical=True)
        for kind, allocation in (("flow_dir", 4), ("flow_acc", 16)):
            observation = evidence["ceilings"][kind]["window_allocation_bytes"]
            observation["observed"] = allocation
            observation["margin"] = observation["ceiling"] - allocation
        records, _trace = verifier._expected_trace(root)
        for record in records:
            if record.get("stage") == "cog_fetch_tiles":
                record["bytes"] = 0
                record["requests"] = 0
        trace = b"".join(proof.canonical_json(record) for record in records)
        verifier._rewrite_case(root, evidence, trace)

        preflight_only = {
            "bytes_received": 1132,
            "case_id": proof.CaseMode.HORIZONTAL.value,
            "completed": True,
            "error": None,
            "etag": None,
            "key": "manifest.json",
            "method": "GET",
            "origin": "hosted",
            "range": None,
            "response_content_length": 1132,
            "response_content_range": None,
            "seq": 1,
            "status": 200,
            "url": proof.HOSTED_BASE + "manifest.json",
        }
        (root / "reads.jsonl").write_bytes(proof.canonical_json(preflight_only))
        (root / "artifact-index.json").write_bytes(
            proof.canonical_json(proof.build_artifact_index(root))
        )

        try:
            verifier.verify_case(root.resolve(), proof.CaseMode.HORIZONTAL)
        except proof.ProofFailure:
            return
        raise AssertionError("preflight-only hosted reads satisfied released-worker evidence verification")


if __name__ == "__main__":
    main()
