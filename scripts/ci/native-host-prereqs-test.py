"""Focused trust-boundary checks for the diagnostic receipt."""

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "native_host_prereqs", Path(__file__).with_name("native-host-prereqs.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReceiptSafetyTest(unittest.TestCase):
    def test_non_url_private_identifier_is_withheld(self):
        self.assertIsNone(MODULE.safe_simulation(b"Origin: private-team\n", b""))
        self.assertIsNone(
            MODULE.safe_simulation(
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3.private-team)\n", b""
            )
        )

    def test_simulation_output_never_resolves_transaction(self):
        argv = [
            "apt-get",
            "-s",
            "--no-install-recommends",
            "--no-upgrade",
            "install",
            "uidmap:amd64=1:4.13+dfsg1-4ubuntu3",
        ]
        cases = (
            (
                "valid-looking Inst",
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3 Ubuntu:24.04/noble [amd64])\n",
                True,
            ),
            (
                "requested Inst without architecture",
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3)\n",
                True,
            ),
            (
                "extra dependency Inst without version",
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3 Ubuntu:24.04/noble [amd64])\nInst libslirp0\n",
                True,
            ),
            (
                "Inst followed by bare Conf",
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3 Ubuntu:24.04/noble [amd64])\nConf uidmap\n",
                True,
            ),
            (
                "conflicting action",
                b"Inst uidmap (1:4.13+dfsg1-4ubuntu3 Ubuntu:24.04/noble [amd64])\nRemv uidmap\n",
                True,
            ),
            ("unknown action", b"Hold uidmap\n", False),
        )
        for name, stdout, disclosed in cases:
            with self.subTest(name=name):
                result = {
                    "state": "OBSERVED",
                    "exit": 0,
                    "reason": "",
                    "out": stdout,
                    "err": b"",
                }
                receipt = MODULE.simulation_receipt(result, argv)
                self.assertEqual(receipt["transaction_status"]["state"], "UNKNOWN")
                self.assertIn(
                    "action/dependency resolution",
                    receipt["transaction_status"]["reason"],
                )
                self.assertIn(
                    "candidate/index binding", receipt["transaction_status"]["reason"]
                )
                self.assertEqual(receipt["argv"], argv)
                self.assertEqual(receipt["exit"], 0)
                self.assertTrue(receipt["output_complete"])
                self.assertEqual(
                    receipt["stdout_sha256"], hashlib.sha256(stdout).hexdigest()
                )
                self.assertEqual(
                    receipt["stderr_sha256"], hashlib.sha256(result["err"]).hexdigest()
                )
                self.assertEqual(receipt["stdout_bytes"], len(stdout))
                self.assertEqual(
                    receipt["stdout"], stdout.decode() if disclosed else None
                )
                if not disclosed:
                    self.assertIn("withheld", receipt["transaction_status"]["reason"])

        for name, result, expected in (
            ("not run", None, "simulation not run"),
            (
                "failed",
                {
                    "state": "OBSERVED",
                    "exit": 100,
                    "reason": "",
                    "out": b"",
                    "err": b"",
                },
                "simulation failed",
            ),
            (
                "timeout",
                {
                    "state": "UNKNOWN",
                    "exit": None,
                    "reason": "command timeout",
                    "out": b"Reading package lists...\n",
                    "err": b"",
                },
                "command timeout",
            ),
            (
                "capped",
                {
                    "state": "UNKNOWN",
                    "exit": None,
                    "reason": "command output exceeds byte limit",
                    "out": b"Reading package lists...\n",
                    "err": b"",
                },
                "byte limit",
            ),
        ):
            with self.subTest(name=name):
                receipt = MODULE.simulation_receipt(result, argv if result else None)
                self.assertEqual(receipt["transaction_status"]["state"], "UNKNOWN")
                self.assertIn(expected, receipt["transaction_status"]["reason"])
                self.assertEqual(receipt["exit"], result["exit"] if result else None)
                self.assertEqual(
                    receipt["output_complete"],
                    result is not None and result["state"] == "OBSERVED",
                )
                if result is not None and result["state"] == "UNKNOWN":
                    self.assertIsNone(receipt["stdout_sha256"])
                    self.assertIsNone(receipt["stderr_sha256"])
                    self.assertEqual(receipt["stdout_bytes"], len(result["out"]))
                    self.assertEqual(
                        receipt["stdout_lines"], len(result["out"].splitlines())
                    )
                    self.assertEqual(receipt["stderr_bytes"], len(result["err"]))
                    self.assertEqual(
                        receipt["stderr_lines"], len(result["err"].splitlines())
                    )
                    self.assertIsNone(receipt["stdout"])

    def test_whole_step_timeout_preserves_receipt_and_kills_child_group(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "receipt.json"
            marker = Path(directory) / "child-finished"
            initial = {"collection_status": "failed", "ready": False}
            receipt.write_text(json.dumps(initial))
            code = """
import importlib.util
import sys
spec = importlib.util.spec_from_file_location("collector", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.run([
    sys.executable, "-c",
    "import pathlib, sys, time; time.sleep(0.5); pathlib.Path(sys.argv[1]).write_text('done')",
    sys.argv[2],
], 4)
"""
            result = subprocess.run(
                [
                    "timeout",
                    "--signal=TERM",
                    "--kill-after=2s",
                    "0.1s",
                    sys.executable,
                    "-c",
                    code,
                    str(Path(MODULE.__file__)),
                    str(marker),
                ],
                capture_output=True,
                timeout=4,
                check=False,
            )
            self.assertEqual(result.returncode, 124)
            time.sleep(0.6)
            self.assertFalse(marker.exists())
            self.assertEqual(json.loads(receipt.read_text()), initial)

    def test_oversized_input_and_output_are_unknown(self):
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory) / "input"
            file.write_bytes(b"x" * 65537)
            record, data = MODULE.read(file, 65536)
            self.assertEqual(record["state"], "UNKNOWN")
            self.assertIsNone(data)
        result = MODULE.run(
            [sys.executable, "-c", "import sys; sys.stdout.write('x'*17000)"]
        )
        self.assertEqual(result["state"], "UNKNOWN")
        self.assertIn("exceeds byte limit", result["reason"])
        self.assertLessEqual(len(result["out"]), 16385)


if __name__ == "__main__":
    unittest.main()
