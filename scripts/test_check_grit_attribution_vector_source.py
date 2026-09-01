#!/usr/bin/env python3
"""Require the vector-source citation in complete hosted-GRIT attribution."""

import importlib.util
from pathlib import Path
import sys


script = Path(__file__).with_name("check_grit_attribution.py")
spec = importlib.util.spec_from_file_location("check_grit_attribution", script)
if spec is None or spec.loader is None:
    raise SystemExit(f"cannot load {script}")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)

raster_only = """GRIT raster dataset archive 10.5281/zenodo.15715535
Wortmann et al. paper 10.1029/2024WR038308
CC BY-NC 4.0
"""
failures = checker.check_attribution_text("probe", raster_only)
if not any("vector dataset archive" in failure for failure in failures):
    raise SystemExit("attribution without the GRIT vector archive was accepted")

complete = raster_only + "GRIT vector dataset archive 10.5281/zenodo.17435232\n"
if checker.check_attribution_text("probe", complete):
    raise SystemExit("complete GRIT source attribution was rejected")

print("hosted GRIT attribution requires both source archives")
