#!/usr/bin/env python3
"""Stage the source-free native Rust/musl CI image context."""

from __future__ import annotations

import argparse
import hashlib
import json
import lzma
import os
import re
import shutil
import sys
import tomllib
import urllib.request
from pathlib import Path, PurePosixPath

SNAPSHOT = "20261106T000000Z"
RELEASES = {
    "noble": "cdb2f31d809f589719a53c6ad15f255b27569c4059542ada282aaa21b8e164b0",
    "noble-updates": "a8b0e257fdb119466303e9cd8c24dc068b17559383eb80b8519d598153da952e",
    "noble-security": "155c4944df938253ac14a34c80d320fb1cea84c0c661bb6d2cbd540003f966ed",
}
BASES = {
    "amd64": "sha256:78d4744ead1f5b19b04d0df558ab182a5300903900755e515b2ca4d10eb2044d",
    "arm64": "sha256:19cad59657037cffa0ac9a48107019b768900ff1071140965827513ac866bd5e",
}
MISE = {
    "version": "2026.10.25",
    "amd64": "3c4d4abd8950e722543aa2ffe58dd9ee84430fc03c8a94d318f64b279229d8ff",
    "arm64": "4e16dbe5c60f191f6c5f7c2f517c1baf53bc27ccf410de43614fec81b9dfb300",
}
PACKAGES = [
    "binutils",
    "ca-certificates",
    "curl",
    "file",
    "gcc",
    "git",
    "gzip",
    "jq",
    "musl-tools",
    "pkg-config",
    "skopeo",
    "tar",
    "xz-utils",
]
FIXTURE = "scripts/ci/native-musl-readiness.sh"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str) -> bytes:
    request = urllib.request.Request(
        url, headers={"User-Agent": "acyclic-native-ci-context/1"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        if response.status != 200:
            raise RuntimeError(f"{url}: HTTP {response.status}")
        return response.read()


def index_hashes(inrelease: bytes, arch: str) -> list[dict[str, str | int]]:
    text = inrelease.decode("ascii")
    in_sha = False
    found = []
    selected = {
        f"{component}/binary-{arch}/Packages.xz" for component in ("main", "universe")
    }
    for line in text.splitlines():
        if line == "SHA256:":
            in_sha = True
            continue
        if in_sha:
            if not line.startswith(" "):
                break
            fields = line.split()
            if len(fields) == 3 and fields[2] in selected:
                found.append(
                    {"path": fields[2], "sha256": fields[0], "size": int(fields[1])}
                )
    if len(found) != len(selected) or {item["path"] for item in found} != selected:
        raise RuntimeError(
            f"signed InRelease lacks a main/universe Packages.xz hash for {arch}"
        )
    return found


def ca_bootstrap(release: bytes, arch: str) -> tuple[bytes, dict[str, str | int]]:
    indexes = index_hashes(release, arch)
    index = next(
        item for item in indexes if item["path"] == f"main/binary-{arch}/Packages.xz"
    )
    if index["size"] <= 0 or index["size"] > 128 * 1024 * 1024:
        raise RuntimeError("signed ca-certificates package index has an invalid size")
    index_url = (
        f"https://snapshot.ubuntu.com/ubuntu/{SNAPSHOT}/dists/noble/{index['path']}"
    )
    compressed = fetch(index_url)
    if len(compressed) != index["size"] or digest(compressed) != index["sha256"]:
        raise RuntimeError("signed ca-certificates package index verification failed")
    try:
        records = lzma.decompress(compressed).decode("utf-8").split("\n\n")
    except (lzma.LZMAError, EOFError, UnicodeDecodeError) as error:
        raise RuntimeError("ca-certificates package index is corrupt") from error
    matches = []
    for record in records:
        fields = {}
        previous = None
        for line in record.splitlines():
            if line.startswith((" ", "\t")):
                if previous is None:
                    raise RuntimeError("invalid continuation in package index stanza")
                fields[previous] += "\n" + line[1:]
                continue
            key, separator, value = line.partition(":")
            if not separator or not key:
                raise RuntimeError("corrupt package index stanza")
            if key in fields:
                raise RuntimeError("duplicate field in package index stanza")
            fields[key] = value.strip()
            previous = key
        if fields.get("Package") == "ca-certificates":
            matches.append(fields)
    if len(matches) != 1:
        raise RuntimeError(
            "signed package index must contain one ca-certificates stanza"
        )
    package = matches[0]
    version, architecture = package.get("Version"), package.get("Architecture")
    filename, size, checksum = (
        package.get("Filename"),
        package.get("Size"),
        package.get("SHA256"),
    )
    if (
        not version
        or not re.fullmatch(r"[A-Za-z0-9.+:~_-]+", version)
        or architecture != "all"
    ):
        raise RuntimeError(
            "ca-certificates stanza has an invalid version or architecture"
        )
    expected_filename = f"pool/main/c/ca-certificates/ca-certificates_{version}_all.deb"
    path = PurePosixPath(filename or "")
    if (
        filename != expected_filename
        or path.is_absolute()
        or ".." in path.parts
        or "" in path.parts
        or "\\" in filename
        or ":" in filename
    ):
        raise RuntimeError("ca-certificates stanza has an unsafe snapshot filename")
    if (
        not size
        or not size.isdecimal()
        or int(size) <= 0
        or int(size) > 50 * 1024 * 1024
    ):
        raise RuntimeError("ca-certificates stanza has an invalid package size")
    if not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum):
        raise RuntimeError("ca-certificates stanza has an invalid SHA256")
    package_url = f"https://snapshot.ubuntu.com/ubuntu/{SNAPSHOT}/{filename}"
    deb = fetch(package_url)
    if len(deb) != int(size) or digest(deb) != checksum:
        raise RuntimeError("ca-certificates package verification failed")
    receipt: dict[str, str | int] = {
        "name": "ca-certificates",
        "version": version,
        "architecture": architecture,
        "filename": filename,
        "size": int(size),
        "sha256": checksum,
        "index_path": index["path"],
        "index_sha256": index["sha256"],
    }
    return deb, receipt


def selected_versions(root: Path) -> tuple[str, str]:
    tools = tomllib.loads(read_regular(root, "mise.toml").decode("utf-8")).get(
        "tools", {}
    )
    rust = tools.get("rust")
    about = tools.get("github:EmbarkStudios/cargo-about")
    rust_version = rust.get("version") if isinstance(rust, dict) else rust
    about_version = about.get("version") if isinstance(about, dict) else about
    if not isinstance(rust_version, str) or not isinstance(about_version, str):
        raise TypeError("mise.toml must select literal Rust and cargo-about versions")
    for tool, version in (("Rust", rust_version), ("cargo-about", about_version)):
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
            raise RuntimeError(f"unsupported {tool} selector: {version!r}")
    return rust_version, about_version


def read_regular(root: Path, relative: str) -> bytes:
    path = root / relative
    for item in path.parents:
        if item == root.parent:
            break
        if item.is_symlink():
            raise RuntimeError(f"symlink is not allowed in staged inputs: {item}")
    if path.is_symlink() or not path.is_file():
        raise RuntimeError(
            f"required input is missing or not a regular file: {relative}"
        )
    return path.read_bytes()


def input_fingerprint(
    platform: str,
    policy: dict,
    files: dict[str, bytes],
    snapshot_files: dict[str, bytes],
) -> tuple[dict, str]:
    """Canonical fingerprint of policy and staged allowlisted input bytes."""
    manifest = {
        "platform": platform,
        "policy": policy,
        "files": {name: digest(data) for name, data in sorted(files.items())},
        "snapshot_files": {
            name: digest(data) for name, data in sorted(snapshot_files.items())
        },
    }
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()
    return manifest, digest(encoded)


def stage(source: Path, output: Path, platform: str, helpers: list[Path]) -> str:
    if platform not in ("linux/amd64", "linux/arm64"):
        raise RuntimeError("platform must be linux/amd64 or linux/arm64")
    if not helpers:
        raise RuntimeError("at least one effective common helper input is required")
    if output.exists() or output.is_symlink():
        raise RuntimeError(f"output directory must not already exist: {output}")
    root = source.resolve(strict=True)
    arch = platform.split("/")[1]
    rust_version, about_version = selected_versions(root)
    source_recipe = read_regular(root, "containers/ci-native/Containerfile")
    files = {FIXTURE: read_regular(root, FIXTURE)}
    helper_receipts = []
    for helper in helpers:
        if (
            any(path.is_symlink() for path in (helper, *helper.parents))
            or not helper.is_file()
        ):
            raise RuntimeError(
                "helper must be a regular file without symlink components"
            )
        helper_receipts.append(
            {"ordinal": len(helper_receipts), "sha256": digest(helper.read_bytes())}
        )

    snapshot_files = {}
    releases = {}
    release_bytes = {}
    for suite, expected in RELEASES.items():
        url = f"https://snapshot.ubuntu.com/ubuntu/{SNAPSHOT}/dists/{suite}/InRelease"
        data = fetch(url)
        if digest(data) != expected:
            raise RuntimeError(f"snapshot InRelease hash mismatch: {suite}")
        release_bytes[suite] = data
        name = f"snapshot/{suite}/InRelease"
        snapshot_files[name] = data
        releases[suite] = {"sha256": expected, "indexes": index_hashes(data, arch)}
    ca_package, ca_receipt = ca_bootstrap(release_bytes["noble"], arch)
    files["bootstrap/ca-certificates.deb"] = ca_package

    target = {
        "linux/amd64": "x86_64-unknown-linux-musl",
        "linux/arm64": "aarch64-unknown-linux-musl",
    }[platform]
    policy = {
        "base_image": f"ubuntu:24.04@{BASES[arch]}",
        "snapshot": SNAPSHOT,
        "releases": releases,
        "packages": PACKAGES,
        "apt_policy": {
            "install_recommends": False,
            "install_suggests": False,
            "allow_removals": False,
            "phased_updates": "always",
            "update_error_mode": "any",
        },
        "rust_version": rust_version,
        "cargo_about_version": about_version,
        "rust_target": target,
        "mise_release": MISE["version"],
        "rust_profile": "minimal",
        "mise_sha256": MISE[arch],
        "helpers": helper_receipts,
        "bootstrap_ca": ca_receipt,
    }
    rendered = source_recipe.decode("utf-8")
    replacements = {
        "@@BASE_DIGEST@@": "@" + BASES[arch],
        "@@ARCH@@": arch,
        "@@SNAPSHOT@@": SNAPSHOT,
        "@@PACKAGES@@": " ".join(PACKAGES),
        "@@MISE_SHA@@": MISE[arch],
        "@@MISE_VERSION@@": MISE["version"],
        "@@RUST_VERSION@@": rust_version,
        "@@NOBLE_HASH@@": RELEASES["noble"],
        "@@UPDATES_HASH@@": RELEASES["noble-updates"],
        "@@SECURITY_HASH@@": RELEASES["noble-security"],
        "@@CA_PACKAGE_SHA@@": ca_receipt["sha256"],
    }
    for marker, value in replacements.items():
        if marker not in rendered:
            raise RuntimeError(
                f"Containerfile is missing required staging marker: {marker}"
            )
        rendered = rendered.replace(marker, value)
    if "@@" in rendered:
        raise RuntimeError("Containerfile has an unresolved staging marker")
    from_lines = re.findall(
        r"^FROM\s+(.+)$", rendered, flags=re.MULTILINE | re.IGNORECASE
    )
    if from_lines != [f"{policy['base_image']} AS native"]:
        raise RuntimeError(
            "Containerfile must use exactly the declared immutable native base"
        )
    files["Containerfile"] = rendered.encode()
    files["mise.toml"] = (
        '[tools]\nrust = { version = "'
        + rust_version
        + '", profile = "minimal" }\n"github:EmbarkStudios/cargo-about" = "'
        + about_version
        + '"\n'
    ).encode()
    fingerprint_inputs, fingerprint = input_fingerprint(
        platform, policy, files, snapshot_files
    )
    output.mkdir(parents=True)
    try:
        for name, data in files.items():
            destination = output / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        for name, data in snapshot_files.items():
            destination = output / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        (output / "native-inputs.json").write_text(
            json.dumps(
                {"schema": 1, **fingerprint_inputs, "fingerprint": fingerprint},
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )
        os.chmod(output / FIXTURE, 0o755)
    except Exception:
        shutil.rmtree(output)
        raise
    return fingerprint


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path.cwd())
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--helper", type=Path, action="append", required=True)
    args = parser.parse_args()
    try:
        print(stage(args.source, args.output_dir, args.platform, args.helper))
    except (OSError, RuntimeError, TypeError, ValueError) as error:
        print(f"native context: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
