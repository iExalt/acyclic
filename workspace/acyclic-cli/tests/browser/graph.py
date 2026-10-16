"""Renders acyclic's graph pages in headless Chromium and WebKit with the network blocked.

For each renderer and browser, a small tracker with a hostile title must draw every item and ghost, show the title as
text and run none of it; screenshots go to target/browser/. Then the 3,500-item fixture is drawn and timed, with a
60-second limit per page: Graphviz must finish in time, and Mermaid's time, or its timeout, is reported.

Usage: graph.py <acyclic binary> <acyclic-fixtures binary>
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

from playwright.sync_api import sync_playwright

ACYCLIC, FIXTURES = (str(Path(arg).resolve()) for arg in sys.argv[1:3])
SHOTS = Path("target/browser").resolve()
HOSTILE = '</script><script>window.pwned = 1</script> <img src=x onerror="window.pwned = 1"> <b>&amp;</b> "quoted" # end'
RENDERERS = ["graphviz", "mermaid"]
LIMIT_MS = 60_000


def acyclic(tracker, *args, user=False):
    env = {
        **os.environ,
        "ACYCLIC_TRACKER": str(tracker),
        "ACYCLIC_ACTOR": "session/browser",
    }
    command = [ACYCLIC, *(["--as-user"] if user else []), *args]
    out = subprocess.run(command, env=env, capture_output=True, text=True, check=False)
    if out.returncode != 0:
        sys.exit(
            f"acyclic {' '.join(args)} failed ({out.returncode}): {out.stdout}{out.stderr}"
        )
    return json.loads(out.stdout) if out.stdout.startswith("{") else out.stdout


def page_file(tracker, directory, renderer, *scope):
    path = directory / f"{renderer}.html"
    summary = acyclic(
        tracker,
        "graph",
        "--format",
        "html",
        "--renderer",
        renderer,
        "--output",
        str(path),
        *scope,
    )
    return path, summary["data"]["items"] + summary["data"]["ghosts"]


def small_tracker(directory):
    tracker = directory / "small"
    acyclic(tracker, "init", str(tracker), "--replica", "mac")
    for item, title, parent in [
        ("home", "Home projects", None),
        ("paint", HOSTILE, "home"),
        ("buy", "Buy paint", "home"),
        ("tiles", "Pick tiles", None),
    ]:
        acyclic(
            tracker,
            "item",
            "add",
            item,
            "--title",
            title,
            *(["--parent", parent] if parent else []),
        )
    acyclic(tracker, "link", "add", "paint", "waits-for", "buy")
    acyclic(tracker, "link", "add", "paint", "relates", "tiles")
    acyclic(tracker, "thread", "add", "kitchen", "--title", "Kitchen </script>")
    acyclic(tracker, "item", "edit", "paint", "--thread", "kitchen")
    return tracker


def large_tracker(directory):
    tracker = directory / "large"
    plan = directory / "plan.json"
    plan.write_text(
        subprocess.run(
            [FIXTURES, "--items", "3500", "--seed", "7"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    acyclic(tracker, "init", str(tracker), "--replica", "mac")
    acyclic(tracker, "import", str(plan), user=True)
    return tracker


def render(browser, path):
    """Loads a page with every request but the file itself aborted; returns its state, time and drawn nodes."""
    context = browser.new_context(viewport={"width": 1280, "height": 900})
    page = context.new_page()
    url = path.as_uri()
    page.route(
        "**/*",
        lambda route: route.continue_() if route.request.url == url else route.abort(),
    )
    try:
        page.goto(url, wait_until="commit")
        page.wait_for_selector(
            "body[data-state=done], body[data-state=error]", timeout=LIMIT_MS
        )
    except Exception:  # noqa: BLE001 - Playwright's timeout type differs by version
        context.close()
        return {"state": "timeout"}
    result = page.evaluate("""() => ({
        state: document.body.dataset.state,
        ms: Number(document.body.dataset.ms),
        renderMs: Number(document.body.dataset.renderMs),
        error: document.getElementById('error').textContent,
        nodes: document.querySelectorAll('#graph svg g.node').length,
        text: document.querySelector('#graph svg')?.textContent ?? '',
        pwned: window.pwned !== undefined,
    })""")
    result["page"] = page
    result["context"] = context
    return result


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    failures = []
    with tempfile.TemporaryDirectory() as temp, sync_playwright() as playwright:
        directory = Path(temp)
        small = small_tracker(directory)
        browsers = {
            name: getattr(playwright, name).launch() for name in ["chromium", "webkit"]
        }
        for renderer in RENDERERS:
            path, expected = page_file(small, directory, renderer)
            for name, browser in browsers.items():
                label = f"{name} {renderer}"
                result = render(browser, path)
                if result["state"] != "done":
                    failures.append(
                        f"{label}: {result['state']} {result.get('error', '')}"
                    )
                    print(failures[-1])
                    continue
                result["page"].screenshot(path=SHOTS / f"{name}-{renderer}-small.png")
                result["context"].close()
                if result["pwned"]:
                    failures.append(f"{label}: a title's script ran")
                if result["nodes"] != expected:
                    failures.append(
                        f"{label}: drew {result['nodes']} nodes, expected {expected}"
                    )
                if " ".join(HOSTILE.split()) not in " ".join(result["text"].split()):
                    failures.append(
                        f"{label}: the hostile title isn't shown as its text"
                    )
                print(
                    f"{label}: {result['nodes']} nodes in {result['ms']} ms, {result['renderMs']} ms drawing"
                )
        large = large_tracker(directory)
        for renderer in RENDERERS:
            path, expected = page_file(large, directory, renderer)
            browser = browsers["chromium"]
            result = render(browser, path)
            label = f"chromium {renderer} 3,500 items"
            if result["state"] == "done":
                result["page"].screenshot(path=SHOTS / f"chromium-{renderer}-large.png")
                result["context"].close()
                print(
                    f"{label}: {result['nodes']} of {expected} nodes in {result['ms']} ms, {result['renderMs']} ms drawing"
                )
                if result["nodes"] != expected:
                    failures.append(
                        f"{label}: drew {result['nodes']} nodes, expected {expected}"
                    )
            elif renderer == "graphviz":
                failures.append(f"{label}: {result['state']} {result.get('error', '')}")
            else:
                print(
                    f"{label}: {result['state']} {result.get('error', '')[:200]} (reported, not a failure)"
                )
        for browser in browsers.values():
            browser.close()
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    main()
