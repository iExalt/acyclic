#!/usr/bin/env python3
"""Exercise native context staging without network access or image builds."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "native_context", Path(__file__).with_name("native-context.py")
)
native_context = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(native_context)


def release_data() -> bytes:
    entries = [
        f" {hashlib.sha256(path.encode()).hexdigest()} 42 {path}"
        for arch in ("amd64", "arm64")
        for component in ("main", "universe")
        for path in (f"{component}/binary-{arch}/Packages.xz",)
    ]
    return (
        "SHA256:\n" + "\n".join(entries) + "\nMD5Sum:\n" + "fake signed fixture\n"
    ).encode()


class ContextStageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "source"
        (self.root / "containers/ci-native").mkdir(parents=True)
        (self.root / "scripts/ci").mkdir(parents=True)
        (self.root / "mise.toml").write_text(
            '[tools]\nrust = "1.2.3"\n"github:EmbarkStudios/cargo-about" = "0.2.3"\n'
        )
        self.template = (
            Path(__file__).resolve().parents[2] / "containers/ci-native/Containerfile"
        ).read_text()
        (self.root / "containers/ci-native/Containerfile").write_text(self.template)
        (self.root / "scripts/ci/native-musl-readiness.sh").write_text("fixture")
        self.helper = self.root / "install.sh"
        self.helper.write_text("helper one")
        self.release = release_data()
        self.releases = {
            suite: hashlib.sha256(self.release).hexdigest()
            for suite in native_context.RELEASES
        }
        self.fetch_patch = patch.object(
            native_context, "fetch", return_value=self.release
        )
        self.hash_patch = patch.object(native_context, "RELEASES", self.releases)
        self.fetch_patch.start()
        self.hash_patch.start()
        self.addCleanup(self.fetch_patch.stop)
        self.addCleanup(self.hash_patch.stop)
        for name in (".git/config", ".env", "private-source.rs", "workspace/secret.rs"):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("must not stage")

    def stage(self, name: str, platform: str = "linux/amd64") -> tuple[str, set[str]]:
        output = self.root / name
        fingerprint = native_context.stage(self.root, output, platform, [self.helper])
        manifest = json.loads((output / "native-inputs.json").read_text())
        files = {
            str(path.relative_to(output))
            for path in output.rglob("*")
            if path.is_file()
        }
        self.assertEqual(manifest["fingerprint"], fingerprint)
        return fingerprint, files

    def test_allowlist_identity_and_both_platforms(self) -> None:
        amd, files = self.stage("out-amd")
        arm, arm_files = self.stage("out-arm", "linux/arm64")
        expected = {
            "Containerfile",
            "mise.toml",
            "native-inputs.json",
            "scripts/ci/native-musl-readiness.sh",
            *[f"snapshot/{suite}/InRelease" for suite in self.releases],
        }
        self.assertEqual(files, expected)
        self.assertEqual(arm_files, expected)
        self.assertNotEqual(amd, arm)
        for name in (".git/config", ".env", "private-source.rs", "workspace/secret.rs"):
            (self.root / name).write_text("changed unrelated data")
        self.assertEqual(amd, self.stage("out-unrelated")[0])
        original_tools = (self.root / "mise.toml").read_bytes()
        (self.root / "mise.toml").write_text(
            '[tools]\nrust = "1.2.4"\n"github:EmbarkStudios/cargo-about" = "0.2.3"\n'
        )
        self.assertNotEqual(amd, self.stage("out-tool")[0])
        (self.root / "mise.toml").write_bytes(original_tools)
        (self.root / "containers/ci-native/Containerfile").write_text(
            self.template + "\n# changed recipe\n"
        )
        self.assertNotEqual(amd, self.stage("out-recipe")[0])
        (self.root / "containers/ci-native/Containerfile").write_text(self.template)
        self.helper.write_text("helper two")
        self.assertNotEqual(amd, self.stage("out-helper")[0])

    def test_rejects_symlinks_missing_helper_and_bad_or_unreachable_snapshot(
        self,
    ) -> None:
        for selected in ("mise.toml", "scripts/ci/native-musl-readiness.sh"):
            path = self.root / selected
            saved = path.read_bytes()
            path.unlink()
            path.symlink_to(self.helper)
            with self.assertRaises(RuntimeError):
                native_context.stage(
                    self.root,
                    self.root / ("bad-" + selected.replace("/", "-")),
                    "linux/amd64",
                    [self.helper],
                )
            path.unlink()
            path.write_bytes(saved)
        with self.assertRaises(RuntimeError):
            native_context.stage(self.root, self.root / "no-helper", "linux/amd64", [])
        with (
            patch.object(native_context, "fetch", return_value=b"corrupt"),
            self.assertRaisesRegex(RuntimeError, "hash mismatch"),
        ):
            native_context.stage(
                self.root, self.root / "bad-release", "linux/amd64", [self.helper]
            )
        with (
            patch.object(native_context, "fetch", side_effect=OSError("unreachable")),
            self.assertRaisesRegex(OSError, "unreachable"),
        ):
            native_context.stage(
                self.root, self.root / "offline", "linux/amd64", [self.helper]
            )

    def test_recipe_must_preserve_declared_install_and_base_pins(self) -> None:
        recipe = self.root / "containers/ci-native/Containerfile"
        for index, invalid in enumerate(
            (
                self.template.replace("@@BASE_DIGEST@@", ""),
                self.template.replace(
                    "FROM ubuntu:24.04@@BASE_DIGEST@@ AS native",
                    "FROM ubuntu:24.04 AS native\n# @@BASE_DIGEST@@",
                ),
                self.template.replace("@@MISE_SHA@@", "unpinned"),
            )
        ):
            with self.subTest(index=index):
                recipe.write_text(invalid)
                with self.assertRaises(RuntimeError):
                    self.stage(f"bad-recipe-{index}")
                self.assertFalse((self.root / f"bad-recipe-{index}").exists())
        recipe.write_text(self.template)


if __name__ == "__main__":
    unittest.main()
