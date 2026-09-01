#!/usr/bin/env python3
"""Reject retained worker raster reads recorded after their localization."""

from __future__ import annotations

import shutil
import sys
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
SCRIPTS = REPOSITORY / "scripts"
sys.path.insert(0, str(SCRIPTS))

import released_wheel_proof as proof  # noqa: E402
import verify_released_wheel_evidence as verifier  # noqa: E402


def move_worker_reads_after_localization(case_root: Path) -> None:
    records = proof.parse_trace_jsonl((case_root / "trace.jsonl").read_bytes())
    fetches = [record for record in records if record.get("stage") == "cog_fetch_tiles"]
    retained = [record for record in records if record.get("stage") != "cog_fetch_tiles"]
    if not fetches:
        raise AssertionError("fixture trace has no worker raster read to reorder")
    reordered = retained + fetches
    (case_root / "trace.jsonl").write_bytes(
        b"".join(proof.canonical_json(record) for record in reordered)
    )
    evidence = proof.strict_json_bytes(
        (case_root / "evidence.json").read_bytes(), canonical=True
    )
    evidence["telemetry"]["accepted_trace_line_numbers"] = [
        next(
            number
            for number, record in enumerate(reordered, 1)
            if record.get("stage") == stage
        )
        for stage in ("raster_localize_flow_dir", "raster_localize_flow_acc")
    ]
    (case_root / "evidence.json").write_bytes(proof.canonical_json(evidence))
    (case_root / "artifact-index.json").write_bytes(
        proof.canonical_json(proof.build_artifact_index(case_root))
    )


def main() -> None:
    proof.set_network_opener(proof.DeniedOpener())
    source = (
        REPOSITORY
        / "docs/evidence/grit-d8-live/prepublication/horizontal-boundary"
    ).resolve()
    with tempfile.TemporaryDirectory() as temporary:
        copied = Path(temporary) / "horizontal-boundary"
        shutil.copytree(source, copied)
        move_worker_reads_after_localization(copied)
        try:
            verifier.verify_case(copied.resolve(), proof.CaseMode.HORIZONTAL)
        except proof.ProofFailure as error:
            if error.code is not proof.FailureCode.EVIDENCE:
                raise AssertionError(
                    f"late worker raster read used wrong failure code: {error.code}"
                ) from error
            if "precedes its localization" not in str(error):
                raise AssertionError(f"late worker raster read used unclear failure: {error}") from error
        else:
            raise AssertionError(
                "worker raster reads recorded after localization were accepted"
            )
    print("released-wheel retained trace read order verified")


if __name__ == "__main__":
    main()
