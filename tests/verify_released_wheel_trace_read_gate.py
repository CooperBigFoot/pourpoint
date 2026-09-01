#!/usr/bin/env python3
"""Prove retained released-wheel cases need a completed worker D8 raster read."""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
SCRIPTS = REPOSITORY / "scripts"
sys.path.insert(0, str(SCRIPTS))

import released_wheel_proof as proof  # noqa: E402
import verify_released_wheel_evidence as verifier  # noqa: E402


def strip_completed_raster_reads(root: Path) -> None:
    records = proof.parse_trace_jsonl((root / "trace.jsonl").read_bytes())
    stripped = [record for record in records if record.get("stage") != "cog_fetch_tiles"]
    (root / "trace.jsonl").write_bytes(
        b"".join(proof.canonical_json(record) for record in stripped)
    )
    (root / "artifact-index.json").write_bytes(
        proof.canonical_json(proof.build_artifact_index(root))
    )


def require_read_gate_rejection(root: Path, case: proof.CaseMode) -> None:
    try:
        verifier.verify_case(root.resolve(), case)
    except proof.ProofFailure as error:
        if error.code is not proof.FailureCode.EVIDENCE:
            raise AssertionError(f"stripped {case.value} used {error.code}, not EVIDENCE") from error
        return
    raise AssertionError(f"stripped {case.value} retained evidence was accepted")


def main() -> None:
    denied = proof.DeniedOpener()
    proof.set_network_opener(denied)
    retained = REPOSITORY / "docs/evidence/grit-d8-live/postpublication"
    cases = (
        (proof.CaseMode.HORIZONTAL, "horizontal-boundary"),
        (proof.CaseMode.DISTANT, "distant-region"),
    )
    for case, directory in cases:
        verifier.verify_case((retained / directory).resolve(), case)

    with tempfile.TemporaryDirectory() as temporary:
        temporary_root = Path(temporary)
        for case, directory in cases:
            copied = temporary_root / directory
            shutil.copytree(retained / directory, copied)
            strip_completed_raster_reads(copied)
            require_read_gate_rejection(copied, case)

    if denied.calls != 0:
        raise AssertionError("offline retained-evidence verification attempted network access")
    print("released-wheel retained trace raster-read gate verified")


if __name__ == "__main__":
    main()
