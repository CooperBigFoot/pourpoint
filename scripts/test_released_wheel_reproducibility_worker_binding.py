#!/usr/bin/env python3
"""Falsify reproducibility evidence whose retained worker unit transcript was replaced."""

from __future__ import annotations

import shutil
import tempfile
from pathlib import Path

import released_wheel_proof as proof
import verify_released_wheel_evidence as verifier


SOURCE = (Path(__file__).resolve().parents[1]
          / "docs/evidence/grit-d8-live/prepublication")


def main() -> int:
    with tempfile.TemporaryDirectory() as temporary_text:
        evidence = Path(temporary_text) / "prepublication"
        shutil.copytree(SOURCE, evidence)
        run = evidence / "reproducibility/horizontal-boundary/run-2"
        (run / "worker.stdout.txt").write_bytes(b"POURPOINT_CANDIDATES=[]\n")
        (run / "artifact-index.json").write_bytes(
            proof.canonical_json(proof.build_artifact_index(run)))
        try:
            verifier.verify_reproducibility(evidence.resolve())
        except proof.ProofFailure:
            return 0
    raise AssertionError(
        "reproducibility verifier accepted replaced released-worker unit transcript")


if __name__ == "__main__":
    raise SystemExit(main())
