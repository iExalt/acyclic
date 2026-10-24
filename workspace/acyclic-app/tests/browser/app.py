"""Drives the app that `acyclic up` serves: pairing, installing, and the Map, as a phone and as a desktop.

`acyclic up` runs on a fixture with a home of its own; the browser pairs through the link it prints, and every request
that leaves the server is aborted. In Chromium, at both sizes, the page must list every item, be installable (which
needs the manifest and its icons fetched through the session's cookie), and draw the Map as the plan says it should:
the default view, a box expanded and collapsed again without moving on screen, archived subtrees shown, and a
thread's members lit. WebKit draws the default view at the phone's size. Chromium then draws the 3,500-item fixture.
What the Map should show is worked out here from the tracker's own answers, not from the app's code. Screenshots go
to target/browser/.

Usage: app.py <acyclic binary> <acyclic-fixtures binary>
"""

import json
import os
import subprocess
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import sync_playwright

ACYCLIC, FIXTURES = (str(Path(arg).resolve()) for arg in sys.argv[1:3])
SHOTS = Path("target/browser").resolve()
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
# How long the 3,500-item Map may take to draw: a loose bound against hanging, not a timing.
LARGE_LIMIT_MS = 60_000


def run(env, *args):
    out = subprocess.run(
        [ACYCLIC, *args], env=env, capture_output=True, text=True, check=False
    )
    if out.returncode != 0:
        sys.exit(
            f"acyclic {' '.join(args)} failed ({out.returncode}): {out.stdout}{out.stderr}"
        )
    return out.stdout


class Plan:
    """The tracker's items, and what the Map's views of them must show."""

    def __init__(self, env):
        ids = [row["id"] for row in json.loads(run(env, "list"))["data"]["items"]]
        rows = json.loads(run(env, "show", *ids))["data"]
        self.items = {row["id"]: row for row in rows}
        self.parent = {id: row["parent"] for id, row in self.items.items()}

    def chain(self, id):
        """The item and its ancestors, nearest first."""
        out = [id]
        while self.parent.get(out[-1]) in self.items:
            out.append(self.parent[out[-1]])
        return out

    def depth(self, id):
        return len(self.chain(id)) - 1

    def shown(self, id, archived):
        return archived or not any(
            self.items[one]["archived"] for one in self.chain(id)
        )

    def default(self, archived=False):
        """The default view's nodes: every item shown down to depth 1, below which every box is collapsed."""
        return [
            id for id in self.items if self.shown(id, archived) and self.depth(id) <= 1
        ]

    def children(self, id, archived=False):
        return [
            one
            for one in self.items
            if self.parent[one] == id and self.shown(one, archived)
        ]

    def lit(self, thread):
        """In the default view, a thread's members as drawn: themselves, or the depth-1 box hiding them, once."""
        drawn = set()
        for id, row in self.items.items():
            if row["thread"] == thread and self.shown(id, False):
                drawn.add(self.chain(id)[-2] if self.depth(id) > 1 else id)
        return len(drawn)


def tracker(directory, env, items):
    plan = directory / f"plan-{items}.json"
    plan.write_text(
        subprocess.run(
            [FIXTURES, "--items", str(items), "--seed", "7"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "mac")
    run(env, "--as-user", "import", str(plan))
    return Plan(env)


def wait_drawn(page, nodes, timeout=20_000):
    """Waits until the Map has drawn exactly `nodes` nodes, and returns its element's data."""
    page.wait_for_function(
        "n => { const m = document.querySelector('[data-testid=map]');"
        " return m && m.dataset.state === 'drawn' && Number(m.dataset.nodes) === n; }",
        arg=nodes,
        timeout=timeout,
    )
    return page.get_by_test_id("map").evaluate("m => ({ ...m.dataset })")


def tap_title(page, view, id):
    """Taps a box's title strip where it is drawn on the canvas."""
    corner = page.get_by_test_id("map").evaluate(
        "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " const r = m.getBoundingClientRect(); return { x: r.x + b.x1, y: r.y + b.y1, w: b.w }; }",
        id,
    )
    x, y = corner["x"] + min(corner["w"] / 2, 60), corner["y"] + 10
    if view.get("has_touch"):
        page.touchscreen.tap(x, y)
    else:
        page.mouse.click(x, y)


def settled_corner(page, id):
    """Where a box's top-left corner is drawn once it has stopped moving: the same for 300 ms of frames.

    Read from the renderer itself, so it doesn't trust the app's own account of when it has finished drawing."""
    return page.get_by_test_id("map").evaluate(
        """(m, id) => new Promise((done) => {
            let last = null, since = performance.now();
            const read = () => {
                const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });
                const now = `${Math.round(b.x1)},${Math.round(b.y1)}`;
                if (now !== last) { last = now; since = performance.now(); }
                if (performance.now() - since >= 300) done({ x: b.x1, y: b.y1 });
                else requestAnimationFrame(read);
            };
            read();
        })""",
        id,
    )


def open_app(browser, view, link, failures, name):
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
    page.locator("[data-testid=item-count], [role=alert]").first.wait_for(
        timeout=15_000
    )
    return context, page, origin, errors


def installable(context, page, origin, name, failures):
    cdp = context.new_cdp_session(page)
    if errors := cdp.send("Page.getInstallabilityErrors")["installabilityErrors"]:
        failures.append(f"{name}: not installable: {errors}")
    manifest = cdp.send("Page.getAppManifest")
    if manifest.get("errors") or not manifest.get("data"):
        failures.append(f"{name}: the manifest didn't load: {manifest.get('errors')}")
        return
    for icon in json.loads(manifest["data"])["icons"]:
        if (status := page.request.get(origin + icon["src"]).status) != 200:
            failures.append(f"{name}: icon {icon['src']} answered {status}")


def map_journey(page, view, plan, name, failures, hint=None):
    """Drives the Map, and returns the box it found the layout moves, for the next run to try first."""
    expected = len(plan.default())
    wait_drawn(page, expected)
    # Expand a box inside a root, then collapse it again: the count follows, and the box holds still on screen. Only
    # a box the layout moves proves the Map pans to match, so boxes are tried, biggest first, until one has moved.
    corner = "(m, id) => { const b = m.cy.getElementById(id).boundingBox({ includeLabels: false }); return [b.x1, b.y1]; }"
    moved, box = 0, None
    for box in sorted(
        (id for id in plan.default() if plan.depth(id) == 1 and plan.children(id)),
        key=lambda id: (id != hint, -len(plan.children(id)), id),
    ):
        # Bring it to full size in the middle of the screen first, as a person would: on a phone, the fitted plan
        # draws it too small to tap.
        page.get_by_test_id("map").evaluate(
            "(m, id) => { m.cy.zoom(1); m.cy.center(m.cy.getElementById(id)); }", box
        )
        placed = page.get_by_test_id("map").evaluate(corner, box)
        # Where it settles after expanding is where it starts collapsing.
        before = settled_corner(page, box)
        for action, nodes in [
            ("expanding", expected + len(plan.children(box))),
            ("collapsing", expected),
        ]:
            tap_title(page, view, box)
            wait_drawn(page, nodes)
            after = settled_corner(page, box)
            if action == "expanding":
                relaid = page.get_by_test_id("map").evaluate(corner, box) != placed
            shift = max(abs(after["x"] - before["x"]), abs(after["y"] - before["y"]))
            moved = max(moved, shift)
            if shift > 2:
                failures.append(
                    f"{name}: {action} {box} moved it by {shift:.0f} px on screen"
                )
            before = after
        if relaid:
            break
    else:
        failures.append(f"{name}: no box moved in the layout when expanded")
        box = None
    # Archived subtrees show when asked, and hide again.
    page.get_by_test_id("show-archived").click()
    wait_drawn(page, len(plan.default(archived=True)))
    page.get_by_test_id("show-archived").click()
    wait_drawn(page, expected)
    # A thread lights up its members as drawn.
    thread, _ = Counter(
        row["thread"] for row in plan.items.values() if row["thread"]
    ).most_common(1)[0]
    page.locator(f"[data-thread='{thread}']").click()
    page.wait_for_function(
        "n => Number(document.querySelector('[data-testid=map]').dataset.lit) === n",
        arg=plan.lit(thread),
    )
    page.screenshot(path=SHOTS / f"app-{name}-thread.png")
    print(
        f"{name}: drew {expected} nodes, expanded and collapsed {box} held within {moved:.1f} px"
    )
    return box


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    failures = []
    with tempfile.TemporaryDirectory() as temp, sync_playwright() as playwright:
        directory = Path(temp)
        servers = []

        def serve(items):
            home = directory / f"home-{items}"
            home.mkdir()
            env = {
                **os.environ,
                "HOME": str(home),
                "ACYCLIC_TRACKER": str(directory / f"tracker-{items}"),
                "ACYCLIC_ACTOR": "session/browser",
            }
            plan = tracker(directory, env, items)
            server = subprocess.Popen(
                [ACYCLIC, "up", "--port", "0"],
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )
            servers.append(server)
            link = json.loads(server.stdout.readline())["data"]["link"]
            port = str(urlsplit(link).port)

            def fresh():
                # A link is good once, and each context pairs afresh.
                return json.loads(run(env, "up", "--link", "--port", port))["data"][
                    "link"
                ]

            return plan, link, fresh

        try:
            plan, link, fresh = serve(350)
            chromium = playwright.chromium.launch()
            hint = None
            for name, view in VIEWS.items():
                context, page, origin, errors = open_app(
                    chromium, view, link, failures, name
                )
                link = fresh()
                count = page.get_by_test_id("item-count").inner_text()
                if count != str(len(plan.items)):
                    failures.append(
                        f"{name}: the page lists {count} items, expected {len(plan.items)}"
                    )
                installable(context, page, origin, name, failures)
                # Both sizes lay the plan out alike, so a box one finds the layout moves, the next tries first.
                hint = map_journey(page, view, plan, name, failures, hint)
                page.screenshot(path=SHOTS / f"app-{name}.png")
                if errors:
                    failures.append(
                        f"{name}: the page threw or logged errors: {errors}"
                    )
                context.close()
            webkit = playwright.webkit.launch()
            context, page, _, errors = open_app(
                webkit, VIEWS["phone"], link, failures, "webkit phone"
            )
            wait_drawn(page, len(plan.default()))
            page.screenshot(path=SHOTS / "app-webkit-phone.png")
            if errors:
                failures.append(
                    f"webkit phone: the page threw or logged errors: {errors}"
                )
            context.close()
            webkit.close()

            large, link, _ = serve(3500)
            context, page, _, errors = open_app(
                chromium, VIEWS["desktop"], link, failures, "3,500 items"
            )
            start = time.monotonic()
            wait_drawn(page, len(large.default()), timeout=LARGE_LIMIT_MS)
            print(
                f"3,500 items: drew the default view in {(time.monotonic() - start) * 1000:.0f} ms after the list"
            )
            page.screenshot(path=SHOTS / "app-large.png")
            if errors:
                failures.append(
                    f"3,500 items: the page threw or logged errors: {errors}"
                )
            context.close()
            chromium.close()
        finally:
            for server in servers:
                server.terminate()
                server.wait(timeout=10)
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    main()
