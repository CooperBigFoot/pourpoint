#!/usr/bin/env python3
"""Bind the fast live mode to the two sealed positive outlets."""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPOSITORY / "scripts"))

import released_wheel_proof as proof  # noqa: E402


def main() -> int:
    declaration = json.loads(
        (REPOSITORY / "docs/evidence/grit-d8-live/prepublication/fixed-cases.json").read_text()
    )
    cases = declaration["cases"]
    assert list(proof.HORIZONTAL_FIXED_OUTLET) == cases["horizontal-boundary"]["coordinate"]
    assert list(proof.DISTANT_FIXED_OUTLET) == cases["distant-region"]["coordinate"]
    parsed = proof.parser().parse_args([
        "live", "--case", "horizontal-boundary", "--fixed-case", "--output-dir", "/tmp/x"
    ])
    assert parsed.fixed_case is True
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
