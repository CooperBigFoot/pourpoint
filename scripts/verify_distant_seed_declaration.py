#!/usr/bin/env python3
"""verified_seed : SeedDeclaration × HorizontalEvidence → DistantResolvedSeed | LoudFailure."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import released_wheel_proof as proof


def read_canonical(path: Path, label: str) -> Any:
    try:
        return proof.strict_json_bytes(path.read_bytes(), canonical=True)
    except OSError as exc:
        proof.fail(proof.FailureCode.EVIDENCE, f"{label} is unreadable at {path}: {exc}")


def parse_coordinate(value: str) -> tuple[float, float]:
    try:
        parts = value.split(",")
        parsed = (float(parts[0]), float(parts[1]))
    except (IndexError, ValueError) as exc:
        raise argparse.ArgumentTypeError("coordinate must be LON,LAT") from exc
    if len(parts) != 2 or not proof.coordinate(parsed):
        raise argparse.ArgumentTypeError("coordinate must be finite LON,LAT")
    return parsed


def verify(declaration_path: Path, horizontal_seed: tuple[float, float]) -> float:
    declaration = read_canonical(declaration_path, "distant seed declaration")
    seed = proof.validate_distant_seed_declaration(declaration)
    emptied = {**declaration, "resolution": {}}
    try:
        proof.validate_distant_seed_declaration(emptied)
    except proof.ProofFailure as exc:
        if exc.code is not proof.FailureCode.EVIDENCE:
            raise
    else:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "emptied distant seed resolution record was accepted")
    distance = proof.spherical_distance_metres(horizontal_seed, seed)
    if distance < 1_000_000:
        proof.fail(proof.FailureCode.EVIDENCE,
                   "distant discovery seed is below 1000000 metres from the horizontal case")
    return distance


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--declaration", required=True, type=Path)
    result.add_argument("--horizontal-seed", required=True, type=parse_coordinate)
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        args = parser().parse_args(argv)
        distance = verify(args.declaration, args.horizontal_seed)
        print(f"PASS: sealed distant discovery seed resolved and is {distance:.3f} metres from the horizontal case")
        return 0
    except proof.ProofFailure as exc:
        print(f"ERROR[{int(exc.code)}]: {exc}", file=sys.stderr)
        return int(exc.code)
    except Exception as exc:
        print(f"ERROR[70]: {type(exc).__name__}: {exc}", file=sys.stderr)
        return int(proof.FailureCode.INTERNAL)


if __name__ == "__main__":
    raise SystemExit(main())
