"""Exercise dependency setup with fake tools; never invoke the system package manager."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("browser-deps.sh")


class BrowserDependencies(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.calls = self.root / "calls"
        self.output = self.root / "output"
        self.cache = self.root / "cache"
        self.cache.mkdir()
        (self.cache / "unrelated.deb").touch()
        tools = self.root / "bin"
        tools.mkdir()
        for name, body in {
            "uv": 'cat "$CASE_OUTPUT"; exit "$CASE_RC"',
            "sudo": 'printf "%s\\n" "$@" >> "$CASE_CALLS"; '
            'if [ "$2" = update ]; then exit "${UPDATE_RC:-0}"; fi; exit "${APT_RC:-0}"',
        }.items():
            tool = tools / name
            tool.write_text(f"#!/bin/sh\n{body}\n")
            tool.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{tools}{os.pathsep}{os.environ['PATH']}",
            "BROWSER_DEPS_CACHE": str(self.cache),
            "CASE_OUTPUT": str(self.output),
            "CASE_CALLS": str(self.calls),
        }

    def run_setup(self, output, code, apt_code=0, update_code=0):
        self.output.write_text(output)
        self.calls.unlink(missing_ok=True)
        return subprocess.run(
            ["sh", str(SCRIPT)],
            check=False,
            env={
                **self.env,
                "CASE_RC": str(code),
                "APT_RC": str(apt_code),
                "UPDATE_RC": str(update_code),
            },
            capture_output=True,
            text=True,
            timeout=5,
        )

    def test_installed_does_not_touch_system_or_replay_cache(self):
        result = self.run_setup("All system dependencies are installed.\n", 0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.calls.exists())

    def test_missing_requests_only_named_packages_with_isolated_archive(self):
        for warm in (False, True):
            with self.subTest(warm=warm):
                if warm:
                    (self.cache / "v2" / "unrelated.deb").touch()
                result = self.run_setup(
                    "Missing system dependencies (2):\n  libalpha1\n  libbeta2:amd64\n",
                    1,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    self.calls.read_text().splitlines(),
                    [
                        "apt-get",
                        "update",
                        "apt-get",
                        "-o",
                        f"Dir::Cache::archives={self.cache}/v2",
                        "install",
                        "-y",
                        "--no-install-recommends",
                        "libalpha1",
                        "libbeta2:amd64",
                    ],
                )

    def test_unknown_output_fails_before_privilege(self):
        for output, code in [
            ("All system dependencies are installed.\n", 1),
            ("Missing system dependencies (1):\n  libalpha1\n", 0),
            ("Missing system dependencies (2):\n  libalpha1\n", 1),
            ("Missing system dependencies (1):\n  --reinstall\n", 1),
            ("Missing system dependencies (1):\n  libalpha1; touch surprise\n", 1),
            ("Missing system dependencies (1):\n  libalpha1\nextra output\n", 1),
            ("", 2),
        ]:
            with self.subTest(output=output, code=code):
                self.assertNotEqual(self.run_setup(output, code).returncode, 0)
                self.assertFalse(self.calls.exists())

    def test_package_manager_failure_propagates_without_repair(self):
        for stage in ("update", "install"):
            with self.subTest(stage=stage):
                result = self.run_setup(
                    "Missing system dependencies (1):\n  libalpha1\n",
                    1,
                    apt_code=7 if stage == "install" else 0,
                    update_code=7 if stage == "update" else 0,
                )
                self.assertEqual(result.returncode, 7)
                calls = self.calls.read_text().splitlines()
                self.assertNotIn("--fix-broken", calls)
                self.assertEqual(calls.count("apt-get"), 2 if stage == "install" else 1)


if __name__ == "__main__":
    unittest.main()
