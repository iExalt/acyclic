"""Drives the app that `acyclic up` serves, in headless Chromium, as a phone and as a desktop.

`acyclic up` runs on a 350-item fixture with a home of its own; the browser pairs through the link it prints, every
request that leaves the server is aborted, and the page must list every item. Chromium must also find the app
installable, which needs the manifest and its icons fetched through the session's cookie. Screenshots go to
target/browser/.

Usage: app.py <acyclic binary> <acyclic-fixtures binary>
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import sync_playwright

ACYCLIC, FIXTURES = (str(Path(arg).resolve()) for arg in sys.argv[1:3])
SHOTS = Path("target/browser").resolve()
ITEMS = 350
# A large Android phone at its default resolution, the kind the app is checked on by hand, and a laptop's window.
VIEWS = {
    "phone": {
        "viewport": {"width": 384, "height": 832},
        "device_scale_factor": 2.8125,
        "is_mobile": True,
        "has_touch": True,
    },
    "desktop": {"viewport": {"width": 1440, "height": 900}},
}


def run(env, *args):
    out = subprocess.run(
        [ACYCLIC, *args], env=env, capture_output=True, text=True, check=False
    )
    if out.returncode != 0:
        sys.exit(
            f"acyclic {' '.join(args)} failed ({out.returncode}): {out.stdout}{out.stderr}"
        )
    return out.stdout


def tracker(directory, env):
    """A clone holding the fixture; returns how many items it has."""
    plan = directory / "plan.json"
    plan.write_text(
        subprocess.run(
            [FIXTURES, "--items", str(ITEMS), "--seed", "7"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "mac")
    run(env, "--as-user", "import", str(plan))
    return len(json.loads(run(env, "list"))["data"]["items"])


def journey(browser, name, view, link, expected, failures):
    origin = "{0.scheme}://{0.netloc}".format(urlsplit(link))
    context = browser.new_context(**view)
    page = context.new_page()
    page.route(
        "**/*",
        lambda route: (
            route.continue_() if route.request.url.startswith(origin) else route.abort()
        ),
    )
    errors = []
    page.on("pageerror", lambda err: errors.append(str(err)))
    # A blocked load, such as one the content policy refuses, shows only as a console error.
    page.on(
        "console",
        lambda message: (
            errors.append(message.text) if message.type == "error" else None
        ),
    )
    page.goto(link)
    if page.url.rstrip("/") != origin:
        failures.append(f"{name}: pairing landed on {page.url}, not the app")
    count = page.get_by_test_id("item-count")
    page.locator("[data-testid=item-count], [role=alert]").first.wait_for(
        timeout=15_000
    )
    if not count.is_visible():
        failures.append(f"{name}: the page shows {page.locator('body').inner_text()!r}")
        context.close()
        return
    if int(count.inner_text()) != expected:
        failures.append(
            f"{name}: the page lists {count.inner_text()} items, expected {expected}"
        )
    cdp = context.new_cdp_session(page)
    installability = cdp.send("Page.getInstallabilityErrors")["installabilityErrors"]
    if installability:
        failures.append(f"{name}: not installable: {installability}")
    manifest = cdp.send("Page.getAppManifest")
    if manifest.get("errors") or not manifest.get("data"):
        failures.append(f"{name}: the manifest didn't load: {manifest.get('errors')}")
    else:
        for icon in json.loads(manifest["data"])["icons"]:
            response = page.request.get(origin + icon["src"])
            if response.status != 200:
                failures.append(
                    f"{name}: icon {icon['src']} answered {response.status}"
                )
    if errors:
        failures.append(f"{name}: the page threw or logged errors: {errors}")
    page.screenshot(path=SHOTS / f"app-{name}.png")
    print(f"{name}: {count.inner_text()} items, installable: {not installability}")
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    failures = []
    with tempfile.TemporaryDirectory() as temp, sync_playwright() as playwright:
        directory = Path(temp)
        home = directory / "home"
        home.mkdir()
        env = {
            **os.environ,
            "HOME": str(home),
            "ACYCLIC_TRACKER": str(directory / "tracker"),
            "ACYCLIC_ACTOR": "session/browser",
        }
        expected = tracker(directory, env)
        server = subprocess.Popen(
            [ACYCLIC, "up", "--port", "0"], env=env, stdout=subprocess.PIPE, text=True
        )
        try:
            started = json.loads(server.stdout.readline())
            link = started["data"]["link"]
            browser = playwright.chromium.launch()
            for name, view in VIEWS.items():
                # Each context pairs afresh, and a link is good once.
                journey(browser, name, view, link, expected, failures)
                port = str(urlsplit(link).port)
                link = json.loads(run(env, "up", "--link", "--port", port))["data"][
                    "link"
                ]
            browser.close()
        finally:
            server.terminate()
            server.wait(timeout=10)
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    main()
