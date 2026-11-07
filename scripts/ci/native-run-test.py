#!/usr/bin/env python3
"""Exercise native adapter identity and lifecycle boundaries without an engine."""

import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


def module(name):
    spec = importlib.util.spec_from_file_location(
        name.replace("-", "_"), Path(__file__).with_name(name + ".py")
    )
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


context = module("native-context")
runner = module("native-run")
REAL_RUN = runner.run
CONFIG = "sha256:" + "a" * 64
CID = "b" * 64


class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        (self.source / "containers/ci-native").mkdir(parents=True)
        (self.source / "scripts/ci").mkdir(parents=True)
        (self.source / "mise.toml").write_text(
            '[tools]\nrust="1.2.3"\n"github:EmbarkStudios/cargo-about"="0.2.3"\n'
        )
        template = (
            Path(__file__).resolve().parents[2] / "containers/ci-native/Containerfile"
        )
        (self.source / "containers/ci-native/Containerfile").write_bytes(
            template.read_bytes()
        )
        (self.source / runner.FIXTURE).write_text("synthetic readiness fixture")
        self.helpers = []
        for name in (
            "build-ci-image.sh",
            "validate-ci-image.py",
            "with-ci-buildkit.sh",
        ):
            helper = self.source / "scripts" / name
            helper.write_text("synthetic helper " + name)
            self.helpers.append(helper)
        lines = [
            f" {hashlib.sha256(p.encode()).hexdigest()} 42 {p}"
            for a in ("amd64", "arm64")
            for c in ("main", "universe")
            for p in (f"{c}/binary-{a}/Packages.xz",)
        ]
        release = ("SHA256:\n" + "\n".join(lines) + "\nMD5Sum:\n").encode()
        self.prepared = self.root / "context"
        with (
            patch.object(context, "fetch", return_value=release),
            patch.object(
                context,
                "RELEASES",
                {s: hashlib.sha256(release).hexdigest() for s in context.RELEASES},
            ),
        ):
            self.fp = context.stage(
                self.source, self.prepared, "linux/amd64", self.helpers
            )
        self.image = self.root / "image"
        self.image.mkdir()
        self.receipt = {
            "target": "native",
            "platform": "linux/amd64",
            "fingerprint": self.fp,
            "image_digest": "sha256:" + "c" * 64,
            "config_digest": CONFIG,
            "archive_sha256": "d" * 64,
        }
        for name in ("image.oci.tar", "exporter.json"):
            (self.image / name).write_text("synthetic archive input")
        (self.image / "manifest.json").write_text(json.dumps(self.receipt))
        self.calls = []
        self.bad_loaded_id = False
        self.payload_exit = 0
        self.cleanup_fails = False
        self.timeout_create = False
        self.addCleanup(patch.stopall)
        patch.dict(
            os.environ,
            {
                "GITHUB_ACTIONS": "true",
                "RUNNER_ENVIRONMENT": "github-hosted",
                "PATH": os.environ["PATH"],
            },
            clear=True,
        ).start()
        patch.object(runner, "run", side_effect=self.fake_run).start()
        patch.object(runner, "stat_is_socket", return_value=True).start()
        original_exists = Path.exists
        patch.object(
            Path,
            "exists",
            lambda p: True if str(p) == "/var/run/docker.sock" else original_exists(p),
        ).start()

    def fake_run(self, command, **_kwargs):
        self.calls.append(command)
        out = ""
        result = 0
        if command[0] == "uname":
            out = "Linux\n" if command[-1] == "-s" else "x86_64\n"
        elif command[0] != "docker" and command[0] != "skopeo":
            out = json.dumps(self.receipt)
        elif "info" in command:
            out = "linux/x86_64\n"
        elif "inspect" in command:
            out = json.dumps(
                {
                    "Id": ("sha256:" + "f" * 64) if self.bad_loaded_id else CONFIG,
                    "Os": "linux",
                    "Architecture": "amd64",
                    "Config": {"Labels": {"io.acyclic.ci.fingerprint": self.fp}},
                }
            )
        elif "create" in command:
            if "--cidfile" in command:
                Path(command[command.index("--cidfile") + 1]).write_text(CID)
            if self.timeout_create:
                raise runner.RunnerTimeout("synthetic create timeout")
            out = CID + "\n"
        elif "wait" in command:
            out = str(self.payload_exit) + "\n"
        elif "rm" in command and self.cleanup_fails:
            result = 1
        return subprocess.CompletedProcess(command, result, out, "")

    def invoke(self, name, mode="candidate"):
        return runner.main(
            [
                str(self.image),
                str(self.prepared),
                str(self.source),
                str(self.root / name),
                mode,
            ]
        )

    def test_both_modes_execute_the_authenticated_immutable_id(self):
        for mode in ("candidate", "pinned"):
            with self.subTest(mode=mode):
                self.assertEqual(self.invoke(mode, mode), 0)
        creates = [c for c in self.calls if "create" in c]
        self.assertEqual(len(creates), 2)
        for command in creates:
            self.assertIn(CONFIG, command)
            self.assertEqual(command[command.index("--entrypoint") + 1], "/bin/sh")
            self.assertEqual(command[command.index("--network") + 1], "none")
            self.assertIn("--read-only", command)
        self.assertTrue(all(c[-1] == CID for c in self.calls if "--force" in c))

    def test_real_command_wrapper_retains_bounded_diagnostics(self):
        command = [
            sys.executable,
            "-c",
            "import sys; print('native stdout sentinel'); print('native stderr sentinel', file=sys.stderr); sys.exit(7)",
        ]
        with self.assertRaises(runner.RunnerError) as raised:
            REAL_RUN(command, env={"PATH": os.environ["PATH"]}, timeout=10)
        self.assertIn("exit status 7", str(raised.exception))
        self.assertIn("native stdout sentinel", str(raised.exception))
        self.assertIn("native stderr sentinel", str(raised.exception))

    def test_bad_inputs_abort_before_any_engine_call(self):
        original = (self.prepared / "native-inputs.json").read_text()
        changed = json.loads(original)
        changed["fingerprint"] = "e" * 64
        (self.prepared / "native-inputs.json").write_text(json.dumps(changed))
        with self.assertRaises(runner.RunnerError):
            self.invoke("bad-fingerprint")
        self.assertEqual(self.calls, [])
        (self.prepared / "native-inputs.json").write_text(original)
        self.helpers[0].write_text("changed effective helper")
        with self.assertRaises(runner.RunnerError):
            self.invoke("stale-helper")
        self.assertFalse(any(c[0] in ("docker", "skopeo") for c in self.calls))

    def test_host_guard_and_missing_artifacts_never_call_engine(self):
        with (
            patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}),
            self.assertRaises(runner.RunnerError),
        ):
            self.invoke("non-hosted")
        self.assertEqual(self.calls, [])
        (self.image / "exporter.json").unlink()
        with self.assertRaises(runner.RunnerError):
            self.invoke("missing-artifact")
        self.assertFalse(any(c[0] in ("docker", "skopeo") for c in self.calls))

    def test_wrong_loaded_config_never_starts_a_payload(self):
        self.bad_loaded_id = True
        with self.assertRaises(runner.RunnerError):
            self.invoke("wrong-id")
        self.assertFalse(any("create" in c for c in self.calls))

    def test_cleanup_failure_cannot_turn_success_green(self):
        self.cleanup_fails = True
        with self.assertRaises(runner.RunnerError):
            self.invoke("cleanup-failure")

    def test_create_timeout_cleans_only_captured_id(self):
        self.timeout_create = True
        with self.assertRaises(runner.RunnerTimeout):
            self.invoke("create-timeout")
        self.assertTrue(any("--force" in c and c[-1] == CID for c in self.calls))

    def test_payload_failure_remains_primary_when_cleanup_also_fails(self):
        self.payload_exit = 23
        self.cleanup_fails = True
        with self.assertRaisesRegex(runner.RunnerError, "23"):
            self.invoke("payload-failure")


if __name__ == "__main__":
    unittest.main()
