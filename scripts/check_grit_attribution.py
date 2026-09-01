#!/usr/bin/env python3
"""Check active hosted-GRIT attribution and immutable live-fire history."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

PUBLIC_BASE = "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"
ACCEPTED_IDENTITY = (1426, "02339ff92cbfd1d2ea57bb5332cb843b98115cd7a7395f64c14fac78d2ed643c")
EXPECTED_D8 = {
    "artifacts": {"flow_dir": "aux/d8/flow_dir.tif", "flow_acc": "aux/d8/flow_acc.tif"},
    "metadata": {"crs": "EPSG:8857", "flow_dir_encoding": "grass", "flow_acc_units": "km2"},
}
ATTRIBUTION_MARKERS = {
    "GRIT vector dataset archive": "10.5281/zenodo.17435232",
    "GRIT raster dataset archive": "10.5281/zenodo.15715535",
    "Wortmann et al. paper": "10.1029/2024WR038308",
    "license": "CC BY-NC 4.0",
}
HISTORY = {
    "hfx": {
        "docs/decisions/2026-07-24-grit-successor-prefix-frozen-v0-3-0.md":
            (1614, "a7256db6a251ec0a5485415058f8766b60b48aeadfd36d223debf6f9a98b903a"),
    },
    "pourpoint": {
        "docs/decisions/2026-07-24-bounded-reads-are-tile-count-independent.md":
            (1149, "ade730ddbbb23f8aaf19254b08d6a7b1ecb1cfa72e4e3b680402532ea3ee62a5"),
        "docs/decisions/2026-07-24-remote-cog-parser-ownership.md":
            (4411, "85ced58aa56486b082ee1fd70a6f289f3a69d883a0f2278214d5c590ece7edd1"),
    },
}
HOSTED_ATTRIBUTION = (
    "hosting/grit-hfx-v0.3.0/NOTICE",
    "hosting/grit-hfx-v0.3.0/CITATION.txt",
    "hosting/grit-hfx-v0.3.0/README.md",
)
EXCLUDED_PREFIXES = (
    "docs/decisions/", "docs/evidence/", "docs/releases/",
    "hosting/grit-2.0.0-rehost-v0.3.0/", "hosting/archive/",
)
EXCLUDED_COMPONENTS = {"test", "tests", "fixture", "fixtures", "golden", "goldens"}
RASTER_PATH = re.compile(r"aux/d8/[A-Za-z0-9_.-]+\.tif")
DENIALS = (
    re.compile(r"GRIT(?:(?!\n\n).){0,240}(?:ships no|does not declare|declares no|lacks|without)(?:(?!\n\n).){0,80}(?:D8|refinement)", re.I | re.S),
    re.compile(r"GRIT(?:(?!\n\n).){0,240}(?:D8|refinement)(?:(?!\n\n).){0,80}(?:not shipped|not declared|unavailable|absent)", re.I | re.S),
    re.compile(r"ships no raster", re.I),
    re.compile(r"does not currently carry an? [`']?hfx\.aux\.d8_raster", re.I),
    re.compile(r"cannot discover them from that manifest and skips terminal refinement", re.I),
)


class OperationalError(Exception):
    """Raised when inputs cannot be loaded or identified."""


@dataclass(frozen=True)
class Repository:
    kind: str
    root: Path
    tracked: frozenset[str]


def digest(data: bytes) -> tuple[int, str]:
    return len(data), hashlib.sha256(data).hexdigest()


def tracked_files(root: Path) -> frozenset[str]:
    process = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"], capture_output=True, check=False
    )
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise OperationalError(f"cannot list tracked files in {root}: {detail}")
    try:
        return frozenset(item for item in process.stdout.decode("utf-8").split("\0") if item)
    except UnicodeDecodeError as error:
        raise OperationalError(f"tracked path is not UTF-8 in {root}: {error}") from error


def identify(root: Path) -> Repository:
    root = root.resolve()
    if not root.is_dir():
        raise OperationalError(f"repository is not a directory: {root}")
    tracked = tracked_files(root)
    matches = [kind for kind, records in HISTORY.items() if set(records).issubset(tracked)]
    if len(matches) != 1:
        raise OperationalError(f"cannot uniquely identify repository {root}; matches={matches}")
    return Repository(matches[0], root, tracked)


def repositories(paths: list[Path]) -> dict[str, Repository]:
    result: dict[str, Repository] = {}
    roots: set[Path] = set()
    for path in paths:
        repository = identify(path)
        if repository.root in roots or repository.kind in result:
            raise OperationalError(f"duplicate repository: {repository.root}")
        roots.add(repository.root)
        result[repository.kind] = repository
    missing = sorted(set(HISTORY) - result.keys())
    if missing:
        raise OperationalError(f"missing required repositories: {', '.join(missing)}")
    return result


def text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise OperationalError(f"cannot read {path}: {error}") from error


def check_history(repos: dict[str, Repository]) -> list[str]:
    failures: list[str] = []
    for kind, records in HISTORY.items():
        repository = repos[kind]
        for relative, expected in records.items():
            if relative not in repository.tracked:
                failures.append(f"{kind}/{relative}: historical record is not tracked")
                continue
            path = repository.root / relative
            try:
                actual = digest(path.read_bytes())
            except OSError as error:
                raise OperationalError(f"cannot read {path}: {error}") from error
            if actual != expected:
                failures.append(f"{kind}/{relative}: historical identity changed; expected {expected}, got {actual}")
    return failures


def accepted_rasters(path: Path) -> tuple[set[str], list[str]]:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise OperationalError(f"cannot read accepted manifest {path}: {error}") from error
    failures: list[str] = []
    if digest(data) != ACCEPTED_IDENTITY:
        failures.append(f"accepted manifest: identity must be {ACCEPTED_IDENTITY}, got {digest(data)}")
    try:
        manifest = json.loads(data)
    except json.JSONDecodeError as error:
        raise OperationalError(f"accepted manifest is invalid JSON: {error}") from error
    if manifest.get("format_version") != "0.3.0" or manifest.get("fabric_name") != "grit":
        failures.append("accepted manifest: expected GRIT HFX 0.3.0")
    auxiliary = manifest.get("auxiliary")
    if not isinstance(auxiliary, list):
        raise OperationalError("accepted manifest auxiliary must be an array")
    declarations = [item for item in auxiliary if isinstance(item, dict) and item.get("schema") == "hfx.aux.d8_raster.v2"]
    if len(declarations) != 1:
        failures.append(f"accepted manifest: expected exactly one D8 declaration, got {len(declarations)}")
        return set(), failures
    declaration = declarations[0]
    for field, expected in EXPECTED_D8.items():
        if declaration.get(field) != expected:
            failures.append(f"accepted manifest: D8 {field} must be {expected}, got {declaration.get(field)}")
    artifacts = declaration.get("artifacts", {})
    rasters = set(artifacts.values()) if isinstance(artifacts, dict) else set()
    return rasters, failures


def is_active_surface(relative: str) -> bool:
    if relative.startswith(EXCLUDED_PREFIXES):
        return False
    parts = Path(relative).parts
    if any(part.lower() in EXCLUDED_COMPONENTS for part in parts[:-1]):
        return False
    name = parts[-1]
    if relative.startswith("docs/") and relative.endswith(".md"):
        return True
    if name in {"README.md", "API.md"}:
        return relative != "hosting/grit-hfx-v0.3.0/AUTHORITY.md"
    return relative == "CHANGELOG.md"


def active_texts(repository: Repository) -> dict[str, str]:
    result: dict[str, str] = {}
    for relative in sorted(repository.tracked):
        if not is_active_surface(relative):
            continue
        content = text(repository.root / relative)
        if relative == "CHANGELOG.md":
            marker = re.search(r"^## \[?Unreleased\]?\s*$", content, re.M | re.I)
            if marker:
                following = re.search(r"^## ", content[marker.end():], re.M)
                end = marker.end() + following.start() if following else len(content)
                content = content[marker.start():end]
            else:
                content = ""
        result[relative] = content
    return result


def check_attribution_text(label: str, content: str) -> list[str]:
    """Return missing hosted-GRIT attribution markers for one text surface."""
    return [
        f"{label}: missing {name}"
        for name, marker in ATTRIBUTION_MARKERS.items()
        if marker not in content
    ]


def requires_complete_attribution(kind: str, relative: str, content: str) -> bool:
    """Return whether an active surface must carry the complete hosted-GRIT notice."""
    return PUBLIC_BASE in content or (kind == "pourpoint" and relative == "docs/credits.md")


def check_attribution(repos: dict[str, Repository], rasters: set[str]) -> tuple[list[str], int]:
    failures: list[str] = []
    offering_count = 0
    for kind, repository in repos.items():
        surfaces = active_texts(repository)
        offerings = {relative: content for relative, content in surfaces.items() if PUBLIC_BASE in content}
        if not offerings:
            failures.append(f"{kind}: no active page offers {PUBLIC_BASE}")
        offering_count += len(offerings)
        for relative, content in surfaces.items():
            if requires_complete_attribution(kind, relative, content):
                failures.extend(check_attribution_text(f"{kind}/{relative}", content))
        for relative, content in offerings.items():
            missing_rasters = sorted(rasters - set(RASTER_PATH.findall(content)))
            undeclared = sorted(set(RASTER_PATH.findall(content)) - rasters)
            if missing_rasters:
                failures.append(f"{kind}/{relative}: does not identify declared rasters {missing_rasters}")
            if undeclared:
                failures.append(f"{kind}/{relative}: claims undeclared rasters {undeclared}")
        for relative, content in surfaces.items():
            for pattern in DENIALS:
                if pattern.search(content):
                    failures.append(f"{kind}/{relative}: stale hosted-GRIT no-refinement claim")
                    break

    hfx = repos["hfx"]
    for relative in HOSTED_ATTRIBUTION:
        if relative not in hfx.tracked:
            failures.append(f"hfx/{relative}: hosted attribution file is not tracked")
            continue
        content = text(hfx.root / relative)
        failures.extend(check_attribution_text(f"hfx/{relative}", content))
    return failures, offering_count


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accepted-manifest", type=Path)
    parser.add_argument("--repo", action="append", type=Path, required=True)
    parser.add_argument("--history-only", action="store_true")
    args = parser.parse_args()
    if args.history_only and args.accepted_manifest is not None:
        parser.error("--accepted-manifest cannot be used with --history-only")
    if not args.history_only and args.accepted_manifest is None:
        parser.error("--accepted-manifest is required unless --history-only is used")
    return args


def main() -> int:
    args = parse_args()
    try:
        repos = repositories(args.repo)
        failures = check_history(repos)
        offering_count = 0
        if not args.history_only:
            rasters, manifest_failures = accepted_rasters(args.accepted_manifest)
            failures.extend(manifest_failures)
            if not manifest_failures:
                attribution_failures, offering_count = check_attribution(repos, rasters)
                failures.extend(attribution_failures)
    except OperationalError as error:
        print(f"grit-attribution operational error: {error}", file=sys.stderr)
        return 2
    if failures:
        for failure in sorted(set(failures)):
            print(f"grit-attribution failure: {failure}")
        return 1
    if args.history_only:
        print("grit-attribution history check passed: 3 historical record(s)")
    else:
        print(f"grit-attribution check passed: {offering_count} offering page(s), 3 attribution file(s), 3 historical record(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
