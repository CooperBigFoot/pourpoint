"""environment_probe : LiveInputMappingRead → RecordedEnvironmentKey"""

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import released_wheel_proof as proof  # noqa: E402


class EnvironmentProbeTests(unittest.TestCase):
    def test_pop_records_a_live_input_read(self) -> None:
        probe = proof.EnvironmentProbe({proof.AUTH_ENV: "ambient-secret"})

        self.assertEqual(probe.pop(proof.AUTH_ENV), "ambient-secret")
        self.assertEqual(probe.live_input_reads, [proof.AUTH_ENV])


if __name__ == "__main__":
    unittest.main()
