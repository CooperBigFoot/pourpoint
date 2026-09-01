#!/usr/bin/env python3
"""Reject stale hosted-GRIT state and incomplete canonical citation guidance."""

import importlib.util
from pathlib import Path
import sys


root = Path(__file__).parents[1]
script = Path(__file__).with_name("check_grit_attribution.py")
spec = importlib.util.spec_from_file_location("check_grit_attribution", script)
if spec is None or spec.loader is None:
    raise SystemExit(f"cannot load {script}")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)

stale_claims = (
    "the hosted GRIT (Global River Topology) dataset ships no raster",
    "Its live `manifest.json` does not currently carry an `hfx.aux.d8_raster.v2` entry",
    "the engine cannot discover them from that manifest and skips terminal refinement",
)
active_text = "\n".join(
    (root / relative).read_text(encoding="utf-8")
    for relative in ("docs/how-it-works.md", "docs/raster-cache.md")
)
for claim in stale_claims:
    if claim in active_text:
        raise SystemExit(f"active hosted-GRIT guidance retains stale claim: {claim}")
    if not any(pattern.search(claim) for pattern in checker.DENIALS):
        raise SystemExit(f"attribution checker does not reject stale claim: {claim}")

credits = (root / "docs/credits.md").read_text(encoding="utf-8")
failures = checker.check_attribution_text("pourpoint/docs/credits.md", credits)
if failures:
    raise SystemExit("\n".join(failures))

requires_complete_attribution = getattr(checker, "requires_complete_attribution", None)
if requires_complete_attribution is None:
    raise SystemExit("attribution checker does not select canonical citation surfaces")
if not requires_complete_attribution("pourpoint", "docs/credits.md", credits):
    raise SystemExit("canonical GRIT citation page is outside complete-attribution enforcement")

print("active hosted-GRIT guidance and canonical citation attribution are current")
