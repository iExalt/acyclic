"""Drives the app that `acyclic up` serves: pairing, installing, and the Map, as a phone and as a desktop.

`acyclic up` runs on a fixture with a home of its own; the browser pairs through the link it prints, and every request
that leaves the server is aborted. In Chromium, at both sizes, the page must list every item, be installable (which
needs the manifest and its icons fetched through the session's cookie), and draw the Map as the plan says it should: the
default view, a box expanded and collapsed again without moving on screen, archived subtrees shown, and a thread's
members lit. On the phone, an item is added, moved, linked and unlinked on the Map, an agent's note is shown as text,
the item's fields are edited, a conflict a second clone wrote is settled, and the item is reopened; a thread is
approved, an agent's proposals to it are kept and dropped, and its level, its flag and the global level are set; from
Needs you's view, a proposal is kept and a waiting thread approved. Chromium's full build, headless, keeps a
subscription it is given, records a pushed notification without displaying a system banner, and opens Needs you's view
it names. WebKit draws the default view at the phone's size, and the Threads view's arrows at the desktop's. In Chromium
and WebKit, a page on another port of the same host can neither write with the paired browser's cookie nor frame the
app. Chromium then draws the 3,500-item fixture. What the Map should show is worked out here from the tracker's own
answers, not from the app's code. Screenshots go to target/browser/.

Usage: app.py <acyclic binary> <acyclic-fixtures binary>
"""

import functools
import inspect
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, quote, urlsplit

from playwright.sync_api import Locator, sync_playwright
from playwright.sync_api import TimeoutError as PlaywrightTimeout

ACYCLIC, FIXTURES = (str(Path(arg).resolve()) for arg in sys.argv[1:3])

# A locator's wait checks again at 0, 20, 50, 100 and 100 ms, then every 500 ms, so a change that lands just after
# 270 ms is seen at 770: each write the journeys wait on lost about half a second. Waits of 150 ms at most, one after
# another, look again within 80 ms however long the change takes.
_wait_for = Locator.wait_for


def _wait_for_promptly(self, state="visible", timeout=None):
    # No limit, as Playwright reads a timeout of 0, has no deadline to slice up to.
    if timeout == 0:
        return _wait_for(self, state=state, timeout=0)
    limit = 30_000 if timeout is None else timeout
    deadline = time.monotonic() + limit / 1000
    while True:
        left = (deadline - time.monotonic()) * 1000
        try:
            return _wait_for(self, state=state, timeout=max(1, min(150, left)))
        except PlaywrightTimeout as err:
            if time.monotonic() >= deadline:
                raise PlaywrightTimeout(f"{self} wasn't {state} within {limit} ms") from err


Locator.wait_for = _wait_for_promptly
SHOTS = Path("target/browser").resolve()
# A large Android phone at its default resolution, the kind the app is checked on by hand, and a laptop's window.
VIEWS = {
    "phone": {
        "viewport": {"width": 384, "height": 832},
        "device_scale_factor": 2.8125,
        "is_mobile": True,
        "has_touch": True,
        # The phone in daylight, the laptop at the desk: each scheme has journeys drawn in it.
        "color_scheme": "light",
    },
    "desktop": {"viewport": {"width": 1440, "height": 900}, "color_scheme": "dark"},
}
# The furthest the Map's first view zooms out, on every screen, as PlanMap.tsx has it.
FIRST_ZOOM = 1
# How long the 3,500-item Map may take to draw: a loose bound against hanging, not a timing.
LARGE_LIMIT_MS = 60_000
# How long the whole run may take on CI: past the first it warns, past the second it fails, so the journeys can't grow
# unnoticed. Shared runners took from 58 to 81 s over runs of the same code, so the limit leaves room for a slow one.
BUDGET_S, LIMIT_S = 60, 120
# Each timed step's name and seconds, for the run's summary.
TIMES = []


def timed(step):
    """Records how long each call of `step` takes, under its name and the view it ran in."""
    signature = inspect.signature(step)

    @functools.wraps(step)
    def timing(*args, **kwargs):
        bound = signature.bind(*args, **kwargs).arguments
        view = bound.get("name", bound.get("items"))
        label = step.__name__ + ("" if view is None else f" ({view})")
        start = time.monotonic()
        try:
            return step(*args, **kwargs)
        finally:
            TIMES.append((label, time.monotonic() - start))

    return timing


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


@timed
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
    """Waits until the Map has drawn exactly `nodes` nodes, and returns its element's data. Its edge count includes
    the links of an item under the pointer, so a check of it moves the pointer off the Map first."""
    page.wait_for_function(
        "n => { const m = document.querySelector('[data-testid=map]');"
        " return m && m.dataset.state === 'drawn' && Number(m.dataset.nodes) === n; }",
        arg=nodes,
        timeout=timeout,
    )
    return page.get_by_test_id("map").evaluate("m => ({ ...m.dataset })")


def pick_thread(page, plan, thread):
    """Picks a thread from the outline as a hand would: on a phone from the projects' sheet, opening projects one by one
    until a row of it shows, unless it is among the threads in no project. Returns the row picked."""
    if page.get_by_test_id("projects-button").count():
        page.get_by_test_id("projects-button").click()
        page.get_by_test_id("projects").wait_for()
    rows = page.locator(f"[data-thread='{thread}']")
    closed = page.locator("[data-toggle-project][aria-expanded=false]")
    while rows.count() == 0 and closed.count():
        closed.first.click()
    rows.first.click()
    return rows.first


def from_menu(page, act, closes=False):
    """Does `act` in the bar's menu, opening it first, as a hand would. A button `closes` the menu, so the next tap
    reaches what it lands on; a setting leaves it open, showing the new value, and Escape closes it after."""
    menu = page.locator("details.menu:has(> [data-testid=menu])")
    if not menu.evaluate("d => d.open"):
        page.get_by_test_id("menu").click()
    act()
    if menu.evaluate("d => d.open") != (not closes):
        raise AssertionError(
            "a button in the menu left it open" if closes else "a setting in the menu closed it"
        )
    if not closes:
        page.keyboard.press("Escape")


def refresh(page):
    from_menu(page, lambda: page.get_by_role("button", name="Refresh").click(), closes=True)


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
    fitted plan draws it too small to tap. `rise` lifts it by that share of the Map's height. The renderer's size is
    read again first: a slot that just came or went has changed the Map's height before the app's observer says so."""
    page.get_by_test_id("map").evaluate(
        "(m, [id, rise]) => { m.cy.resize(); m.cy.zoom(1); m.cy.center(m.cy.getElementById(id)); m.cy.panBy({ x: 0, y: -rise * m.clientHeight }); }",
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


# The Map's colours as drawn and as the page declares them, as #rrggbb, with what must hold still across a restyle.
READ_SCHEME = """m => {
    const hex = (c) => '#' + c.match(/[\\d.]+/g).slice(0, 3).map((x) => Math.round(Number(x)).toString(16).padStart(2, '0')).join('');
    // A custom property as the browser resolves it, since the build may write a colour shorter.
    const probe = document.body.appendChild(document.createElement('i'));
    const v = (n) => { probe.style.color = `var(${n})`; return hex(getComputedStyle(probe).color); };
    const leaf = m.cy.nodes('.leaf.stage-ready, .leaf.stage-unthreaded').first();
    const closed = m.cy.nodes('.collapsed').not('.lit, .selected, .dim').first();
    const open = m.cy.nodes('.box').not('.lit, .selected').first();
    const plain = m.cy.edges().not('.lit, .crossing').first();
    const badge = document.querySelector('[data-testid=thread-panel] .badge');
    return {
        css: { ground: v('--ground'), tile: v('--tile'), box: v('--box'), rule: v('--rule'), muted: v('--muted'), link: v('--link'), ink: v('--ink') },
        leaf: leaf.nonempty() ? hex(leaf.style('background-color')) : null,
        leafDim: leaf.hasClass('dim'),
        closed: closed.nonempty() ? [hex(closed.style('background-color')), hex(closed.style('border-color'))] : null,
        open: open.nonempty() ? [hex(open.style('background-color')), hex(open.style('border-color'))] : null,
        edge: plain.nonempty() ? hex(plain.style('line-color')) : null,
        lit: [...new Set(m.cy.edges('.lit').map((e) => hex(e.style('line-color'))))],
        badge: badge && hex(getComputedStyle(badge).backgroundColor),
        _: probe.remove(),
        marks: m.cy.nodes('.leaf').map((n) => decodeURIComponent(n.data('mark') || '')).join(''),
        still: { zoom: m.cy.zoom(), pan: m.cy.pan(), n: m.cy.elements().length, layouts: m.dataset.layouts,
                 lit: m.cy.nodes('.lit').length, selected: m.cy.nodes('.selected').length },
    };
}"""


def faded(colour, under):
    """`colour` at 35 % over `under`, as style.ts's `faded` draws a dimmed tile."""
    return "#" + "".join(
        f"{int(0.35 * int(colour[at : at + 2], 16) + 0.65 * int(under[at : at + 2], 16) + 0.5):02x}"
        for at in (1, 3, 5)
    )


def luminance(colour):
    channels = [int(colour[at : at + 2], 16) / 255 for at in (1, 3, 5)]
    r, g, b = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    x, y = sorted([luminance(a), luminance(b)], reverse=True)
    return (x + 0.05) / (y + 0.05)


def scheme_switch(page, name, failures):
    """The system's scheme changed under an open Map, with a thread lit: the Map is restyled in place, every colour the
    page's own for the new scheme, the lit route its badge's, and nothing else moves; and back again."""
    drawn = page.get_by_test_id("map")
    before = drawn.evaluate(READ_SCHEME)

    def follows(scheme, now):
        css = now["css"]
        wrong = []
        # A dimmed tile is drawn opaque, in its colour faded over its box's.
        tile = faded(css["tile"], css["box"]) if now["leafDim"] else css["tile"]
        if now["leaf"] not in (None, tile):
            wrong.append(f"a tile {now['leaf']}, not {tile}")
        if now["closed"] not in (None, [css["tile"], css["muted"]]):
            wrong.append(f"a closed box {now['closed']}")
        if now["open"] not in (None, [css["box"], css["rule"]]):
            wrong.append(f"an open box {now['open']}")
        if now["edge"] not in (None, css["link"]):
            wrong.append(f"a link {now['edge']}")
        if now["lit"] != [now["badge"]] or contrast(now["badge"], css["ground"]) < 3:
            wrong.append(f"the lit route {now['lit']} against its badge {now['badge']}")
        if now["still"] != before["still"]:
            wrong.append(f"the view moved: {before['still']} to {now['still']}")
        if wrong:
            failures.append(f"{name}: in the {scheme} scheme the Map drew " + "; ".join(wrong))

    follows("dark", before)
    for scheme in ("light", "dark"):
        page.emulate_media(color_scheme=scheme)
        # Restyled once a plain link is drawn in the scheme's link colour, which the page declares at once.
        page.wait_for_function(
            f"m => {{ const now = ({READ_SCHEME})(m); return now.edge === now.css.link"
            f" && (now.css.link === {before['css']['link']!r}) === {'false' if scheme == 'light' else 'true'}; }}",
            arg=drawn.element_handle(),
            timeout=5_000,
        )
        now = drawn.evaluate(READ_SCHEME)
        follows(scheme, now)
        if scheme == "light":
            if now["css"] == before["css"]:
                failures.append(f"{name}: the page kept its dark colours in the light scheme")
            left = {before["css"][key] for key in ("ink", "muted")} & {
                c.lower() for c in re.findall(r"#[0-9a-fA-F]{6}", now["marks"])
            }
            if left:
                failures.append(f"{name}: tiles' marks kept dark colours {left} in the light scheme")
            page.screenshot(path=SHOTS / f"app-{name}-light.png")
        elif now["css"] != before["css"]:
            failures.append(f"{name}: switching back left {now['css']}, not {before['css']}")
    print(f"{name}: switched the scheme to light and back, the Map restyled in place")


@timed
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
    # A blocked load, such as one the content policy refuses, shows only as a console error, and a style the Map's
    # renderer can't use only as a warning. WebKit on Linux says it ignores the viewport's keyboard setting, which only
    # Chromium and Firefox read.
    page.on(
        "console",
        lambda message: (
            errors.append(message.text)
            if message.type in ("error", "warning") and '"interactive-widget"' not in message.text
            else None
        ),
    )
    page.goto(link)
    if page.url.rstrip("/") != origin:
        failures.append(f"{name}: pairing landed on {page.url}, not the app")
    # The shell shows once the plan has loaded; a failed load is an alert.
    page.locator(".shell, [role=alert]").first.wait_for(timeout=15_000)
    return context, page, origin, errors


def to_map(page):
    """Chooses the Map, which the journeys drawing on it start from: the app opens on the Overview."""
    page.locator("[data-view=map]").click()
    page.locator("[data-view=map][aria-current=page]").wait_for(timeout=5_000)
    page.get_by_test_id("overview").wait_for(state="detached", timeout=5_000)


@timed
def overview_journey(page, view, name, failures):
    """The app opens on the Overview: its four lists, a thread's row taking the user to the Map with its panel open, a
    needs row going to Needs you's view at its entry, that view marked as the one shown, and on a phone Back from the
    Map coming home."""
    overview = page.get_by_test_id("overview")
    overview.wait_for(timeout=5_000)
    headings = overview.locator(".overview-section h2").all_inner_texts()
    if headings != ["Needs you", "In progress", "Up next", "Projects"]:
        failures.append(f"{name}: the Overview's lists are {headings}")
    row = overview.locator("[data-overview-thread]").first
    thread = row.get_attribute("data-overview-thread")
    row.click()
    panel = page.locator("[data-testid=thread-panel]:not([aria-busy])")
    try:
        panel.wait_for(timeout=10_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: {thread}'s row on the Overview opened no panel")
    if page.locator("[data-view=map][aria-current=page]").count() != 1:
        failures.append(f"{name}: {thread}'s row on the Overview didn't go to the Map")
    # Going to a list view leaves the thread, whose panel would only stand over the list.
    page.locator("[data-view=overview]").click()
    try:
        panel.wait_for(state="detached", timeout=5_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: going to the Overview left {thread}'s panel open over it")
    entry =overview.locator(".overview-section").first.locator("button.overview-row").nth(1)
    entry.click()
    needs = page.get_by_test_id("needs")
    needs.wait_for(timeout=5_000)
    overview.wait_for(state="detached", timeout=5_000)
    at = needs.locator("[data-at=true]")
    if at.count() != 1 or not at.evaluate("e => e === document.activeElement"):
        failures.append(f"{name}: a needs row on the Overview didn't open Needs you at its entry, focused")
    # Needs you is a view: the one shown, its name the bar's title on a phone, the address saying so.
    current = page.locator("[data-view][aria-current=page]").evaluate_all("l => l.map(e => e.dataset.view)")
    title = page.locator(".bar h1").all_inner_texts()
    if current != ["needs"] or title not in ([], ["Needs you"]) or "view=needs" not in page.url:
        failures.append(f"{name}: Needs you shown read as current {current}, titled {title}, at {page.url}")
    page.locator("[data-view=overview]").click()
    needs.wait_for(state="detached", timeout=5_000)
    if view.get("has_touch"):
        # A phone's Back from another view comes home to the Overview, inside the app.
        url = page.url
        to_map(page)
        page.go_back()
        try:
            overview.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from the Map didn't come home to the Overview")
        if page.url != url:
            failures.append(f"{name}: Back from the Map left the app")
    print(f"{name}: the Overview listed {headings}, and its rows opened what they name")


@timed
def deep_link_journey(context, origin, view, plan, name, failures):
    """A link opens what it names, as if the user had opened it: Back closes it, then leaves the app; a reload opens it
    again; quick Escapes leave the address saying what's open; a stale link says so rather than opening nothing; an old
    Needs you address opens its view; and Back from an item or a thread opened on the Overview or in Needs you goes
    back there, as the address says, Needs you scrolled as it was left."""
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda err: errors.append(str(err)))
    page.goto("about:blank")
    id = min(one for one in plan.default() if plan.depth(one) == 1)
    link = f"{origin}/?item={quote(id)}"
    page.goto(link)
    title = page.locator("[data-testid=inspector] h2")
    try:
        title.wait_for(timeout=10_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: a link to {id} opened no item")
        page.close()
        return
    if title.inner_text() != plan.items[id]["title"] or page.url != link:
        failures.append(f"{name}: a link to {id} opened {title.inner_text()!r} at {page.url}")
    page.go_back()
    title.wait_for(state="detached", timeout=5_000)
    # The Map stays shown; on a phone it is away from home, one more Back.
    want = [f"{origin}/?view=map"] + ([f"{origin}/"] if view.get("has_touch") else [])
    for url in want:
        page.wait_for_function("u => location.href === u", arg=url, timeout=5_000)
        if url != want[-1]:
            page.go_back()
    page.go_back()
    if page.url != "about:blank":
        failures.append(f"{name}: Back from a link's last layer landed on {page.url}, not the page before the app")
    if not view.get("has_touch"):
        # Reading the address back is the same at either size, so one reload proves it.
        page.goto(link)
        title.wait_for(timeout=10_000)
        page.reload()
        title.wait_for(timeout=10_000)
        if title.inner_text() != plan.items[id]["title"]:
            failures.append(f"{name}: reloading a link to {id} opened {title.inner_text()!r}")
        # An item left open over another view opens there again, not on the Map.
        page.goto(f"{origin}/?view=threads&item={quote(id)}")
        title.wait_for(timeout=10_000)
        if page.get_by_test_id("threads-view").count() != 1:
            failures.append(f"{name}: an item open over Threads reopened on {page.url}, not over Threads")
        # A link to the tile drawn furthest right, where the panel will open, brings it out from under the panel.
        drawn = "() => document.querySelector('[data-testid=map]')?.dataset.state === 'drawn'"
        page.goto(f"{origin}/?view=map")
        page.wait_for_function(drawn, timeout=20_000)
        far = page.get_by_test_id("map").evaluate(
            "m => m.cy.nodes('.leaf').max(n => n.renderedBoundingBox().x2).ele.id()"
        )
        page.goto(f"{origin}/?item={quote(far)}")
        title.wait_for(timeout=10_000)
        page.wait_for_function(drawn, timeout=20_000)
        clear = page.evaluate(
            """id => { const m = document.querySelector('[data-testid=map]'); const r = m.getBoundingClientRect();
                const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });
                const panel = document.querySelector('[data-testid=inspector]').getBoundingClientRect();
                return [Math.round(r.left + b.x1), Math.round(r.left + b.x2), Math.round(panel.left)]; }""",
            far,
        )
        if not (clear[0] >= 0 and clear[1] <= clear[2]):
            failures.append(f"{name}: a link to {far} drew it at {clear[:2]}, under the panel from {clear[2]}")
    # A link to an item gone says so, and opens the rest it names: Needs you's view, from an address of when it opened
    # over the others, and a thread over it. Two layers, a phone's view away from home one of them, closed quicker than
    # the history moves, leave the address saying what's open.
    page.goto(f"{origin}/?item=no-such-item&thread=thread-0&needs")
    notice = page.get_by_test_id("notice")
    notice.wait_for(timeout=10_000)
    if "isn't in the plan" not in notice.inner_text():
        failures.append(f"{name}: a link to a missing item said {notice.inner_text()!r}")
    page.get_by_test_id("needs").wait_for(timeout=5_000)
    page.keyboard.press("Escape")
    page.keyboard.press("Escape")
    page.locator("[data-testid=thread-panel]").wait_for(state="detached", timeout=5_000)
    page.wait_for_timeout(300)
    if page.url != (f"{origin}/" if view.get("has_touch") else f"{origin}/?view=needs"):
        failures.append(f"{name}: after two quick Escapes the address is {page.url}")
    # An item opened from the Overview shows on the Map, and Back comes back to a bare address. On a phone, where Back
    # is the way out and the entries stack: not while its editor holds typing; one opened from Needs you comes back to
    # it, scrolled as it was and the focus on the row picked; and a thread opened from the Overview comes back to it.
    overview = page.get_by_test_id("overview")
    page.locator("[data-view=overview]").click()
    overview.wait_for(timeout=5_000)
    inspector = page.get_by_test_id("inspector")
    phone = view.get("has_touch")
    page.locator("[data-overview-project]").first.click()
    inspector.wait_for(timeout=5_000)
    if phone:
        inspector.locator("[data-action=edit]").click()
        inspector.locator("input[name=title]").fill("Typed, not saved")
        page.go_back()
        try:
            page.get_by_text("Unsaved changes").first.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back over an editor holding typing didn't say so")
        if not inspector.is_visible() or overview.count():
            failures.append(f"{name}: Back over an editor holding typing left the item")
        inspector.get_by_role("button", name="Cancel", exact=True).click()
    page.go_back()
    try:
        overview.wait_for(timeout=5_000)
        page.wait_for_function("() => location.search === ''", timeout=5_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: Back from an item opened on the Overview landed on {page.url}")
    if phone:
        page.locator("[data-view=needs]").click()
        needs = page.get_by_test_id("needs")
        needs.wait_for(timeout=5_000)
        # Scrolled down the list, so coming back shows whether it kept its place.
        top = needs.evaluate("n => (n.scrollTop = Math.min(400, n.scrollHeight - n.clientHeight))")
        # A row already in view, so the click doesn't scroll the list first.
        picked = needs.evaluate(
            """n => { const box = n.getBoundingClientRect();
                return [...n.querySelectorAll('[data-select]')].find(r => { const b = r.getBoundingClientRect();
                    return b.height > 0 && b.top >= box.top && b.bottom <= box.bottom; })?.dataset.select; }"""
        )
        needs.locator(f"[data-select='{picked}']").click()
        title.wait_for(timeout=10_000)
        page.go_back()
        try:
            needs.wait_for(timeout=5_000)
            page.wait_for_function("() => location.search === '?view=needs'", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from an item opened in Needs you landed on {page.url}")
        kept = needs.evaluate("(n, id) => [n.scrollTop, document.activeElement?.dataset.select === id]", picked)
        if top == 0 or abs(kept[0] - top) > 1 or not kept[1]:
            failures.append(
                f"{name}: Back to Needs you from {picked} came back scrolled {kept[0]} of {top}, focus there {kept[1]}"
            )
        page.go_back()
        overview.wait_for(timeout=5_000)
        # Up next's threads have an item ready, which the panel lists.
        page.locator("[aria-labelledby=overview-next] [data-overview-thread]").first.click()
        panel = page.locator("[data-testid=thread-panel]:not([aria-busy])")
        panel.wait_for(timeout=10_000)
        # An item chosen from the thread's panel closes back to the thread, and the thread back to the Overview.
        chosen = panel.locator("[data-select]").first
        if not chosen.count():
            failures.append(f"{name}: Up next's first thread lists no item, so Back from one went unchecked")
        else:
            chosen.click()
            title.wait_for(timeout=5_000)
            page.go_back()
            panel.wait_for(timeout=5_000)
        page.go_back()
        try:
            overview.wait_for(timeout=5_000)
            page.wait_for_function("() => location.search === ''", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from a thread opened on the Overview landed on {page.url}")
    if errors:
        failures.append(f"{name}: links threw {errors}")
    page.close()
    kept = "Back kept to it" if view.get("has_touch") else "Back and a reload kept to it, one far right came clear of the panel"
    print(f"{name}: a link opened {id}, {kept}, a stale link said so, and Back went where things were opened")

@timed
def command_journey(page, view, origin, plan, name, failures):
    """The command menu finds an item by its title and opens it, the address saying so, and gives the focus back when
    dismissed; on a phone, from the Map, Back then closes the item and comes home. Wide, it goes to Needs you, whose
    entries take the page's width in columns, and a thread one names goes to the Map and closes back."""
    id = min(one for one in plan.items if plan.depth(one) == 2 and plan.shown(one, False))
    title = plan.items[id]["title"]
    to_map(page)
    menu = page.get_by_test_id("command-menu")
    if view.get("has_touch"):
        page.locator("[data-action=search]").tap()
    else:
        page.keyboard.press("ControlOrMeta+k")
    menu.wait_for(timeout=5_000)
    page.keyboard.type(title)
    row = page.locator(f".command-row[data-command='item:{id}']")
    row.wait_for(timeout=5_000)
    if view.get("has_touch"):
        row.tap()
    else:
        # The first found is chosen; ↓ and ↑ come back to it.
        page.keyboard.press("ArrowDown")
        page.keyboard.press("ArrowUp")
        if row.get_attribute("aria-selected") != "true":
            failures.append(f"{name}: the command menu didn't choose {id} first for its own title")
        page.keyboard.press("Enter")
    heading = page.locator("[data-testid=inspector] h2")
    heading.wait_for(timeout=5_000)
    menu.wait_for(state="detached", timeout=5_000)
    # Over whatever else is open, such as a thread's panel the journeys before left.
    if heading.inner_text() != title or parse_qs(urlsplit(page.url).query).get("item") != [id]:
        failures.append(f"{name}: the command menu opened {heading.inner_text()!r} at {page.url}, not {id}")
    if view.get("has_touch"):
        page.go_back()
        heading.wait_for(state="detached", timeout=5_000)
        page.go_back()
        page.get_by_test_id("overview").wait_for(timeout=5_000)
        if page.url != f"{origin}/":
            failures.append(f"{name}: Back after the command menu's item didn't come home: {page.url}")
    else:
        # Dismissed, the focus goes back to where it was: the item's panel, still open under it.
        if not page.evaluate("() => !!document.activeElement?.closest('[data-testid=inspector]')"):
            failures.append(f"{name}: the item the command menu opened didn't take the focus")
        page.keyboard.press("ControlOrMeta+k")
        menu.wait_for(timeout=5_000)
        page.keyboard.press("Escape")
        menu.wait_for(state="detached", timeout=5_000)
        back = page.evaluate("() => !!document.activeElement?.closest('[data-testid=inspector]')")
        if not back or not heading.count():
            failures.append(f"{name}: Escape from the command menu didn't leave the item open, focused")
        page.keyboard.press("Escape")
        heading.wait_for(state="detached", timeout=5_000)
        page.keyboard.press("ControlOrMeta+k")
        menu.wait_for(timeout=5_000)
        page.keyboard.type("needs")
        page.locator(".command-row[data-command='view:needs']").click()
        needs = page.get_by_test_id("needs")
        needs.wait_for(timeout=5_000)
        columns = needs.locator("[data-need]").evaluate_all(
            "l => [new Set(l.map(e => e.offsetLeft)).size, Math.min(...l.map(e => e.offsetWidth))]"
        )
        if columns[0] < 2 or columns[1] < 360:
            failures.append(f"{name}: Needs you's entries stood in {columns[0]} columns, {columns[1]} px wide")
        page.screenshot(path=SHOTS / f"app-{name}-needs.png")
        # A thread an entry names goes to the Map, framed, as every thread goes; closing it comes back here.
        needs.locator("[data-pick-thread]").first.click()
        try:
            page.locator("[data-testid=thread-panel]:not([aria-busy])").wait_for(timeout=5_000)
            if page.locator("[data-view=map][aria-current=page]").count() != 1:
                failures.append(f"{name}: a thread picked in Needs you stayed off the Map, at {page.url}")
            page.keyboard.press("Escape")
            needs.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: a thread picked in Needs you didn't open, or close back to Needs you")
        to_map(page)
    print(f"{name}: the command menu found {id} by its title and opened it")


@timed
def keys_journey(page, plan, name, failures):
    """Single keys go to views and edit the item shown, a key typed in a field is the field's, and the menu's switch
    turns them all off."""
    view = page.locator("[data-view][aria-current=page]")
    page.keyboard.press("g")
    page.keyboard.press("t")
    page.get_by_test_id("threads-view").wait_for(timeout=5_000)
    page.keyboard.press("g")
    page.keyboard.press("m")
    page.get_by_test_id("threads-view").wait_for(state="detached", timeout=5_000)
    id = min(one for one in plan.default() if plan.depth(one) == 1)
    page.keyboard.press("ControlOrMeta+k")
    page.keyboard.type(plan.items[id]["title"])
    page.keyboard.press("Enter")
    page.locator("[data-testid=inspector] h2").wait_for(timeout=5_000)
    page.keyboard.press("e")
    form = page.get_by_test_id("item-form")
    form.wait_for(timeout=5_000)
    title = form.locator("input").first
    # E opens the editor on its title, so what is typed goes there.
    if not title.evaluate("field => field === document.activeElement"):
        failures.append(f"{name}: the editor didn't open on its title")
        title.focus()
    before = title.input_value()
    page.keyboard.press("End")
    page.keyboard.press("e")
    if title.input_value() != before + "e" or page.get_by_test_id("item-form").count() != 1:
        failures.append(f"{name}: E typed in the title field wasn't the field's: {title.input_value()!r}")
    form.get_by_role("button", name="Cancel").click()
    form.wait_for(state="detached", timeout=5_000)
    page.keyboard.press("Escape")
    page.locator("[data-testid=inspector]").wait_for(state="detached", timeout=5_000)
    from_menu(page, lambda: page.get_by_test_id("shortcuts").click())
    # Off the switch, which as a field would take the keys for itself.
    page.evaluate("() => document.activeElement?.blur()")
    page.keyboard.press("g")
    page.keyboard.press("t")
    page.wait_for_timeout(200)
    if view.get_attribute("data-view") != "map":
        failures.append(f"{name}: with single-key shortcuts off, G then T still went to {view.get_attribute('data-view')}")
    from_menu(page, lambda: page.get_by_test_id("shortcuts").click())
    # ⌘Z takes back the autonomy just set, once, however often it is pressed.
    level = page.get_by_test_id("global-level")
    page.get_by_test_id("menu").click()
    was = level.input_value()
    to = next(one for one in level.locator("option").evaluate_all("os => os.map(o => o.value)") if one != was)
    level.select_option(to)
    page.get_by_test_id("notice").get_by_role("button", name="Undo").wait_for(timeout=10_000)
    page.evaluate("() => document.activeElement?.blur()")
    writes = []
    page.on(
        "request",
        lambda request: (
            writes.append(request.post_data)
            if request.url.endswith("/api/command") and (request.post_data or "").startswith('{"level":{"level"')
            else None
        ),
    )
    page.keyboard.press("ControlOrMeta+z")
    page.keyboard.press("ControlOrMeta+z")
    page.get_by_text("Autonomy set back to").wait_for(timeout=10_000)
    page.wait_for_timeout(300)
    if level.input_value() != was or len(writes) != 1:
        failures.append(f"{name}: ⌘Z twice set the autonomy to {level.input_value()}, in {len(writes)} writes")
    print(f"{name}: single keys went to views and edited {id}, left typing alone, and switched off; ⌘Z undid once")


@timed
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


@timed
def pairing_journey(context, page, origin, env, name, failures):
    """The paired browser's cookie is the app's alone: page script can't read it, and a page on another port of the
    same host, which the browser sends it to, can neither write with it nor frame the app.

    A cross-origin JSON post needs a preflight the server never answers, so the browser refuses it before it is sent;
    the attacker's one way to send the cookie with a write is a form, which can't say it holds JSON. That the server
    also checks a cookie's request comes from its own origin is proven by its unit tests, since no browser sends one.
    """
    port = urlsplit(origin).port
    cookies = [c for c in context.cookies(origin) if c["name"].startswith("acyclic-up")]
    held = [(c["name"], c["httpOnly"], c["sameSite"], c["path"]) for c in cookies]
    if held != [(f"acyclic-up-{port}", True, "Strict", "/")]:
        failures.append(f"{name}: the paired browser holds the cookies {held}")
    if "acyclic-up" in page.evaluate("document.cookie"):
        failures.append(f"{name}: the page's script can read the session's cookie")
    form, fetched = f"cross-port-{name.replace(' ', '-')}", f"cross-port-fetch-{name.replace(' ', '-')}"
    # A text/plain form's body is its field's name, '=' and its value: together, a command that would add an item.
    body = json.dumps({"item": {"add": {"item": form, "title": "Cross-port", "notes": "="}}})
    field, value = body.split("=", 1)
    write = json.dumps({"item": {"add": {"item": fetched, "title": "Cross-port"}}})
    pages = {
        "/form": f"""<form method=post action="{origin}/api/command" enctype=text/plain>
            <input type=hidden name='{field}' value='{value}'></form>""",
        "/frame": f'<iframe src="{origin}/"></iframe>',
    }

    class Attacker(BaseHTTPRequestHandler):
        def do_GET(self):
            page = pages[self.path].encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(page)))
            self.end_headers()
            self.wfile.write(page)

        def log_message(self, *_):
            pass

    # Another port of the same host, served from loopback as a local server's page would be: the same site, so the
    # browser sends the cookie with its requests. A page from nowhere, such as one a route fulfils, isn't local, and
    # Chromium would refuse its loads from loopback before the server's own refusals were tried.
    server = ThreadingHTTPServer(("127.0.0.1", 0), Attacker)
    # Polled often, so shutting it down is quick.
    threading.Thread(target=server.serve_forever, args=(0.05,), daemon=True).start()
    attacker = f"http://127.0.0.1:{server.server_address[1]}"
    other = context.new_page()
    other.route(
        "**/*",
        lambda route: (
            route.continue_()
            if route.request.url.startswith((attacker + "/", origin + "/"))
            else route.abort()
        ),
    )
    other.goto(attacker + "/form")
    sent = other.evaluate(
        "([url, body]) => fetch(url, {method: 'POST', credentials: 'include',"
        " headers: {'Content-Type': 'application/json'}, body}).then(() => 'answered', () => 'refused')",
        [f"{origin}/api/command", write],
    )
    if sent != "refused":
        failures.append(f"{name}: another port's JSON write was {sent}, not refused by the browser")
    with other.expect_navigation(url=f"{origin}/api/command") as navigation:
        other.evaluate("document.forms[0].submit()")
    answer = navigation.value
    refused = (answer.status, answer.json()["error"]["code"]) if answer else None
    # 401 no-session would mean the cookie never went, and the check would prove nothing.
    if refused != (415, "not-json"):
        failures.append(f"{name}: another port's form write was answered {refused}, not 415 not-json")
    # The page's load waits for its frame's; a refused frame holds an engine's error page, which has no #root.
    other.goto(attacker + "/frame", timeout=5_000)
    frames = other.frames
    framed = len(frames) != 2 or frames[1].evaluate("!!document.getElementById('root')")
    other.close()
    server.shutdown()
    server.server_close()
    listed = {row["id"] for row in json.loads(run(env, "list"))["data"]["items"]}
    if landed := listed & {form, fetched}:
        failures.append(f"{name}: another port's writes reached the tracker: {sorted(landed)}")
    if framed:
        failures.append(f"{name}: a page on another port framed the app, or no frame was drawn")
    print(f"{name}: the cookie stayed the app's, and another port could neither write with it nor frame the app")


def first_view_readable(page, name, failures):
    """The Map opens at 1:1 or closer, with an item, not a box, wholly in view, title and all."""
    view = page.get_by_test_id("map").evaluate(
        "m => { const w = m.clientWidth, h = m.clientHeight; return { zoom: m.cy.zoom(),"
        " shown: m.cy.nodes().not('.box, .junction').some(n => { const b = n.renderedBoundingBox();"
        " return b.x1 >= 0 && b.y1 >= 0 && b.x2 <= w && b.y2 <= h; }) }; }"
    )
    if view["zoom"] < FIRST_ZOOM or not view["shown"]:
        failures.append(
            f"{name}: the Map opened unreadably small or showing nothing whole: {view}"
        )
    # One layout for a first load: a second, at a shape guessed before the Map was measured, would re-pack the plan
    # as it is first seen.
    asked = page.get_by_test_id("map").get_attribute("data-layouts")
    if asked != "1":
        failures.append(f"{name}: the first load asked for {asked} layouts, not one")


def stages_drawn(page, plan, name, failures):
    """Each tile drawn carries the stage the tracker gives it, as a class its tint and marks follow."""
    drawn_stages = page.get_by_test_id("map").evaluate(
        "m => Object.fromEntries(m.cy.nodes('.leaf').map(n => [n.id(),"
        " (n.classes().find(c => c.startsWith('stage-')) ?? '').slice(6) || null]))"
    )
    # A parent drawn as a tile, everything under it archived, has no stage of its own: its state says what it can.
    told = {
        id: plan.items[id].get("stage")
        or (state if (state := plan.items[id]["state"]) in ("done", "parked", "in-progress") else None)
        for id in drawn_stages
    }
    if not any(told.values()):
        failures.append(f"{name}: no tile with a stage drawn to check")
    elif drawn_stages != told:
        wrong = sorted(id for id in told if drawn_stages[id] != told[id])[:5]
        failures.append(f"{name}: tiles drawn with the wrong stage: {wrong}")


@timed
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
    else:
        # A notch of a Mac's mouse wheel, 80 pixels, zooms 12%, by the app's own step and not the renderer's as well.
        # centre() sets the zoom to 1 first, so the notch is well short of the furthest zoom in.
        drawn = page.get_by_test_id("map")
        centre(page, id)
        zoom = drawn.evaluate("m => m.cy.zoom()")
        box = drawn.bounding_box()
        page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
        page.mouse.wheel(0, -80)
        page.wait_for_function(
            "z => document.querySelector('[data-testid=map]').cy.zoom() !== z", arg=zoom
        )
        ratio = drawn.evaluate("m => m.cy.zoom()") / zoom
        if abs(ratio - 1.12) > 0.01:
            failures.append(
                f"{name}: a wheel notch zoomed the Map by {ratio:.3f}, not 1.12"
            )
    # Expand a box inside a root, then collapse it again: the count follows, and the box holds still on screen. Only
    # a box the layout moves proves the Map pans to match, so boxes are tried, biggest first, until one has moved.
    # The pan that holds it still knows nothing of the screen's size or of touch, so the phone, which finds the box
    # first, only taps; the desktop, starting from the box it found, measures that it held still.
    still = not view.get("has_touch")
    corner = "(m, id) => { const b = m.cy.getElementById(id).boundingBox({ includeLabels: false }); return [b.x1, b.y1]; }"
    moved, box, checked = 0, None, False
    for box in sorted(
        (id for id in plan.default() if plan.depth(id) == 1 and plan.children(id)),
        key=lambda id: (id != hint, -len(plan.children(id)), id),
    ):
        centre(page, box)
        placed = page.get_by_test_id("map").evaluate(corner, box)
        # Where it settles after expanding is where it starts collapsing.
        before = settled_corner(page, box) if still else None
        for action, nodes in [
            ("expanding", expected + len(plan.children(box))),
            ("collapsing", expected),
        ]:
            tap_box(page, view, box)
            wait_drawn(page, nodes)
            if action == "expanding":
                relaid = page.get_by_test_id("map").evaluate(corner, box) != placed
                if not checked:
                    stages_drawn(page, plan, name, failures)
                    checked = True
            if not still:
                continue
            after = settled_corner(page, box)
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
        # For the eye only, so a fixed wait will do: the renderer loads its images after drawing, and draws them later.
        centre(page, archived)
        page.wait_for_timeout(200)
        page.screenshot(path=SHOTS / f"app-{name}-archived.png")
        # Selected, then hidden with the rest: what's selected is drawn, so its panel and its address go with it.
        tap_middle(page, view, archived)
        page.get_by_test_id("inspector").wait_for(timeout=5_000)
    from_menu(page, lambda: page.get_by_test_id("show-archived").click())
    wait_drawn(page, expected)
    if archived is not None and (page.get_by_test_id("inspector").count() or "item=" in page.url):
        failures.append(f"{name}: hiding archived items left {archived} open, at {page.url}")
    # A thread lights up its members as drawn.
    thread, _ = Counter(
        row["thread"] for row in plan.items.values() if row["thread"]
    ).most_common(1)[0]
    # Picked with none of its items in view, the nearest comes into view, clear of the panel the pick opens.
    # In braces, so the renderer itself isn't sent back: serialising it takes seconds.
    page.get_by_test_id("map").evaluate(
        "m => { m.cy.pan({ x: 2 * m.clientWidth, y: 2 * m.clientHeight }); }"
    )
    pick_thread(page, plan, thread)
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
        "m => { const lit = m.cy.edges('.lit'); const badge = document.querySelector('[data-testid=thread-panel] .badge');"
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
    if name == "desktop":
        scheme_switch(page, name, failures)
    panel = page.locator("[data-testid=thread-panel]:not([aria-busy])")
    panel.wait_for()
    if panel.locator("[role=alert]").count():
        failures.append(f"{name}: the thread's panel failed: {panel.inner_text()}")
    page.screenshot(path=SHOTS / f"app-{name}-thread.png")
    # Escape closes the panel the pick opened, and focus goes back to the thread picked, where its row is still in
    # view; picked again, it reopens.
    button = page.locator(f".threads [data-thread='{thread}']").first
    page.keyboard.press("Escape")
    back = button.count() == 0 or button.evaluate("b => b === document.activeElement")
    if panel.count() or not back:
        failures.append(
            f"{name}: Escape left the thread's panel open, or focus away from {thread}"
        )
    pick_thread(page, plan, thread)
    panel.wait_for()
    if view.get("has_touch"):
        # A phone's Back closes what's open one layer at a time, and stays in the app: the outline's sheet over the
        # thread's panel, then the panel.
        sheet = page.get_by_test_id("projects")
        page.get_by_test_id("projects-button").click()
        sheet.wait_for()
        page.go_back()
        sheet.wait_for(state="detached")
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
        # In the app, on the Map, the address saying so.
        if not stayed or urlsplit(page.url).query != "view=map":
            failures.append(
                f"{name}: Back didn't close the outline's sheet, then the thread's panel, in the app: at {page.url}"
            )
        if not panel.count():
            pick_thread(page, plan, thread)
        panel.wait_for()
        # Closed in the app, a layer gives its entry back, so the next Back closes what's open then. Under the panel,
        # the Map, away from the Overview, holds an entry of its own.
        page.get_by_test_id("projects-button").click()
        sheet.get_by_role("button", name="Close", exact=True).click()
        sheet.wait_for(state="detached")
        try:
            page.wait_for_function("() => history.state?.layers === 2", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(
                f"{name}: closing the outline's sheet left the history at {page.evaluate('JSON.stringify(history.state)')}"
            )
        # A jump back over both entries, as a long press on Back makes, closes both.
        page.get_by_test_id("projects-button").click()
        sheet.wait_for()
        page.evaluate("history.go(-2)")
        try:
            sheet.wait_for(state="detached", timeout=5_000)
            panel.wait_for(state="detached", timeout=5_000)
            page.wait_for_function("() => history.state?.layers === 1", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(
                f"{name}: going back two entries didn't close both the outline's sheet and the thread's panel"
            )
        if not panel.count():
            pick_thread(page, plan, thread)
        panel.wait_for()
    print(
        f"{name}: drew {expected} nodes, expanded and collapsed {box}"
        + (f" held within {moved:.1f} px" if still else "")
    )
    return box


# A note as an agent might write one, which must show as text and never run.
HOSTILE = '<img src=x onerror="window.pwned = true">'


@timed
def outline_journey(page, view, plan, name, failures):
    """Selecting an item marks its project and its thread's row there, opening the project. On a phone the projects
    come as a sheet over the item's, and Back closes the sheet first, staying in the app."""
    picked = next(
        id for id in sorted(plan.default()) if plan.depth(id) == 1 and plan.items[id]["thread"]
    )
    project, thread = plan.chain(picked)[-1], plan.items[picked]["thread"]
    centre(page, picked)
    tap_box(page, view, picked, chevron=False)
    inspector = page.get_by_test_id("inspector")
    inspector.wait_for()
    phone = view.get("has_touch")
    sheet = page.get_by_test_id("projects")
    if phone:
        page.get_by_test_id("projects-button").click()
        try:
            sheet.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: the projects' sheet didn't show over {picked}'s")
            return
    marked = page.locator(f".project[aria-current=true] [data-select-project='{project}']")
    row = page.locator(f"[data-thread='{thread}'][data-project='{project}'][aria-current=true]")
    try:
        # The project opens itself once the outline has drawn.
        row.wait_for(timeout=5_000)
        missed = not marked.count()
    except PlaywrightTimeout:
        missed = True
    if missed:
        failures.append(f"{name}: selecting {picked} didn't mark project {project} and {thread}'s row there")
    if phone:
        url = page.url
        page.go_back()
        try:
            sheet.wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back left the projects' sheet open")
        if page.url != url:
            failures.append(f"{name}: Back from the projects' sheet left the app")
        try:
            inspector.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from the projects' sheet closed {picked}'s too")
    # The item's thread, picked from its panel, closes the item and opens the thread's panel, focus going to it.
    inspector.locator(f"[data-pick-thread='{thread}']").click()
    panel = page.get_by_test_id("thread-panel")
    try:
        inspector.wait_for(state="detached", timeout=5_000)
        panel.wait_for(timeout=5_000)
        if not panel.evaluate("p => p.contains(document.activeElement)"):
            failures.append(f"{name}: picking {thread} from {picked}'s panel left focus outside the thread's")
    except PlaywrightTimeout:
        failures.append(f"{name}: picking {thread} from {picked}'s panel didn't swap it for the thread's")
    page.keyboard.press("Escape")
    if not phone:
        # With a project open, a long name in the outline is cut short rather than the sidebar scrolling sideways.
        wide = page.locator(".sidebar-body").evaluate("b => b.scrollWidth - b.clientWidth")
        if wide > 0:
            failures.append(f"{name}: the sidebar's outline is {wide} px wider than the sidebar")
        # The sidebar folds to a rail that keeps Needs you's count, remembered here, and unfolds from the same place.
        fold = page.locator("[data-action=fold]")
        spot = fold.bounding_box()
        fold.click()
        rail = page.locator(".sidebar").bounding_box()
        kept = page.get_by_test_id("needs-count").is_visible()
        again = fold.bounding_box()
        remembered = page.evaluate("localStorage.getItem('acyclic.sidebar')")
        fold.click()
        if rail["width"] > 56 or not kept or remembered != "folded" or abs(again["y"] - spot["y"]) > 1:
            failures.append(
                f"{name}: folded, the sidebar was {rail['width']} px, count shown {kept}, remembered {remembered!r},"
                f" the fold button moving from {spot['y']} to {again['y']}"
            )
        if page.locator(".sidebar").bounding_box()["width"] < 200:
            failures.append(f"{name}: the sidebar didn't unfold")
    print(f"{name}: selecting {picked} marked its project and its thread's row, and its thread opened from it")


@timed
def threads_journey(page, view, env, name, failures):
    """The Threads view puts each thread in the wave `thread list` gives it, with an arrow for each wait side by side
    and none on a phone; picking a tile opens its thread's panel, and the Map comes back as it was."""
    listed = json.loads(run(env, "thread", "list"))["data"]
    want = {id: str(at + 1) for at, ids in enumerate(listed["waves"]) for id in ids}
    want |= {id: "anytime" for id in listed["anytime"]}
    page.locator("[data-view=threads]").click()
    page.get_by_test_id("threads-view").wait_for()
    got = page.locator(".wave [data-thread-tile]").evaluate_all(
        "tiles => tiles.map(tile => [tile.dataset.threadTile, tile.closest('.wave').dataset.wave])"
    )
    if dict(got) != want or len(got) != len(want):
        failures.append(f"{name}: the Threads view's waves {got} aren't the tracker's {want}")
    waits = sum(len(row["waits_for"]) for row in listed["threads"] if row["id"] in want)
    arrows = 0 if view.get("has_touch") else waits
    try:
        page.wait_for_function(
            f"() => document.querySelectorAll('.arrows g').length === {arrows}", timeout=5_000
        )
    except PlaywrightTimeout:
        drawn = page.locator(".arrows g").count()
        failures.append(f"{name}: the Threads view drew {drawn} arrows, expected {arrows}")
    if arrows:
        page.screenshot(path=SHOTS / f"app-{name}-threads.png")
        if skips := arrows_clear(page, want, name, failures):
            # Lit, an arrow that skips a wave, for a look at how it passes the tiles.
            page.locator(f"[data-thread-tile='{skips[0][0]}']").hover()
            page.screenshot(path=SHOTS / f"app-{name}-threads-lit.png")
            page.mouse.move(0, 0)
    # A thread with work to start, whose panel offers it; picked already, a click would unpick it, so it is unpicked.
    ready = [row["id"] for row in json.loads(run(env, "ready"))["data"]["ready"] if row["id"] in want]
    picked = ready[0] if ready else next(iter(want))
    tile = page.locator(f"[data-thread-tile='{picked}']")
    if tile.get_attribute("aria-pressed") == "true":
        tile.click()
    tile.click()
    try:
        page.locator("[data-testid=thread-panel]:not([aria-busy])").wait_for(timeout=10_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: picking {picked}'s tile opened no panel")
    if tile.get_attribute("aria-pressed") != "true":
        failures.append(f"{name}: picking {picked}'s tile didn't mark it")
    # An item chosen from a thread's panel is shown on the Map: the view goes back to it, the item selected.
    row = page.locator("[data-testid=thread-panel] [data-select]").first
    if row.count() == 0:
        failures.append(f"{name}: {picked}'s panel offers no item to start, so going back to the Map went unchecked")
        page.keyboard.press("Escape")
        page.locator("[data-view=map]").click()
    else:
        chosen = row.get_attribute("data-select")
        row.click()
        try:
            page.get_by_test_id("threads-view").wait_for(state="detached", timeout=5_000)
            page.get_by_test_id("inspector").wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: choosing {chosen} from {picked}'s panel left the Threads view over the Map")
        if page.locator("[data-view=map]").get_attribute("aria-current") != "page":
            failures.append(f"{name}: choosing {chosen} didn't switch to the Map")
        # Closing the item goes back to where it was chosen.
        page.keyboard.press("Escape")
        try:
            page.get_by_test_id("threads-view").wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: closing {chosen} didn't go back to the Threads view")
        page.locator("[data-view=map]").click()
    page.get_by_test_id("threads-view").wait_for(state="detached")
    # On a phone, Back from another view goes home, as the system's back gesture would leave the app otherwise; the
    # thread's panel, still picked, over the view, closes first. The Map is chosen again for the journeys after.
    if view.get("has_touch"):
        page.locator("[data-view=threads]").click()
        page.get_by_test_id("threads-view").wait_for()
        if page.get_by_test_id("thread-panel").count():
            page.go_back()
            page.get_by_test_id("thread-panel").wait_for(state="detached", timeout=5_000)
            if not page.get_by_test_id("threads-view").is_visible():
                failures.append(f"{name}: Back from a thread's panel over the Threads view left the view too")
        page.go_back()
        try:
            page.get_by_test_id("threads-view").wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from the Threads view didn't go home to the Overview")
        to_map(page)
    print(f"{name}: the Threads view put {len(want)} threads in their waves, with {arrows} arrows")


def arrows_clear(page, want, name, failures):
    """Checks every arrow as drawn, a point each 4 px along it, against the tiles' boxes as drawn: none crosses a tile,
    other than its own two at its ends, and the view scrolls to all of it. One must skip a wave, or the check proves
    little."""
    found = page.evaluate(
        """() => {
            const view = document.querySelector('.waves');
            const origin = view.getBoundingClientRect();
            const tiles = [...document.querySelectorAll('.wave [data-thread-tile]')].map(tile => [tile.dataset.threadTile,
                tile.getBoundingClientRect()]);
            const crossed = [];
            let outside = 0;
            for (const arrow of document.querySelectorAll('.arrows g')) {
                const line = arrow.querySelector('path:not(.head)');
                const matrix = line.getScreenCTM();
                const length = line.getTotalLength();
                for (let along = 0; along <= length; along += 4) {
                    const local = line.getPointAtLength(along);
                    const x = matrix.a * local.x + matrix.c * local.y + matrix.e;
                    const y = matrix.b * local.x + matrix.d * local.y + matrix.f;
                    const scrolledX = x - origin.left + view.scrollLeft;
                    const scrolledY = y - origin.top + view.scrollTop;
                    if (scrolledX < 0 || scrolledY < 0 || scrolledX > view.scrollWidth || scrolledY > view.scrollHeight) outside++;
                    for (const [id, box] of tiles) {
                        const own = (id === arrow.dataset.from && along < 10) || (id === arrow.dataset.to && along > length - 10);
                        if (!own && x > box.left + 1 && x < box.right - 1 && y > box.top + 1 && y < box.bottom - 1) {
                            crossed.push(`${arrow.dataset.from}->${arrow.dataset.to} over ${id}`);
                        }
                    }
                }
            }
            const pairs = [...document.querySelectorAll('.arrows g')].map(g => [g.dataset.from, g.dataset.to]);
            return { crossed: [...new Set(crossed)], outside, pairs };
        }"""
    )
    if found["crossed"]:
        failures.append(f"{name}: arrows cross tiles: {found['crossed'][:5]}")
    if found["outside"]:
        failures.append(f"{name}: {found['outside']} points of the arrows lie where the view doesn't scroll")
    skips = [
        (a, b)
        for a, b in found["pairs"]
        if want.get(a, "").isdigit() and want.get(b, "").isdigit() and int(want[b]) - int(want[a]) > 1
    ]
    if not skips:
        failures.append(f"{name}: no arrow skips a wave, so none was checked crossing one")
    return skips


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


def item_action(page, action):
    """Runs one of the item panel's actions from its menu, where all but Edit are."""
    page.get_by_test_id("item-menu").click()
    page.locator(f"[data-testid=inspector] [data-action={action}]").click()


def unlink(page, testid):
    """Removes a panel's first link of a kind, its button shown as a pointer comes over its row."""
    row = page.locator(f"[data-testid={testid}] .link").first
    row.hover()
    row.locator(".unlink").click()


def wait_inspector(page, selector, state="visible"):
    page.locator(f"[data-testid=inspector] {selector}").first.wait_for(
        state=state, timeout=10_000
    )


def held(page, id, before, what, name, failures):
    after = settled_corner(page, id)
    shift = max(abs(after["x"] - before["x"]), abs(after["y"] - before["y"]))
    if shift > 2:
        failures.append(f"{name}: {what} moved {id} by {shift:.0f} px on screen")


@timed
def edit_journey(page, view, env, plan, box, name, failures):
    """Adds, moves, links and unlinks on the Map, as the phone would, checking the Map and then the tracker after each.

    Works in `box`, a box map_journey has seen laid out: it is opened, and two of its leaves are used. Expects the
    first leaf's notes to be HOSTILE. Starts from a fresh page, so whatever the journeys before it opened on the Map
    is closed again."""
    page.reload()
    to_map(page)
    wait_drawn(page, len(plan.default()))
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
    # Half a pixel's slack for a sheet still rising, whose box is measured between pixels.
    if view.get("has_touch") and (reach[0] < 43.5 or not reach[1]):
        failures.append(
            f"{name}: too small to tap: close {reach[0]} px, chip reached {reach[1]}"
        )

    # Add inside the first leaf: the Map opens the leaf to show it, the leaf holds still, and the tracker has it.
    added = "journey-added"
    before = settled_corner(page, first)
    item_action(page, "add-child")
    form = page.get_by_test_id("add-form")
    form.locator("input[name=title]").fill("Journey: added!")
    if form.locator("input[name=id], select[name=project]").count():
        failures.append(f"{name}: the add form asked for an id, or a project inside an item")
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
    # Drawn, its panel takes the focus the form had, and says it is saving rather than the state core hasn't placed.
    drawn = page.evaluate(
        """() => {
            const panel = document.querySelector("[data-testid=inspector]");
            const status = [...(panel?.querySelectorAll("dt") ?? [])].find((dt) => dt.textContent === "Status");
            return { focused: panel?.contains(document.activeElement), status: status?.nextElementSibling?.textContent };
        }"""
    )
    if drawn != {"focused": True, "status": "Saving…"}:
        failures.append(f"{name}: a drawn add's panel had neither the focus nor Saving…: {drawn}")
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
    item_action(page, "move")
    prompt = page.get_by_test_id("prompt").bounding_box()
    if prompt is None or prompt["height"] > 60:
        failures.append(f"{name}: the move prompt covers the Map: {prompt}")
    # The menu over a mode closes alone: on an Escape pressed as it opens, and on a tap on the Map, which picks nothing.
    menu = page.locator("details.menu:has(> [data-testid=menu])")
    for close in (
        lambda: page.keyboard.press("Escape"),
        # In the middle of the Map, below the open menu, which the backdrop must stop it reaching.
        lambda: tap_middle(page, view, second),
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
    item_action(page, "waits-for")
    # A target refused, the box holding it, says why in the slot, whole however narrow the screen: the link waits on.
    tap_middle(page, view, second)
    refused = page.locator(".slot [data-testid=notice]")
    try:
        refused.wait_for(timeout=5_000)
        whole = refused.locator("span").first.evaluate("s => s.scrollWidth <= s.clientWidth + 1")
        if not whole or not page.get_by_test_id("prompt").is_visible():
            failures.append(f"{name}: a refused target's reason was cut, or the link stopped waiting")
    except PlaywrightTimeout:
        failures.append(f"{name}: a refused target said nothing in the slot")
    page.locator(".slot [aria-label=Dismiss]").click()
    page.locator(".slot").wait_for(state="detached", timeout=5_000)
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
    # Unlinked from the inspector: gone from the Map and the tracker. Its Undo waits in the slot at the stage's foot,
    # which takes its height from the Map without moving what the Map shows.
    page.locator(".slot [aria-label=Dismiss]").click()
    page.locator(".slot").wait_for(state="detached", timeout=5_000)
    before = settled_corner(page, added)
    unlink(page, "waits-for")
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    page.locator(".slot [data-testid=notice]").wait_for(timeout=5_000)
    held(page, added, before, "Undo's slot appearing", name, failures)
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
    # Undone, the link is back; unlinked again and linked meanwhile from elsewhere, the undo refuses, sending nothing.
    notice = page.get_by_test_id("notice")
    notice.get_by_role("button", name="Undo").click()
    wait_inspector(page, f"[data-testid=waits-for] [data-select='{first}']")
    if "again" not in notice.inner_text():
        failures.append(f"{name}: undoing the unlink said {notice.inner_text()!r}")
    check_tracker(
        env,
        added,
        lambda row: row["links"]["waits_for"] == [first],
        f"{added} waiting for {first} again, undone",
        failures,
        name,
    )
    unlink(page, "waits-for")
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    notice.get_by_role("button", name="Undo").wait_for(timeout=5_000)
    run(env, "--as-user", "link", "add", added, "waits-for", first)
    # Sent, the undo would add a link already there, which the tracker wouldn't show: the writes the app sends tell.
    sent = []
    listen = lambda request: (
        request.url.endswith("/api/command")
        and (request.post_data or "").startswith('{"link"')
        and sent.append(request.post_data)
    )
    page.on("request", listen)
    notice.get_by_role("button", name="Undo").click()
    page.get_by_text("Changed since, so it can't be undone").wait_for(timeout=10_000)
    page.remove_listener("request", listen)
    if sent:
        failures.append(f"{name}: a refused undo sent {sent}")
    # Unlinked once more, as the journeys after expect: a new wait in an approved thread needs the user.
    unlink(page, "waits-for")
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    page.screenshot(path=SHOTS / f"app-{name}-edited.png")
    print(
        f"{name}: added, moved, linked and unlinked {added}, the pinned boxes holding still"
    )


@timed
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
        # A setting leaves the menu open, showing what it now is.
        if not page.locator("details.menu:has(> [data-testid=menu])").evaluate("d => d.open"):
            failures.append(
                f"{name}: Enter on the autonomy list wrote, and closed the menu"
            )
            page.get_by_test_id("menu").click()
        levels.focus()
        page.keyboard.press("ArrowUp")
        page.keyboard.press("Escape")
        page.get_by_test_id("menu").click()
        if writes != [step] or levels.input_value() != step:
            failures.append(
                f"{name}: Escape after an arrow key wrote {writes[1:]} or left {levels.input_value()} showing"
            )
        # Enter on the list with nothing chosen leaves it open too.
        levels.focus()
        page.keyboard.press("Enter")
        if not page.locator("details.menu:has(> [data-testid=menu])").evaluate("d => d.open"):
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
        if page.locator("details.menu:has(> [data-testid=menu])").evaluate("d => d.open"):
            page.get_by_test_id("menu").click()
        print(
            f"{name}: arrow keys on the autonomy list wrote once, on Enter, and a pick at once"
        )
    finally:
        page.remove_listener("request", sent)


@timed
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
        or not page.locator(".slot [data-testid=notice]").get_by_text("Unsaved changes").count()
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


@timed
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
    # Drawn at once, the edit is saved only once the app is no longer busy.
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
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
    item_action(page, "reopen")
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


@timed
def thread_journey(page, env, plan, thread, name, failures):
    """Approves the thread from its panel, keeps and drops an agent's proposals, and sets its level, its
    project-management flag and the global level, checking the panel and then the tracker."""
    panel = page.get_by_test_id("thread-panel")
    # Picking a thread already picked unpicks it.
    pick_thread(page, plan, thread)
    try:
        panel.wait_for(timeout=3_000)
    except PlaywrightTimeout:
        pick_thread(page, plan, thread)

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
    # Its items, in the order their work runs, the first eight shown.
    listed = panel.locator(".thread-items li").count()
    if listed != min(members, 8):
        failures.append(f"{name}: the panel lists {listed} of {thread}'s {members} items")
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
    # The choice shows the pick at once and saves it in the background; the tracker is checked once both land.
    panel.locator("select[name=pm]").select_option("apply")
    settled_control(page, "[data-testid=thread-panel] select[name=pm]", "value", "apply")

    def level():
        page.get_by_test_id("global-level").select_option("everything")
        settled_control(page, "[data-testid=global-level]", "value", "everything")

    from_menu(page, level)
    # A Theme picked on the device shows at once, over the system's scheme, and System gives it back.
    ground = "() => getComputedStyle(document.documentElement).backgroundColor"
    lit = page.evaluate(ground)
    from_menu(page, lambda: page.get_by_test_id("theme").select_option("dark"))
    if lit == page.evaluate(ground) or page.evaluate("document.documentElement.dataset.theme") != "dark":
        failures.append(f"{name}: picking Dark over a light system left the page {lit}")
    from_menu(page, lambda: page.get_by_test_id("theme").select_option("system"))
    # Settings show at once and save in the background, one after another: the tracker has them once the last lands.
    deadline = time.monotonic() + 10
    while True:
        after = shown()
        if (after["approved"], after["level"], after["pm"]) == (True, "everything", True) and json.loads(
            run(env, "level")
        )["data"]["global"] == "everything":
            break
        if time.monotonic() > deadline:
            break
        page.wait_for_timeout(200)
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


@timed
def needs_journey(page, env, plan, thread, name, failures):
    """Keeps an agent's proposal and approves a waiting thread from Needs you's view, whose count follows. A thread or
    an item an entry names goes to the Map, and closing it, by Escape or its ✕, comes back to the view."""
    count = page.get_by_test_id("needs-count")
    # The queue's count shows beside its name, on a phone as anywhere.
    if not count.is_visible():
        failures.append(f"{name}: the needs-you count isn't shown beside its name")
    page.locator("[data-view=needs]").click()
    needs = page.get_by_test_id("needs")
    try:
        needs.wait_for(timeout=10_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: the Needs you tab showed no list")
        return
    # A thread picked from the list goes to the Map, as any thread does; closing it comes back to the view.
    viewed = "() => new URLSearchParams(location.search).get('view')"
    needs.locator("[data-pick-thread]").first.click()
    try:
        page.get_by_test_id("thread-panel").wait_for(timeout=5_000)
        if page.evaluate(viewed) != "map":
            failures.append(f"{name}: a thread picked from Needs you stayed off the Map, at {page.url}")
        page.keyboard.press("Escape")
        page.get_by_test_id("thread-panel").wait_for(state="detached", timeout=5_000)
        needs.wait_for(timeout=5_000)
        if page.evaluate(viewed) != "needs":
            failures.append(f"{name}: closing a thread from Needs you left the view at {page.url}")
    except PlaywrightTimeout:
        failures.append(f"{name}: a thread picked from Needs you didn't open, or close back to the list")
    # An item likewise, selected on the Map; its panel's ✕ comes back.
    row = needs.locator("button.row[data-select]").first
    if row.count():
        row.click()
        inspector = page.get_by_test_id("inspector")
        try:
            inspector.wait_for(timeout=5_000)
            if page.locator("[data-view=map][aria-current=page]").count() != 1:
                failures.append(f"{name}: an item picked from Needs you stayed off the Map, at {page.url}")
            inspector.get_by_role("button", name="Close", exact=True).click()
            needs.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: an item picked from Needs you didn't open, or its ✕ didn't come back to the list")
    else:
        failures.append(f"{name}: no needs-you entry selects an item, so coming back from one went unchecked")
    # The queue has the name its push notification gives it, apart from the in-review stage.
    named = (page.locator("[data-view=needs]").get_attribute("aria-label"), page.locator(".bar h1").inner_text())
    if not named[0].startswith("Needs you:") or named[1] != "Needs you":
        failures.append(f"{name}: the queue's tab and title read {named}, not Needs you")
    outside = next(
        id
        for id, row in sorted(plan.items.items())
        if row["thread"] not in (thread, None)
    )
    run(env, "item", "edit", outside, "--thread", thread)
    before = int(count.inner_text())
    # The count alone says something waits, as an inbox's does: no "!" beside it.
    if page.get_by_test_id("needs-badge").count() or not count.is_visible():
        failures.append(f"{name}: Needs you shows a badge, or no count")
    refresh(page)
    page.wait_for_function(
        "n => Number(document.querySelector('[data-testid=needs-count]').textContent) === n",
        arg=before + 1,
        timeout=10_000,
    )
    # The Approve filter shows only what waits for a yes or no, and All shows everything again.
    entries = needs.locator("[data-need]")
    every = entries.count()
    needs.locator("[data-filter=approve]").click()
    kinds = {key.split(":")[0] for key in entries.evaluate_all("l => l.map(e => e.dataset.need)")}
    if not kinds or not kinds <= {"proposal", "unapproved", "awaiting-approval", "new-blocker"}:
        failures.append(f"{name}: the Approve filter shows {sorted(kinds)}")
    needs.locator("[data-filter=all]").click()
    if entries.count() != every:
        failures.append(f"{name}: All shows {entries.count()} of {every} entries")
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
    # The keyboard stays in the list, on what took the entry's place, and the approval is said.
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    kept = page.evaluate(
        "() => document.querySelector('[data-testid=needs]').contains(document.activeElement)"
    )
    said = page.locator("p[role=status]").inner_text()
    if not kept or not said.startswith("Approved "):
        failures.append(
            f"{name}: approving from the list left focus at {page.evaluate('document.activeElement.tagName')}"
            f" and said {said!r}"
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
        # The pick shows in the outline, a project it works in opened to show it: on a phone, in the projects' sheet.
        sheet = page.get_by_test_id("projects-button")
        if sheet.count():
            sheet.click()
        try:
            page.locator(f"[data-thread='{picked}'][aria-pressed=true]").first.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: an entry's text didn't pick {picked}, or its row is out of view")
        if sheet.count():
            page.get_by_test_id("projects").get_by_role("button", name="Close", exact=True).click()
        panel.get_by_role("button", name="Close").click()
        # Picked from the list, the thread showed on the Map: closing it comes back to the list.
        try:
            needs.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: closing a thread picked from the needs-you list didn't come back to it")
    print(f"{name}: kept a proposal and approved {approved} in Needs you, and came back from what it named")


@timed
def push_journey(context, page, env, origin, name, failures):
    """Subscribes a device as the browser would, delivers a push through the service worker, and opens what it names.

    Headless Chromium reaches no real push service, so the subscription is one the page makes up, with a real P-256 key,
    and the message comes in through DevTools rather than from the server. The worker's display call is recorded
    without sending a notification to the operating system.
    """
    context.grant_permissions(["notifications"], origin=origin)
    # The switch, in the menu, reads the permission as the menu opens.
    page.get_by_test_id("menu").click()
    notify = page.get_by_test_id("notify")
    notify.wait_for(timeout=10_000)
    if notify.is_checked() or not notify.is_enabled():
        failures.append(
            f"{name}: with permission and no subscription, the switch isn't off and ready"
        )
    page.keyboard.press("Escape")
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
    # Record at the OS display boundary: even headless Chrome can show desktop banners.
    # Keep the real push event and worker handler, but never invoke native notification UI.
    worker = context.service_workers[0]
    worker.evaluate(
        """() => {
            self.recordedNotifications = [];
            self.registration.showNotification = async (title, options) => {
                self.recordedNotifications.push({ title, body: options.body, url: options.data?.url, tag: options.tag });
            };
        }"""
    )
    cdp.send(
        "ServiceWorker.deliverPushMessage",
        {
            "origin": origin,
            "registrationId": registrations[0]["registrationId"],
            "data": json.dumps(message),
        },
    )
    deadline = time.monotonic() + 10
    shown = []
    while not shown and time.monotonic() < deadline:
        shown = worker.evaluate("self.recordedNotifications")
        if not shown:
            page.wait_for_timeout(50)
    expected = {**message, "tag": "needs"}
    if shown != [expected]:
        failures.append(f"{name}: the push recorded {shown}, not {expected}")
    # Opening the notification shows what it names, Needs you's view, in the app already open, without reloading it:
    # the worker asks the page as its click handler does, and the page answers.
    page.get_by_test_id("overview").wait_for(timeout=5_000)
    page.evaluate("() => { window.stayed = true; }")
    answered = worker.evaluate(
        """async (search) => {
            const [client] = await self.clients.matchAll({ type: "window" });
            const channel = new MessageChannel();
            const answer = new Promise((resolve) => { channel.port1.onmessage = () => resolve(true); });
            client.postMessage({ kind: "open", search }, [channel.port2]);
            return Promise.race([answer, new Promise((resolve) => setTimeout(() => resolve(false), 5000))]);
        }""",
        "?needs",
    )
    page.get_by_test_id("needs").wait_for(timeout=10_000)
    page.locator("[data-view=needs][aria-current=page]").wait_for(timeout=5_000)
    if not answered or not page.evaluate("() => window.stayed === true"):
        failures.append(f"{name}: a notification's link reloaded the open app, or it didn't answer ({answered})")
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
        f"{name}: subscribed, recorded a pushed notification, and opened Needs you's view"
    )


def report(took, failures):
    """Prints the run's slowest steps, adds them all to CI's summary, and holds the run to its budget."""
    steps = Counter()
    for label, seconds in TIMES:
        steps[label] += seconds
    rows = steps.most_common()
    print(f"took {took:.1f} s; slowest: " + ", ".join(f"{label} {s:.1f} s" for label, s in rows[:5]))
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as out:
            out.write(f"### The app's journeys: {took:.1f} s (budget {BUDGET_S} s, limit {LIMIT_S} s)\n\n")
            out.write("| Step | Seconds |\n|---|---|\n")
            out.writelines(f"| {label} | {s:.1f} |\n" for label, s in rows)
            out.write(f"| everything else | {took - sum(steps.values()):.1f} |\n")
    if took > LIMIT_S:
        failures.append(f"the journeys took {took:.1f} s, past their {LIMIT_S} s limit")
    elif took > BUDGET_S:
        print(f"::warning title=The app's journeys::took {took:.1f} s, over their {BUDGET_S} s budget")


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    failures = []
    began = time.monotonic()
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
                # The sidebar counts the plan's items; a phone's bar leaves the count out.
                counted = page.get_by_test_id("item-count")
                count = counted.inner_text() if counted.count() else str(len(plan.items))
                if count != str(len(plan.items)):
                    failures.append(
                        f"{name}: the page lists {count} items, expected {len(plan.items)}"
                    )
                installable(context, page, origin, name, failures)
                overview_journey(page, view, name, failures)
                to_map(page)
                # Both sizes lay the plan out alike, so a box one finds the layout moves, the next tries first.
                hint = map_journey(page, view, plan, name, failures, hint)
                outline_journey(page, view, plan, name, failures)
                threads_journey(page, view, env, name, failures)
                deep_link_journey(context, origin, view, plan, name, failures)
                command_journey(page, view, origin, plan, name, failures)
                if name == "desktop":
                    pairing_journey(context, page, origin, env, name, failures)
                    keys_journey(page, plan, name, failures)
                    level_keys_journey(page, name, failures)
                if name == "phone" and hint is not None:
                    # Edits run once, on the phone the app is used on; the plan is read again after them.
                    leaf = min(
                        id for id in plan.children(hint) if not plan.children(id)
                    )
                    # Read by the reload edit_journey starts with.
                    run(env, "--as-user", "item", "edit", leaf, "--notes", HOSTILE)
                    edit_journey(page, view, env, plan, hint, name, failures)
                    form_journey(
                        page, view, env, remote, "journey-added", name, failures
                    )
                    failed_add_journey(page, errors, name, failures)
                    thread_journey(page, env, plan, "thread-0", name, failures)
                    needs_journey(page, env, plan, "thread-0", name, failures)
                    plan = Plan(env)
                page.screenshot(path=SHOTS / f"app-{name}.png")
                if errors:
                    failures.append(
                        f"{name}: the page threw or logged errors: {errors}"
                    )
                context.close()
            # The full build supports push delivery; the journey records display calls before sending a push.
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
            context, page, origin, errors = open_app(
                webkit, VIEWS["phone"], link, failures, "webkit phone"
            )
            pairing_journey(context, page, origin, env, "webkit phone", failures)
            to_map(page)
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
                page.locator("details.menu:has(> [data-testid=menu])").evaluate("d => d.open")
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
            # WebKit draws the Threads view's arrows where Chromium does, clear of the tiles.
            link = fresh()
            context, page, _, errors = open_app(
                webkit,
                {**VIEWS["desktop"], "color_scheme": "light"},
                link,
                failures,
                "webkit desktop",
            )
            listed = json.loads(run(env, "thread", "list"))["data"]
            want = {id: str(at + 1) for at, ids in enumerate(listed["waves"]) for id in ids}
            page.locator("[data-view=threads]").click()
            page.locator(".arrows g").first.wait_for(timeout=10_000)
            arrows_clear(page, want, "webkit desktop", failures)
            if errors:
                failures.append(
                    f"webkit desktop: the page threw or logged errors: {errors}"
                )
            context.close()
            webkit.close()

            large, link, _, _, _ = serve(3500)
            context, page, _, errors = open_app(
                chromium, VIEWS["desktop"], link, failures, "3,500 items"
            )
            to_map(page)
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
    report(time.monotonic() - began, failures)
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    main()
