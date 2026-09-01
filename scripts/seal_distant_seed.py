#!/usr/bin/env python3
"""seal : AbsentPath × OneReleasedWheelProbe → ImmutableResolvedSeedDeclaration."""

from __future__ import annotations

import argparse
import os
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import released_wheel_proof as proof

Probe = Callable[[], dict[str, Any]]


def seal_declaration(path: Path, probe: Probe) -> str:
    if path.exists():
        proof.read_distant_seed_declaration(path)
        return "verified"
    value = probe()
    proof.validate_distant_seed_declaration(value)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
                dir=path.parent, prefix=f".{path.name}.", delete=False) as output:
            temporary = Path(output.name)
            output.write(proof.canonical_json(value))
            output.flush()
            os.fsync(output.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError:
            proof.read_distant_seed_declaration(path)
            return "verified"
        proof.read_distant_seed_declaration(path)
        return "sealed"
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def live_probe(path: Path) -> dict[str, Any]:
    wheel_path = proof.validate_live_environment(path, dict(os.environ))
    wheel = proof.verify_wheel(wheel_path)
    with tempfile.TemporaryDirectory(dir=path.parent, prefix=".distant-seed-probe-") as text:
        root = Path(text)
        install = root / "install-target"
        staging = root / "staging"
        for directory in (install, staging):
            directory.mkdir()
        proof.install_wheel(wheel_path, install, staging / "install.stdout.txt",
                            staging / "install.stderr.txt")
        attempt = proof.run_worker(root, install, proof.HOSTED_BASE,
                                   proof.CaseMode.DISTANT,
                                   proof.DISTANT_DISCOVERY_SEED, 0)
        return {
            "dataset": {"base": proof.HOSTED_BASE,
                        "manifest": {"byte_count": proof.PUBLISHED_MANIFEST.content_length,
                                     "sha256": proof.PUBLISHED_MANIFEST.sha256}},
            "discovery_seed": list(proof.DISTANT_DISCOVERY_SEED),
            "resolution": attempt.result,
            "schema": proof.DISTANT_SEED_SCHEMA,
            "wheel": wheel,
        }


def self_test_resume() -> None:
    with tempfile.TemporaryDirectory() as text:
        root = Path(text)
        path = root / "distant-seed.json"
        calls = 0

        def probe() -> dict[str, Any]:
            nonlocal calls
            calls += 1
            return proof._synthetic_distant_seed_declaration(
                proof.DISTANT_DISCOVERY_SEED)

        if seal_declaration(path, probe) != "sealed" or calls != 1:
            proof.fail(proof.FailureCode.EVIDENCE,
                       "absent declaration did not perform exactly one probe")
        original = path.read_bytes()
        original_stat = path.stat()

        def forbidden_probe() -> dict[str, Any]:
            proof.fail(proof.FailureCode.EVIDENCE,
                       "complete declaration was probed or rewritten")

        if seal_declaration(path, forbidden_probe) != "verified":
            proof.fail(proof.FailureCode.EVIDENCE,
                       "complete declaration did not resume by verification")
        if path.read_bytes() != original or path.stat().st_ino != original_stat.st_ino:
            proof.fail(proof.FailureCode.EVIDENCE,
                       "complete declaration was rewritten")
    print("PASS: complete seed verified with zero hosted reads; absent seed probed once and sealed")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--declaration", type=Path,
                        default=proof.DISTANT_SEED_DECLARATION)
    result.add_argument("--self-test", action="store_true")
    result.add_argument("--section", choices=["resume"])
    return result


def main(argv: list[str] | None = None) -> int:
    try:
        args = parser().parse_args(argv)
        if args.self_test:
            if args.section not in {None, "resume"}:
                proof.fail(proof.FailureCode.CONFIG, "unknown self-test section")
            self_test_resume()
        else:
            path = args.declaration.resolve()
            outcome = seal_declaration(path, lambda: live_probe(path))
            reads = "zero hosted reads" if outcome == "verified" else "one live released-wheel probe"
            print(f"PASS: distant discovery seed {outcome} ({reads}) at {path}")
        return 0
    except proof.ProofFailure as exc:
        print(f"ERROR[{int(exc.code)}]: {exc}", file=sys.stderr)
        return int(exc.code)
    except Exception as exc:
        print(f"ERROR[70]: {type(exc).__name__}: {exc}", file=sys.stderr)
        return int(proof.FailureCode.INTERNAL)


if __name__ == "__main__":
    raise SystemExit(main())
