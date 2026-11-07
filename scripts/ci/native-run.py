#!/usr/bin/env python3
"""Run the native musl readiness fixture in a verified hosted Docker image."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import tomllib
import uuid
from pathlib import Path, PurePosixPath
from typing import Any

DOCKER_HOST = "unix:///var/run/docker.sock"
FIXTURE = "scripts/ci/native-musl-readiness.sh"
BOOTSTRAP_CA = "bootstrap/ca-certificates.deb"
SUITES = {"noble", "noble-updates", "noble-security"}
SHA256 = re.compile(r"[0-9a-f]{64}\Z")


class RunnerError(RuntimeError):
    pass


class RunnerTimeout(RunnerError):
    pass


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def ensure_no_symlink(path: Path) -> None:
    absolute = path.absolute()
    if any(part.is_symlink() for part in (absolute, *absolute.parents)):
        raise RunnerError("symlink path component is not allowed")


def real_directory(value: str, label: str) -> Path:
    path = Path(value)
    ensure_no_symlink(path)
    resolved = path.resolve(strict=True)
    if not resolved.is_dir():
        raise RunnerError(f"{label} is not a directory")
    return resolved


def regular_file(root: Path, relative: str) -> Path:
    rel = PurePosixPath(relative)
    if (
        rel.is_absolute()
        or not rel.parts
        or any(p in ("", ".", "..") for p in rel.parts)
    ):
        raise RunnerError(f"unsafe manifest path: {relative!r}")
    path = root.joinpath(*rel.parts)
    for parent in (path, *path.parents):
        if parent == root.parent:
            break
        if parent.is_symlink():
            raise RunnerError(f"symlink in staged context: {relative}")
    if path.is_symlink() or not path.is_file():
        raise RunnerError(f"staged input is missing or not a regular file: {relative}")
    return path


def load_context(context: Path) -> tuple[dict[str, Any], str, str, str]:
    manifest_path = regular_file(context, "native-inputs.json")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise RunnerError(f"cannot read native input manifest: {exc}") from exc
    if not isinstance(manifest, dict) or manifest.get("schema") != 1:
        raise RunnerError("unsupported native input manifest")
    platform = manifest.get("platform")
    policy = manifest.get("policy")
    files = manifest.get("files")
    snapshots = manifest.get("snapshot_files")
    fingerprint = manifest.get("fingerprint")
    if platform not in ("linux/amd64", "linux/arm64") or not isinstance(policy, dict):
        raise RunnerError("invalid native platform or policy")
    if (
        not isinstance(files, dict)
        or not isinstance(snapshots, dict)
        or not isinstance(fingerprint, str)
        or not SHA256.fullmatch(fingerprint)
    ):
        raise RunnerError("invalid native input hashes")
    allowed = {"Containerfile", "mise.toml", FIXTURE, BOOTSTRAP_CA} | {
        f"snapshot/{s}/InRelease" for s in SUITES
    }
    if set(files) != {"Containerfile", "mise.toml", FIXTURE, BOOTSTRAP_CA} or set(
        snapshots
    ) != {f"snapshot/{s}/InRelease" for s in SUITES}:
        raise RunnerError(
            "native context manifest does not match the strict file allowlist"
        )
    for name, expected in {**files, **snapshots}.items():
        if (
            not isinstance(expected, str)
            or not SHA256.fullmatch(expected)
            or sha256_file(regular_file(context, name)) != expected
        ):
            raise RunnerError(f"staged input hash mismatch: {name}")
    releases = policy.get("releases")
    if not isinstance(releases, dict) or set(releases) != SUITES:
        raise RunnerError("native snapshot release set is invalid")
    arch = platform.split("/")[1]
    expected_indexes = {
        f"{component}/binary-{arch}/Packages.xz" for component in ("main", "universe")
    }
    for suite in SUITES:
        release = releases[suite]
        if (
            not isinstance(release, dict)
            or release.get("sha256") != snapshots[f"snapshot/{suite}/InRelease"]
        ):
            raise RunnerError("snapshot Release digest disagrees with staged bytes")
        indexes = release.get("indexes")
        if (
            not isinstance(indexes, list)
            or any(not isinstance(item, dict) for item in indexes)
            or {item.get("path") for item in indexes} != expected_indexes
        ):
            raise RunnerError("snapshot package index set is invalid")
        if any(
            not isinstance(item.get("sha256"), str)
            or not SHA256.fullmatch(item["sha256"])
            or not isinstance(item.get("size"), int)
            or item["size"] < 0
            for item in indexes
        ):
            raise RunnerError("snapshot package index receipt is invalid")
    ca = policy.get("bootstrap_ca")
    ca_index_path = f"main/binary-{arch}/Packages.xz"
    ca_index_sha = next(
        item["sha256"]
        for item in releases["noble"]["indexes"]
        if item["path"] == ca_index_path
    )
    if (
        not isinstance(ca, dict)
        or ca.get("name") != "ca-certificates"
        or ca.get("architecture") != "all"
    ):
        raise RunnerError("missing or invalid CA bootstrap package metadata")
    filename = ca.get("filename")
    version = ca.get("version")
    if (
        not isinstance(filename, str)
        or not re.fullmatch(
            r"pool/main/c/ca-certificates/ca-certificates_[A-Za-z0-9.+~_-]+_all\.deb",
            filename,
        )
        or not isinstance(version, str)
        or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+:~_-]*", version)
    ):
        raise RunnerError("unsafe CA bootstrap package identity")
    if (
        ca.get("sha256") != files[BOOTSTRAP_CA]
        or not isinstance(ca.get("size"), int)
        or ca["size"] <= 0
        or ca["size"] != regular_file(context, BOOTSTRAP_CA).stat().st_size
        or ca.get("index_path") != ca_index_path
        or ca.get("index_sha256") != ca_index_sha
    ):
        raise RunnerError("CA bootstrap metadata disagrees with authenticated inputs")
    expected_files = allowed | {"native-inputs.json"}
    allowed_dirs = {"."}
    for name in allowed:
        parent = PurePosixPath(name).parent
        while str(parent) != ".":
            allowed_dirs.add(str(parent))
            parent = parent.parent
    actual_files = set()
    for item in context.rglob("*"):
        if item.is_symlink():
            raise RunnerError(f"symlink in staged context: {item.relative_to(context)}")
        relative = item.relative_to(context).as_posix()
        if item.is_file():
            actual_files.add(relative)
        elif item.is_dir():
            if relative not in allowed_dirs:
                raise RunnerError("staged context contains an unexpected directory")
        else:
            raise RunnerError("special file in staged context")
    if actual_files != expected_files:
        raise RunnerError("staged context contains missing or unexpected files")
    canonical = {
        key: manifest[key] for key in ("platform", "policy", "files", "snapshot_files")
    }
    encoded = json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode()
    if hashlib.sha256(encoded).hexdigest() != fingerprint:
        raise RunnerError("native input fingerprint mismatch")

    versions = (policy.get("rust_version"), policy.get("cargo_about_version"))
    if any(
        not isinstance(v, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", v)
        for v in versions
    ):
        raise RunnerError("invalid selected Rust/cargo-about versions")
    try:
        tool_config = tomllib.loads(
            regular_file(context, "mise.toml").read_text(encoding="utf-8")
        )
    except (OSError, ValueError) as exc:
        raise RunnerError(f"invalid staged Mise config: {exc}") from exc
    tools = tool_config.get("tools", {})
    if not isinstance(tools, dict):
        raise RunnerError("staged Mise tools table is invalid")
    rust = tools.get("rust")
    if (
        policy.get("rust_profile") != "minimal"
        or not isinstance(rust, dict)
        or rust.get("profile") != "minimal"
    ):
        raise RunnerError("staged Rust profile must match the minimal native policy")
    about = tools.get("github:EmbarkStudios/cargo-about")
    selected = (
        rust.get("version") if isinstance(rust, dict) else rust,
        about.get("version") if isinstance(about, dict) else about,
    )
    if selected != versions:
        raise RunnerError("staged tool config disagrees with native policy")
    target = {
        "linux/amd64": "x86_64-unknown-linux-musl",
        "linux/arm64": "aarch64-unknown-linux-musl",
    }[platform]
    if policy.get("rust_target") != target:
        raise RunnerError("native Rust target does not match platform")
    return manifest, platform, fingerprint, target


def run(
    command: list[str], *, env: dict[str, str], timeout: int = 30, check: bool = True
) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            command,
            env=env,
            text=True,
            capture_output=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise RunnerTimeout(
            f"{Path(command[0]).name} timed out\nstdout: {str(exc.stdout or '')[-4096:]}\nstderr: {str(exc.stderr or '')[-4096:]}"
        ) from exc
    if check and result.returncode:
        raise RunnerError(
            f"{Path(command[0]).name} failed with exit status {result.returncode}\nstdout: {result.stdout[-4096:]}\nstderr: {result.stderr[-4096:]}"
        )
    return result


def write_json(path: Path, data: dict[str, Any]) -> None:
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image_directory")
    parser.add_argument("prepared_context")
    parser.add_argument("source")
    parser.add_argument("new_output_directory")
    parser.add_argument("mode", choices=("candidate", "pinned"))
    args = parser.parse_args(argv)

    if (
        os.environ.get("GITHUB_ACTIONS") != "true"
        or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
    ):
        raise RunnerError(
            "native image execution is restricted to GitHub-hosted Actions runners"
        )
    if os.getuid() == 0:
        raise RunnerError("native image execution must run as a non-root user")
    for name in (
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "DOCKER_TLS_VERIFY",
        "DOCKER_CERT_PATH",
    ):
        if os.environ.get(name):
            raise RunnerError(f"refusing inherited Docker override: {name}")

    image_dir = real_directory(args.image_directory, "image directory")
    context = real_directory(args.prepared_context, "prepared context")
    source = real_directory(args.source, "source checkout")
    out_arg = Path(args.new_output_directory)
    if out_arg.exists() or out_arg.is_symlink() or out_arg.name in ("", ".", ".."):
        raise RunnerError("output directory must be a new path")
    ensure_no_symlink(out_arg.parent)
    output_parent = out_arg.parent.resolve(strict=True)
    if not output_parent.is_dir():
        raise RunnerError("output parent must exist")
    output = output_parent / out_arg.name

    manifest, native_platform, fingerprint, target = load_context(context)
    for other in (image_dir, context, source):
        if (
            output == other
            or output.is_relative_to(other)
            or other.is_relative_to(output)
        ):
            raise RunnerError(
                "output directory must be separate from image, context and source"
            )
    machine = run(["uname", "-s"], env=dict(os.environ)).stdout.strip()
    arch = run(["uname", "-m"], env=dict(os.environ)).stdout.strip()
    expected_machine = {
        "linux/amd64": ("Linux", "x86_64"),
        "linux/arm64": ("Linux", "aarch64"),
    }[native_platform]
    if (machine, arch) != expected_machine:
        raise RunnerError("host architecture does not match prepared native platform")
    source_fixture = regular_file(source, FIXTURE)
    if sha256_file(source_fixture) != manifest["files"][FIXTURE]:
        raise RunnerError("source readiness fixture differs from prepared context")
    source_mise = regular_file(source, "mise.toml")
    try:
        source_tools = tomllib.loads(source_mise.read_text(encoding="utf-8")).get(
            "tools", {}
        )
    except (OSError, ValueError) as exc:
        raise RunnerError(f"invalid source Mise config: {exc}") from exc
    policy = manifest["policy"]
    helpers = policy.get("helpers")
    helper_names = ["build-ci-image.sh", "validate-ci-image.py", "with-ci-buildkit.sh"]
    if (
        not isinstance(helpers, list)
        or any(not isinstance(item, dict) for item in helpers)
        or [item.get("ordinal") for item in helpers] != list(range(len(helper_names)))
    ):
        raise RunnerError("native policy must identify all three common helper inputs")
    for index, helper in enumerate(helpers):
        expected = helper.get("sha256")
        if (
            not isinstance(expected, str)
            or not SHA256.fullmatch(expected)
            or sha256_file(regular_file(source, "scripts/" + helper_names[index]))
            != expected
        ):
            raise RunnerError("source common helper differs from prepared context")
    if not isinstance(source_tools, dict):
        raise RunnerError("source Mise tools table is invalid")
    source_versions = (
        source_tools.get("rust"),
        source_tools.get("github:EmbarkStudios/cargo-about"),
    )
    source_versions = tuple(
        v.get("version") if isinstance(v, dict) else v for v in source_versions
    )
    if source_versions != (policy["rust_version"], policy["cargo_about_version"]):
        raise RunnerError("source selected tool versions differ from prepared context")

    expected_artifacts = {"image.oci.tar", "exporter.json", "manifest.json"}
    actual_artifacts = set()
    for item in image_dir.iterdir():
        if item.is_symlink() or not item.is_file():
            raise RunnerError("image directory contains a non-regular artifact")
        actual_artifacts.add(item.name)
    if actual_artifacts != expected_artifacts:
        raise RunnerError(
            "image directory must contain exactly the three validated artifacts"
        )
    archive = regular_file(image_dir, "image.oci.tar")
    exporter = regular_file(image_dir, "exporter.json")
    stored_manifest = regular_file(image_dir, "manifest.json")
    validator = regular_file(source, "scripts/validate-ci-image.py")
    validator_env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "PYTHONNOUSERSITE": "1",
    }
    validated = run(
        [
            sys.executable,
            str(validator),
            str(archive),
            str(exporter),
            "native",
            native_platform,
            fingerprint,
        ],
        env=validator_env,
        timeout=180,
    )
    try:
        receipt = json.loads(validated.stdout)
        stored = json.loads(stored_manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise RunnerError(f"invalid image validation receipt: {exc}") from exc
    if (
        not isinstance(receipt, dict)
        or not isinstance(stored, dict)
        or receipt != stored
    ):
        raise RunnerError(
            "stored image receipt differs from independently validated archive"
        )
    config_digest = receipt.get("config_digest")
    if not isinstance(config_digest, str) or not re.fullmatch(
        r"sha256:[0-9a-f]{64}", config_digest
    ):
        raise RunnerError("validator returned an invalid image config digest")

    output.mkdir(mode=0o700)
    home = output / "client-home"
    docker_config = output / "docker-config"
    xdg_config = output / "xdg-config"
    client_tmp = output / "client-tmp"
    payload = output / "payload-output"
    payload_home = output / "payload-home"
    cargo_home = output / "cargo-home"
    for directory in (
        home,
        docker_config,
        xdg_config,
        client_tmp,
        payload,
        payload_home,
        cargo_home,
    ):
        directory.mkdir(mode=0o700)
    log_path = output / "run.log"
    receipt_path = output / "receipt.json"
    receipt.update({"mode": args.mode, "status": "prepared"})
    write_json(receipt_path, receipt)
    client_env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(home),
        "DOCKER_CONFIG": str(docker_config),
        "DOCKER_HOST": DOCKER_HOST,
        "XDG_CONFIG_HOME": str(xdg_config),
        "TMPDIR": str(client_tmp),
    }
    docker = ["docker", "--host", DOCKER_HOST]
    container_id = None
    cidfile = output / "container.cid"
    task_tag = f"localhost/acyclic-native-readiness:{uuid.uuid4().hex}"
    tag_attempted = False
    log_chunks: list[str] = []

    def log(message: str) -> None:
        log_chunks.append(message.rstrip() + "\n")
        try:
            log_path.write_text("".join(log_chunks), encoding="utf-8")
        except OSError:
            pass

    logs_collected = False
    operation_failed = False
    cleanup_failures: list[str] = []
    old_handlers = {}

    def interrupted(signum: int, _frame: Any) -> None:
        raise KeyboardInterrupt(f"received signal {signum}")

    for sig in (signal.SIGINT, signal.SIGTERM):
        old_handlers[sig] = signal.signal(sig, interrupted)
    try:
        sock = Path("/var/run/docker.sock")
        if not sock.exists() or not stat_is_socket(sock):
            raise RunnerError("hosted Docker socket is unavailable")
        daemon = run(
            docker + ["info", "--format", "{{.OSType}}/{{.Architecture}}"],
            env=client_env,
        ).stdout.strip()
        daemon_arch = {
            "x86_64": "amd64",
            "amd64": "amd64",
            "aarch64": "arm64",
            "arm64": "arm64",
        }
        daemon_os, slash, daemon_machine = daemon.partition("/")
        if (
            slash != "/"
            or daemon_os != "linux"
            or daemon_arch.get(daemon_machine) != native_platform.split("/")[1]
        ):
            raise RunnerError(
                "Docker daemon architecture does not match native platform"
            )
        tag_attempted = True
        run(
            [
                "skopeo",
                "copy",
                "--dest-daemon-host",
                DOCKER_HOST,
                f"oci-archive:{archive}",
                f"docker-daemon:{task_tag}",
            ],
            env=client_env,
            timeout=180,
        )
        log("image import complete")
        image_ref = task_tag if args.mode == "candidate" else config_digest
        inspected = run(
            docker + ["image", "inspect", "--format", "{{json .}}", image_ref],
            env=client_env,
        )
        image = json.loads(inspected.stdout)
        if not isinstance(image, dict):
            raise RunnerError("Docker returned invalid image inspection data")
        image_id = image.get("Id")
        image_config = image.get("Config")
        labels = image_config.get("Labels") if isinstance(image_config, dict) else None
        if not isinstance(labels, dict):
            labels = {}
        if (
            image_id != config_digest
            or image.get("Os") != "linux"
            or image.get("Architecture") != native_platform.split("/")[1]
            or labels.get("io.acyclic.ci.fingerprint") != fingerprint
        ):
            raise RunnerError(
                "loaded Docker image identity, platform or fingerprint label mismatch"
            )
        receipt.update({"docker_image_id": image_id, "status": "loaded"})
        write_json(receipt_path, receipt)

        uid, gid = os.getuid(), os.getgid()
        docker_create = docker + [
            "create",
            "--cidfile",
            str(cidfile),
            "--entrypoint",
            "/bin/sh",
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--cpus",
            "4",
            "--memory",
            "16g",
            "--pids-limit",
            "512",
            "--shm-size",
            "256m",
            "--tmpfs",
            "/tmp:rw,nosuid,nodev,size=64m",
            "--user",
            f"{uid}:{gid}",
            "--workdir",
            "/workspace",
            "--mount",
            f"type=bind,src={source},dst=/source,readonly",
            "--mount",
            f"type=bind,src={payload},dst=/output",
            "--mount",
            f"type=bind,src={payload_home},dst=/home/builder",
            "--mount",
            f"type=bind,src={cargo_home},dst=/cargo",
        ]
        env_values = {
            "HOME": "/home/builder",
            "CARGO_HOME": "/cargo",
            "EXPECTED_RUST_VERSION": policy["rust_version"],
            "EXPECTED_CARGO_ABOUT_VERSION": policy["cargo_about_version"],
            "CC_x86_64_unknown_linux_musl": "musl-gcc",
            "CC_aarch64_unknown_linux_musl": "musl-gcc",
        }
        for name, value in env_values.items():
            docker_create += ["--env", f"{name}={value}"]
        payload_script = (
            'test ! -w /source; test -w /output; test -w "$HOME"; test -w "$CARGO_HOME"; exec /source/'
            + FIXTURE
            + ' "$@"'
        )
        docker_create += [
            image_id,
            "-eu",
            "-c",
            payload_script,
            "native-payload",
            target,
            "/output",
        ]
        created = run(docker_create, env=client_env).stdout.strip()
        recorded = (
            cidfile.read_text(encoding="ascii").strip() if cidfile.exists() else ""
        )
        container_id = recorded if re.fullmatch(r"[0-9a-f]{12,64}", recorded) else None
        if not container_id or created != container_id:
            raise RunnerError("Docker did not return a matching captured container ID")
        run(docker + ["start", container_id], env=client_env)
        waited = run(docker + ["wait", container_id], env=client_env, timeout=180)
        try:
            exit_code = int(waited.stdout.strip())
        except ValueError as exc:
            raise RunnerError("Docker wait returned an invalid exit code") from exc
        logs = run(docker + ["logs", container_id], env=client_env, check=False)
        log(logs.stdout + logs.stderr)
        logs_collected = True
        receipt.update(
            {"status": "passed" if exit_code == 0 else "failed", "exit_code": exit_code}
        )
        write_json(receipt_path, receipt)
        if exit_code:
            raise RunnerError(f"readiness payload exited with {exit_code}")
    except BaseException as exc:
        operation_failed = True
        if container_id and not logs_collected:
            try:
                logs = run(
                    docker + ["logs", container_id],
                    env=client_env,
                    check=False,
                    timeout=10,
                )
                log(logs.stdout + logs.stderr)
                logs_collected = True
            except (OSError, RunnerError, subprocess.TimeoutExpired):
                log("diagnostic warning: could not retrieve container logs")
        receipt["status"] = (
            "interrupted"
            if isinstance(
                exc,
                (
                    KeyboardInterrupt,
                    RunnerTimeout,
                    TimeoutError,
                    subprocess.TimeoutExpired,
                ),
            )
            else "error"
        )
        receipt["error"] = f"{type(exc).__name__}: {exc}"
        try:
            write_json(receipt_path, receipt)
        except OSError:
            pass
        log(f"ERROR: {receipt['error']}")
        raise
    finally:
        if container_id is None and cidfile.exists():
            try:
                candidate = cidfile.read_text(encoding="ascii").strip()
                if re.fullmatch(r"[0-9a-f]{12,64}", candidate):
                    container_id = candidate
            except OSError:
                pass
        if container_id:
            try:
                cleanup = run(
                    docker + ["rm", "--force", container_id],
                    env=client_env,
                    check=False,
                    timeout=30,
                )
                if cleanup.returncode:
                    cleanup_failures.append("container removal failed")
            except (OSError, RunnerError, subprocess.TimeoutExpired):
                cleanup_failures.append("container removal failed")
        if tag_attempted:
            try:
                cleanup = run(
                    docker + ["image", "rm", task_tag],
                    env=client_env,
                    check=False,
                    timeout=30,
                )
                if cleanup.returncode:
                    cleanup_failures.append("task image tag removal failed")
            except (OSError, RunnerError, subprocess.TimeoutExpired):
                cleanup_failures.append("task image tag removal failed")
        if cleanup_failures:
            receipt["cleanup_failed"] = True
            receipt["cleanup_errors"] = cleanup_failures
            if not operation_failed:
                receipt["status"] = "cleanup_failed"
            try:
                write_json(receipt_path, receipt)
            except OSError:
                pass
            log("cleanup failed: " + ", ".join(cleanup_failures))
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)
    if cleanup_failures and not operation_failed:
        raise RunnerError("cleanup failed after readiness payload success")
    print(
        "native readiness completed; receipt.json and run.log retained in output directory"
    )
    return 0


def stat_is_socket(path: Path) -> bool:
    import stat

    return stat.S_ISSOCK(path.stat().st_mode)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as error:
        print(
            f"native-run: filesystem operation failed ({type(error).__name__})",
            file=sys.stderr,
        )
        raise SystemExit(1)
    except (RunnerError, subprocess.TimeoutExpired) as error:
        print(f"native-run: {error}", file=sys.stderr)
        raise SystemExit(1)
