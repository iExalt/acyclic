#!/usr/bin/env python
"""Validate the single-platform OCI archive emitted by the CI image builder."""

import hashlib
import json
import os
import re
import sys
import tarfile

MAX_JSON = 8 * 1024 * 1024
MAX_MEMBERS = 4096
DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def fail(message):
    raise ValueError(message)


def one(archive, name):
    matches = [m for m in archive.getmembers() if m.name == name]
    if len(matches) != 1 or not matches[0].isfile():
        fail(f"missing or duplicate {name}")
    return matches[0]


def json_member(archive, member):
    if member.size > MAX_JSON:
        fail("oversized image JSON")
    with archive.extractfile(member) as stream:
        try:
            return json.loads(stream.read(MAX_JSON + 1))
        except (ValueError, TypeError) as exc:
            fail(f"invalid image JSON: {exc}")


def json_named(archive, name):
    return json_member(archive, one(archive, name))


def blob(archive, descriptor):
    if not isinstance(descriptor, dict):
        fail("invalid blob descriptor")
    digest = descriptor.get("digest")
    size = descriptor.get("size")
    if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
        fail("invalid sha256 digest")
    if not isinstance(size, int) or size < 0:
        fail("invalid descriptor size")
    member = one(archive, "blobs/sha256/" + digest[7:])
    if member.size != size:
        fail(f"blob size mismatch: {digest}")
    h = hashlib.sha256()
    with archive.extractfile(member) as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    if h.hexdigest() != digest[7:]:
        fail(f"blob digest mismatch: {digest}")
    return member, "blobs/sha256/" + digest[7:]


def main():
    if len(sys.argv) != 6:
        fail("usage: validate-ci-image.py ARCHIVE METADATA TARGET PLATFORM FINGERPRINT")
    archive_path, metadata_path, target, platform, fingerprint = sys.argv[1:]
    if platform not in ("linux/amd64", "linux/arm64") or not re.fullmatch(
        r"[0-9a-f]{64}", fingerprint
    ):
        fail("invalid requested platform or fingerprint")
    if os.path.getsize(metadata_path) > MAX_JSON:
        fail("oversized exporter metadata")
    os_name, arch = platform.split("/", 1)
    with tarfile.open(archive_path, "r:") as archive:
        members = archive.getmembers()
        if len(members) > MAX_MEMBERS:
            fail("too many OCI archive members")
        names = [m.name for m in members]
        if len(names) != len(set(names)):
            fail("duplicate archive member")
        if any(
            not (m.isfile() or m.isdir())
            or m.name.startswith("/")
            or ".." in m.name.split("/")
            for m in members
        ):
            fail("unsafe archive member")
        layout = json_named(archive, "oci-layout")
        if not isinstance(layout, dict) or layout.get("imageLayoutVersion") != "1.0.0":
            fail("unsupported OCI layout")
        index = json_named(archive, "index.json")
        if not isinstance(index, dict) or index.get("schemaVersion") != 2:
            fail("unsupported OCI index")
        descs = index.get("manifests")
        if (
            not isinstance(descs, list)
            or len(descs) != 1
            or not isinstance(descs[0], dict)
        ):
            fail("expected one OCI image manifest")
        desc = descs[0]
        if desc.get("platform") != {"architecture": arch, "os": os_name}:
            fail("OCI platform mismatch")
        manifest_member, manifest_name = blob(archive, desc)
        manifest = json_member(archive, manifest_member)
        if not isinstance(manifest, dict) or manifest.get("schemaVersion") != 2:
            fail("unsupported image manifest")
        manifest_digest = desc.get("digest")
        config_descriptor = manifest.get("config")
        config_member, config_name = blob(archive, config_descriptor)
        config = json_member(archive, config_member)
        if (
            not isinstance(config, dict)
            or config.get("os") != os_name
            or config.get("architecture") != arch
        ):
            fail("image config platform mismatch")
        image_config = config.get("config")
        labels = image_config.get("Labels") if isinstance(image_config, dict) else None
        if (
            not isinstance(labels, dict)
            or labels.get("io.acyclic.ci.fingerprint") != fingerprint
        ):
            fail("image fingerprint mismatch")
        layers = manifest.get("layers")
        if not isinstance(layers, list) or not layers:
            fail("image has no layers")
        referenced = {manifest_name, config_name}
        for layer in layers:
            _, name = blob(archive, layer)
            referenced.add(name)
        blobs = {m.name for m in members if m.isfile() and m.name.startswith("blobs/")}
        if blobs != referenced:
            fail("OCI archive contains missing or unreferenced blobs")
        allowed = {"oci-layout", "index.json"} | referenced
        if any(m.isfile() and m.name not in allowed for m in members):
            fail("unsupported OCI archive file")
    with open(metadata_path, encoding="utf-8") as stream:
        metadata = json.load(stream)
    image_digest = (
        metadata.get("containerimage.digest") if isinstance(metadata, dict) else None
    )
    if (
        image_digest != manifest_digest
        or not isinstance(image_digest, str)
        or not DIGEST.fullmatch(image_digest)
    ):
        fail("exporter metadata image digest mismatch")
    h = hashlib.sha256()
    with open(archive_path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    print(
        json.dumps(
            {
                "target": target,
                "platform": platform,
                "fingerprint": fingerprint,
                "image_digest": image_digest,
                "config_digest": config_descriptor["digest"],
                "archive_sha256": h.hexdigest(),
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    try:
        main()
    except (
        OSError,
        tarfile.TarError,
        ValueError,
        KeyError,
        AttributeError,
        TypeError,
    ) as exc:
        print(f"validate-ci-image.py: {exc}", file=sys.stderr)
        sys.exit(1)
