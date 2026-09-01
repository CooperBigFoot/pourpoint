"""live_negative_mode : NegativeLiveInvocation → PreNetworkConfigFailure"""

import argparse
import tempfile
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import released_wheel_proof as proof  # noqa: E402


class EnteredNetwork(Exception):
    pass


class LiveNegativeModeTests(unittest.TestCase):
    def test_negative_live_mode_fails_before_network_or_worker(self) -> None:
        args = proof.parser().parse_args([
            "live", "--case", proof.CaseMode.NEGATIVE.value,
            "--positive-evidence", "unused.json",
            "--output-dir", "unused-output",
        ])
        original_validate = proof.validate_live_environment
        original_verify_wheel = proof.verify_wheel
        original_load_positive = proof._load_positive
        original_install = proof.install_wheel
        original_preflight = proof.preflight_hosted
        original_worker = proof.run_worker
        calls = {"network": 0, "worker": 0}

        def entered_network(_proxy: object) -> None:
            calls["network"] += 1
            raise EnteredNetwork

        def entered_worker(*_args: object, **_kwargs: object) -> object:
            calls["worker"] += 1
            raise AssertionError("negative live mode entered released worker")

        try:
            with tempfile.TemporaryDirectory() as temporary_text:
                root = Path(temporary_text)
                args.output_dir = str(root / "output")
                proof.validate_live_environment = lambda _output, _env: root / "wheel.whl"
                proof.verify_wheel = lambda _wheel: {}
                proof._load_positive = lambda _case, _path: ({}, proof.HORIZONTAL_FIXED_OUTLET)
                proof.install_wheel = lambda *_args: None
                proof.preflight_hosted = entered_network
                proof.run_worker = entered_worker
                with self.assertRaises(proof.ProofFailure) as raised:
                    proof.run_live(args)
            self.assertEqual(raised.exception.code, proof.FailureCode.CONFIG)
            self.assertIn("negative", str(raised.exception).lower())
            self.assertEqual(calls, {"network": 0, "worker": 0})
        finally:
            proof.validate_live_environment = original_validate
            proof.verify_wheel = original_verify_wheel
            proof._load_positive = original_load_positive
            proof.install_wheel = original_install
            proof.preflight_hosted = original_preflight
            proof.run_worker = original_worker


if __name__ == "__main__":
    unittest.main()
