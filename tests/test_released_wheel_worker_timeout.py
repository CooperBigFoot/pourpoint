"""run_worker_timeout : SleepingReleasedWorker → BoundedWorkerFailure"""

import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import released_wheel_proof as proof  # noqa: E402


class ReleasedWorkerTimeoutTests(unittest.TestCase):
    def test_real_sleeping_worker_is_terminated_with_worker_failure(self) -> None:
        original_source = proof.WORKER_SOURCE
        original_timeout = getattr(proof, "WORKER_TIMEOUT_SECONDS", None)
        proof.WORKER_SOURCE = "import time; time.sleep(0.75)"
        proof.WORKER_TIMEOUT_SECONDS = 0.10
        try:
            with tempfile.TemporaryDirectory() as temporary_text:
                temporary = Path(temporary_text)
                (temporary / "staging").mkdir()
                install_target = temporary / "install-target"
                install_target.mkdir()
                started = time.monotonic()
                with self.assertRaises(proof.ProofFailure) as raised:
                    proof.run_worker(
                        temporary, install_target, proof.HOSTED_BASE,
                        proof.CaseMode.HORIZONTAL, proof.HORIZONTAL_FIXED_OUTLET, 1,
                        ambient_environment={},
                    )
                elapsed = time.monotonic() - started
            self.assertEqual(raised.exception.code, proof.FailureCode.WORKER)
            self.assertIn("timed out", str(raised.exception))
            self.assertLess(elapsed, 0.50)
        finally:
            proof.WORKER_SOURCE = original_source
            if original_timeout is None:
                del proof.WORKER_TIMEOUT_SECONDS
            else:
                proof.WORKER_TIMEOUT_SECONDS = original_timeout


if __name__ == "__main__":
    unittest.main()
