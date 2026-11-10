#!/usr/bin/env python3
"""Publish an immutable verified alias for the exact tested OCI index."""

import hashlib
import json
import re
import subprocess
import sys


def skopeo(*args):
    return subprocess.check_output(["skopeo", *args], timeout=120)


def promote(repository, tag, digest):
    if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
        raise ValueError("only stable release tags can be verified")
    if not re.fullmatch(r"sha256:[a-f0-9]{64}", digest):
        raise ValueError("expected the tested sha256 index digest")
    source = f"docker://{repository}@{digest}"
    raw = skopeo("inspect", "--raw", source)
    if "sha256:" + hashlib.sha256(raw).hexdigest() != digest:
        raise ValueError("registry index does not match the tested digest")
    platforms = [entry["platform"] for entry in json.loads(raw)["manifests"]]
    if len(platforms) != 2 or sorted(
        (p["os"], p["architecture"]) for p in platforms
    ) != [("linux", "amd64"), ("linux", "arm64")]:
        raise ValueError("expected exactly the two verified Linux architectures")
    alias = f"verified-{tag}"
    # Listing must succeed: an auth/network failure must never mean the alias is absent.
    tags = json.loads(skopeo("list-tags", f"docker://{repository}"))["Tags"] or []
    destination = f"docker://{repository}:{alias}"
    if alias in tags:
        existing = skopeo("inspect", "--raw", destination)
        if existing != raw:
            raise ValueError("refusing to replace an existing verified alias")
        print(f"{alias} already identifies {digest}")
        return
    skopeo("copy", "--all", "--preserve-digests", source, destination)
    if skopeo("inspect", "--raw", destination) != raw:
        raise ValueError("verified alias does not preserve the tested index")
    print(f"{alias} identifies {digest}")


if __name__ == "__main__":
    promote(*sys.argv[1:])
