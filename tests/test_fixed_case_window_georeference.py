#!/usr/bin/env python3
"""Reject a fixed-case raster whose retained X georeference was relocated."""

from __future__ import annotations

import shutil
import struct
import sys
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPOSITORY / "scripts"))

import released_wheel_proof as proof  # noqa: E402
import verify_released_wheel_evidence as verifier  # noqa: E402


def _relocate_model_x(path: Path) -> None:
    data = bytearray(path.read_bytes())
    order = "<" if data[:2] == b"II" else ">"
    ifd_offset = struct.unpack_from(order + "I", data, 4)[0]
    entry_count = struct.unpack_from(order + "H", data, ifd_offset)[0]
    for index in range(entry_count):
        entry_offset = ifd_offset + 2 + 12 * index
        tag, field_type, count, value_offset = struct.unpack_from(
            order + "HHII", data, entry_offset
        )
        if tag == 33922:
            if field_type != 12 or count < 6:
                raise AssertionError("unexpected ModelTiepointTag shape")
            model_x_offset = value_offset + 3 * 8
            model_x = struct.unpack_from(order + "d", data, model_x_offset)[0]
            struct.pack_into(order + "d", data, model_x_offset, model_x + 1_000_000.0)
            path.write_bytes(data)
            return
    raise AssertionError("ModelTiepointTag is absent")


def main() -> int:
    source = REPOSITORY / "docs/evidence/grit-d8-live/prepublication"
    with tempfile.TemporaryDirectory() as temporary:
        copied = Path(temporary) / "prepublication"
        copied.mkdir()
        shutil.copy2(source / "fixed-cases.json", copied / "fixed-cases.json")
        for name in (proof.CaseMode.HORIZONTAL.value, proof.CaseMode.DISTANT.value):
            shutil.copytree(source / name, copied / name)

        horizontal = copied / proof.CaseMode.HORIZONTAL.value
        _relocate_model_x(horizontal / "flow-dir.window.tif")
        (horizontal / "artifact-index.json").write_bytes(
            proof.canonical_json(proof.build_artifact_index(horizontal))
        )
        try:
            verifier.verify_fixed_cases((copied / "fixed-cases.json").resolve())
        except proof.ProofFailure:
            return 0
    raise AssertionError("relocated retained raster georeference was accepted")


if __name__ == "__main__":
    raise SystemExit(main())
