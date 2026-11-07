"""Fixed, bounded observations for the diagnostic native builder job."""

import datetime as dt
import email.utils
import hashlib
import json
import os
import pwd
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path

PACKAGES = (
    "uidmap",
    "libsubid4",
    "slirp4netns",
    "libslirp0",
    "docker-ce-rootless-extras",
    "dbus-user-session",
    "docker-ce",
    "docker-ce-cli",
    "containerd.io",
)
REQUEST = ("uidmap", "slirp4netns", "docker-ce-rootless-extras")
TOOLS = {
    "rootlesskit": ("docker-ce-rootless-extras", "rootlesskit"),
    "slirp4netns": ("slirp4netns",),
    "newuidmap": ("uidmap",),
    "newgidmap": ("uidmap",),
}
PUBLIC_VERSION = {
    "uidmap": r"1:4\.13\+dfsg1-4ubuntu3(?:\.[0-9]+)?",
    "libsubid4": r"1:4\.13\+dfsg1-4ubuntu3(?:\.[0-9]+)?",
    "slirp4netns": r"1\.2\.3-1build2",
    "libslirp0": r"4\.7\.0-1ubuntu3(?:\.[0-9]+)?",
    "dbus-user-session": r"1\.14\.10-4ubuntu4(?:\.[0-9]+)?",
    "containerd.io": r"(?:1|2)\.[0-9]{1,2}\.[0-9]{1,2}-1",
    "docker-ce": r"5:[0-9]{1,2}\.[0-9]{1,2}\.[0-9]{1,2}-1~ubuntu\.24\.04~noble",
    "docker-ce-cli": r"5:[0-9]{1,2}\.[0-9]{1,2}\.[0-9]{1,2}-1~ubuntu\.24\.04~noble",
    "docker-ce-rootless-extras": r"5:[0-9]{1,2}\.[0-9]{1,2}\.[0-9]{1,2}-1~ubuntu\.24\.04~noble",
}
APT_LISTS = "/var/lib/apt/lists/"
FILES = {
    "ubuntu_source": ("/etc/apt/sources.list.d/ubuntu.sources", 65536),
    "docker_source": ("/etc/apt/sources.list.d/docker.sources", 65536),
    "ubuntu_keyring": ("/usr/share/keyrings/ubuntu-archive-keyring.gpg", 65536),
    "docker_keyring": ("/etc/apt/keyrings/docker.asc", 65536),
    "rootlesskit_profile": ("/etc/apparmor.d/rootlesskit", 65536),
    "ubuntu_release": (
        APT_LISTS + "archive.ubuntu.com_ubuntu_dists_noble_InRelease",
        1048576,
    ),
    "security_release": (
        APT_LISTS + "security.ubuntu.com_ubuntu_dists_noble-security_InRelease",
        1048576,
    ),
    "docker_release": (
        APT_LISTS + "download.docker.com_linux_ubuntu_dists_noble_InRelease",
        1048576,
    ),
}
ACTIVE = None


def item(state, value=None, reason=""):
    return {"state": state, "value": value, "reason": reason}


def public_version(name, version):
    return bool(version and re.fullmatch(PUBLIC_VERSION[name], version))


def stop(_signal, _frame):
    if ACTIVE:
        try:
            os.killpg(ACTIVE.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    raise SystemExit(124)


signal.signal(signal.SIGTERM, stop)


def read(path, limit):
    try:
        with open(path, "rb") as stream:
            data = stream.read(limit + 1)
    except FileNotFoundError:
        return item("ABSENT", reason="fixed file absent"), None
    except OSError:
        return item("UNKNOWN", reason="fixed file unreadable"), None
    if len(data) > limit:
        return item("UNKNOWN", reason="fixed file exceeds byte limit"), None
    return item(
        "OBSERVED", {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
    ), data


def run(argv, seconds=4):
    """Capture 16 KiB per stream, killing the command group on timeout or overflow."""
    global ACTIVE
    signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
    try:
        proc = subprocess.Popen(
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            env={**os.environ, "LC_ALL": "C", "LANG": "C"},
        )
    except OSError:
        signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
        return {
            "state": "UNKNOWN",
            "exit": None,
            "reason": "command unavailable",
            "out": b"",
            "err": b"",
        }
    ACTIVE = proc
    signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
    streams = {proc.stdout: bytearray(), proc.stderr: bytearray()}
    selector = selectors.DefaultSelector()
    for stream in streams:
        os.set_blocking(stream.fileno(), False)
        selector.register(stream, selectors.EVENT_READ)
    deadline, reason = time.monotonic() + seconds, ""
    try:
        while selector.get_map() and not reason:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                reason = "command timeout"
                break
            for key, _ in selector.select(remaining):
                stream = key.fileobj
                chunk = os.read(
                    stream.fileno(), min(4096, 16385 - len(streams[stream]))
                )
                if not chunk:
                    selector.unregister(stream)
                else:
                    streams[stream].extend(chunk)
                    if len(streams[stream]) > 16384:
                        reason = "command output exceeds byte limit"
                        break
        if not reason:
            try:
                proc.wait(timeout=max(0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                reason = "command timeout"
    finally:
        if reason:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        proc.wait()
        ACTIVE = None
        selector.close()
        proc.stdout.close()
        proc.stderr.close()
    return {
        "state": "UNKNOWN" if reason else "OBSERVED",
        "exit": None if reason else proc.returncode,
        "reason": reason,
        "out": bytes(streams[proc.stdout]),
        "err": bytes(streams[proc.stderr]),
    }


def text(result, stream="out"):
    if result["state"] != "OBSERVED":
        return None
    try:
        return result[stream].decode()
    except UnicodeError:
        return None


def query(result):
    return {key: result[key] for key in ("state", "exit", "reason")}


def installed(result):
    out, err = text(result), text(result, "err")
    if out is None or err is None or result["exit"] not in (0, 1):
        return None
    present, absent = {}, set()
    for line in out.splitlines():
        parts = line.split("\t")
        if len(parts) != 4 or parts[0] not in PACKAGES or parts[0] in present:
            return None
        name, state, version, arch = parts
        if state != "install ok installed" or arch not in ("amd64", "arm64", "all"):
            return None
        present[name] = {"version": version, "architecture": arch}
    for line in err.splitlines():
        match = re.fullmatch(
            r"dpkg-query: no packages found matching ([a-z0-9.+-]+)", line
        )
        if not match or match.group(1) not in PACKAGES:
            return None
        absent.add(match.group(1))
    if absent & present.keys() or bool(absent) != (result["exit"] == 1):
        return None
    return present, absent


def candidates(result):
    out = text(result)
    if out is None or result["exit"] != 0:
        return {}
    found, name = {}, None
    for line in out.splitlines():
        header = re.fullmatch(r"([a-z0-9.+-]+):", line)
        if header:
            name = header.group(1) if header.group(1) in PACKAGES else None
        elif name and (match := re.fullmatch(r"\s*Candidate: (\S+)", line)):
            if name in found:
                return {}
            found[name] = match.group(1)
    return found


def safe_simulation(stdout, stderr):
    """Disclose raw text only when every line uses a closed public grammar."""
    try:
        out, err = stdout.decode(), stderr.decode()
    except UnicodeError:
        return None
    fixed = {
        "Reading package lists...",
        "Building dependency tree...",
        "Reading state information...",
        "The following NEW packages will be installed:",
        "The following additional packages will be installed:",
    }
    for line in out.splitlines() + err.splitlines():
        if not line or line in fixed:
            continue
        if re.fullmatch(
            r"[0-9]{1,3} upgraded, [0-9]{1,3} newly installed, "
            r"[0-9]{1,3} to remove and [0-9]{1,3} not upgraded\.",
            line,
        ):
            continue
        if (
            line.startswith("  ")
            and line.split()
            and all(name in PUBLIC_VERSION for name in line.split())
        ):
            continue
        match = re.fullmatch(
            r"(Inst|Conf|Remv|Purg) ([a-z0-9.+-]+)(?: \[([^]]+)\])?"
            r"(?: \(([^ )]+)(?: (Ubuntu:24\.04/(?:noble|noble-updates|noble-security)|Docker:stable)"
            r" \[(amd64|arm64)\])?\))?",
            line,
        )
        if not match:
            return None
        _kind, name, old, version, _origin, _arch = match.groups()
        if name not in PUBLIC_VERSION or any(
            not public_version(name, v) for v in (old, version) if v
        ):
            return None
    return {"stdout": out, "stderr": err}


def simulation_receipt(
    result, argv=None, no_run_reason="exact request set unavailable"
):
    reason = "complete action/dependency resolution and authenticated candidate/index binding unverified"
    simulation = {
        "execution_status": "NOT_RUN",
        "reason": "simulation not run",
        "argv": argv,
        "exit": None,
        "output_complete": False,
        "stdout": None,
        "stderr": None,
        "stdout_sha256": None,
        "stderr_sha256": None,
        "stdout_bytes": None,
        "stderr_bytes": None,
        "stdout_lines": None,
        "stderr_lines": None,
    }
    if result is None:
        reason += f"; {no_run_reason}; simulation not run"
    else:
        simulation["execution_status"] = result["state"]
        simulation["output_complete"] = result["state"] == "OBSERVED"
        simulation["exit"] = result["exit"]
        for stream in ("out", "err"):
            label = "stdout" if stream == "out" else "stderr"
            data = result[stream]
            if simulation["output_complete"]:
                simulation[label + "_sha256"] = hashlib.sha256(data).hexdigest()
            simulation[label + "_bytes"] = len(data)
            simulation[label + "_lines"] = len(data.splitlines())
        if result["state"] != "OBSERVED":
            reason += "; " + (result["reason"] or "simulation incomplete")
            simulation["reason"] = result["reason"] or "simulation incomplete"
        else:
            simulation["reason"] = (
                "simulation completed" if result["exit"] == 0 else "simulation failed"
            )
            if result["exit"] != 0:
                reason += "; simulation failed"
            safe = safe_simulation(result["out"], result["err"])
            if safe is None:
                reason += "; raw simulation withheld: non-public or unknown line"
                simulation["reason"] += "; raw simulation withheld"
            else:
                simulation["stdout"], simulation["stderr"] = (
                    safe["stdout"],
                    safe["stderr"],
                )
    simulation["transaction_status"] = item("UNKNOWN", reason=reason)
    return simulation


def release_dates(data):
    top = data[:8192]
    if len(data) > 8192 and b"\n\n" not in top:
        return item("UNKNOWN", reason="Release header exceeds byte limit")
    try:
        lines = top.split(b"\n\n", 1)[0].decode().splitlines()
    except UnicodeError:
        return item("UNKNOWN", reason="Release header encoding invalid")
    dates = {}
    for name in ("Date", "Valid-Until"):
        found = [
            line[len(name) + 2 :] for line in lines if line.startswith(name + ": ")
        ]
        normalized = None
        if len(found) == 1 and len(found[0]) <= 80:
            try:
                normalized = email.utils.parsedate_to_datetime(found[0]).isoformat()
            except (TypeError, ValueError):
                pass
        dates[name.lower().replace("-", "_")] = normalized
    return item("OBSERVED", dates, "dates are observations; authentication UNKNOWN")


def main(receipt_path):
    target = Path(receipt_path)
    with target.open("rb") as stream:
        initial = stream.read(262145)
    if len(initial) > 262144:
        raise ValueError("initialized receipt exceeds byte limit")
    receipt = json.loads(initial)
    receipt["collected_at_utc"] = dt.datetime.now(dt.UTC).isoformat()
    receipt["ready"] = receipt["candidate_ready"] = False
    blockers = [
        "diagnostic-only revision",
        "candidate/index authentication UNKNOWN",
        "namespace execution unverified",
    ]
    evidence = {}
    checkout, architecture = (
        run(["git", "rev-parse", "HEAD"]),
        run(["dpkg", "--print-architecture"]),
    )
    sha, arch = (text(checkout) or "").strip(), (text(architecture) or "").strip()
    receipt["fields"]["checkout_sha"] = (
        item("OBSERVED", sha)
        if checkout["exit"] == 0 and re.fullmatch(r"[0-9a-f]{40}", sha)
        else item("UNKNOWN", reason="actual checkout unavailable")
    )
    receipt["fields"]["native_architecture"] = (
        item("OBSERVED", arch)
        if architecture["exit"] == 0 and arch in ("amd64", "arm64")
        else item("UNKNOWN", reason="host architecture unavailable")
    )
    evidence["query_exits"] = {
        "checkout": query(checkout),
        "architecture": query(architecture),
    }
    if arch != {"linux/amd64": "amd64", "linux/arm64": "arm64"}.get(
        receipt["fields"]["matrix_architecture"]["value"]
    ):
        blockers.append("matrix and host architecture mismatch")

    namespace = {"caller": item("OBSERVED", {"uid": os.getuid(), "gid": os.getgid()})}
    for name in (
        "kernel.unprivileged_userns_clone",
        "user.max_user_namespaces",
        "kernel.apparmor_restrict_unprivileged_userns",
    ):
        state, data = read("/proc/sys/" + name.replace(".", "/"), 65536)
        value = data.decode("ascii").strip() if data and data.isascii() else None
        namespace[name] = (
            item("OBSERVED", value)
            if value and value.isdecimal()
            else item("UNKNOWN", reason=state["reason"] or "invalid sysctl")
        )
    for kind in ("subuid", "subgid"):
        state, data = read("/etc/" + kind, 65536)
        ranges = []
        if data:
            username = pwd.getpwuid(os.getuid()).pw_name
            for line in data.decode(errors="replace").splitlines():
                parts = line.split(":")
                if (
                    len(parts) == 3
                    and parts[0] == username
                    and all(x.isdecimal() for x in parts[1:])
                    and int(parts[2])
                ):
                    ranges.append([int(parts[1]), int(parts[2])])
        namespace[kind] = (
            item("OBSERVED", ranges)
            if ranges
            else item("UNKNOWN", reason=state["reason"] or "caller ranges absent")
        )
    evidence["namespace"] = namespace
    if (
        namespace["kernel.unprivileged_userns_clone"]["value"] != "1"
        or (int(namespace["user.max_user_namespaces"]["value"] or 0) == 0)
        or namespace["kernel.apparmor_restrict_unprivileged_userns"]["value"] != "1"
    ):
        blockers.append("namespace sysctl disabled or unknown")

    tools = {}
    for name, owners in TOOLS.items():
        resolved = shutil.which(name)
        if not resolved:
            tools[name] = item("ABSENT", reason="tool absent")
            continue
        location = str(Path(resolved).resolve())
        if location not in (f"/usr/bin/{name}", f"/bin/{name}"):
            tools[name] = item("UNKNOWN", reason="tool path outside fixed public paths")
            continue
        try:
            info = Path(location).stat()
        except OSError:
            tools[name] = item("UNKNOWN", reason="tool metadata unreadable")
            continue
        owner = run(["dpkg-query", "-S", location])
        owner_text = (text(owner) or "").strip()
        matched = next((x for x in owners if owner_text == f"{x}: {location}"), None)
        tools[name] = item(
            "OBSERVED",
            {
                "path": location,
                "uid": info.st_uid,
                "gid": info.st_gid,
                "mode": format(stat.S_IMODE(info.st_mode), "04o"),
                "dpkg_owner": item("OBSERVED", matched)
                if matched and owner["exit"] == 0
                else item("UNKNOWN", reason="dpkg owner unverified"),
                "owner_query_exit": owner["exit"],
            },
        )
    evidence["tools"] = tools

    files, contents = {}, {}
    for name, (location, limit) in FILES.items():
        files[name], contents[name] = read(location, limit)
        if name.endswith("_release") and contents[name] is not None:
            files[name]["header"] = release_dates(contents[name])
    evidence["fixed_files"] = files
    binary = (tools["rootlesskit"]["value"] or {}).get("path")
    profile_name = None
    if binary and contents["rootlesskit_profile"]:
        try:
            for line in contents["rootlesskit_profile"].decode().splitlines():
                match = re.fullmatch(
                    r"\s*profile\s+(?:(rootlesskit)\s+)?"
                    + re.escape(binary)
                    + r"\s+(?:flags=\([a-z,]+\)\s*)?\{\s*",
                    line,
                )
                if match:
                    profile_name = match.group(1) or binary
                    break
        except UnicodeError:
            pass
    loaded_state, loaded_data = read("/sys/kernel/security/apparmor/profiles", 65536)
    loaded = bool(
        profile_name and loaded_data and (profile_name + " (").encode() in loaded_data
    )
    evidence["rootlesskit_profile"] = {
        "path_bound": item("OBSERVED", True)
        if profile_name
        else item("UNKNOWN", reason="profile binding unverified"),
        "loaded": item("OBSERVED", True)
        if loaded
        else item(
            "UNKNOWN",
            reason=loaded_state["reason"] or "matching loaded profile unverified",
        ),
    }

    identities = {}
    patterns = {
        "python": r"Python ([0-9]+\.[0-9]+\.[0-9]+)",
        "mise": r"(?:mise )?([0-9]{4}\.[0-9]{1,2}\.[0-9]{1,2})",
        "buildkit": r"buildctl github\.com/moby/buildkit v([0-9]+\.[0-9]+\.[0-9]+)(?: [A-Za-z0-9.+_-]+)?",
    }
    for name, argv in (
        ("python", [sys.executable, "--version"]),
        ("mise", ["mise", "--version"]),
        ("buildkit", ["buildctl", "--version"]),
    ):
        result = run(argv)
        match = re.fullmatch(
            patterns[name], (text(result) or text(result, "err") or "").strip()
        )
        identities[name] = (
            item("OBSERVED", match.group(1))
            if result["exit"] == 0 and match
            else item("UNKNOWN", reason="host identity unavailable or unsafe")
        )
    evidence["host_identities"] = identities

    dpkg = run(
        [
            "dpkg-query",
            "-W",
            "-f=${binary:Package}\t${Status}\t${Version}\t${Architecture}\n",
            *PACKAGES,
        ]
    )
    policy = run(["apt-cache", "policy", *PACKAGES])
    found = installed(dpkg)
    present, absent = found if found is not None else ({}, set())
    candidate = candidates(policy)
    evidence["query_exits"].update(installed=query(dpkg), candidate=query(policy))
    packages = {}
    for name in PACKAGES:
        state = (
            item("OBSERVED", present[name])
            if name in present and public_version(name, present[name]["version"])
            else item("ABSENT", reason="explicit dpkg absence")
            if name in absent
            else item("UNKNOWN", reason="installed query incomplete or version unsafe")
        )
        version = candidate.get(name)
        packages[name] = {
            "installed": state,
            "candidate": item("OBSERVED", version)
            if public_version(name, version)
            else item("UNKNOWN", reason="candidate missing or unsafe"),
            "candidate_index_binding": item(
                "UNKNOWN", reason="index not authenticated to candidate"
            ),
        }
        if state["state"] == "OBSERVED" and state["value"]["architecture"] not in (
            arch,
            "all",
        ):
            blockers.append(f"{name} installed architecture differs from host")
    evidence["packages"] = packages

    requested = []
    for name in REQUEST:
        state = packages[name]["installed"]["state"]
        version = packages[name]["candidate"]["value"]
        if state == "ABSENT" and version and arch in ("amd64", "arm64"):
            if name == "docker-ce-rootless-extras" and (
                packages["docker-ce"]["installed"]["state"] != "OBSERVED"
                or present["docker-ce"]["version"] != version
            ):
                blockers.append("Docker extras differs from installed Docker CE")
            else:
                requested.append(f"{name}:{arch}={version}")
        elif state != "OBSERVED":
            blockers.append(f"{name} exact request unavailable")
    evidence["requested_transaction"] = requested

    no_run_reason = (
        "all requested packages already installed"
        if all(packages[name]["installed"]["state"] == "OBSERVED" for name in REQUEST)
        else "exact request set unavailable"
    )
    simulation = simulation_receipt(None, no_run_reason=no_run_reason)
    if (
        requested
        and all(
            packages[name]["installed"]["state"] in ("OBSERVED", "ABSENT")
            for name in REQUEST
        )
        and len(requested)
        == sum(packages[name]["installed"]["state"] == "ABSENT" for name in REQUEST)
    ):
        argv = [
            "apt-get",
            "-s",
            "--no-install-recommends",
            "--no-upgrade",
            "install",
            *requested,
        ]
        simulation = simulation_receipt(run(argv, 20), argv)
    evidence["apt_simulation"] = simulation
    if any(x["state"] != "OBSERVED" for x in files.values()):
        blockers.append("fixed source, keyring or Release observation incomplete")
    if any(x["state"] != "OBSERVED" for x in namespace.values()):
        blockers.append("namespace observation incomplete")
    if (
        any(x["state"] != "OBSERVED" for x in tools.values())
        or not profile_name
        or not loaded
    ):
        blockers.append("rootless tool or AppArmor observation incomplete")
    blockers.append("transaction unresolved")
    receipt["collection_status"] = "completed"
    receipt["evidence"], receipt["blockers"] = evidence, sorted(set(blockers))
    encoded = (json.dumps(receipt, sort_keys=True) + "\n").encode()
    if len(encoded) > 262144:
        receipt["evidence"] = item("UNKNOWN", reason="receipt section budget exceeded")
        receipt["collection_status"] = "partial"
        receipt["blockers"].append("receipt section budget exceeded")
        encoded = (json.dumps(receipt, sort_keys=True) + "\n").encode()
    temporary = target.with_suffix(".tmp")
    temporary.write_bytes(encoded)
    temporary.replace(target)


if __name__ == "__main__":
    try:
        main(sys.argv[1])
    except Exception as error:  # noqa: BLE001
        print(
            f"host prerequisite collection failed: {type(error).__name__}",
            file=sys.stderr,
        )
        raise SystemExit(1) from None
