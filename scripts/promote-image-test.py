"""Eligibility and immutable alias failure cases, without registry credentials."""

import hashlib
import importlib.util
import json
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "promotion", Path(__file__).with_name("promote-image.py")
)
promotion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(promotion)


class Promotion(unittest.TestCase):
    def setUp(self):
        self.raw = json.dumps(
            {
                "manifests": [
                    {"platform": {"os": "linux", "architecture": arch}}
                    for arch in ("amd64", "arm64")
                ]
            }
        ).encode()
        self.digest = "sha256:" + hashlib.sha256(self.raw).hexdigest()

    def test_new_alias_preserves_tested_digest(self):
        with patch.object(
            promotion, "skopeo", side_effect=[self.raw, b'{"Tags":[]}', b"", self.raw]
        ) as call:
            promotion.promote("example.invalid/image", "v1.2.3", self.digest)
            self.assertEqual(
                call.call_args_list[2].args,
                (
                    "copy",
                    "--all",
                    "--preserve-digests",
                    f"docker://example.invalid/image@{self.digest}",
                    "docker://example.invalid/image:verified-v1.2.3",
                ),
            )

    def test_rerun_is_noop_but_conflicting_alias_fails(self):
        for existing, accepted in [(self.raw, True), (b"different", False)]:
            with (
                self.subTest(accepted=accepted),
                patch.object(
                    promotion,
                    "skopeo",
                    side_effect=[self.raw, b'{"Tags":["verified-v1.2.3"]}', existing],
                ) as call,
            ):
                if accepted:
                    promotion.promote("example.invalid/image", "v1.2.3", self.digest)
                else:
                    with self.assertRaises(ValueError):
                        promotion.promote(
                            "example.invalid/image", "v1.2.3", self.digest
                        )
                self.assertEqual(call.call_count, 3)

    def test_wrong_digest_or_platforms_never_publish(self):
        for raw in [b"different", json.dumps({"manifests": []}).encode()]:
            digest = (
                self.digest
                if raw == b"different"
                else "sha256:" + hashlib.sha256(raw).hexdigest()
            )
            with patch.object(promotion, "skopeo", return_value=raw) as call:
                with self.assertRaises(ValueError):
                    promotion.promote("example.invalid/image", "v1.2.3", digest)
                self.assertEqual(call.call_count, 1)

    def test_nonstable_tags_never_touch_registry(self):
        for tag in ["latest", "v1.2.3-rc.1", "v01.2.3", "v1.2.3-amd64"]:
            with patch.object(promotion, "skopeo") as call:
                with self.assertRaises(ValueError):
                    promotion.promote("example.invalid/image", tag, self.digest)
                call.assert_not_called()

    def test_registry_failure_never_means_missing(self):
        with patch.object(
            promotion, "skopeo", side_effect=[self.raw, RuntimeError("unavailable")]
        ) as call:
            with self.assertRaises(RuntimeError):
                promotion.promote("example.invalid/image", "v1.2.3", self.digest)
            self.assertEqual(call.call_count, 2)


if __name__ == "__main__":
    unittest.main()
