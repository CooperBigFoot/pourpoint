#!/usr/bin/env python3
"""Focused discrimination tests for the active public-documentation checker."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check_public_docs.py")
SPEC = importlib.util.spec_from_file_location("check_public_docs", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class ClaimTextErrorsTests(unittest.TestCase):
    def labels(self, text: str) -> set[str]:
        errors = CHECKER.claim_text_errors(Path("README.md"), text)
        return {error.split(": ", 2)[1] for error in errors}

    def test_stale_package_install_forms_are_rejected(self) -> None:
        for command in (
            "pip install pyshed",
            "uv pip install pyshed",
            "uv add pyshed",
        ):
            with self.subTest(command=command):
                self.assertIn("Stale package instruction", self.labels(command))

    def test_current_package_install_forms_are_accepted(self) -> None:
        for command in (
            "pip install pourpoint",
            "uv pip install pourpoint",
            "uv add pourpoint",
        ):
            with self.subTest(command=command):
                self.assertNotIn("Stale package instruction", self.labels(command))

    def test_generic_http_support_is_rejected_but_r2_is_accepted(self) -> None:
        generic = "Supported roots include local directories and HTTP(S) URLs."
        bounded = "Supported roots include Cloudflare R2 HTTP(S) URLs."
        self.assertIn("Overstated generic HTTP(S) support", self.labels(generic))
        self.assertNotIn("Overstated generic HTTP(S) support", self.labels(bounded))

    def test_retired_hosted_address_is_rejected(self) -> None:
        retired = "https://basin-delineations-public.upstream.tech/merit/hfx-v0.2.0/"
        errors = CHECKER.claim_text_errors(Path("README.md"), retired)
        self.assertTrue(any("retired hosted dataset address" in error for error in errors))

    def test_prospective_organization_name_is_rejected(self) -> None:
        self.assertIn("Prospective-organization naming", self.labels("SCALGO"))


if __name__ == "__main__":
    unittest.main()
