#!/usr/bin/env python3
"""Check current public documentation claims and local Markdown links."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[1]
CANONICAL_HOSTED_ROOT = (
    "https://basin-delineations-public.upstream.tech/grit/hfx-v0.3.0/"
)
HOSTED_URL_RE = re.compile(
    r"https://basin-delineations-public\.upstream\.tech/[^\s<>`)\]\"]+"
)
EXCLUDED_PARTS = {
    ".git",
    ".worktrees",
    "decisions",
    "evidence",
    "releases",
    "fixtures",
    "goldens",
    "planning",
}
EXCLUDED_FILES = {"AGENTS.md", "CHANGELOG.md", "CLAUDE.md", "CONTEXT.md"}
PROHIBITED = {
    "Prospective-organization naming": re.compile(r"\bSCALGO\b", re.IGNORECASE),
    "Pending release language": re.compile(r"pending 0\.2\.1", re.IGNORECASE),
    "Unsubstantiated production-adoption language": re.compile(
        r"live-fired in production", re.IGNORECASE
    ),
    "Incorrect no-disk remote claim": re.compile(
        r"nothing (?:is )?(?:copied|downloaded)|never (?:lands|downloaded|copied)",
        re.IGNORECASE,
    ),
    "Incorrect nearest-only snapping claim": re.compile(
        r"(?:nudg(?:e|ed)s?|snap(?:s|ped)?) (?:the |your )?point "
        r"(?:on)?to the nearest river channel",
        re.IGNORECASE,
    ),
    "Overstated exact-point refinement": re.compile(
        r"trims? .* to the exact point", re.IGNORECASE
    ),
    "Overstated generic HTTP(S) support": re.compile(
        r"Supported roots include(?:(?!\n\n).)*?(?<!Cloudflare R2 )HTTP\(S\) URLs",
        re.IGNORECASE | re.DOTALL,
    ),
    "Stale package instruction": re.compile(
        r"(?:uv\s+pip\s+install|pip\s+install|uv\s+add)\s+pyshed\b",
        re.IGNORECASE,
    ),
}

REQUIRED = {
    "README.md": (
        "released Python package is version 0.3.0",
        "Development Status :: 4 - Beta",
        "GRIT 2.0.0 HFX dataset",
        "`fabric_version`: `1.0.0`",
        "`format_version`: `0.3.0`",
        "`adapter_version`: `grit-global-2.1.0`",
        "exactly one dataset hosted by this project",
        "Installing pourpoint does not grant commercial rights",
        "Evaluation and collaboration",
        "Cloudflare R2 HTTP(S) URLs",
    ),
    "CONTRIBUTING.md": ("OIDC Trusted Publishing", "RELEASING.md"),
    "docs/index.md": ("released version 0.3.0", "Evaluation and collaboration"),
    "docs/guide/datasets.md": (
        "Every raw or source hydrofabric must first be compiled",
        "EPSG:4326 with `cells`",
        "EPSG:8857 with `cells`",
        "EPSG:8857 with `km2`",
        "EPSG:4326 with `km2` is rejected",
    ),
    "crates/python/README.md": (
        "Released 0.3.0 documentation",
        "Main development documentation",
        "exactly one dataset hosted by this project",
    ),
    "crates/python/API.md": ("main development branch", "pourpoint-v0.3.0"),
}

LINK_RE = re.compile(r"(?<!!)\[[^]\n]+\]\(([^)]+)\)")


def active_markdown_files() -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "*.md"],
        check=True,
        capture_output=True,
        text=True,
    )
    files = []
    for name in result.stdout.splitlines():
        relative = Path(name)
        if any(part in EXCLUDED_PARTS for part in relative.parts):
            continue
        if relative.name in EXCLUDED_FILES:
            continue
        files.append(ROOT / relative)
    return sorted(files)


def claim_text_errors(relative: Path, text: str) -> list[str]:
    errors = []
    for label, pattern in PROHIBITED.items():
        for match in pattern.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            errors.append(f"{relative}:{line}: {label}: {match.group(0)!r}")
    for match in HOSTED_URL_RE.finditer(text):
        if not match.group(0).startswith(CANONICAL_HOSTED_ROOT):
            line = text.count("\n", 0, match.start()) + 1
            errors.append(
                f"{relative}:{line}: retired hosted dataset address: "
                f"{match.group(0)!r}"
            )
    return errors


def check_claims() -> list[str]:
    errors = []
    for path in active_markdown_files():
        relative = path.relative_to(ROOT)
        text = path.read_text(encoding="utf-8")
        errors.extend(claim_text_errors(relative, text))
    for relative, markers in REQUIRED.items():
        text = (ROOT / relative).read_text(encoding="utf-8")
        for marker in markers:
            if marker not in text:
                errors.append(
                    f"{relative}: missing required public-doc marker: {marker!r}"
                )
    return errors


def check_links() -> list[str]:
    errors = []
    for path in active_markdown_files():
        text = path.read_text(encoding="utf-8")
        for match in LINK_RE.finditer(text):
            raw = match.group(1).strip()
            target_text = raw.split(maxsplit=1)[0].strip("<>")
            if (
                not target_text
                or target_text.startswith("#")
                or "://" in target_text
                or target_text.startswith("mailto:")
            ):
                continue
            target_text = unquote(target_text.split("#", 1)[0].split("?", 1)[0])
            if not target_text:
                continue
            target = (path.parent / target_text).resolve()
            if not target.exists():
                relative = path.relative_to(ROOT)
                line = text.count("\n", 0, match.start()) + 1
                errors.append(f"{relative}:{line}: missing local Markdown target: {target_text}")
    return errors


def main() -> int:
    errors = check_claims() + check_links()
    if errors:
        print("public documentation check failed:")
        for error in errors:
            print(f"- {error}")
        return 1
    print("public documentation claims and local Markdown links: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
