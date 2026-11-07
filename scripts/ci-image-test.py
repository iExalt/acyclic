#!/usr/bin/env python
"""Synthetic OCI and wrapper failure cases; no BuildKit daemon or registry."""

import hashlib
import io
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tarfile
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts/validate-ci-image.py"
BUILDER = ROOT / "scripts/build-ci-image.sh"
BOOTSTRAP = ROOT / "scripts/with-ci-buildkit.sh"
FP = "a" * 64


def blob(data):
    return "sha256:" + hashlib.sha256(data).hexdigest(), data


def archive(
    path,
    *,
    arch="amd64",
    fingerprint=FP,
    duplicate=False,
    tamper=False,
    bad_digest=False,
):
    config = {
        "os": "linux",
        "architecture": "amd64",
        "config": {"Labels": {"io.acyclic.ci.fingerprint": fingerprint}},
    }
    cd, cb = blob(json.dumps(config, separators=(",", ":")).encode())
    ld, lb = blob(b"synthetic layer")
    manifest = {
        "schemaVersion": 2,
        "config": {"digest": cd, "size": len(cb)},
        "layers": [{"digest": ld, "size": len(lb)}],
    }
    if bad_digest:
        manifest["config"]["digest"] = "sha256:invalid"
    md, mb = blob(json.dumps(manifest, separators=(",", ":")).encode())
    index = {
        "schemaVersion": 2,
        "manifests": [
            {
                "digest": md,
                "size": len(mb),
                "platform": {"os": "linux", "architecture": arch},
            }
        ],
    }
    entries = {
        "oci-layout": b'{"imageLayoutVersion":"1.0.0"}',
        "index.json": json.dumps(index).encode(),
        f"blobs/sha256/{cd[7:]}": cb,
        f"blobs/sha256/{ld[7:]}": (b"tampered layer" if tamper else lb),
        f"blobs/sha256/{md[7:]}": mb,
    }
    with tarfile.open(path, "w") as tf:
        for name, data in entries.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
            if duplicate and name == "index.json":
                tf.addfile(info, io.BytesIO(data))
    meta = path.with_suffix(".json")
    meta.write_text(json.dumps({"containerimage.digest": md}), encoding="utf-8")
    return meta, cd


def validate(path, meta, platform="linux/amd64", fingerprint=FP):
    return subprocess.run(
        [
            sys.executable,
            str(VALIDATOR),
            str(path),
            str(meta),
            "image",
            platform,
            fingerprint,
        ],
        capture_output=True,
        text=True,
    )


def validator_cases(tmp):
    path = tmp / "image [literal] with spaces.tar"
    meta, config_digest = archive(path)
    valid = validate(path, meta)
    assert valid.returncode == 0, valid.stderr
    result = json.loads(valid.stdout)
    assert result["target"] == "image" and result["config_digest"] == config_digest
    assert validate(path, meta, "linux/arm64").returncode != 0
    assert validate(path, meta, fingerprint="b" * 64).returncode != 0
    for suffix, kwargs in (
        ("duplicate", {"duplicate": True}),
        ("tampered", {"tamper": True}),
        ("bad-platform", {"arch": "arm64"}),
        ("bad-digest", {"bad_digest": True}),
    ):
        bad = tmp / f"{suffix}.tar"
        assert validate(bad, archive(bad, **kwargs)[0]).returncode != 0
    empty = tmp / "empty.tar"
    empty.write_bytes(b"")
    assert validate(empty, meta).returncode != 0
    wrong = tmp / "wrong.json"
    wrong.write_text('{"containerimage.digest":"sha256:' + "0" * 64 + '"}')
    assert validate(path, wrong).returncode != 0


def wrapper_failures(tmp):
    context = tmp / "context [literal]"
    context.mkdir()
    (context / "Containerfile").write_text("FROM scratch\n")
    existing = tmp / "already exists"
    existing.mkdir()
    refused = subprocess.run(
        [str(BUILDER), str(context), "image", "linux/amd64", FP, str(existing)],
        cwd=ROOT,
        capture_output=True,
    )
    assert refused.returncode != 0 and existing.is_dir()
    fake = tmp / "fake bin"
    fake.mkdir()
    calls, envlog = tmp / "calls [path]", tmp / "envlog [path]"
    for tool in ("rootlesskit", "slirp4netns", "buildctl", "buildkitd"):
        p = fake / tool
        p.write_text("#!/bin/sh\nexit 0\n")
        p.chmod(0o755)
    launcher = fake / "buildctl-daemonless.sh"
    launcher.write_text(
        "#!/bin/sh\n"
        f"echo call >> {shlex.quote(str(calls))}\n"
        f"env > {shlex.quote(str(envlog))}\n"
        "printf partial-oci > image.oci.tar\nprintf partial-exporter > exporter.json\n"
        "echo 'synthetic partial-export failure' >&2\nexit 23\n"
    )
    launcher.chmod(0o755)
    out = tmp / "failed , ! output"
    env = dict(
        os.environ,
        PATH=str(fake) + os.pathsep + os.environ["PATH"],
        AWS_SECRET_ACCESS_KEY="SYNTHETIC_SECRET_MARKER",
        BUILDCTL_HOST="forbidden",
    )
    failed = subprocess.run(
        [str(BUILDER), str(context), "image", "linux/amd64", FP, str(out)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    assert failed.returncode == 23 and not out.exists()
    assert "synthetic partial-export failure" in failed.stderr
    assert len(calls.read_text().splitlines()) == 1
    inherited = envlog.read_text()
    assert (
        "SYNTHETIC_SECRET_MARKER" not in inherited and "BUILDCTL_HOST" not in inherited
    )
    assert "USER=" + os.environ.get("USER", "") in inherited
    assert "USER=root" not in inherited
    assert (
        "BUILDKITD_FLAGS=--rootless --containerd-worker=false --oci-max-parallelism=4"
        in inherited
    )
    assert "PATH=" + env["PATH"] in inherited
    for name in (
        "HOME",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "DOCKER_CONFIG",
    ):
        value = next(
            line.split("=", 1)[1]
            for line in inherited.splitlines()
            if line.startswith(name + "=")
        )
        assert value != os.environ.get(name) and "ci-image-env." in value
    assert not any(p.name.startswith(".ci-image.") for p in tmp.iterdir())

    source_archive = tmp / "synthetic OCI.tar"
    source_meta, config_digest = archive(source_archive)
    source_metadata = json.loads(source_meta.read_text())
    success_launcher = fake / "buildctl-daemonless.sh"
    success_calls = tmp / "success calls"
    success_launcher.write_text(
        "#!/bin/sh\n"
        f"echo call >> {shlex.quote(str(success_calls))}\n"
        f"cp {shlex.quote(str(source_archive))} image.oci.tar\n"
        f"cp {shlex.quote(str(source_meta))} exporter.json\n"
    )
    success_launcher.chmod(0o755)
    success_out = tmp / "successful , output [literal]"
    successful = subprocess.run(
        [str(BUILDER), str(context), "image", "linux/amd64", FP, str(success_out)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    assert successful.returncode == 0, successful.stderr
    assert len(success_calls.read_text().splitlines()) == 1
    assert {item.name for item in success_out.iterdir()} == {
        "image.oci.tar",
        "exporter.json",
        "manifest.json",
    }
    manifest = json.loads((success_out / "manifest.json").read_text())
    assert manifest == {
        "target": "image",
        "platform": "linux/amd64",
        "fingerprint": FP,
        "image_digest": source_metadata["containerimage.digest"],
        "config_digest": config_digest,
        "archive_sha256": hashlib.sha256(source_archive.read_bytes()).hexdigest(),
    }

    race_launcher = fake / "buildctl-daemonless.sh"
    race_out = tmp / "raced , output"
    sentinel = race_out / "owner file"
    race_launcher.write_text(
        "#!/bin/sh\n"
        f"mkdir {shlex.quote(str(race_out))}\n"
        f"printf keep > {shlex.quote(str(sentinel))}\n"
        f"cp {shlex.quote(str(source_archive))} image.oci.tar\n"
        f"cp {shlex.quote(str(source_meta))} exporter.json\n"
    )
    race_launcher.chmod(0o755)
    raced = subprocess.run(
        [str(BUILDER), str(context), "image", "linux/amd64", FP, str(race_out)],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    assert raced.returncode != 0 and raced.stderr.strip()
    assert sentinel.read_text() == "keep"
    assert {item.name for item in race_out.iterdir()} == {"owner file"}

    preexisting = tmp / "existing buildkit tools"
    preexisting.mkdir()
    marker = preexisting / "owner file"
    marker.write_text("keep")
    rejected = subprocess.run(
        [str(BOOTSTRAP), str(preexisting), "true"], cwd=ROOT, capture_output=True
    )
    assert rejected.returncode != 0 and b"already exists" in rejected.stderr
    assert marker.read_text() == "keep"


def main():
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        validator_cases(tmp)
        wrapper_failures(tmp)
    print("synthetic OCI and wrapper cases passed")


if __name__ == "__main__":
    main()
