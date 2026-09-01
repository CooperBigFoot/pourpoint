"""Falsifiers for the sealed distant discovery-seed declaration."""

from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import released_wheel_proof as proof


class DistantSeedDeclarationTests(unittest.TestCase):
    def test_resolution_record_fields_are_validated(self) -> None:
        declaration = json.loads(proof.DISTANT_SEED_DECLARATION.read_text())
        invalid_fields = {
            "area_km2": "not-a-number",
            "geometry_wkb_hex": "00",
            "resolution_method": "",
            "resolved_outlet": ["not", "coordinates"],
            "terminal_unit_id": None,
            "upstream_unit_ids": [],
        }
        for field, invalid in invalid_fields.items():
            with self.subTest(field=field):
                altered = copy.deepcopy(declaration)
                altered["resolution"][field] = invalid
                with self.assertRaises(proof.ProofFailure):
                    proof.validate_distant_seed_declaration(altered)


if __name__ == "__main__":
    unittest.main()
