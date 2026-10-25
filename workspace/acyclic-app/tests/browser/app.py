"""Drives the app that `acyclic up` serves: pairing, installing, and the Map, as a phone and as a desktop.

`acyclic up` runs on a fixture with a home of its own; the browser pairs through the link it prints, and every request
that leaves the server is aborted. In Chromium, at both sizes, the page must list every item, be installable (which
needs the manifest and its icons fetched through the session's cookie), and draw the Map as the plan says it should:
the default view, a box expanded and collapsed again without moving on screen, archived subtrees shown, and a
thread's members lit. On the phone, an item is added, moved, linked and unlinked on the Map, an agent's note is shown
as text, the item's fields are edited, a conflict a second clone wrote is settled, and the item is reopened; a
thread is approved, an agent's proposals to it are kept and dropped, and its level, its flag and the global level
are set; from the needs-you list, a proposal is kept and a waiting thread approved. Chromium's full build, headless, keeps a
subscription it is given, shows a pushed notification, and opens the needs-you list it names. WebKit draws the default view at the phone's size. Chromium then draws the 3,500-item fixture.
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

from playwright.sync_api import TimeoutError as PlaywrightTimeout
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
# The furthest the Map's first view zooms out, as reveal.ts has it.
READABLE = 0.85
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


def tracker(directory, env, items, remote=None):
    """A tracker holding the seeded fixture, synced to `remote` if given, so a second clone can write apart from it."""
    plan = directory / f"plan-{items}.json"
    plan.write_text(
        subprocess.run(
            [FIXTURES, "--items", str(items), "--seed", "7"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    remote_args = [] if remote is None else ["--remote", str(remote)]
    run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "mac", *remote_args)
    run(env, "--as-user", "import", str(plan))
    if remote is not None:
        run(env, "sync")
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


def from_menu(page, act):
    """Does `act` in the bar's menu, opening it first, as a hand would. Acting closes the menu, so the next tap reaches
    what it lands on rather than only closing the menu."""
    menu = page.locator("details.menu")
    if not menu.evaluate("d => d.open"):
        page.get_by_test_id("menu").click()
    act()
    if menu.evaluate("d => d.open"):
        raise AssertionError("acting in the menu left it open, to swallow the next tap")


def refresh(page):
    from_menu(page, lambda: page.get_by_role("button", name="Refresh").click())


# The Map's helpers reach the renderer through `evaluate`, which sends back whatever its function returns: return plain
# values only, since the renderer's own objects take seconds to serialise.


def tap_box(page, view, id, chevron=True):
    """Taps a box where it is drawn on the canvas: its chevron, which opens or closes it, or else its title, which
    selects it."""
    corner = page.get_by_test_id("map").evaluate(
        "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " const r = m.getBoundingClientRect(); return { x: r.x + b.x1, y: r.y + b.y1, w: b.w, z: m.cy.zoom() }; }",
        id,
    )
    # The chevron is in the middle of the 36 px square at the title strip's left; the title starts after it.
    along = 18 * corner["z"] if chevron else min(corner["w"] / 2, 120 * corner["z"])
    x, y = corner["x"] + along, corner["y"] + 18 * corner["z"]
    if view.get("has_touch"):
        page.touchscreen.tap(x, y)
    else:
        page.mouse.click(x, y)


def centre(page, id, rise=0):
    """Brings a node to full size in the middle of the screen, as a person would before tapping it: on a phone, the
    fitted plan draws it too small to tap. `rise` lifts it by that share of the Map's height."""
    page.get_by_test_id("map").evaluate(
        "(m, [id, rise]) => { m.cy.zoom(1); m.cy.center(m.cy.getElementById(id)); m.cy.panBy({ x: 0, y: -rise * m.clientHeight }); }",
        [id, rise],
    )


def tap_middle(page, view, id, rise=0):
    """Taps the middle of a node where it is drawn, after bringing it to full size, `rise` above the middle."""
    centre(page, id, rise)
    middle = page.get_by_test_id("map").evaluate(
        "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " const r = m.getBoundingClientRect(); return { x: r.x + (b.x1 + b.x2) / 2, y: r.y + (b.y1 + b.y2) / 2 }; }",
        id,
    )
    if view.get("has_touch"):
        page.touchscreen.tap(middle["x"], middle["y"])
    else:
        page.mouse.click(middle["x"], middle["y"])


def drag_pans(page, view, id):
    """Drags from the middle of a node, by finger on a phone and by mouse on a laptop: whether the Map panned with it,
    and selected nothing."""
    centre(page, id)
    drawn = page.get_by_test_id("map")
    middle = drawn.evaluate(
        "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " const r = m.getBoundingClientRect(); return { x: r.x + (b.x1 + b.x2) / 2, y: r.y + (b.y1 + b.y2) / 2 }; }",
        id,
    )
    before = drawn.evaluate("m => m.cy.pan()")
    x, y = middle["x"], middle["y"]
    steps = [(x + 15 * n, y + 10 * n) for n in range(1, 7)]
    if view.get("has_touch"):
        cdp = page.context.new_cdp_session(page)
        touch = lambda kind, points: cdp.send(
            "Input.dispatchTouchEvent",
            {"type": kind, "touchPoints": [{"x": a, "y": b} for a, b in points]},
        )
        touch("touchStart", [(x, y)])
        for step in steps:
            touch("touchMove", [step])
        touch("touchEnd", [])
        cdp.detach()
    else:
        page.mouse.move(x, y)
        page.mouse.down()
        for a, b in steps:
            page.mouse.move(a, b)
        page.mouse.up()
    after = drawn.evaluate("m => m.cy.pan()")
    moved = (after["x"] - before["x"], after["y"] - before["y"])
    return abs(moved[0] - 90) < 2 and abs(
        moved[1] - 60
    ) < 2 and not page.get_by_test_id("inspector").count(), moved


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
    # A blocked load, such as one the content policy refuses, shows only as a console error. WebKit on Linux says it
    # ignores the viewport's keyboard setting, which only Chromium and Firefox read.
    page.on(
        "console",
        lambda message: (
            errors.append(message.text)
            if message.type == "error" and '"interactive-widget"' not in message.text
            else None
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


def first_view_readable(page, name, failures):
    """The Map opens at a zoom its titles can be read at, with an item, not a box, wholly in view, title and all."""
    view = page.get_by_test_id("map").evaluate(
        "m => { const w = m.clientWidth, h = m.clientHeight; return { zoom: m.cy.zoom(),"
        " shown: m.cy.nodes().not('.box').some(n => { const b = n.renderedBoundingBox();"
        " return b.x1 >= 0 && b.y1 >= 0 && b.x2 <= w && b.y2 <= h; }) }; }"
    )
    if view["zoom"] < READABLE or not view["shown"]:
        failures.append(
            f"{name}: the Map opened unreadably small or showing nothing whole: {view}"
        )


def map_journey(page, view, plan, name, failures, hint=None):
    """Drives the Map, and returns the box it found the layout moves, for the next run to try first."""
    expected = len(plan.default())
    # The canvas draws labels in whatever face is loaded, so the first draw waits for the app's own.
    if wait_drawn(page, expected).get("font") != "true":
        failures.append(f"{name}: the Map drew before its font loaded")
    first_view_readable(page, name, failures)
    # Dragging pans from anywhere, an item or a box as well as the background.
    for id in [
        next(id for id in plan.default() if plan.depth(id) == 1),
        next(id for id in plan.default() if plan.depth(id) == 0 and plan.children(id)),
    ]:
        panned, moved = drag_pans(page, view, id)
        if not panned:
            failures.append(
                f"{name}: a drag from {id} moved the Map {moved}, not with it, or selected it"
            )
    if view.get("has_touch"):
        # Two fingers from an item pinch, zooming the Map, rather than panning it with the first.
        drawn = page.get_by_test_id("map")
        centre(page, id)
        box = drawn.evaluate(
            "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
            " const r = m.getBoundingClientRect(); return { x: r.x + (b.x1 + b.x2) / 2, y: r.y + (b.y1 + b.y2) / 2 }; }",
            id,
        )
        zoom = drawn.evaluate("m => m.cy.zoom()")
        cdp = page.context.new_cdp_session(page)
        x, y = box["x"], box["y"]
        for kind, spread in [("touchStart", 10)] + [
            ("touchMove", 10 + 12 * n) for n in range(1, 7)
        ]:
            cdp.send(
                "Input.dispatchTouchEvent",
                {
                    "type": kind,
                    "touchPoints": [
                        {"x": x - spread, "y": y},
                        {"x": x + spread, "y": y},
                    ],
                },
            )
        cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
        cdp.detach()
        after = drawn.evaluate(
            "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
            " const r = m.getBoundingClientRect(); return { x: r.x + (b.x1 + b.x2) / 2, y: r.y + (b.y1 + b.y2) / 2 }; }",
            id,
        )
        # Spread about the item's middle, it zooms about it, so the middle stays put.
        drift = max(abs(after["x"] - x), abs(after["y"] - y))
        if drawn.evaluate("m => m.cy.zoom()") <= zoom or drift > 8:
            failures.append(
                f"{name}: a pinch from {id} didn't zoom the Map about it (drifted {drift:.0f} px)"
            )
    # Expand a box inside a root, then collapse it again: the count follows, and the box holds still on screen. Only
    # a box the layout moves proves the Map pans to match, so boxes are tried, biggest first, until one has moved.
    corner = "(m, id) => { const b = m.cy.getElementById(id).boundingBox({ includeLabels: false }); return [b.x1, b.y1]; }"
    moved, box = 0, None
    for box in sorted(
        (id for id in plan.default() if plan.depth(id) == 1 and plan.children(id)),
        key=lambda id: (id != hint, -len(plan.children(id)), id),
    ):
        centre(page, box)
        placed = page.get_by_test_id("map").evaluate(corner, box)
        # Where it settles after expanding is where it starts collapsing.
        before = settled_corner(page, box)
        for action, nodes in [
            ("expanding", expected + len(plan.children(box))),
            ("collapsing", expected),
        ]:
            tap_box(page, view, box)
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
    from_menu(page, lambda: page.get_by_test_id("show-archived").click())
    wait_drawn(page, len(plan.default(archived=True)))
    archived = next(
        (id for id in plan.default(archived=True) if plan.items[id]["archived"]), None
    )
    if archived is not None:
        # For the eye: crossed out. The renderer loads its images after drawing, and draws them a moment later.
        centre(page, archived)
        page.wait_for_timeout(200)
        page.screenshot(path=SHOTS / f"app-{name}-archived.png")
    from_menu(page, lambda: page.get_by_test_id("show-archived").click())
    wait_drawn(page, expected)
    # A thread lights up its members as drawn.
    thread, _ = Counter(
        row["thread"] for row in plan.items.values() if row["thread"]
    ).most_common(1)[0]
    # Picked with none of its items in view, the nearest comes into view, clear of the panel the pick opens.
    # In braces, so the renderer itself isn't sent back: serialising it takes seconds.
    page.get_by_test_id("map").evaluate(
        "m => { m.cy.pan({ x: 2 * m.clientWidth, y: 2 * m.clientHeight }); }"
    )
    page.locator(f"[data-thread='{thread}']").click()
    page.wait_for_function(
        "n => Number(document.querySelector('[data-testid=map]').dataset.lit) === n",
        arg=plan.lit(thread),
    )
    page.locator("[data-testid=thread-panel]:not([aria-busy])").wait_for()
    if not page.get_by_test_id("map").evaluate(
        """m => {
            const r = m.getBoundingClientRect();
            const sheet = m.parentElement.querySelector('.inspector')?.getBoundingClientRect();
            const across = sheet && sheet.width >= r.width * 0.9;
            const open = { x2: sheet && !across ? sheet.left - r.left : r.width, y2: sheet && across ? sheet.top - r.top : r.height };
            return m.cy.nodes('.lit').some(n => {
                const b = n.renderedBoundingBox({ includeLabels: false });
                return b.x2 > 0 && b.y2 > 0 && b.x1 < open.x2 && b.y1 < open.y2;
            });
        }"""
    ):
        failures.append(f"{name}: picking {thread} left all of its items out of view")
    # Its links are its route, drawn in its line's colour.
    route = page.get_by_test_id("map").evaluate(
        "m => { const lit = m.cy.edges('.lit'); const badge = document.querySelector(`[data-thread='${"
        + repr(thread)
        + "}'] .badge`);"
        " return { n: lit.length, said: Number(m.dataset.litEdges),"
        " colours: [...new Set(lit.map(e => e.style('line-color')))], line: badge && getComputedStyle(badge).backgroundColor }; }"
    )
    same = [colour.replace(" ", "") for colour in route["colours"]] == [
        (route["line"] or "").replace(" ", "")
    ]
    if route["n"] == 0 or route["n"] != route["said"] or not same:
        failures.append(
            f"{name}: {thread}'s links aren't drawn in its line's colour: {route}"
        )
    panel = page.locator("[data-testid=thread-panel]:not([aria-busy])")
    panel.wait_for()
    if panel.locator("[role=alert]").count():
        failures.append(f"{name}: the thread's panel failed: {panel.inner_text()}")
    page.screenshot(path=SHOTS / f"app-{name}-thread.png")
    # Escape closes the panel the pick opened, and focus goes back to the thread picked; picked again, it reopens.
    button = page.locator(f"[data-thread='{thread}']")
    page.keyboard.press("Escape")
    if panel.count() or not button.evaluate("b => b === document.activeElement"):
        failures.append(
            f"{name}: Escape left the thread's panel open, or focus away from {thread}"
        )
    button.click()
    panel.wait_for()
    if view.get("has_touch"):
        # A phone's Back closes what's open one layer at a time, and stays in the app: the needs-you list over the
        # thread's panel, then the panel.
        needs = page.get_by_test_id("needs")
        page.get_by_test_id("needs-button").click()
        needs.wait_for()
        url = page.url
        page.go_back()
        needs.wait_for(state="detached")
        try:
            panel.wait_for(timeout=5_000)
            stayed = True
        except PlaywrightTimeout:
            stayed = False
        page.go_back()
        try:
            panel.wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            stayed = False
        if not stayed or page.url != url:
            failures.append(
                f"{name}: Back didn't close the needs-you list, then the thread's panel, in the app"
            )
        if not panel.count():
            button.click()
        panel.wait_for()
        # Closed in the app, a layer gives its entry back, so the next Back closes what's open then.
        page.get_by_test_id("needs-button").click()
        needs.get_by_role("button", name="Close").click()
        needs.wait_for(state="detached")
        try:
            page.wait_for_function("() => history.state?.layers === 1", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(
                f"{name}: closing the needs-you list left the history at {page.evaluate('JSON.stringify(history.state)')}"
            )
        # A jump back over both entries, as a long press on Back makes, closes both.
        page.get_by_test_id("needs-button").click()
        needs.wait_for()
        page.evaluate("history.go(-2)")
        try:
            needs.wait_for(state="detached", timeout=5_000)
            panel.wait_for(state="detached", timeout=5_000)
            page.wait_for_function("() => history.state === null", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(
                f"{name}: going back two entries didn't close both the needs-you list and the thread's panel"
            )
        if not panel.count():
            button.click()
        panel.wait_for()
    print(
        f"{name}: drew {expected} nodes, expanded and collapsed {box} held within {moved:.1f} px"
    )
    return box


# A note as an agent might write one, which must show as text and never run.
HOSTILE = '<img src=x onerror="window.pwned = true">'


def item(env, id):
    """The item as the tracker shows it, or None while there is no such item."""
    out = subprocess.run(
        [ACYCLIC, "show", id], env=env, capture_output=True, text=True, check=False
    )
    return json.loads(out.stdout)["data"][0] if out.returncode == 0 else None


def check_tracker(env, id, check, what, failures, name):
    """Checks that the tracker shows an edit the app has already shown: polling the CLI before then would contend with
    the server for the clone's lock."""
    if (row := item(env, id)) is None or not check(row):
        failures.append(f"{name}: the tracker doesn't show {what}")


def wait_inspector(page, selector, state="visible"):
    page.locator(f"[data-testid=inspector] {selector}").first.wait_for(
        state=state, timeout=10_000
    )


def held(page, id, before, what, name, failures):
    after = settled_corner(page, id)
    shift = max(abs(after["x"] - before["x"]), abs(after["y"] - before["y"]))
    if shift > 2:
        failures.append(f"{name}: {what} moved {id} by {shift:.0f} px on screen")


def edit_journey(page, view, env, plan, box, name, failures):
    """Adds, moves, links and unlinks on the Map, as the phone would, checking the Map and then the tracker after each.

    Works in `box`, a box map_journey has seen laid out: it is opened, and two of its leaves are used. Expects the
    first leaf's notes to be HOSTILE."""
    leaves = sorted(id for id in plan.children(box) if not plan.children(id))[:2]
    if len(leaves) < 2:
        failures.append(f"{name}: {box} has fewer than two leaves to edit around")
        return
    first, second = leaves
    centre(page, box)
    tap_box(page, view, box)
    base = int(wait_drawn(page, len(plan.default()) + len(plan.children(box)))["nodes"])
    # A tap on the box's title past its chevron selects the box, and leaves it open.
    tap_box(page, view, box, chevron=False)
    title = plan.items[box]["title"]
    page.get_by_test_id("inspector").get_by_role(
        "heading", name=title, exact=True
    ).wait_for(timeout=10_000)
    if page.get_by_test_id("map").get_attribute("data-nodes") != str(base):
        failures.append(f"{name}: a tap on {box}'s title opened or closed it")
    # Selecting shows the item's notes, as text. The leaf sits above the inspector, as a person would hold it, so the
    # Map need not pan to show what is added inside it.
    tap_middle(page, view, first, rise=0.2)
    notes = page.locator("[data-testid=inspector] [data-testid=notes]")
    notes.get_by_text(HOSTILE, exact=True).wait_for(timeout=10_000)
    if notes.locator("img").count() or page.evaluate("window.pwned === true"):
        failures.append(f"{name}: the inspector ran a note as markup")
    # A fingertip reaches a close button and a chip from 44 px apart, though the chip is drawn smaller.
    reach = page.evaluate(
        """() => {
            const at = (el, dy) => { const r = el.getBoundingClientRect();
                return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2 + dy)?.closest('button') === el; };
            const close = document.querySelector('[data-testid=inspector] .close');
            const chip = document.querySelector('[data-testid=inspector] .chip');
            return [close.getBoundingClientRect().height, at(chip, -21) && at(chip, 21)];
        }"""
    )
    if view.get("has_touch") and (reach[0] < 44 or not reach[1]):
        failures.append(
            f"{name}: too small to tap: close {reach[0]} px, chip reached {reach[1]}"
        )

    # Add inside the first leaf: the Map opens the leaf to show it, the leaf holds still, and the tracker has it.
    added = "journey-added"
    before = settled_corner(page, first)
    page.locator("[data-action=add-child]").click()
    form = page.get_by_test_id("add-form")
    form.locator("input[name=title]").fill("Journey: added!")
    if form.locator("input[name=id]").input_value() != added:
        failures.append(
            f"{name}: the add form didn't make the id {added} from the title"
        )
    # It is drawn and selected at once, before the server answers, which is held back until then.
    waiting = []
    page.route(
        "**/api/command",
        lambda route: (
            waiting.append(route)
            if '"add"' in (route.request.post_data or "")
            else route.fallback()
        ),
    )
    form.locator("button[type=submit]").click()
    wait_inspector(page, f".facts [data-select='{first}']")
    wait_drawn(page, base + 1)
    if len(waiting) != 1 or form.count():
        failures.append(f"{name}: an add wasn't drawn until the server answered")
    page.unroute("**/api/command")
    for route in waiting:
        route.continue_()
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    wait_drawn(page, base + 1)
    held(page, first, before, "adding inside it", name, failures)
    check_tracker(
        env,
        added,
        lambda row: row["parent"] == first,
        f"{added} under {first}",
        failures,
        name,
    )

    # Move it into the second leaf, picked on the Map: the prompt leaves the Map free, and the new parent holds still.
    page.locator("[data-action=move]").click()
    prompt = page.get_by_test_id("prompt").bounding_box()
    if prompt is None or prompt["height"] > 60:
        failures.append(f"{name}: the move prompt covers the Map: {prompt}")
    # The menu over a mode closes alone: on an Escape pressed as it opens, and on a tap on the Map, which picks nothing.
    menu = page.locator("details.menu")
    for close in (
        lambda: page.keyboard.press("Escape"),
        lambda: tap_middle(page, view, second, rise=0.2),
    ):
        page.get_by_test_id("menu").click()
        close()
        if (
            menu.evaluate("d => d.open")
            or not page.get_by_test_id("prompt").is_visible()
            or page.get_by_test_id("notice").count()
        ):
            # The rest of the journey needs the move still waiting, so stop here, naming why.
            failures.append(f"{name}: closing the menu didn't leave the move waiting")
            return
    # Above the inspector that opens after the move, likewise.
    centre(page, second, rise=0.2)
    before = settled_corner(page, second)
    tap_middle(page, view, second, rise=0.2)
    wait_inspector(page, f".facts [data-select='{second}']")
    wait_drawn(page, base + 1)
    held(page, second, before, "moving an item into it", name, failures)
    inside = page.get_by_test_id("map").evaluate(
        "(m, [child, parent]) => { const c = m.cy.getElementById(child).boundingBox();"
        " const p = m.cy.getElementById(parent).boundingBox();"
        " return c.x1 >= p.x1 && c.x2 <= p.x2 && c.y1 >= p.y1 && c.y2 <= p.y2; }",
        [added, second],
    )
    if not inside:
        failures.append(f"{name}: {added} isn't drawn inside {second} after the move")
    check_tracker(
        env,
        added,
        lambda row: row["parent"] == second,
        f"{added} under {second}",
        failures,
        name,
    )

    # It waits for the first leaf: the Map draws the link from the first leaf to it, and the tracker has it that way.
    edge = f"{first}->{added}"
    page.locator("[data-action=waits-for]").click()
    tap_middle(page, view, first)
    wait_inspector(page, f"[data-testid=waits-for] [data-select='{first}']")
    page.wait_for_function(
        "([edge, source]) => { const e = document.querySelector('[data-testid=map]').cy.getElementById(edge);"
        " return e.nonempty() && e.source().id() === source; }",
        arg=[edge, first],
        timeout=10_000,
    )
    check_tracker(
        env,
        added,
        lambda row: row["links"]["waits_for"] == [first],
        f"{added} waiting for {first}",
        failures,
        name,
    )
    # Unlinked from the inspector: gone from the Map and the tracker.
    page.locator("[data-testid=waits-for] .unlink").click()
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    page.wait_for_function(
        "edge => document.querySelector('[data-testid=map]').cy.getElementById(edge).empty()",
        arg=edge,
        timeout=10_000,
    )
    check_tracker(
        env,
        added,
        lambda row: row["links"]["waits_for"] == [],
        f"{added} no longer waiting",
        failures,
        name,
    )
    page.screenshot(path=SHOTS / f"app-{name}-edited.png")
    print(
        f"{name}: added, moved, linked and unlinked {added}, the pinned boxes holding still"
    )


def level_keys_journey(page, name, failures):
    """Arrow keys on the closed autonomy list move a draft, written once on Enter and dropped on Escape, while a pick
    from the open list, opened from the keyboard too, writes at once. Where a browser's arrow keys open the list, as on
    macOS, and its headless list takes no keys, there is nothing to check."""
    writes = []

    def sent(request):
        body = request.post_data or ""
        if request.url.endswith("/api/command") and body.startswith(
            '{"level":{"level"'
        ):
            writes.append(json.loads(body)["level"]["level"])

    page.on("request", sent)
    try:
        levels = page.get_by_test_id("global-level")
        page.get_by_test_id("menu").click()
        was = levels.input_value()
        order = levels.evaluate("s => [...s.options].map(o => o.value)")
        levels.focus()
        page.keyboard.press("ArrowDown")
        # A write is wrong on any platform; no change shows a platform whose arrow keys open the list.
        page.wait_for_timeout(300)
        if writes:
            failures.append(f"{name}: an arrow key on the autonomy list wrote {writes}")
            return
        if levels.input_value() == was:
            page.keyboard.press("Escape")
            print(
                f"{name}: arrow keys open the autonomy list here, so there is no draft to check"
            )
            return
        page.keyboard.press("ArrowDown")
        page.wait_for_timeout(300)
        if writes:
            failures.append(f"{name}: arrow keys on the autonomy list wrote {writes}")
            return
        page.keyboard.press("Enter")
        step = order[min(order.index(was) + 2, len(order) - 1)]
        settled_control(page, "[data-testid=global-level]", "value", step)
        if writes != [step]:
            failures.append(
                f"{name}: Enter after two arrow keys wrote {writes}, not once {step}"
            )
        if page.locator("details.menu").evaluate("d => d.open"):
            failures.append(
                f"{name}: Enter on the autonomy list wrote, but left the menu open"
            )
            page.get_by_test_id("menu").click()
        page.get_by_test_id("menu").click()
        levels.focus()
        page.keyboard.press("ArrowUp")
        page.keyboard.press("Escape")
        page.get_by_test_id("menu").click()
        if writes != [step] or levels.input_value() != step:
            failures.append(
                f"{name}: Escape after an arrow key wrote {writes[1:]} or left {levels.input_value()} showing"
            )
        # Only an act closes the menu: Enter on the list with nothing chosen isn't one.
        levels.focus()
        page.keyboard.press("Enter")
        if not page.locator("details.menu").evaluate("d => d.open"):
            failures.append(
                f"{name}: Enter on the autonomy list, unchanged, closed the menu"
            )
            page.get_by_test_id("menu").click()
        # Space opens the list; the keys in it reach only the list, and Enter there picks.
        levels.focus()
        for key in ["Space", "ArrowUp", "Enter"]:
            page.keyboard.press(key)
            # The list opens in its own time, as it does under a person's fingers.
            page.wait_for_timeout(100)
        picked = order[order.index(step) - 1]
        try:
            settled_control(page, "[data-testid=global-level]", "value", picked)
        except PlaywrightTimeout:
            failures.append(
                f"{name}: a pick from the list opened by Space left {levels.input_value()} showing, not {picked}"
            )
        if writes != [step, picked]:
            failures.append(
                f"{name}: a pick from the list opened by Space wrote {writes[1:]}, not once {picked}"
            )
        if page.locator("details.menu").evaluate("d => d.open"):
            page.get_by_test_id("menu").click()
        print(
            f"{name}: arrow keys on the autonomy list wrote once, on Enter, and a pick at once"
        )
    finally:
        page.remove_listener("request", sent)


def failed_add_journey(page, errors, name, failures):
    """An add the server refuses opens its form again with what was typed, and nothing stays drawn; one whose answer
    is lost after the server saved it stays drawn, with the form closed."""
    nodes = int(page.get_by_test_id("map").get_attribute("data-nodes"))
    refusal = json.dumps(
        {
            "ok": False,
            "error": {
                "code": "invalid",
                "message": "Refused for the journey",
                "details": None,
            },
        }
    )

    def add(title, answer):
        page.route(
            "**/api/command",
            lambda route: (
                answer(route)
                if '"add"' in (route.request.post_data or "")
                else route.fallback()
            ),
        )
        page.locator("[data-action=add-top]").click()
        form = page.get_by_test_id("add-form")
        form.locator("input[name=title]").fill(title)
        form.locator("button[type=submit]").click()
        return form

    # Escape never throws away what was typed: it says how to leave instead. From nothing selected, so the form's
    # opener is the bar's Add, which is still there when the form closes.
    if page.get_by_test_id("inspector").count():
        page.get_by_test_id("inspector").get_by_role("button", name="Close").click()
    page.locator("[data-action=add-top]").click()
    form = page.get_by_test_id("add-form")
    form.locator("input[name=title]").fill("Typed, then Escape")
    page.keyboard.press("Escape")
    if (
        not form.count()
        or not form.get_by_test_id("notice").get_by_text("Unsaved changes").count()
    ):
        failures.append(f"{name}: Escape threw away an add form's typed title")
    # Nor does Back.
    page.go_back()
    if not form.count():
        failures.append(f"{name}: Back threw away an add form's typed title")
    if form.count():
        form.get_by_role("button", name="Cancel").click()
    # Gone with the form: what was said of it, and focus, back to what opened it.
    try:
        page.get_by_test_id("notice").wait_for(state="detached", timeout=5_000)
    except PlaywrightTimeout:
        pass
    if page.get_by_test_id("notice").count() or not page.locator(
        "[data-action=add-top]"
    ).evaluate("b => b === document.activeElement"):
        failures.append(
            f"{name}: after cancelling an add form, its notice stayed, or focus didn't go back to Add"
        )
    form = add(
        "Refused add",
        lambda route: route.fulfill(
            status=200, content_type="application/json", body=refusal
        ),
    )
    page.get_by_test_id("notice").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    if (
        not form.count()
        or form.locator("input[name=title]").input_value() != "Refused add"
        or page.get_by_test_id("map").evaluate(
            "m => m.cy.getElementById('refused-add').nonempty()"
        )
        or page.get_by_test_id("inspector").count()
    ):
        failures.append(
            f"{name}: a refused add didn't give back its form, or stayed drawn"
        )
    form.get_by_role("button", name="Cancel").click()

    def lost(route):
        route.fetch()
        route.abort()

    logged = len(errors)
    form = add("Lost answer", lost)
    page.get_by_test_id("notice").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    page.wait_for_function(
        "n => Number(document.querySelector('[data-testid=map]').dataset.nodes) === n",
        arg=nodes + 1,
    )
    if form.count() or not page.get_by_test_id("map").evaluate(
        "m => m.cy.getElementById('lost-answer').nonempty()"
    ):
        failures.append(
            f"{name}: an add saved without an answer opened its form again, or wasn't drawn"
        )
        if form.count():
            form.get_by_role("button", name="Cancel").click()
    # A refused edit opens the editor again with what was typed, over the item as it was.
    inspector = page.get_by_test_id("inspector")
    inspector.get_by_role("heading", name="Lost answer").wait_for(timeout=10_000)
    page.route(
        "**/api/command",
        lambda route: (
            route.fulfill(status=200, content_type="application/json", body=refusal)
            if '"edit"' in (route.request.post_data or "")
            else route.fallback()
        ),
    )
    page.locator("[data-action=edit]").click()
    editor = page.get_by_test_id("item-form")
    editor.locator("input[name=title]").fill("Refused edit")
    editor.locator("textarea[name=notes]").fill("Notes typed on the phone")
    editor.locator("button[type=submit]").click()
    page.get_by_test_id("notice").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    if (
        not editor.count()
        or editor.locator("input[name=title]").input_value() != "Refused edit"
        or editor.locator("textarea[name=notes]").input_value()
        != "Notes typed on the phone"
        or inspector.get_by_role("heading", name="Lost answer").count() != 1
    ):
        failures.append(
            f"{name}: a refused edit didn't give back what was typed over the item as it was"
        )
    if editor.count():
        editor.get_by_role("button", name="Cancel").click()
    # A saved add whose read back failed stays drawn through a later add the server refuses.
    page.route(
        "**/api/command",
        lambda route: (
            route.fulfill(status=500, body="")
            if route.request.post_data == '"list"'
            else route.fallback()
        ),
    )
    page.locator("[data-action=add-top]").click()
    form = page.get_by_test_id("add-form")
    form.locator("input[name=title]").fill("Saved unread")
    form.locator("button[type=submit]").click()
    page.get_by_test_id("notice").get_by_text("Saved, but the refresh failed").wait_for(
        timeout=10_000
    )
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    form = add(
        "Refused after",
        lambda route: route.fulfill(
            status=200, content_type="application/json", body=refusal
        ),
    )
    form.locator("input[name=title]").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    if (
        not page.get_by_test_id("map").evaluate(
            "m => m.cy.getElementById('saved-unread').nonempty()"
        )
        or form.locator("input[name=title]").input_value() != "Refused after"
    ):
        failures.append(
            f"{name}: a saved add went missing when the next add was refused"
        )
    if form.count():
        form.get_by_role("button", name="Cancel").click()
    # A refresh that fails keeps the plan shown, and offers another.
    nodes = page.get_by_test_id("map").get_attribute("data-nodes")
    page.route(
        "**/api/command",
        lambda route: (
            route.fulfill(status=500, body="")
            if route.request.post_data == '"list"'
            else route.fallback()
        ),
    )
    page.evaluate("document.dispatchEvent(new Event('visibilitychange'))")
    notice = page.get_by_test_id("notice")
    notice.get_by_text("Couldn't refresh").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    if (
        page.get_by_test_id("map").get_attribute("data-nodes") != nodes
        or notice.get_by_role("button", name="Refresh").count() != 1
    ):
        failures.append(
            f"{name}: a failed refresh lost the plan, or offered no Refresh"
        )
    notice.get_by_role("button", name="Refresh").click()
    # The browser logs the answer it lost, and the read refused, as it should.
    errors[logged:] = [
        one
        for one in errors[logged:]
        if "net::ERR_FAILED" not in one and "status of 500" not in one
    ]
    print(
        f"{name}: a refused add or edit gave back what was typed; an add saved without an answer stayed drawn"
    )


def in_open_view(page, id):
    """Whether a node is drawn wholly inside the Map and clear of the sheet or pane over it, once the Map settles."""
    settled_corner(page, id)
    return page.get_by_test_id("map").evaluate(
        """(m, id) => {
            const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });
            const r = m.getBoundingClientRect();
            const s = m.parentElement.querySelector('.inspector, .sheet')?.getBoundingClientRect();
            const box = { x1: r.left + b.x1, y1: r.top + b.y1, x2: r.left + b.x2, y2: r.top + b.y2 };
            const clear = !s || box.x2 <= s.left || box.y2 <= s.top || box.x1 >= s.right || box.y1 >= s.bottom;
            return clear && box.x1 >= r.left && box.y1 >= r.top && box.x2 <= r.right && box.y2 <= r.bottom;
        }""",
        id,
    )


def form_journey(page, view, env, remote, id, name, failures):
    """Edits the selected item's fields, settles a conflict two clones wrote, and reopens it, as the phone would."""
    inspector = page.get_by_test_id("inspector")
    # Escape in an editor with nothing changed closes the editor alone.
    page.locator("[data-action=edit]").click()
    form = page.get_by_test_id("item-form")
    form.locator("input[name=title]").focus()
    page.keyboard.press("Escape")
    if form.count() or not inspector.count():
        failures.append(f"{name}: Escape in an unchanged editor didn't close it alone")
    # Fields: only what changed is sent.
    page.locator("[data-action=edit]").click()
    form.locator("input[name=title]").fill("Edited in the app")
    form.locator("select[name=size]").select_option("m")
    form.locator("textarea[name=notes]").fill("Plain notes")
    if view == VIEWS["phone"]:
        # A keyboard shrinks the page to a sliver over the title being typed: Save and Cancel, at the form's foot,
        # must stay in it.
        size = page.viewport_size
        drawn = page.get_by_test_id("map")
        widest = drawn.evaluate("m => m.cy.minZoom()")
        page.set_viewport_size({"width": size["width"], "height": 360})
        # The Map shrinks with the page, and lets a pinch zoom out as far as fits it now; a plan that fits at full
        # zoom can't show it, since zooming out stops there.
        if widest >= 1:
            failures.append(
                f"{name}: the plan fits the Map at full zoom, so a resize can't show its furthest zoom out following"
            )
        else:
            try:
                page.wait_for_function(
                    "z => document.querySelector('[data-testid=map]').cy.minZoom() !== z",
                    arg=widest,
                    timeout=5_000,
                )
            except PlaywrightTimeout:
                failures.append(
                    f"{name}: the Map's furthest zoom out stayed {widest} when it shrank"
                )
        reachable = form.evaluate(
            "f => { f.querySelector('input[name=title]').focus();"
            " return [...f.querySelectorAll(':scope > .actions button')].every(b => {"
            " const r = b.getBoundingClientRect();"
            " return document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === b; }); }"
        )
        page.set_viewport_size(size)
        if not reachable:
            failures.append(
                f"{name}: with a keyboard up, the edit form's Save and Cancel are out of reach"
            )
    # Meanwhile, an agent sets the gate, and coming back to the app reads it: saving sends only what the user changed.
    run(env, "--as-user", "item", "edit", id, "--gate", "Set elsewhere")
    reads = int(page.locator("main.stage").get_attribute("data-reads"))
    page.evaluate("document.dispatchEvent(new Event('visibilitychange'))")
    page.wait_for_function(
        "n => Number(document.querySelector('main.stage').dataset.reads) > n",
        arg=reads,
    )
    form.locator("button[type=submit]").click()
    inspector.get_by_role("heading", name="Edited in the app").wait_for(timeout=10_000)
    if not in_open_view(page, id):
        failures.append(f"{name}: after saving, {id} is off the Map or under the sheet")
    check_tracker(
        env,
        id,
        lambda row: (
            (row["title"], row["size"], row["notes"], row["gate"])
            == ("Edited in the app", "m", "Plain notes", "Set elsewhere")
        ),
        f"{id}'s new fields, and the gate set while the editor was open",
        failures,
        name,
    )

    # A conflict: a second clone and the server's own write the title apart, and both sync. `up` never syncs, so its
    # clone is synced here first, for the second clone to start from what the app wrote.
    run(env, "sync")
    other = {
        **env,
        "ACYCLIC_TRACKER": str(Path(env["ACYCLIC_TRACKER"]).parent / "other"),
    }
    run(
        other,
        "init",
        other["ACYCLIC_TRACKER"],
        "--replica",
        "other",
        "--remote",
        str(remote),
    )
    run(other, "sync")
    run(other, "--as-user", "item", "edit", id, "--title", "Title from the other clone")
    run(env, "--as-user", "item", "edit", id, "--title", "Title from this clone")
    run(other, "sync")
    run(env, "sync")
    refresh(page)
    conflict = page.locator("[data-testid=conflicts] [data-conflict=title]")
    conflict.wait_for(timeout=10_000)
    conflict.locator("[data-keep='Title from the other clone']").click()
    page.get_by_test_id("conflicts").wait_for(state="detached", timeout=10_000)
    inspector.get_by_role("heading", name="Title from the other clone").wait_for(
        timeout=10_000
    )
    check_tracker(
        env,
        id,
        lambda row: row["title"] == "Title from the other clone",
        f"{id}'s conflict settled",
        failures,
        name,
    )
    needs = json.loads(run(env, "needs"))["data"]
    if id in json.dumps(needs):
        failures.append(
            f"{name}: acyclic needs still lists {id} after the conflict was settled"
        )

    # Reopen: finished elsewhere, reopened here.
    run(env, "--as-user", "done", id, "--force", "--note", "finished for the journey")
    refresh(page)
    page.locator("[data-action=reopen]").click()
    page.locator("[data-action=reopen]").wait_for(state="detached", timeout=10_000)
    check_tracker(
        env, id, lambda row: row["state"] != "done", f"{id} reopened", failures, name
    )
    page.screenshot(path=SHOTS / f"app-{name}-forms.png")
    print(
        f"{name}: edited {id}'s fields, settled a conflict two clones wrote, and reopened it"
    )


def settled_control(page, selector, prop, value):
    """Waits until a control shows `value` and is enabled again, once the edit it sent is saved and read back."""
    page.wait_for_function(
        "([selector, prop, value]) => { const c = document.querySelector(selector);"
        " return c && !c.disabled && c[prop] === value; }",
        arg=[selector, prop, value],
        timeout=10_000,
    )


def thread_journey(page, env, plan, thread, name, failures):
    """Approves the thread from its panel, keeps and drops an agent's proposals, and sets its level, its
    project-management flag and the global level, checking the panel and then the tracker."""
    panel = page.get_by_test_id("thread-panel")
    # Picking a thread already picked unpicks it.
    pick = page.locator(f"[data-thread='{thread}']")
    pick.click()
    if pick.get_attribute("aria-pressed") != "true":
        pick.click()

    def shown():
        return json.loads(run(env, "thread", "show", thread))["data"][0]

    # Approve, naming what it approves; revoke first if the fixture approved it already.
    if (
        panel.locator("[data-action=revoke]")
        .or_(panel.locator("[data-action=approve]"))
        .first.get_attribute("data-action")
        == "revoke"
    ):
        panel.locator("[data-action=revoke]").click()
    approve = panel.locator("[data-action=approve]")
    members = len(shown()["members"])
    if approve.inner_text() != f"Approve {members} items":
        failures.append(f"{name}: the approve button reads {approve.inner_text()!r}")
    approve.click()
    panel.locator("[data-action=revoke]").wait_for(timeout=10_000)
    # An agent proposes two items join the approved thread: one is kept, one dropped.
    outside = sorted(id for id, row in plan.items.items() if row["thread"] != thread)[
        :2
    ]
    for id in outside:
        run(env, "item", "edit", id, "--thread", thread)
    refresh(page)
    rows = panel.locator("[data-proposal]")
    rows.nth(1).wait_for(timeout=10_000)
    first = rows.filter(has_text=plan.items[outside[0]]["title"])
    first.locator("[data-action=keep]").click()
    first.wait_for(state="detached", timeout=10_000)
    rows.locator("[data-action=drop]").first.click()
    panel.get_by_test_id("proposals").wait_for(state="detached", timeout=10_000)
    # Its own level, its project-management flag, and the global level.
    panel.locator("select[name=level]").select_option("everything")
    settled_control(
        page, "[data-testid=thread-panel] select[name=level]", "value", "everything"
    )
    # The box shows the tracker's flag, so it ticks once the edit is saved and read back, not on the tap.
    panel.locator("input[name=pm]").click()
    settled_control(page, "[data-testid=thread-panel] input[name=pm]", "checked", True)

    def level():
        page.get_by_test_id("global-level").select_option("everything")
        settled_control(page, "[data-testid=global-level]", "value", "everything")

    from_menu(page, level)
    after = shown()
    if (after["approved"], after["level"], after["pm"]) != (True, "everything", True):
        failures.append(
            f"{name}: the tracker shows {thread} as {after['approved'], after['level'], after['pm']}"
        )
    threads_of = {
        id: json.loads(run(env, "show", id))["data"][0]["thread"] for id in outside
    }
    if threads_of != {outside[0]: thread, outside[1]: plan.items[outside[1]]["thread"]}:
        failures.append(f"{name}: keep and drop left {threads_of}")
    if json.loads(run(env, "level"))["data"]["global"] != "everything":
        failures.append(f"{name}: the global level didn't change")
    page.screenshot(path=SHOTS / f"app-{name}-thread-panel.png")
    print(
        f"{name}: approved {thread}, kept and dropped proposals, and set its level, its flag and the global level"
    )


def needs_journey(page, env, plan, thread, name, failures):
    """Keeps an agent's proposal and approves a waiting thread from the needs-you list, whose count follows."""
    count = page.get_by_test_id("needs-count")
    page.get_by_test_id("needs-button").click()
    needs = page.get_by_test_id("needs")
    needs.wait_for(timeout=10_000)
    outside = next(
        id
        for id, row in sorted(plan.items.items())
        if row["thread"] not in (thread, None)
    )
    run(env, "item", "edit", outside, "--thread", thread)
    before = int(count.inner_text())
    refresh(page)
    page.wait_for_function(
        "n => Number(document.querySelector('[data-testid=needs-count]').textContent) === n",
        arg=before + 1,
        timeout=10_000,
    )
    proposal = needs.locator("[data-need^='proposal:']").filter(
        has_text=plan.items[outside]["title"]
    )
    proposal.locator("[data-action=keep]").click()
    proposal.wait_for(state="detached", timeout=10_000)
    waiting = needs.locator("[data-need^='awaiting-approval:']").first
    approved = waiting.get_attribute("data-need").split(":", 1)[1]
    # The text names the thread beside its badge and picks it, so no row repeats the thread below.
    if (
        waiting.locator(f":scope > button[data-pick-thread='{approved}']").count() != 1
        or waiting.locator(".rows .line").count() != 0
    ):
        failures.append(
            f"{name}: the entry for {approved} should pick it from its text, with no row repeating it"
        )
    waiting.locator("[data-action=approve]").click()
    needs.locator(f"[data-need='awaiting-approval:{approved}']").wait_for(
        state="detached", timeout=10_000
    )
    if int(count.inner_text()) >= before:
        failures.append(
            f"{name}: the needs count stayed at {count.inner_text()} after two were met"
        )
    if json.loads(run(env, "show", outside))["data"][0]["thread"] != thread:
        failures.append(
            f"{name}: keeping the proposal didn't put {outside} in {thread}"
        )
    if not json.loads(run(env, "thread", "show", approved))["data"][0]["approved"]:
        failures.append(f"{name}: approving {approved} from the list didn't approve it")
    page.screenshot(path=SHOTS / f"app-{name}-needs.png")
    picker = needs.locator("[data-need] > button[data-pick-thread]").first
    if picker.count() == 0:
        failures.append(f"{name}: no entry's text picks its thread")
    else:
        picked = picker.get_attribute("data-pick-thread")
        picker.click()
        panel = page.get_by_test_id("thread-panel")
        panel.wait_for(timeout=10_000)
        if (
            page.locator(
                f".threads [data-thread='{picked}'][aria-pressed=true]"
            ).count()
            != 1
        ):
            failures.append(f"{name}: an entry's text didn't pick {picked}")
        panel.get_by_role("button", name="Close").click()
    print(f"{name}: kept a proposal and approved {approved} from the needs-you list")


def push_journey(context, page, env, origin, name, failures):
    """Subscribes a device as the browser would, delivers a push through the service worker, and opens what it names.

    Headless Chromium reaches no real push service, so the subscription is one the page makes up, with a real P-256 key,
    and the message comes in through DevTools rather than from the server.
    """
    context.grant_permissions(["notifications"], origin=origin)
    # The switch reads the permission as the list opens.
    page.get_by_test_id("needs-button").click()
    notify = page.get_by_test_id("notify")
    notify.wait_for(timeout=10_000)
    if notify.is_checked() or not notify.is_enabled():
        failures.append(
            f"{name}: with permission and no subscription, the switch isn't off and ready"
        )
    endpoint = "https://push.example/journey"
    answer = page.evaluate(
        """async (endpoint) => {
            const pair = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
            const raw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
            const b64 = (bytes) => btoa(String.fromCharCode(...bytes)).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
            const keys = { p256dh: b64(raw), auth: b64(crypto.getRandomValues(new Uint8Array(16))) };
            const response = await fetch("/api/push/subscribe", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ endpoint, keys }),
            });
            return response.json();
        }""",
        endpoint,
    )
    devices = Path(env["HOME"]) / ".config/acyclic/push/subscriptions.json"
    if not answer.get("ok") or endpoint not in devices.read_text():
        failures.append(f"{name}: the server didn't keep the subscription: {answer}")
    cdp = context.new_cdp_session(page)
    registrations = []
    cdp.on(
        "ServiceWorker.workerRegistrationUpdated",
        lambda event: registrations.extend(event["registrations"]),
    )
    cdp.send("ServiceWorker.enable")
    page.wait_for_function("navigator.serviceWorker.ready.then(() => true)")
    deadline = time.monotonic() + 10
    while not registrations and time.monotonic() < deadline:
        page.wait_for_timeout(50)
    message = {
        "title": "Needs you",
        "body": "A proposal for the journey",
        "url": "/?needs",
    }
    # Headless Chromium can lose a notification shown while its notifications are being read: probed, about half the
    # runs that read at once after delivering saw none, 4 of 4 that read once beforehand saw none, and 9 of 9 that
    # waited 0.5 s saw it. So each read waits 0.5 s after a delivery, and a lost one is delivered again; a worker that
    # shows nothing fails all five.
    shown, deliveries = [], 0
    while not shown and deliveries < 5:
        deliveries += 1
        cdp.send(
            "ServiceWorker.deliverPushMessage",
            {
                "origin": origin,
                "registrationId": registrations[0]["registrationId"],
                "data": json.dumps(message),
            },
        )
        page.wait_for_timeout(500)
        shown = context.service_workers[0].evaluate(
            """async () => (await self.registration.getNotifications({ tag: "needs" }))
                .map((one) => ({ title: one.title, body: one.body, url: one.data?.url }))"""
        )
    if shown != [message]:
        failures.append(f"{name}: the push showed {shown}, not {message}")
    # Opening the notification goes to what it names: the needs-you list.
    page.goto(origin + message["url"])
    page.get_by_test_id("needs").wait_for(timeout=10_000)
    page.evaluate(
        """(endpoint) => fetch("/api/push/unsubscribe", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ endpoint }),
        })""",
        endpoint,
    )
    if endpoint in devices.read_text():
        failures.append(f"{name}: unsubscribing left the device")
    page.screenshot(path=SHOTS / f"app-{name}-notify.png")
    print(
        f"{name}: subscribed, showed a pushed notification after {deliveries} deliveries, and opened the needs-you list"
    )


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
            # The small tracker syncs with an origin, so the journeys can write a conflict from a second clone.
            remote = None
            if items < 1000:
                remote = directory / f"origin-{items}.git"
                subprocess.run(["git", "init", "-q", "--bare", str(remote)], check=True)
            plan = tracker(directory, env, items, remote)
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

            return plan, link, fresh, env, remote

        try:
            plan, link, fresh, env, remote = serve(350)
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
                if name == "desktop":
                    level_keys_journey(page, name, failures)
                if name == "phone" and hint is not None:
                    # Edits run once, on the phone the app is used on; the plan is read again after them.
                    leaf = min(
                        id for id in plan.children(hint) if not plan.children(id)
                    )
                    run(env, "--as-user", "item", "edit", leaf, "--notes", HOSTILE)
                    refresh(page)
                    edit_journey(page, view, env, plan, hint, name, failures)
                    form_journey(
                        page, view, env, remote, "journey-added", name, failures
                    )
                    failed_add_journey(page, errors, name, failures)
                    thread_journey(page, env, plan, "t0", name, failures)
                    needs_journey(page, env, plan, "t0", name, failures)
                    plan = Plan(env)
                page.screenshot(path=SHOTS / f"app-{name}.png")
                if errors:
                    failures.append(
                        f"{name}: the page threw or logged errors: {errors}"
                    )
                context.close()
            # Chromium's headless shell refuses notifications; its full build, headless, shows them.
            notifying = playwright.chromium.launch(channel="chromium")
            context, page, origin, errors = open_app(
                notifying, VIEWS["phone"], link, failures, "notifying phone"
            )
            link = fresh()
            push_journey(context, page, env, origin, "phone", failures)
            if errors:
                failures.append(
                    f"notifying phone: the page threw or logged errors: {errors}"
                )
            context.close()
            notifying.close()
            webkit = playwright.webkit.launch()
            context, page, _, errors = open_app(
                webkit, VIEWS["phone"], link, failures, "webkit phone"
            )
            wait_drawn(page, len(plan.default()))
            page.screenshot(path=SHOTS / "app-webkit-phone.png")
            # WebKit too closes the menu on a tap outside it, and the tap reaches nothing: a tap that reached this
            # collapsed box would select it, and open the inspector.
            target = next(id for id in plan.default() if plan.depth(id) == 1)
            drawn = page.get_by_test_id("map")
            before = drawn.get_attribute("data-nodes")
            page.get_by_test_id("menu").click()
            tap_middle(page, VIEWS["phone"], target)
            if (
                page.locator("details.menu").evaluate("d => d.open")
                or page.get_by_test_id("inspector").count()
                or drawn.get_attribute("data-state") != "drawn"
                or drawn.get_attribute("data-nodes") != before
            ):
                failures.append(
                    "webkit phone: a tap outside the menu didn't close it alone"
                )
            if errors:
                failures.append(
                    f"webkit phone: the page threw or logged errors: {errors}"
                )
            context.close()
            webkit.close()

            large, link, _, _, _ = serve(3500)
            context, page, _, errors = open_app(
                chromium, VIEWS["desktop"], link, failures, "3,500 items"
            )
            start = time.monotonic()
            wait_drawn(page, len(large.default()), timeout=LARGE_LIMIT_MS)
            print(
                f"3,500 items: drew the default view in {(time.monotonic() - start) * 1000:.0f} ms after the list"
            )
            first_view_readable(page, "3,500 items", failures)
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
