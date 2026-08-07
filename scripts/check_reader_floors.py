#!/usr/bin/env python3
"""Check published offering prose for bare public dataset addresses."""

from __future__ import annotations

import argparse
from pathlib import Path, PurePosixPath
import subprocess
import sys


HOST_PATTERN = "basin-delineations-public"
READER_FLOOR = "Reader floor: pourpoint 0.3.0"
EXCLUDED_README_DIRECTORIES = {
    "test",
    "tests",
    "fixture",
    "fixtures",
    "golden",
    "goldens",
}


class CheckError(Exception):
    """An operational error that prevents the policy check from completing."""


def safe_relative_path(value: str, label: str) -> PurePosixPath:
    candidate = PurePosixPath(value)
    if not value or candidate.is_absolute() or ".." in candidate.parts:
        raise CheckError(f"invalid {label} path: {value!r}")
    return candidate


def parse_mkdocs(root: Path) -> tuple[PurePosixPath, set[PurePosixPath]]:
    config_path = root / "mkdocs.yml"
    try:
        text = config_path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise CheckError(f"cannot read mkdocs.yml: {error}") from error

    docs_dir_value: str | None = None
    exclude_docs: set[PurePosixPath] = set()
    seen_docs_dir = False
    seen_exclude_docs = False
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.startswith("docs_dir:"):
            if seen_docs_dir:
                raise CheckError("duplicate docs_dir key in mkdocs.yml")
            seen_docs_dir = True
            value = line[len("docs_dir:") :].strip()
            if not value:
                raise CheckError("docs_dir must be a plain scalar")
            if value[0:1] in {"'", '"'}:
                quote = value[0]
                if len(value) < 2 or value[-1] != quote:
                    raise CheckError("malformed quoted docs_dir scalar")
                value = value[1:-1]
            elif value.endswith(("'", '"')):
                raise CheckError("malformed quoted docs_dir scalar")
            docs_dir_value = value
        elif line.startswith("exclude_docs:"):
            if seen_exclude_docs:
                raise CheckError("duplicate exclude_docs key in mkdocs.yml")
            seen_exclude_docs = True
            if line[len("exclude_docs:") :].strip() != "|":
                raise CheckError("exclude_docs must use a literal block")
            index += 1
            while index < len(lines):
                block_line = lines[index]
                if block_line and not block_line[0].isspace():
                    index -= 1
                    break
                entry = block_line.strip()
                if entry and not entry.startswith("#"):
                    exclude_docs.add(safe_relative_path(entry, "exclude_docs"))
                index += 1
        index += 1

    docs_dir = safe_relative_path(docs_dir_value or "docs", "docs_dir")
    return docs_dir, exclude_docs


def tracked_paths(root: Path) -> list[PurePosixPath]:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        raise CheckError(f"cannot run git: {error}") from error
    if result.returncode != 0:
        diagnostic = result.stderr.decode("utf-8", errors="replace").strip()
        raise CheckError(f"git ls-files failed: {diagnostic or 'unknown error'}")
    try:
        names = result.stdout.decode("utf-8").split("\0")
    except UnicodeError as error:
        raise CheckError("git ls-files returned a non-UTF-8 path") from error
    return sorted(PurePosixPath(name) for name in names if name)


def read_lines(root: Path, path: PurePosixPath) -> list[str]:
    try:
        return (root / Path(*path.parts)).read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        raise CheckError(f"cannot read {path.as_posix()}: {error}") from error


def under(path: PurePosixPath, directory: PurePosixPath) -> PurePosixPath | None:
    try:
        return path.relative_to(directory)
    except ValueError:
        return None


def offering_slices(
    root: Path,
    paths: list[PurePosixPath],
    docs_dir: PurePosixPath,
    excluded_docs: set[PurePosixPath],
) -> list[tuple[PurePosixPath, int, list[str]]]:
    slices: list[tuple[PurePosixPath, int, list[str]]] = []
    path_set = set(paths)
    for path in paths:
        docs_relative = under(path, docs_dir)
        if docs_relative is not None:
            if path.suffix != ".md":
                continue
            if docs_relative in excluded_docs:
                continue
            if docs_relative.parts and docs_relative.parts[0] in {"releases", "decisions"}:
                continue
            slices.append((path, 1, read_lines(root, path)))
            continue

        if path.name in {"README.md", "API.md"}:
            directory_parts = (part.lower() for part in path.parts[:-1])
            if not any(part in EXCLUDED_README_DIRECTORIES for part in directory_parts):
                slices.append((path, 1, read_lines(root, path)))

    changelog = PurePosixPath("CHANGELOG.md")
    if changelog in path_set:
        lines = read_lines(root, changelog)
        headings = [index for index, line in enumerate(lines) if line == "## Unreleased"]
        if len(headings) > 1:
            raise CheckError("CHANGELOG.md contains duplicate ## Unreleased headings")
        if headings:
            start = headings[0] + 1
            end = next(
                (index for index in range(start, len(lines)) if lines[index].startswith("## ")),
                len(lines),
            )
            slices.append((changelog, start + 1, lines[start:end]))
    return sorted(slices, key=lambda item: item[0].as_posix())


def find_bare_occurrences(
    slices: list[tuple[PurePosixPath, int, list[str]]],
) -> list[tuple[PurePosixPath, int]]:
    failures: list[tuple[PurePosixPath, int]] = []
    for path, first_line, lines in slices:
        for index, line in enumerate(lines):
            if HOST_PATTERN not in line:
                continue
            neighbors = lines[max(0, index - 1) : index + 2]
            if not any(READER_FLOOR in neighbor for neighbor in neighbors):
                failures.append((path, first_line + index))
    return failures


def run(root_argument: str) -> int:
    root = Path(root_argument).resolve()
    if not root.is_dir():
        raise CheckError(f"root is not a directory: {root_argument}")
    docs_dir, excluded_docs = parse_mkdocs(root)
    paths = tracked_paths(root)
    slices = offering_slices(root, paths, docs_dir, excluded_docs)
    failures = find_bare_occurrences(slices)
    if not failures:
        print("reader-floor check passed: 0 bare occurrence(s)")
        return 0

    page_count = len({path for path, _ in failures})
    print(
        f"reader-floor check failed: {len(failures)} bare occurrence(s) "
        f"in {page_count} offering page(s)"
    )
    for path, line_number in failures:
        print(f"{path.as_posix()}:{line_number}: bare occurrence of {HOST_PATTERN}")
    return 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    arguments = parser.parse_args()
    try:
        return run(arguments.root)
    except CheckError as error:
        print(f"reader-floor check error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
