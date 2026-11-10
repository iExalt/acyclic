#!/usr/bin/env python3
"""Upgrade state written by supported releases using the actual candidate image.

Needs Docker and Git. All containers have no network and share only a disposable
fixture directory; no developer HOME, credentials or tracker are mounted.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
import uuid
from contextlib import ExitStack
from pathlib import Path

ENGINE = os.environ.get("CONTAINER_ENGINE", "docker")

# Keep the floor when adding a new baseline: a deployment may skip releases.
BASELINES = (
    "ghcr.io/iexalt/acyclic-server:v0.9.0@sha256:4f9c5b714a259ef64aa5d668b7e7a143a8f5b4a43e55d5c64bb96a63460af502",
    "ghcr.io/iexalt/acyclic-server:v0.10.0@sha256:ebebed87cde417978e14958235ab79daaf2d6c0bc488aa43530d3f9a0759e3a9",
)


def run(*args, check=True):
    if args[0] == "docker":
        args = (ENGINE, *args[1:])
    result = subprocess.run(
        args, check=False, capture_output=True, text=True, timeout=90
    )
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} failed: {result.stdout}{result.stderr}")
    return result


def check(candidate, baseline):
    with (
        tempfile.TemporaryDirectory(
            prefix="acyclic-compat-", dir=os.environ.get("ACYCLIC_COMPAT_TMP")
        ) as temporary,
        ExitStack() as cleanup,
    ):
        root = Path(temporary)
        run("git", "init", "--bare", str(root / "origin"))
        volume = f"acyclic-compat-{uuid.uuid4().hex}"
        keeper = f"{volume}-copy"
        run("docker", "volume", "create", volume)
        cleanup.callback(run, "docker", "volume", "rm", volume)
        run(
            "docker",
            "create",
            "--name",
            keeper,
            "--network",
            "none",
            "--user",
            "0",
            "-v",
            f"{volume}:/state",
            "--entrypoint",
            "sh",
            candidate,
            "-c",
            "chown -R 10001:10001 /state",
        )
        cleanup.callback(run, "docker", "rm", "-f", keeper)
        run("docker", "cp", str(root / "origin"), f"{keeper}:/state/origin")
        run("docker", "start", "-a", keeper)
        common = [
            "--network",
            "none",
            "--user",
            "10001:10001",
            "-v",
            f"{volume}:/state",
            "-e",
            "HOME=/state/home",
        ]

        def cli(image, *args):
            answer = run("docker", "run", "--rm", *common, image, *args)
            value = json.loads(answer.stdout)
            assert value["ok"], value
            return value["data"]

        def command(image, *args):
            return cli(image, "--tracker", "/state/tracker", *args)

        cli(
            baseline,
            "init",
            "/state/tracker",
            "--replica",
            "fixture",
            "--remote",
            "/state/origin",
        )
        command(baseline, "initiative", "add", "--title", "Synthetic")
        first = command(baseline, "item", "add", "--title", "Prerequisite")["item"]
        second = command(
            baseline, "item", "add", "--title", "Finished", "--waits-for", first
        )["item"]
        for item in (first, second):
            command(baseline, "item", "done", item, "--evidence", "fixture")
        command(baseline, "--actor", "session/fixture", "item", "reopen", first)
        command(baseline, "--actor", "session/fixture", "thread", "claim", first)
        command(baseline, "sync")

        def api(name, route, payload=None):
            args = [
                "docker",
                "exec",
                name,
                "wget",
                "-q",
                "-T",
                "2",
                "-O",
                "-",
                "--header",
                "Host: fixture.example",
                "--header",
                "Tailscale-User-Login: user@example.com",
                "--header",
                "Content-Type: application/json",
            ]
            if payload is not None:
                args += ["--post-data", json.dumps(payload)]
            result = run(*args, f"http://127.0.0.1:7477{route}")
            answer = json.loads(result.stdout)
            assert answer["ok"], answer
            return answer["data"]

        def serve(image, test):
            name = f"acyclic-compat-{uuid.uuid4().hex}"
            run(
                "docker",
                "run",
                "-d",
                "--name",
                name,
                *common,
                image,
                "--tracker",
                "/state/tracker",
                "serve",
                "--host",
                "fixture.example",
                "--login",
                "user@example.com",
                "--sync-every",
                "0",
            )
            try:
                deadline = time.monotonic() + 30
                while True:
                    try:
                        api(name, "/api/push/key")
                        break
                    except (RuntimeError, json.JSONDecodeError):
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(0.2)
                test(name)
            finally:
                run("docker", "stop", "--time", "10", name, check=False)
                run("docker", "rm", "-f", name)

        baseline_public = []
        subscription = {
            "endpoint": "https://push.example.invalid/fixture",
            "keys": {
                "p256dh": "BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU",
                "auth": "CQkJCQkJCQkJCQkJCQkJCQ",
            },
        }

        def populate(name):
            baseline_public.append(api(name, "/api/push/key")["key"])
            # A valid synthetic P-256 public key. Network isolation prevents delivery.
            api(name, "/api/push/subscribe", subscription)
            needs = api(name, "/api/command", {"needs": None})["needs"]
            observed = [entry["identity"] for entry in needs if entry.get("notify")]
            assert observed, needs
            api(
                name,
                "/api/command",
                {
                    "command": {"item": {"add": {"title": "Baseline write"}}},
                    "expect": [],
                    "observed": observed,
                },
            )

        def baseline_state(name):
            populate(name)
            deadline = time.monotonic() + 60
            while True:
                sent = run(
                    "docker",
                    "exec",
                    name,
                    "cat",
                    "/state/home/.config/acyclic/push/sent.json",
                    check=False,
                )
                if sent.returncode == 0 and json.loads(sent.stdout):
                    return
                if time.monotonic() >= deadline:
                    raise AssertionError("baseline did not persist acknowledged needs")
                time.sleep(0.2)

        serve(baseline, baseline_state)
        push = root / "baseline-push"
        run("docker", "cp", f"{keeper}:/state/home/.config/acyclic/push", str(push))
        saved = {
            file.name: file.read_bytes() for file in push.iterdir() if file.is_file()
        }
        assert {"vapid.key", "subscriptions.json", "sent.json"} <= saved.keys(), (
            saved.keys()
        )
        assert json.loads(saved["subscriptions.json"]) and json.loads(
            saved["sent.json"]
        )
        before = command(baseline, "item", "list")["items"]
        command(candidate, "doctor")
        assert command(candidate, "item", "list")["items"] == before

        def upgraded(name):
            public = api(name, "/api/push/key")["key"]
            assert public == baseline_public[0], "VAPID identity changed"
            api(
                name,
                "/api/push/subscribe",
                {**subscription, "endpoint": "https://push.example.invalid/candidate"},
            )
            api(name, "/api/command", {"item": {"add": {"title": "Candidate write"}}})

        serve(candidate, upgraded)
        command(candidate, "sync")
        cli(
            candidate,
            "init",
            "/state/peer",
            "--replica",
            "peer",
            "--remote",
            "/state/origin",
        )
        cli(candidate, "--tracker", "/state/peer", "sync")
        peer = cli(candidate, "--tracker", "/state/peer", "item", "list")["items"]
        assert any(item["title"] == "Candidate write" for item in peer), peer
        cli(
            candidate,
            "--tracker",
            "/state/peer",
            "item",
            "add",
            "--title",
            "Peer write",
        )
        cli(candidate, "--tracker", "/state/peer", "sync")
        command(candidate, "sync")
        assert any(
            item["title"] == "Peer write"
            for item in command(candidate, "item", "list")["items"]
        )

        def restarted(name):
            assert api(name, "/api/push/key")["key"] == baseline_public[0]
            assert any(
                item["title"] == "Peer write"
                for item in api(name, "/api/command", {"item": "list"})["items"]
            )

        serve(candidate, restarted)
        after = root / "candidate-push"
        run("docker", "cp", f"{keeper}:/state/home/.config/acyclic/push", str(after))
        subscriptions = json.loads((after / "subscriptions.json").read_bytes())
        assert len(subscriptions) == 2, subscriptions
        assert json.loads(saved["subscriptions.json"])[0] in subscriptions
        for name, contents in saved.items():
            if name == "subscriptions.json":
                continue
            assert (after / name).read_bytes() == contents, (
                f"push state changed: {name}"
            )
        print(
            f"compatibility passed: {baseline.split('@')[0]} -> {candidate}", flush=True
        )


if __name__ == "__main__":
    for release in BASELINES:
        run("docker", "pull", release)
        check(sys.argv[1], release)
