"""Drives the app that `acyclic up` serves: installing, and the Map, as a phone and as a desktop.

`acyclic up` runs on a fixture with a home of its own; the browser opens the address it prints, and every request
that leaves the server is aborted. In Chromium, at both sizes, the page must list every item, be installable (which
needs the manifest and its icons), and draw the Map as the plan says it should: the
default view, a box expanded and collapsed again without moving on screen, and a thread's
members lit. On the phone, an item is added, moved, linked and unlinked on the Map, an agent's note is shown as text,
the item's fields are edited, a conflict a second clone wrote is settled, and the item is reopened; a thread's panel
lists its items; from Needs you's view, a conflict is settled and threads that wait on each other are named. Chromium's
full build, headless, keeps a
subscription it is given, records a pushed notification without displaying a system banner, and opens Needs you's view
it names. WebKit draws the default view at the phone's size, and the Threads view's arrows at the desktop's. In Chromium
and WebKit, a page on another port of the same host can neither write to the app nor frame it. Chromium then draws the 3,500-item fixture. What the Map should show is worked out here from the tracker's own
answers, not from the app's code. Screenshots go to target/browser/.

Usage: app.py <acyclic binary> <acyclic-fixtures binary>
"""

import functools
import inspect
import json
import os
import re
import select
import subprocess
import sys
import tempfile
import threading
import time
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from itertools import pairwise
from pathlib import Path
from urllib.parse import parse_qs, quote, urlsplit

from playwright.sync_api import Locator, Page, sync_playwright
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
# Details are included in the whole run and may overlap the named steps; never add them to TIMES.
DETAILS = []


@contextmanager
def detail(label):
    start = time.monotonic()
    try:
        yield
    finally:
        DETAILS.append((label, time.monotonic() - start))


_screenshot = Page.screenshot


def _capture_time(self, *args, **kwargs):
    # Capture wall time includes rendering, transport and writing the original image, not just encoding it.
    with detail(f"capture {Path(kwargs.get('path') or 'unnamed').name}"):
        return _screenshot(self, *args, **kwargs)


Page.screenshot = _capture_time


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


def show_notice(page, expected_text=None):
    """Wait for the expected notice, then explicitly open its pressure disclosure."""
    notice = page.get_by_test_id("notice")
    notice.wait_for(state="attached")
    if expected_text is not None:
        notice.get_by_text(expected_text).wait_for(state="attached")
    trigger = page.get_by_test_id("notice-trigger")
    if trigger.count() and trigger.get_attribute("aria-expanded") == "false":
        trigger.click()
    notice.wait_for(state="visible")
    return notice


def close_notice_details(page):
    """Close only the disclosure before continuing with the underlying task."""
    trigger = page.get_by_test_id("notice-trigger")
    if trigger.count() and trigger.get_attribute("aria-expanded") == "true":
        page.keyboard.press("Escape")
        page.wait_for_function(
            "() => document.querySelector('[data-testid=notice-trigger]')?.getAttribute('aria-expanded') === 'false'"
        )


def reveal_need_target(target):
    """Open only a chosen member's own collapsed details before an explicit pick."""
    if target.is_visible():
        return
    identity = target.evaluate("e => [e.closest('[data-need]')?.dataset.need, e.dataset.pickThread, e.dataset.select]")
    details = target.locator("xpath=ancestor::*[contains(concat(' ', normalize-space(@class), ' '), ' need-details ')][1]")
    assert details.count() == 1 and details.evaluate("e => e.hidden"), "target is hidden outside collapsed details"
    entry = target.locator("xpath=ancestor::*[@data-need][1]")
    toggle = entry.locator(":scope > .need-disclosure-toggle")
    assert toggle.count() == 1 and toggle.get_attribute("aria-expanded") == "false"
    toggle.click()
    target.wait_for(state="visible")
    assert target.evaluate("e => [e.closest('[data-need]')?.dataset.need, e.dataset.pickThread, e.dataset.select]") == identity


class Plan:
    """The tracker's items under the initiative in context, which the Map draws alone, and what its views of them must
    show. The app puts the busiest initiative in context first, which is the fixture's; the items of the others are
    kept apart in `elsewhere`."""

    def __init__(self, env):
        ids = [row["id"] for row in json.loads(run(env, "item", "list"))["data"]["items"]]
        rows = json.loads(run(env, "item", "show", *ids))["data"]
        # The app puts the busiest initiative first in context, as `switcherRows` in src/initiatives.ts orders them.
        by_initiative = Counter(row["initiative"] for row in rows)
        self.initiative = by_initiative.most_common(1)[0][0]
        self.elsewhere = {row["id"]: row for row in rows if row["initiative"] != self.initiative}
        self.items = {row["id"]: row for row in rows if row["initiative"] == self.initiative}
        self.parent = {id: row["parent"] for id, row in self.items.items()}
        # The fixture's first thread: its ids rise in the order its records were made.
        self.first_thread = min(
            row["id"] for row in json.loads(run(env, "thread", "list"))["data"]["threads"]
        )

    def chain(self, id):
        """The item and its ancestors, nearest first."""
        out = [id]
        while self.parent.get(out[-1]) in self.items:
            out.append(self.parent[out[-1]])
        return out

    def depth(self, id):
        return len(self.chain(id)) - 1

    def folded(self):
        """The epics at the top that are finished, which the Map folds at once: each has leaves, all done or canceled."""
        parents = {parent for parent in self.parent.values() if parent in self.items}
        open_roots = {self.chain(id)[-1] for id in self.items if id not in parents and self.items[id]["stage"] not in ("done", "canceled")}
        return {id for id in self.items if self.depth(id) == 0 and id in parents and id not in open_roots}

    def default(self):
        """The default view's nodes: every item shown down to depth 1, below which every box is collapsed, and a
        finished epic at the top folded into one."""
        folded = self.folded()
        return [id for id in self.items if self.depth(id) <= 1 and (self.depth(id) == 0 or self.chain(id)[-1] not in folded)]

    def children(self, id):
        return [one for one in self.items if self.parent[one] == id]

    def lit(self, thread):
        """In the default view, a thread's members as drawn: themselves, or the depth-1 box hiding them, once."""
        drawn = set()
        for id, row in self.items.items():
            if row["thread"] == thread:
                drawn.add(self.chain(id)[-2] if self.depth(id) > 1 else id)
        return len(drawn)


LONG_CODE = "workspace/acyclic-app/src/a/very/long/path/that/has/no/spaces/to/break/at/" + "x" * 60 + "/file.tsx"


def add_thread_cycles(env):
    """Two cycles of two threads waiting on each other, in a project of their own, so Needs you names two. In each
    thread, a leaf waits on a leaf the other holds, which waits for nothing, so no cycle of items closes. Made apart
    from the fixture's items, which journeys add to and link, and which a seed's threads never form a cycle among."""

    def add(*args):
        return json.loads(run(env, *args))["data"]

    project = add("item", "add", "--title", "Cycles")["item"]
    for cycle in ("A", "B"):
        threads = [add("thread", "add", "--title", f"Cycle {cycle}{n}")["thread"] for n in (1, 2)]
        held = [
            add("item", "add", "--title", f"Cycle {cycle}{n} holds", "--parent", project, "--thread", thread)["item"]
            for n, thread in enumerate(threads, 1)
        ]
        for n, thread in enumerate(threads, 1):
            add(
                "item", "add", "--title", f"Cycle {cycle}{n} waits", "--parent", project, "--thread", thread,
                "--waits-for", held[n % 2],
            )


# Trackers below this many items are the small ones: they sync with an origin, so a second clone can write a conflict,
# and hold the cycles of threads Needs you names. The large one needs neither.
SMALL_TRACKER = 1000


@timed
def tracker(directory, env, items, remote=None, cycles=False):
    """A tracker holding the seeded fixture, synced to `remote` if given, so a second clone can write apart from it, and
    two cycles of threads if `cycles`."""
    plan = directory / f"plan-{items}.json"
    # Any seed's threads come in many waves, so arrows skip waves, and none makes threads wait on each other in a
    # cycle; the cycles Needs you names are made below.
    with detail(f"tracker {items}: generate"):
        data = json.loads(
            subprocess.run(
                [FIXTURES, "--items", str(items), "--seed", "3"],
                capture_output=True,
                text=True,
                check=True,
            ).stdout
        )
    plan.write_text(json.dumps(data))

    remote_args = [] if remote is None else ["--remote", str(remote)]
    run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "mac", *remote_args)
    with detail(f"tracker {items}: import"):
        run(env, "import", str(plan))
    # A root's parent is the plan's initiative, which is no item: the roots are the items with no parent here.
    ids = {item["id"] for item in data["items"]}
    parent = {item["id"]: item.get("parent") if item.get("parent") in ids else None for item in data["items"]}
    waits = {item["id"]: item.get("waits_for") for item in data["items"]}

    def line(id):
        while id is not None:
            yield id
            id = parent.get(id)

    if cycles:
        add_thread_cycles(env)
    # Done work waiting on open work, so Needs you holds an entry that names items. The open leaf and everything above
    # it wait for nothing, so the link closes no cycle.
    done = next(item["id"] for item in data["items"] if item.get("state") == "done")
    unwaiting = next(
        item["id"]
        for item in data["items"]
        if item.get("state") != "done"
        and item["id"] not in parent.values()
        and not any(waits[one] for one in line(item["id"]))
    )
    run(env, "item", "edit", done, "--waits-for", unwaiting)
    # A long path in backticks in the notes of the item the link journeys open, which the panel must wrap.
    first = min(
        item["id"]
        for item in data["items"]
        if parent.get(item["id"]) is not None and parent.get(parent[item["id"]]) is None
    )
    run(env, "item", "edit", first, "--notes", f"Kept in `{LONG_CODE}` for good.")
    if remote is not None:
        run(env, "sync")
    with detail(f"tracker {items}: final read"):
        return Plan(env)


def wait_drawn(page, nodes, timeout=20_000):
    """Waits until the Map has drawn exactly `nodes` nodes and its tiles have stopped gliding, and returns its element's
    data. Its edge count includes the links of an item under the pointer, so a check of it moves the pointer off the
    Map first."""
    page.wait_for_function(
        "n => { const m = document.querySelector('[data-testid=map]');"
        " return m && m.dataset.state === 'drawn' && Number(m.dataset.nodes) === n"
        # A write's moves glide for about 200 ms: positions are read once the renderer has nothing left animating.
        " && !m.cy?.elements().some((e) => e.animated()); }",
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


def settled_map(page):
    """The Map once it has settled: laid out, drawn, no camera intent pending or tile animating, the view held for two frames.

    The app says so itself, in data-settled, so the wait doesn't depend on how long a loaded machine takes."""
    map = page.locator("[data-testid=map][data-settled=true]")
    map.wait_for(timeout=10_000)
    return map


def settled_corner(page, id):
    """Where a box's top-left corner is drawn once the Map has settled, read from the renderer itself."""
    settled_map(page)
    return page.get_by_test_id("map").evaluate(
        """(m, id) => {
            const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });
            return { x: b.x1, y: b.y1 };
        }""",
        id,
    )


# The Map's colours as drawn and as the page declares them, as #rrggbb, with what must hold still across a restyle.
READ_SCHEME = """m => {
    const hex = (c) => '#' + c.match(/[\\d.]+/g).slice(0, 3).map((x) => Math.round(Number(x)).toString(16).padStart(2, '0')).join('');
    // A custom property as the browser resolves it, since the build may write a colour shorter.
    const probe = document.body.appendChild(document.createElement('i'));
    const v = (n) => { probe.style.color = `var(${n})`; return hex(getComputedStyle(probe).color); };
    const leaf = m.cy.nodes('.leaf.stage-ready').first();
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


# Answers the page's reduced-motion query as "reduce" and every other query as the browser does, so the Map's tiles
# snap to their places instead of gliding for about 200 ms. Only the Map's script reads that query: the stylesheet's own
# reduced-motion rules, which stop the sheets' rise, are not touched, as they would be by emulating the preference.
MAP_SNAPS = """() => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (query) => {
        const media = real(query);
        if (!String(query).includes('prefers-reduced-motion')) return media;
        return new Proxy(media, {
            get: (target, key) => {
                if (key === 'matches') return true;
                const value = Reflect.get(target, key);
                return typeof value === 'function' ? value.bind(target) : value;
            },
        });
    };
}"""


@timed
def open_app(browser, view, link, failures, name, glide=False):
    """Opens the app. The Map's tiles snap to their places, so a journey reads where they are without waiting out a
    glide; `glide` leaves them gliding, for the journeys that are about that."""
    origin = "{0.scheme}://{0.netloc}".format(urlsplit(link))
    context = browser.new_context(**view)
    if not glide:
        context.add_init_script(f"({MAP_SNAPS})()")
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
        failures.append(f"{name}: opening the app landed on {page.url}")
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
    if not view.get("has_touch"):
        # Up next, whole, sits in the first view at 1280 x 800, with two initiatives' work under their headings.
        # The initiative in context comes first and the plan's own fills Up next, so the second is put in context to show both.
        size = page.viewport_size
        page.set_viewport_size({"width": 1280, "height": 800})
        switch = page.locator("button.initiative")
        switch.nth(1).click()
        page.locator("button.initiative[aria-current=true]").nth(0).wait_for(timeout=5_000)
        fold = overview.evaluate(
            """o => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => {
              const s = o.querySelector('#overview-next').closest('section');
              done({
                bottom: Math.max(...[...s.children].map((e) => e.getBoundingClientRect().bottom)),
                groups: s.querySelectorAll('.group-head').length,
                height: innerHeight,
              });
            })))"""
        )
        page.set_viewport_size(size)
        switch.first.click()
        if fold["groups"] < 2 or fold["bottom"] > fold["height"]:
            failures.append(f"{name}: Up next ({fold['groups']} initiatives) ends at {fold['bottom']:.0f} px of {fold['height']}")
        else:
            print(f"{name}: Up next, under {fold['groups']} initiatives, ends at {fold['bottom']:.0f} of {fold['height']} px")
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
    # The last row, so with several the view opens at one that isn't simply its first.
    entry = overview.locator(".overview-section").first.locator("button.overview-row").last
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


def nav_entry(page):
    """The persisted navigation fields, separate from the visible panel and its draft."""
    return page.evaluate(
        """() => { const s=history.state,p=s?.place,c=p?.cameFrom;
          const text=v=>typeof v==='string'?v:null;
          return {url:location.href,position:Number.isSafeInteger(s?.position)?s.position:null,
            layers:Number.isSafeInteger(s?.layers)?s.layers:null,
            parents:Array.isArray(s?.parents)&&s.parents.length<=8&&s.parents.every(Number.isSafeInteger)?s.parents.slice():null,
            wide:typeof s?.wide==='boolean'?s.wide:null,view:text(p?.view),item:text(p?.item),thread:text(p?.thread),
            cameFrom:c?{view:text(c.view),by:text(c.by),item:text(c.item)}:null,needsAt:text(p?.needsAt)}; }"""
    )


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
    # A long path in the notes wraps inside the panel, which gets no sideways scrollbar for it.
    fit = page.get_by_test_id("inspector").evaluate(
        """p => {
            const edge = p.getBoundingClientRect().right;
            const code = [...p.querySelectorAll("[data-testid=notes] code")];
            return { code: code.length, over: Math.max(0, ...code.map((c) => c.getBoundingClientRect().right - edge)),
                     wide: p.scrollWidth - p.clientWidth };
        }"""
    )
    if fit["code"] != 1 or fit["over"] > 0.5 or fit["wide"] > 0:
        failures.append(f"{name}: a long path in {id}'s notes ran {fit['over']:.0f} px past its panel, scrolling it {fit['wide']} px ({fit['code']} code)")
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
        page.goto(f"{origin}/?view=map")
        far = page.wait_for_function(
            """() => {
                const map = document.querySelector('[data-testid=map]');
                if (!map?.cy) return false;
                let id = null, right = -Infinity;
                map.cy.nodes('node.leaf, node.collapsed').forEach(node => {
                    const box = node.renderedBoundingBox({ includeLabels: false });
                    if ([box.x1, box.x2, box.y1, box.y2].every(Number.isFinite)
                        && box.x2 > box.x1 && box.y2 > box.y1 && box.x2 > right) {
                        id = node.id();
                        right = box.x2;
                    }
                });
                return id;
            }""",
            timeout=20_000,
        ).json_value()
        page.goto(f"{origin}/?item={quote(far)}")
        title.wait_for(timeout=10_000)
        clear = page.wait_for_function(
            """id => {
                const m = document.querySelector('[data-testid=map]');
                const node = m?.cy?.getElementById(id);
                const panel = document.querySelector('[data-testid=inspector]');
                if (!node?.nonempty() || !node.hasClass('selected') || !panel) return false;
                const r = m.getBoundingClientRect();
                const b = node.renderedBoundingBox({ includeLabels: false });
                const p = panel.getBoundingClientRect();
                if (![b.x1, b.x2, b.y1, b.y2, r.left, p.left].every(Number.isFinite)
                    || b.x2 <= b.x1 || b.y2 <= b.y1) return false;
                const x1 = Math.round(r.left + b.x1);
                const x2 = Math.round(r.left + b.x2);
                const panelLeft = Math.round(p.left);
                return x1 >= 0 && x2 <= panelLeft ? [x1, x2, panelLeft] : false;
            }""",
            arg=far,
            timeout=20_000,
        )
        clear = clear.json_value()
        if not (clear[0] >= 0 and clear[1] <= clear[2]):
            failures.append(f"{name}: a link to {far} drew it at {clear[:2]}, under the panel from {clear[2]}")
    # A link to an item gone says so, and opens the rest it names: Needs you's view, from an address of when it opened
    # over the others, and a thread over it. Two layers, a phone's view away from home one of them, closed quicker than
    # the history moves, leave the address saying what's open.
    page.goto(f"{origin}/?item=no-such-item&thread={plan.first_thread}&needs")
    notice = show_notice(page)
    if "isn't in the plan" not in notice.inner_text():
        failures.append(f"{name}: a link to a missing item said {notice.inner_text()!r}")
    page.get_by_test_id("needs").wait_for(timeout=5_000)
    close_notice_details(page)
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
    page.wait_for_function(
        "() => location.search === '' && history.state?.layers === 0 && history.state?.place?.view === 'overview'",
        timeout=5_000,
    )
    overview_entry = nav_entry(page)
    page.locator("[data-overview-project]").first.click()
    inspector.wait_for(timeout=5_000)
    if phone:
        inspector.locator("[data-action=edit]").click()
        draft = inspector.locator("input[name=title]")
        draft.fill("Typed, not saved")
        page.go_back()
        try:
            show_notice(page, "Unsaved changes").get_by_text("Unsaved changes").wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back over an editor holding typing didn't say so")
        landed = nav_entry(page)
        if (
            not inspector.is_visible()
            or overview.count()
            or draft.input_value() != "Typed, not saved"
            or landed != overview_entry
        ):
            failures.append(
                f"{name}: Back over the dirty editor lost its draft or overwrote the landed Overview: {landed}"
            )
        close_notice_details(page)
        inspector.get_by_role("button", name="Cancel", exact=True).click()
    else:
        page.go_back()
    try:
        overview.wait_for(timeout=5_000)
        page.wait_for_function(
            "() => location.search === '' && history.state?.place?.view === 'overview' "
            "&& history.state?.layers === 0",
            timeout=5_000,
        )
    except PlaywrightTimeout:
        failures.append(
            f"{name}: Back from an item opened on the Overview landed on {page.url}"
        )
    if phone and page.url != f"{origin}/":
        failures.append(
            f"{name}: cancelling the dirty editor landed on {page.url}, not the Overview root"
        )
    # Forward opens it again from the place its entry holds, where it came from too: closing it goes back to the
    # Overview, not to the Map an address alone would leave.
    page.go_forward()
    try:
        inspector.wait_for(timeout=5_000)
        inspector.get_by_role("button", name="Close").click()
        overview.wait_for(timeout=5_000)
        page.wait_for_function("() => location.search === ''", timeout=5_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: Forward to an item opened on the Overview didn't close back to it, at {page.url}")
    if phone:
        page.locator("[data-view=needs]").click()
        needs = page.get_by_test_id("needs")
        needs.wait_for(timeout=5_000)
        # Scrolled down the list, so coming back shows whether it kept its place. A shorter page, so the fixture's few
        # entries, with the ring of threads among them, scroll.
        size = page.viewport_size
        page.set_viewport_size({"width": size["width"], "height": 480})
        # On to the next item's row when none shows there, as when a tall entry fills the view.
        top = needs.evaluate(
            """n => { n.scrollTop = Math.min(400, n.scrollHeight - n.clientHeight);
                const box = n.getBoundingClientRect();
                const rows = [...n.querySelectorAll('[data-select]')].map(r => r.getBoundingClientRect());
                const shown = rows.some(b => b.height > 0 && b.top >= box.top && b.bottom <= box.bottom);
                const below = rows.find(b => b.height > 0 && b.bottom > box.bottom);
                if (!shown && below) n.scrollTop += below.bottom - box.bottom;
                return n.scrollTop; }"""
        )
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
        kept = needs.evaluate(
            "(n, id) => [n.scrollTop, document.activeElement?.dataset.select === id]",
            picked,
        )
        page.set_viewport_size(size)
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
        open_full(page, "thread-panel")
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
    # A parent's status is its stage, from its leaves: finished work reads done, and unstarted work
    # says whether it is ready or waiting. Reuse the fixture and the page already open for links.
    parents = [row for row in plan.items.values() if row.get("progress")]
    cases = {
        "done": next((row for row in parents if row.get("stage") == "done"), None),
        "unstarted": next((row for row in parents if row.get("stage") in ("ready", "waiting")), None),
    }
    for case, parent in cases.items():
        if parent is None:
            failures.append(f"{name}: fixture has no {case} parent, so its Status went unchecked")
            continue
        page.goto(f"{origin}/?item={quote(parent['id'])}")
        title.wait_for(timeout=10_000)
        open_full(page, "inspector")
        shown = inspector.evaluate(
            """panel => {
                const status = [...panel.querySelectorAll('dt')].find(dt => dt.textContent === 'Status')?.nextElementSibling;
                return {
                    stage: status?.querySelector(':scope > span > [data-stage]')?.getAttribute('data-stage'),
                    counts: Object.fromEntries([...status.querySelectorAll('.stage-counts [data-stage]')]
                        .map(mark => [mark.dataset.stage, Number(mark.querySelector('.count').textContent)])),
                };
            }"""
        )
        counts = parent["progress"]["counts"]
        expected = {stage.replace("_", "-"): count for stage, count in counts.items() if stage != "leaves"}
        expected = {stage: count for stage, count in expected.items() if count}
        if shown != {"stage": parent["stage"], "counts": expected}:
            failures.append(f"{name}: parent {parent['id']} Status {shown} disagrees with {parent['stage']}, {expected}")
        page.screenshot(path=SHOTS / f"app-{name}-parent-{case}.png")
    if errors:
        failures.append(f"{name}: links threw {errors}")
    page.close()
    kept = "Back kept to it" if view.get("has_touch") else "Back and a reload kept to it, one far right came clear of the panel"
    print(f"{name}: a link opened {id}, {kept}, a stale link said so, and Back went where things were opened")

@timed
def history_journey(context, origin, view, plan, name, failures):
    """Back retraces linked items; a queued follow keeps the entry it lands on, and Search preserves a dirty form."""
    parents = set(plan.parent.values())
    leaves = {id for id in plan.items if id not in parents}
    chain = next(
        (
            (a, b, c)
            for a in sorted(leaves)
            for b in plan.items[a]["links"]["waits_for"]
            if b in leaves
            for c in plan.items[b]["links"]["waits_for"]
            if c in leaves and len({a, b, c}) == 3
        ),
        None,
    )
    if chain is None:
        failures.append(
            f"{name}: history fixture has no three distinct visible linked leaves"
        )
        return
    a, b, c = chain
    page = context.new_page()
    errors = []

    def opened(id):
        page.wait_for_function(
            "id => new URLSearchParams(location.search).get('item') === id && !!document.querySelector('[data-testid=inspector] h2')",
            arg=id,
            timeout=5_000,
        )
        assert (
            page.locator("[data-testid=inspector] h2").inner_text()
            == plan.items[id]["title"]
        ), f"wrong Inspector for {id}"

    def at_entry(entry):
        page.wait_for_function(
            "([position,layers]) => history.state?.position === position && history.state?.layers === layers"
            " && !document.querySelector('[data-testid=command-menu]')",
            arg=[entry["position"], entry["layers"]],
            timeout=5_000,
        )
        assert nav_entry(page) == entry, (
            f"restored entry differs: {nav_entry(page)} != {entry}"
        )

    def search(key, query):
        page.locator("[data-action=search]").click()
        menu = page.get_by_test_id("command-menu")
        menu.wait_for(timeout=5_000)
        menu.get_by_role("combobox").fill(query)
        menu.locator(f"[data-command='{key}']").click(timeout=5_000)
        menu.wait_for(state="detached", timeout=5_000)

    try:
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto(f"{origin}/?item={quote(a)}")
        opened(a)
        first = nav_entry(page)
        for id in (b, c):
            page.locator(f"[data-testid=waits-for] [data-select='{id}']").click(
                timeout=5_000
            )
            opened(id)
        third = nav_entry(page)
        page.go_back()
        opened(b)
        page.go_forward()
        opened(c)
        page.evaluate("history.go(-2)")
        opened(a)
        at_entry(first)
        page.evaluate("history.go(2)")
        opened(c)
        at_entry(third)

        page.locator("[data-action=search]").click()
        menu = page.get_by_test_id("command-menu")
        menu.wait_for(timeout=5_000)
        page.evaluate("""() => {
          const go=history.go.bind(history); window.__historyGo=go; window.__heldClose=null; window.__historyPops=0;
          addEventListener('popstate',()=>window.__historyPops++);
          history.go=delta=>{if(delta<0 && !window.__heldClose){window.__heldClose={delta,release:()=>go(delta)};return;}go(delta);};
        }""")
        menu.get_by_role("button", name="Close", exact=True).click()
        page.wait_for_function("() => window.__heldClose?.delta === -1", timeout=5_000)
        page.locator(f"[data-testid=waited-on-by] [data-select='{b}']").click(
            timeout=5_000
        )
        page.wait_for_function(
            "([id,title]) => document.querySelector('[data-testid=map]').cy.getElementById(id).hasClass('selected')"
            " && document.querySelector('[data-testid=inspector] h2')?.textContent?.trim() === title",
            arg=[b, plan.items[b]["title"]],
            timeout=5_000,
        )
        assert page.evaluate("() => window.__historyPops === 0"), (
            "held close navigated before release"
        )
        assert parse_qs(urlsplit(page.url).query).get("item") == [c], (
            "queued follow rewrote the departing entry"
        )
        page.evaluate("() => window.__heldClose.release()")
        opened(b)
        page.wait_for_function("() => window.__historyPops === 1", timeout=5_000)
        assert nav_entry(page)["position"] == third["position"] + 1, (
            "queued follow did not append its visit"
        )
        page.evaluate("() => {history.go=window.__historyGo;}")
        page.go_back()
        opened(c)
        at_entry(third)
        page.get_by_test_id("inspector").get_by_role(
            "button", name="Close", exact=True
        ).click()
        page.wait_for_function(
            "depth => history.state?.layers === depth && !new URLSearchParams(location.search).has('item')"
            " && !document.querySelector('[data-testid=inspector]')",
            arg=1 if view.get("has_touch") else 0,
            timeout=5_000,
        )
        assert page.url == f"{origin}/?view=map", (
            f"X did not close the visit trail: {page.url}"
        )

        page.close()
        page = context.new_page()
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto(f"{origin}/?item={quote(a)}")
        opened(a)
        item_entry = nav_entry(page)
        inspector = page.get_by_test_id("inspector")
        inspector.locator("[data-action=edit]").click()
        draft = inspector.locator("input[name=title]")
        draft.fill("History draft, not saved")
        search("view:needs", "Needs you")
        show_notice(page, "Unsaved changes").get_by_text("Unsaved changes").wait_for(timeout=5_000)
        at_entry(item_entry)
        assert draft.input_value() == "History draft, not saved", (
            "Search refusal lost the draft"
        )
        close_notice_details(page)
        before = int(page.locator("main.stage").get_attribute("data-reads"))
        search("refresh", "Refresh")
        page.wait_for_function(
            "before => Number(document.querySelector('main.stage').dataset.reads) > before",
            arg=before,
            timeout=5_000,
        )
        at_entry(item_entry)
        assert draft.input_value() == "History draft, not saved", (
            "Refresh lost the draft"
        )
        inspector.get_by_role("button", name="Cancel", exact=True).click()
        page.screenshot(path=SHOTS / f"app-{name}-history.png")
        assert not errors, errors
    except (PlaywrightTimeout, AssertionError) as error:
        state = nav_entry(page)
        page.screenshot(path=SHOTS / f"app-{name}-history-failure.png")
        failures.append(
            f"{name}: linked-item history or Search-over-form failed: {error}; {state}"
        )
    finally:
        if not page.is_closed():
            page.close()


@timed
def command_journey(page, view, origin, plan, name, failures):
    """The command menu finds an item by its title and opens it, the address saying so, and gives the focus back when
    dismissed; on a phone, from the Map, Back then closes the item and comes home. Wide, it goes to Needs you, whose
    entries take the page's width in columns, and a thread one names goes to the Map and closes back."""
    search_selection_journey(page.context, origin, plan, name, failures)
    id = min(one for one in plan.items if plan.depth(one) == 2)
    title_counts = Counter(item["title"] for item in plan.items.values())
    unique_titles = [
        one
        for one, item in plan.items.items()
        if plan.depth(one) == 2
       
        and title_counts[item["title"]] == 1
    ]
    if unique_titles:
        id = min(unique_titles)
    else:
        failures.append(f"{name}: the plan has no unique visible title for touch Enter coverage")
    title = plan.items[id]["title"]
    to_map(page)
    if view.get("has_touch"):
        # Earlier journeys send hardware keys on this page. A new document leaves the phone's keyboard hint unset,
        # so this proves the command menu's cold touch behavior rather than a menu opened after a keyboard was used.
        cold = page.context.new_page()
        cold.route(
            "**/*",
            lambda route: route.continue_()
            if route.request.url.startswith(origin)
            else route.abort(),
        )
        cold.goto(origin)
        cold.locator(".shell, [role=alert]").first.wait_for(timeout=15_000)
        cold.locator("[data-action=search]").tap()
        menu = cold.get_by_test_id("command-menu")
        menu.wait_for(timeout=5_000)
        query = menu.locator("input")
        rows = menu.locator(".command-row")
        if (
            query.get_attribute("aria-activedescendant") is not None
            or rows.evaluate_all(
                "rows => rows.some(row => row.getAttribute('aria-selected') === 'true')"
            )
        ):
            failures.append(f"{name}: a cold touch command menu started with a selected result")
        before = cold.url
        cold.keyboard.press("Enter")
        if menu.count() != 1 or cold.url != before:
            failures.append(f"{name}: Enter on a blank cold touch command menu ran a command")
        query.fill(title)
        row = cold.locator(f".command-row[data-command='item:{id}']")
        row.wait_for(timeout=5_000)
        if (
            query.get_attribute("aria-activedescendant") is not None
            or rows.evaluate_all(
                "rows => rows.some(row => row.getAttribute('aria-selected') === 'true')"
            )
        ):
            failures.append(f"{name}: typing in a touch command menu selected a result")
        query.press("Enter")
        cold.locator("[data-testid=inspector] h2").wait_for(timeout=5_000)
        if parse_qs(urlsplit(cold.url).query).get("item") != [id]:
            failures.append(f"{name}: Enter on an unset touch exact title didn't open {id}: {cold.url}")

        # Reload gives the second arrow check a fresh menu and an unset selection after the first key.
        cold.goto(origin)
        cold.locator(".shell, [role=alert]").first.wait_for(timeout=15_000)
        cold.locator("[data-action=search]").tap()
        menu = cold.get_by_test_id("command-menu")
        menu.wait_for(timeout=5_000)
        rows = menu.locator(".command-row")
        cold.keyboard.press("ArrowDown")
        if rows.first.get_attribute("aria-selected") != "true":
            failures.append(f"{name}: ArrowDown from unset touch selection didn't choose the first command")
        menu.get_by_role("button", name="Close").tap()
        menu.wait_for(state="detached", timeout=5_000)
        cold.locator("[data-action=search]").tap()
        menu.wait_for(timeout=5_000)
        rows = menu.locator(".command-row")
        query = menu.locator("input")
        if query.get_attribute("aria-activedescendant") is not None:
            failures.append(f"{name}: a fresh touch command menu didn't start with unset selection")
        cold.keyboard.press("ArrowUp")
        if rows.last.get_attribute("aria-selected") != "true":
            failures.append(f"{name}: ArrowUp from unset touch selection didn't choose the last command")
        cold.close()
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
        # Home is the bare address, but for the initiative in context, which the address keeps for a reload.
        opened = {k: v for k, v in parse_qs(urlsplit(page.url).query).items() if k != "initiative"}
        if page.url.split("?")[0] != f"{origin}/" or opened:
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
        # Going to another view from the menu, with the item still open, leaves the item, as the tabs do.
        page.keyboard.press("ControlOrMeta+k")
        menu.wait_for(timeout=5_000)
        page.keyboard.type("needs")
        page.locator(".command-row[data-command='view:needs']").click()
        needs = page.get_by_test_id("needs")
        needs.wait_for(timeout=5_000)
        try:
            heading.wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: going to Needs you from the command menu left {id}'s panel over it")
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


def search_selection_journey(context, origin, plan, name, failures):
    """Search opens a deep item before its canvas node exists, and recentres one already visible but off centre."""
    parents = set(plan.parent.values())
    titles = Counter(row["title"] for row in plan.items.values())
    leaves = sorted(
        id
        for id, row in plan.items.items()
        if id not in parents
        and plan.depth(id) > 1
        and titles[row["title"]] == 1
    )
    if not leaves:
        failures.append(f"{name}: fixture has no unique visible deep leaf for Search framing")
        return
    item = next((id for id in leaves if plan.items[id].get("thread") is None), leaves[0])
    title = plan.items[item]["title"]
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))

    def search_item():
        page.locator("[data-action=search]").click()
        menu = page.get_by_test_id("command-menu")
        menu.wait_for(timeout=5_000)
        menu.get_by_role("combobox").fill(title)
        row = menu.locator(f"[data-command='item:{item}']")
        row.wait_for(timeout=5_000)
        row.click(timeout=5_000)
        menu.wait_for(state="detached", timeout=5_000)

    def opened_and_framed(phase):
        page.wait_for_function(
            "([id,title]) => { const m=document.querySelector('[data-testid=map]'), c=m?.cy, n=c?.getElementById(id);"
            " const selected=c?.nodes('.selected').map(one=>one.id())??[], fit=document.querySelector('.map-fit');"
            " const panel=m?.parentElement.querySelector('.inspector,.sheet');"
            " if (!m || !n || n.empty() || !fit || selected.length!==1 || selected[0]!==id ||"
            " new URLSearchParams(location.search).get('item')!==id ||"
            " document.querySelector('[data-testid=inspector] h2')?.textContent?.trim()!==title || c.zoom()<.85) return false;"
            " const mr=m.getBoundingClientRect(), fr=fit.getBoundingClientRect(), pr=panel?.getBoundingClientRect();"
            " const shift=panel?new DOMMatrixReadOnly(getComputedStyle(panel).transform).m42:0;"
            " const sheet=pr && pr.width>=mr.width*.9;"
            " const open={x1:0,y1:fr.bottom-mr.top+8,x2:pr&&!sheet?pr.left-mr.left:mr.width,"
            " y2:pr&&sheet?pr.top-shift-mr.top:mr.height};"
            " const b=n.renderedBoundingBox({includeLabels:false}), cx=(b.x1+b.x2-open.x1-open.x2)/2,"
            " cy=(b.y1+b.y2-open.y1-open.y2)/2;"
            " return b.x1>=open.x1+8 && b.y1>=open.y1+8 && b.x2<=open.x2-8 && b.y2<=open.y2-8 &&"
            " Math.max(Math.abs(cx),Math.abs(cy))<=1; }",
            arg=[item, title],
            timeout=7_000,
        )
        fit = fit_occlusion(page, lit_only=False)
        geometry = page.get_by_test_id("map").evaluate(
            """(m,id) => { const n=m.cy.getElementById(id),b=n.renderedBoundingBox({includeLabels:false});
              return {zoom:m.cy.zoom(),box:{x1:b.x1,y1:b.y1,x2:b.x2,y2:b.y2}}; }""",
            item,
        )
        if fit is None:
            failures.append(f"{name}: {phase} has no Fit control or open Map area")
            return
        open_box = fit["open"]
        box = geometry["box"]
        center_error = {
            "x": (box["x1"] + box["x2"] - open_box["x1"] - open_box["x2"]) / 2,
            "y": (box["y1"] + box["y2"] - open_box["y1"] - open_box["y2"]) / 2,
        }
        contained = (
            box["x1"] >= open_box["x1"] + 8
            and box["y1"] >= open_box["y1"] + 8
            and box["x2"] <= open_box["x2"] - 8
            and box["y2"] <= open_box["y2"] - 8
        )
        if geometry["zoom"] < 0.85 or not contained or max(map(abs, center_error.values())) > 1:
            failures.append(
                f"{name}: {phase} left Search target {item} at zoom {geometry['zoom']}, "
                f"contained={contained}, center_error={center_error}, open={open_box}"
            )

    try:
        page.goto(origin)
        page.get_by_test_id("overview").wait_for(timeout=7_000)
        missing_before = page.evaluate(
            "id => { const c=document.querySelector('[data-testid=map]')?.cy;"
            " return !c || c.getElementById(id).empty(); }",
            item,
        )
        if not missing_before:
            failures.append(f"{name}: deep Search fixture target {item} was already in the cold Map core")
        search_item()
        opened_and_framed("cold Overview Search")
        page.screenshot(path=SHOTS / f"app-{name}-search-cold.png")

        # Keep the current selected target fully inside the actual open area, but away from its centre.
        page.get_by_test_id("map").evaluate("m => { m.cy.zoom(0.32); }")
        fit = fit_occlusion(page, lit_only=False)
        if fit is None:
            failures.append(f"{name}: the already-visible Search setup has no open Map area")
            return
        open_box = fit["open"]
        page.get_by_test_id("map").evaluate(
            """(m,args) => { const [id,x1,x2,y1,y2]=args,n=m.cy.getElementById(id);
              const b=n.renderedBoundingBox({includeLabels:false}), cx=(b.x1+b.x2)/2, cy=(b.y1+b.y2)/2;
              const max=(x2-x1-(b.x2-b.x1))/2-12, offset=Math.min(60,max);
              if(offset<24 || y2-y1-(b.y2-b.y1)<24) throw new Error('target cannot be placed visibly off-centre');
              m.cy.panBy({x:(x1+x2)/2+offset-cx,y:(y1+y2)/2-cy}); }""",
            [item, open_box["x1"], open_box["x2"], open_box["y1"], open_box["y2"]],
        )
        setup = page.get_by_test_id("map").evaluate(
            """(m,id) => { const n=m.cy.getElementById(id),b=n.renderedBoundingBox({includeLabels:false});
              return {zoom:m.cy.zoom(),box:{x1:b.x1,y1:b.y1,x2:b.x2,y2:b.y2}}; }""",
            item,
        )
        setup_fit = fit_occlusion(page, lit_only=False)
        setup_open = setup_fit["open"] if setup_fit is not None else None
        if setup_open is None:
            failures.append(f"{name}: lost the open Map area while setting the visible Search target")
            return
        setup_box = setup["box"]
        setup_error = {
            "x": (setup_box["x1"] + setup_box["x2"] - setup_open["x1"] - setup_open["x2"]) / 2,
            "y": (setup_box["y1"] + setup_box["y2"] - setup_open["y1"] - setup_open["y2"]) / 2,
        }
        if (
            abs(setup["zoom"] - 0.32) > 0.001
            or setup_box["x1"] < setup_open["x1"] + 8
            or setup_box["y1"] < setup_open["y1"] + 8
            or setup_box["x2"] > setup_open["x2"] - 8
            or setup_box["y2"] > setup_open["y2"] - 8
            or abs(setup_error["x"]) < 24
        ):
            failures.append(f"{name}: already-visible Search precondition failed: {setup}, {setup_open}, {setup_error}")
        page.screenshot(path=SHOTS / f"app-{name}-search-visible-before.png")
        search_item()
        opened_and_framed("already-visible Search")
        page.screenshot(path=SHOTS / f"app-{name}-search-visible-after.png")
    except Exception as error:
        failures.append(f"{name}: Search selection framing failed: {error}")
        page.screenshot(path=SHOTS / f"app-{name}-search-selection-failure.png")
    finally:
        if errors:
            failures.append(f"{name}: Search selection page errors: {errors}")
        page.close()


def header_marks(page, testid):
    """Where a panel's title and its Close button are once it has come to rest."""
    page.wait_for_function(
        "id => document.querySelector(`[data-testid=${id}]`)?.getAnimations().length === 0", arg=testid
    )
    return page.get_by_test_id(testid).evaluate(
        """p => {
            const h2 = p.querySelector("h2").getBoundingClientRect();
            const close = p.querySelector("button[aria-label=Close]").getBoundingClientRect();
            return { title_x: h2.left, title_y: h2.top, close_y: close.top, close_right: close.right };
        }"""
    )


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
    shown = header_marks(page, "inspector")
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
    # The New item panel's title and Close button sit where the item panel's do.
    page.locator("[data-action=add-top]").click()
    page.get_by_test_id("add-form").wait_for(timeout=5_000)
    added = header_marks(page, "add-form")
    if any(abs(added[key] - shown[key]) > 0.5 for key in shown):
        failures.append(f"{name}: New item's header {added} isn't the item panel's {shown}")
    page.get_by_test_id("add-form").get_by_role("button", name="Cancel").click()
    page.get_by_test_id("add-form").wait_for(state="detached", timeout=5_000)
    from_menu(page, lambda: page.get_by_test_id("shortcuts").click())
    # Off the switch, which as a field would take the keys for itself.
    page.evaluate("() => document.activeElement?.blur()")
    page.keyboard.press("g")
    page.keyboard.press("t")
    page.wait_for_timeout(200)
    if view.get_attribute("data-view") != "map":
        failures.append(f"{name}: with single-key shortcuts off, G then T still went to {view.get_attribute('data-view')}")
    from_menu(page, lambda: page.get_by_test_id("shortcuts").click())
    # ⌘Z takes back the edit just saved, once, however often it is pressed.
    page.keyboard.press("ControlOrMeta+k")
    page.keyboard.type(plan.items[id]["title"])
    page.keyboard.press("Enter")
    heading = page.locator("[data-testid=inspector] h2")
    heading.wait_for(timeout=5_000)
    page.keyboard.press("e")
    form.wait_for(timeout=5_000)
    was = title.input_value()
    title.fill(f"{was} for undo")
    form.locator("button[type=submit]").click()
    notice = show_notice(page)
    notice.get_by_role("button", name="Undo").wait_for(timeout=10_000)
    page.evaluate("() => document.activeElement?.blur()")
    writes = []

    def wrote_edit(request):
        if not request.url.endswith("/api/command") or not request.post_data:
            return
        body = json.loads(request.post_data)
        command = body.get("command", body) if isinstance(body, dict) else body
        if isinstance(command, dict) and isinstance(command.get("item"), dict) and "edit" in command["item"]:
            writes.append(request.post_data)

    page.on("request", wrote_edit)
    page.keyboard.press("ControlOrMeta+z")
    page.keyboard.press("ControlOrMeta+z")
    show_notice(page, f"Set {was} back").get_by_text(f"Set {was} back").wait_for(timeout=10_000)
    page.wait_for_timeout(300)
    if heading.inner_text() != was or len(writes) != 1:
        failures.append(f"{name}: ⌘Z twice left the title {heading.inner_text()!r}, in {len(writes)} writes")
    page.keyboard.press("Escape")
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
def cross_origin_journey(context, origin, env, name, failures):
    """A page on another port of the same host can neither write to the app nor frame it.

    A cross-origin JSON post needs a preflight the server never answers, so the browser refuses it before it is sent;
    the attacker's one other way to write is a form, which can't say it holds JSON. That the server also refuses a
    request from another origin is proven by its unit tests.
    """
    # Ids of this view's own, so a write that got through is found.
    tag = name.encode().hex()[:10].ljust(10, "0")
    form, fetched = f"01900000-0000-7000-8000-{tag}0f", f"01900000-0000-7000-8000-{tag}1f"
    # A text/plain form's body is its field's name, '=' and its value: together, a command that would add an item.
    body = json.dumps({"item": {"add": {"id": form, "title": "Cross-port", "notes": "="}}})
    field, value = body.split("=", 1)
    write = json.dumps({"item": {"add": {"id": fetched, "title": "Cross-port"}}})
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

    # Another port of the same host, served from loopback as a local server's page would be. A page from nowhere, such as one a route fulfils, isn't local, and
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
    if refused != (415, "not-json"):
        failures.append(f"{name}: another port's form write was answered {refused}, not 415 not-json")
    # The page's load waits for its frame's; a refused frame holds an engine's error page, which has no #root.
    other.goto(attacker + "/frame", timeout=5_000)
    frames = other.frames
    framed = len(frames) != 2 or frames[1].evaluate("!!document.getElementById('root')")
    other.close()
    server.shutdown()
    server.server_close()
    listed = {row["id"] for row in json.loads(run(env, "item", "list"))["data"]["items"]}
    if landed := listed & {form, fetched}:
        failures.append(f"{name}: another port's writes reached the tracker: {sorted(landed)}")
    if framed:
        failures.append(f"{name}: a page on another port framed the app, or no frame was drawn")
    print(f"{name}: another port could neither write to the app nor frame it")


def first_view_readable(page, name, failures):
    """The Map opens at 1:1 or closer, with an item, not a box, wholly in view, title and all."""
    settled_map(page)
    view = page.get_by_test_id("map").evaluate(
        "m => { const w = m.clientWidth, h = m.clientHeight; return { zoom: m.cy.zoom(),"
        " shown: m.cy.nodes().not('.box, .junction').some(n => { const b = n.renderedBoundingBox();"
        " return b.x1 >= 0 && b.y1 >= 0 && b.x2 <= w && b.y2 <= h; }) }; }"
    )
    if view["zoom"] < FIRST_ZOOM or not view["shown"]:
        failures.append(
            f"{name}: the Map opened unreadably small or showing nothing whole: {view}"
        )
    if fit_control := fit_occlusion(page, lit_only=False):
        if (
            not fit_control["safe"]
            or fit_control["item_overlaps"]
            or fit_control["chevron_overlaps"]
            or fit_control["title_overlaps"]
            or fit_control["title_fragments"]
        ):
            failures.append(f"{name}: the Fit control covers first-view content: {fit_control}")
    else:
        failures.append(f"{name}: the Map's first view has no Fit control to keep clear")
    # One layout for a first load: a second, at a shape guessed before the Map was measured, would re-pack the plan
    # as it is first seen.
    asked = page.get_by_test_id("map").get_attribute("data-layouts")
    if asked != "1":
        failures.append(f"{name}: the first load asked for {asked} layouts, not one")


def fit_occlusion(page, lit_only):
    """Checks that automatic Map views keep their first item and ancestor chevrons clear of Fit."""
    return page.get_by_test_id("map").evaluate(
        """(m, litOnly) => {
            const map = m.getBoundingClientRect();
            const button = document.querySelector('.map-fit');
            if (!button) return null;
            const rect = button.getBoundingClientRect();
            const fit = { x1: rect.left - map.left, y1: rect.top - map.top,
                x2: rect.right - map.left, y2: rect.bottom - map.top };
            const zoom = m.cy.zoom();
            const hit = parseFloat(getComputedStyle(m).getPropertyValue('--hit')) || 44;
            const safeTop = 12 + hit + 8;
            const overlaps = (a, b) => a.x1 < b.x2 && a.x2 > b.x1 && a.y1 < b.y2 && a.y2 > b.y1;
            const panel = m.parentElement.querySelector('.inspector, .sheet');
            const panelRect = panel?.getBoundingClientRect();
            const panelShift = panel ? new DOMMatrixReadOnly(getComputedStyle(panel).transform).m42 : 0;
            const sheet = panelRect && panelRect.width >= map.width * .9;
            const open = { x1: 0, y1: Math.min(map.height, fit.y2 + 8), x2: panelRect && !sheet
                ? panelRect.left - map.left : map.width, y2: panelRect && sheet
                ? panelRect.top - panelShift - map.top : map.height };
            const items = m.cy.nodes(litOnly ? '.lit' : '.leaf, .collapsed').map(n => {
                const b = n.renderedBoundingBox({ includeLabels: true });
                return { id: n.id(), b: { x1: b.x1, y1: b.y1, x2: b.x2, y2: b.y2 } };
            });
            const itemOverlaps = items.filter(item => overlaps(item.b, fit)).map(item => item.id);
            const chevronOverlaps = [];
            const titleOverlaps = [];
            const titleFragments = [];
            const ctx = document.createElement('canvas').getContext('2d');
            for (const node of m.cy.nodes('.box')) {
                const b = node.renderedBoundingBox({ includeLabels: false });
                const holdsItem = items.some(item => item.b.x1 >= b.x1 - 1 && item.b.x2 <= b.x2 + 1
                    && item.b.y1 >= b.y1 - 1 && item.b.y2 <= b.y2 + 1);
                if (!holdsItem) continue;
                const chevron = { x1: b.x1 + 14 * zoom, x2: b.x1 + 22 * zoom,
                    y1: b.y1 + 24 * zoom, y2: b.y1 + 28 * zoom };
                if (overlaps(chevron, fit)) chevronOverlaps.push(node.id());
                if (parseFloat(node.style('text-opacity') || '1') <= 0) continue;
                const label = String(node.data('label') || '').split('\\n')[0];
                ctx.font = `${node.style('font-weight')} ${node.style('font-size')} ${node.style('font-family')}`;
                const x1 = b.x1 + (parseFloat(node.style('text-margin-x')) || 0) * zoom;
                const width = ctx.measureText(label).width * zoom;
                const y1 = b.y1 + (parseFloat(node.style('text-margin-y')) || 0) * zoom;
                const height = (parseFloat(node.style('font-size')) || 13) * 1.25 * zoom;
                const title = { x1, x2: x1 + width, y1, y2: y1 + height };
                if (overlaps(title, fit)) titleOverlaps.push(node.id());
                if (title.x1 < open.x1 && title.x2 > open.x1 && title.y1 < open.y2 && title.y2 > open.y1)
                    titleFragments.push({ id: node.id(), label, title });
            }
            return { fit, safeTop, open, safe: Math.abs(fit.y1 - 12) < 0.5 && fit.y2 + 8 <= safeTop + 0.5,
                item_overlaps: itemOverlaps, chevron_overlaps: chevronOverlaps,
                title_overlaps: titleOverlaps, title_fragments: titleFragments };
        }""",
        lit_only,
    )


def stages_drawn(page, plan, name, failures):
    """Each tile drawn carries the stage the tracker gives it, as a class its tint and marks follow."""
    drawn_stages = page.get_by_test_id("map").evaluate(
        "m => Object.fromEntries(m.cy.nodes('.leaf').map(n => [n.id(),"
        " (n.classes().find(c => c.startsWith('stage-')) ?? '').slice(6) || null]))"
    )
    # A parent drawn as a tile with no leaves to count has no stage of its own: its state says what it can.
    told = {
        id: plan.items[id].get("stage")
        or (state if (state := plan.items[id]["state"]) in ("done", "parked") else None)
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
    origin = "{0.scheme}://{0.netloc}".format(urlsplit(page.url))
    fresh = page.context.new_page()
    try:
        fresh.goto(f"{origin}/?view=map")
        wait_drawn(fresh, expected)
        first_view_readable(fresh, name, failures)
    finally:
        fresh.close()
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
    # Expand a box inside a root, then collapse it again: the count follows, and the root holds still on screen, as
    # DESIGN.md has it, though the box may move inside it. Only a box the layout moves shows the screen holds at the
    # root, not at the box, so boxes are tried, biggest first, until one has moved. The pan that holds the root still
    # knows nothing of the screen's size or of touch, so the phone, which finds the box first, only taps; the desktop,
    # starting from the box it found, measures that its root held still.
    still = not view.get("has_touch")
    corner = "(m, id) => { const b = m.cy.getElementById(id).boundingBox({ includeLabels: false }); return [b.x1, b.y1]; }"
    moved, box, checked = 0, None, False
    for box in sorted(
        (id for id in plan.default() if plan.depth(id) == 1 and plan.children(id)),
        key=lambda id: (id != hint, -len(plan.children(id)), id),
    ):
        centre(page, box)
        root = plan.chain(box)[-1]
        placed = page.get_by_test_id("map").evaluate(corner, box)
        # Where it settles after expanding is where it starts collapsing.
        before = settled_corner(page, root) if still else None
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
            after = settled_corner(page, root)
            shift = max(abs(after["x"] - before["x"]), abs(after["y"] - before["y"]))
            moved = max(moved, shift)
            if shift > 2:
                failures.append(
                    f"{name}: {action} {box} moved its root {root} by {shift:.0f} px on screen"
                )
            before = after
        if relaid:
            break
    else:
        failures.append(f"{name}: no box moved in the layout when expanded")
        box = None
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
    framed = fit_occlusion(page, lit_only=True)
    if framed is None:
        failures.append(f"{name}: the framed thread has no Fit control to keep clear")
    elif (
        not framed["safe"]
        or framed["item_overlaps"]
        or framed["chevron_overlaps"]
        or framed["title_overlaps"]
        or framed["title_fragments"]
    ):
        failures.append(f"{name}: the thread frame overlaps Fit or leaves a clipped ancestor title: {framed}")
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
    fit_button = page.get_by_role("button", name="Fit the whole plan")
    if fit_button.count() != 1 or not fit_button.is_visible():
        failures.append(f"{name}: the Map has no available Fit control beside the thread panel")
    else:
        before = page.get_by_test_id("map").evaluate(
            "m => ({ selected: m.cy.nodes('.selected').map(n => n.id()).sort(),"
            " collapsed: m.cy.nodes('.collapsed').map(n => n.id()).sort(), lit: m.dataset.lit })"
        )
        fit_button.click()
        fitted = page.get_by_test_id("map").evaluate(
            "m => { const r = m.getBoundingClientRect(); const panel = m.parentElement.querySelector('.inspector, .sheet');"
            " const b = m.cy.elements().renderedBoundingBox({ includeLabels: true });"
            " const fit = document.querySelector('.map-fit').getBoundingClientRect();"
            " const box = panel ? panel.getBoundingClientRect() : null;"
            " const sheet = box && box.width >= r.width * .9;"
            " const x2 = box && !sheet ? box.left - r.left : r.width;"
            " const shift = panel ? new DOMMatrixReadOnly(getComputedStyle(panel).transform).m42 : 0;"
            " const y1 = fit.bottom - r.top + 8;"
            " const y2 = box && sheet ? box.top - shift - r.top : r.height;"
            " return { inside: b.x1 >= 22 && b.y1 >= y1 + 22 && b.x2 <= x2 - 22 && b.y2 <= y2 - 22,"
            " selected: m.cy.nodes('.selected').map(n => n.id()).sort(),"
            " collapsed: m.cy.nodes('.collapsed').map(n => n.id()).sort(), lit: m.dataset.lit }; }"
        )
        if not fitted["inside"]:
            failures.append(f"{name}: Fit left part of the whole plan outside the uncovered Map: {fitted}")
        if any(before[key] != fitted[key] for key in ("selected", "collapsed", "lit")):
            failures.append(f"{name}: Fit changed selection, folds or the picked thread: {before} -> {fitted}")
        page.screenshot(path=SHOTS / f"app-{name}-fit.png")
    if view.get("has_touch"):
        # Which layer a phone's Back closes, the outline's sheet over the thread's panel and then the panel, is
        # nav.test.ts's; that Back moves through the history, the deep link journey's.
        sheet = page.get_by_test_id("projects")
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
    # The switcher: a row to each initiative, the first in context until one is chosen; choosing another moves the mark
    # and the address, a phone's bar names it with the sheet closed, and a reload keeps it. The first is chosen again
    # after, so what New item defaults to is as the journeys after expect.
    phone = view.get("has_touch")
    sheet = page.get_by_test_id("projects")
    if phone:
        page.get_by_test_id("projects-button").click()
        sheet.wait_for(timeout=5_000)
    choices = page.locator("button.initiative")
    first, chosen = choices.first, choices.nth(1)
    if choices.count() < 2 or first.get_attribute("aria-current") != "true":
        failures.append(f"{name}: the switcher didn't list the initiatives with the first in context")
    else:
        initiative = chosen.get_attribute("data-initiative")
        title = chosen.locator(".initiative-name").inner_text()
        was = first.get_attribute("data-initiative")
        marked_now = lambda id: f"button.initiative[data-initiative='{id}'][aria-current=true]"
        chosen.click()
        try:
            page.locator(marked_now(initiative)).wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: choosing {title} didn't mark it in the switcher")
        if f"initiative={initiative}" not in page.url:
            failures.append(f"{name}: choosing {title} left the address {page.url}")
        # The Map draws the initiative in context alone: the other's epic and its tasks, none of the first's, and it
        # settles on them. A tracker with nothing in the other leaves the Map empty.
        try:
            wait_drawn(page, len(plan.elsewhere))
            settled_map(page)
        except PlaywrightTimeout:
            drawn = page.get_by_test_id("map").evaluate("m => ({ ...m.dataset })")
            failures.append(f"{name}: choosing {title} left the Map {drawn}, not settled on its {len(plan.elsewhere)} items")
        if phone:
            page.get_by_test_id("projects-button").click()
            sheet.wait_for(state="detached", timeout=5_000)
        # A wait that leaves the initiative is a stub on its tile; tapped, it follows the item, putting the item's
        # initiative in context, and the second is chosen again after.
        waiter = next((id for id, row in plan.elsewhere.items() if row["links"]["waits_for"]), None)
        if waiter is None:
            failures.append(f"{name}: no task in {title} waits for an item of the first initiative, so the stub went unchecked")
        else:
            wanted = plan.elsewhere[waiter]["links"]["waits_for"][0]
            try:
                drawn = page.get_by_test_id("map").evaluate("(m, id) => m.cy.getElementById(id).length", f"stub {waiter}")
                if drawn != 1:
                    failures.append(f"{name}: {waiter}'s wait on {wanted}, in another initiative, drew {drawn} stubs, not 1")
                else:
                    tap_middle(page, view, f"stub {waiter}")
                    page.wait_for_url(re.compile(f"item={wanted}.*initiative={was}|initiative={was}.*item={wanted}"), timeout=10_000)
                    page.get_by_test_id("inspector").wait_for(timeout=10_000)
                    page.keyboard.press("Escape")
                    page.get_by_test_id("inspector").wait_for(state="detached", timeout=5_000)
                    if phone:
                        page.get_by_test_id("projects-button").click()
                        sheet.wait_for(timeout=5_000)
                    chosen.click()
                    page.locator(marked_now(initiative)).wait_for(timeout=5_000)
                    if phone:
                        page.get_by_test_id("projects-button").click()
                        sheet.wait_for(state="detached", timeout=5_000)
                    wait_drawn(page, len(plan.elsewhere))
                    settled_map(page)
            except PlaywrightTimeout:
                failures.append(f"{name}: tapping {waiter}'s stub didn't open {wanted} in its own initiative, at {page.url}")
        page.reload()
        if phone:
            page.get_by_test_id("in-context").wait_for(timeout=10_000)
            if page.get_by_test_id("in-context").inner_text() != title:
                failures.append(f"{name}: the phone's bar didn't name {title} after a reload")
            page.get_by_test_id("projects-button").click()
            sheet.wait_for(timeout=5_000)
        try:
            page.locator(marked_now(initiative)).wait_for(timeout=10_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: a reload didn't keep {title} in context")
        page.locator(f"button.initiative[data-initiative='{was}']").click()
        if phone:
            page.get_by_test_id("projects-button").click()
            sheet.wait_for(state="detached", timeout=5_000)
        wait_drawn(page, len(plan.default()))
        # An item opened from another initiative puts its own in context: an address naming one of the first's items
        # under the second's lands on the first, and the Map draws it and the item selected.
        leaf = next(id for id in plan.default() if not plan.children(id))
        page.goto(f"{page.url.split('?')[0]}?item={leaf}&initiative={initiative}")
        try:
            page.wait_for_url(re.compile(f"initiative={was}"), timeout=10_000)
            wait_drawn(page, len(plan.default()))
            page.get_by_test_id("inspector").wait_for(timeout=10_000)
            page.keyboard.press("Escape")
            page.get_by_test_id("inspector").wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: opening {leaf} under {title} left the address {page.url}, not its own initiative")
    # Only a leaf is in a thread, and the Map draws one once the box above it is open.
    picked = next(
        id
        for id in sorted(plan.items)
        if plan.depth(id) == 2 and not plan.children(id) and plan.items[id]["thread"]
    )
    box = plan.parent[picked]
    project, thread = plan.chain(picked)[-1], plan.items[picked]["thread"]
    centre(page, box)
    tap_box(page, view, box)
    page.wait_for_function(
        "id => document.querySelector('[data-testid=map]').cy.getElementById(id).length === 1",
        arg=picked,
        timeout=10_000,
    )
    tap_middle(page, view, picked)
    inspector = page.get_by_test_id("inspector")
    inspector.wait_for()
    if phone:
        # The item's sheet opens at a height that ends on a row, its Thread row whole or not at all, not cut through
        # its text under the tab bar. Measured once it has come to rest.
        page.wait_for_function(
            "() => { const p = document.querySelector('[data-testid=inspector]'); return p.getAnimations().length === 0; }"
        )
        rows = inspector.evaluate(
            """p => {
              const bottom = Math.min(innerHeight, p.getBoundingClientRect().bottom,
                document.querySelector('.tabbar')?.getBoundingClientRect().top ?? innerHeight);
              return [...p.querySelectorAll('dl.facts dt, dl.facts dd')].map((e) => {
                const r = e.getBoundingClientRect();
                return { text: e.innerText.slice(0, 24), top: r.top, bottom: r.bottom, cut: r.top < bottom && r.bottom > bottom + 0.5 };
              }).concat([{ edge: bottom }]);
            }"""
        )
        cut = [row["text"] for row in rows if row.get("cut")]
        shown = [row for row in rows if row.get("text") == "Thread"]
        if cut or not shown:
            failures.append(f"{name}: {picked}'s first height cut through {cut}, Thread row drawn: {bool(shown)}")
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
        # A tap leaves no pressed look behind: the pointer stays where it came down on a touch screen, so a tint
        # shown on hover would stay on the handle and the chevron until the next tap.
        for what, control in (
            ("the sheet's handle", sheet.locator(".sheet-handle")),
            ("a project's chevron", sheet.locator("[data-toggle-project]").first),
        ):
            control.tap()
            tint = control.evaluate(
                "e => ({ hover: matchMedia('(hover: hover)').matches, bg: getComputedStyle(e).backgroundColor })"
            )
            if tint["hover"] or tint["bg"] != "rgba(0, 0, 0, 0)":
                failures.append(f"{name}: after a tap, {what} shows {tint['bg']} (hover capable: {tint['hover']})")
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
    open_full(page, "inspector")
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
        # Hovering a project's row fills all of it, its chevron as much as its title. A row that isn't the marked one,
        # which is filled already.
        row = page.locator(".project:not([aria-current=true]) > .project-row").first
        row.wait_for(timeout=5_000)
        if not page.evaluate("matchMedia('(hover: hover)').matches"):
            failures.append(f"{name}: the browser has no hover, so the outline's hover wasn't checked")
        page.mouse.move(0, 0)
        rest = row.evaluate("r => getComputedStyle(r).backgroundColor")
        for part in ("chevron", "title"):
            row.locator(f"button.{'chevron' if part == 'chevron' else 'project-title'}").hover()
            lit = row.evaluate("r => getComputedStyle(r).backgroundColor")
            if lit == rest:
                failures.append(f"{name}: hovering a project's {part} left its row {lit}, as it is at rest")
        page.mouse.move(0, 0)
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


def waves_in_context(plan, listed, anytime=True):
    """The wave each thread is in as the Threads view of the initiative in context numbers it: the threads with a task
    in it, in the waves the tracker computed across every thread, a wave left with none dropped so the numbers have no
    gaps. A thread of one stands for its loose task."""
    members = {row["thread"] for row in plan.items.values() if row["thread"]}
    members |= {row["id"] for row in listed["threads"] if row.get("task") in plan.items}
    kept = [[id for id in ids if id in members] for ids in listed["waves"]]
    want = {id: str(at + 1) for at, ids in enumerate(ids for ids in kept if ids) for id in ids}
    if anytime:
        want |= {id: "anytime" for id in listed["anytime"] if id in members}
    return want


@timed
def threads_journey(page, view, env, plan, name, failures):
    """The Threads view names every wait, and shows related arrows on hover when wide; picking a tile opens its panel."""
    if page.locator("[data-testid=thread-panel], [data-testid=inspector]").count():
        page.keyboard.press("Escape")
        page.wait_for_function(
            "() => !document.querySelector('[data-testid=thread-panel], [data-testid=inspector]')",
            timeout=5_000,
        )
    to_map(page)
    listed = json.loads(run(env, "thread", "list"))["data"]
    want = waves_in_context(plan, listed)
    # The other initiatives' threads are left out, and the proof is not vacuous: the tracker has some.
    if len(want) == len(listed["threads"]) and plan.elsewhere:
        failures.append(f"{name}: the tracker has no thread outside the initiative in context to leave out")
    page.locator("[data-view=threads]").click()
    page.get_by_test_id("threads-view").wait_for()
    got = page.locator(".wave [data-thread-tile]").evaluate_all(
        "tiles => tiles.map(tile => [tile.dataset.threadTile, tile.closest('.wave').dataset.wave])"
    )
    if dict(got) != want or len(got) != len(want):
        failures.append(f"{name}: the Threads view's waves {got} aren't the tracker's {want}")
    if page.get_by_role("button", name="Fit the whole plan").count():
        failures.append(f"{name}: the Map's Fit control appeared over the Threads view")
    waits = sum(len(row["waits_for"]) for row in listed["threads"] if row["id"] in want)
    arrows = 0 if view.get("has_touch") else waits
    if page.locator(".arrows g").count():
        failures.append(f"{name}: the Threads view showed connectors before a tile was hovered")
    page.screenshot(path=SHOTS / f"app-{name}-threads.png")
    if arrows:
        verify_thread_arrows(page, listed, want, name, failures)
    elif page.locator(".arrows g").count():
        failures.append(f"{name}: the phone drew connectors without hover support")
    # A thread with work to start, whose panel offers it. Its tile goes to the Map with the panel open, as a thread's
    # row anywhere does.
    ready = [row["id"] for row in json.loads(run(env, "thread", "ready"))["data"]["ready"] if row["id"] in want]
    picked = ready[0] if ready else next(iter(want))
    tile = page.locator(f"[data-thread-tile='{picked}']")
    panel = page.locator("[data-testid=thread-panel]:not([aria-busy])")
    tile.click()
    try:
        page.get_by_test_id("threads-view").wait_for(state="detached", timeout=5_000)
        panel.wait_for(timeout=10_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: picking {picked}'s tile didn't open its panel on the Map")
    # An item chosen from the panel closes back to the thread, whose own close goes back to the Threads view.
    open_full(page, "thread-panel")
    row = page.locator("[data-testid=thread-panel] [data-select]").first
    if row.count() == 0:
        failures.append(f"{name}: {picked}'s panel offers no item to start, so closing back from one went unchecked")
    else:
        chosen = row.get_attribute("data-select")
        row.click()
        page.get_by_test_id("inspector").wait_for(timeout=5_000)
        page.keyboard.press("Escape")
        try:
            panel.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: closing {chosen} didn't go back to {picked}'s panel")
    page.keyboard.press("Escape")
    try:
        page.get_by_test_id("threads-view").wait_for(timeout=5_000)
    except PlaywrightTimeout:
        failures.append(f"{name}: closing {picked}'s panel didn't go back to the Threads view")
    # A tile's "after" names each thread it waits on in words, not by its dot alone.
    named = page.locator(".thread-tile .after > span").evaluate_all(
        "spans => spans.map(s => [s.title, s.querySelector('.visually-hidden')?.textContent ?? ''])"
    )
    if not named:
        failures.append(f"{name}: no tile said what it comes after, so naming those threads went unchecked")
    elif any(not title or title != text for title, text in named):
        failures.append(f"{name}: a tile's after dots weren't each named: {named[:3]}")
    # On a phone, Back from another view goes home, as the system's back gesture would leave the app otherwise; a
    # thread's panel kept over the view, picked on the Map, closes first.
    if view.get("has_touch"):
        tile.click()
        panel.wait_for(timeout=10_000)
        page.locator("[data-view=threads]").click()
        page.get_by_test_id("threads-view").wait_for()
        if tile.get_attribute("aria-current") != "true":
            failures.append(f"{name}: {picked}'s tile wasn't marked with its thread picked on the Map")
        page.go_back()
        try:
            panel.wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back didn't close {picked}'s panel over the Threads view")
        if not page.get_by_test_id("threads-view").is_visible():
            failures.append(f"{name}: Back from a thread's panel over the Threads view left the view too")
        page.go_back()
        try:
            page.get_by_test_id("threads-view").wait_for(state="detached", timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: Back from the Threads view didn't go home to the Overview")
    to_map(page)
    print(f"{name}: the Threads view put {len(want)} threads in their waves, with {arrows} arrows")


def verify_thread_arrows(page, listed, want, name, failures):
    """Shows every expected wait by hovering its endpoints, checking all routes and their union."""
    expected = {
        (dependency, row["id"])
        for row in listed["threads"]
        if row["id"] in want
        for dependency in row["waits_for"]
        if dependency in want
    }
    seen = set()
    checked_geometry = set()
    skips = set()
    lit_screenshot = False
    for thread in sorted(want):
        incident = {pair for pair in expected if thread in pair}
        if not incident:
            continue
        page.locator(f"[data-thread-tile='{thread}']").hover()
        page.wait_for_function(
            "count => document.querySelectorAll('.arrows g').length === count",
            arg=len(incident),
            timeout=5_000,
        )
        drawn = page.locator(".arrows g").evaluate_all("rows => rows.map(g => [g.dataset.from, g.dataset.to])")
        if set(map(tuple, drawn)) != incident:
            failures.append(f"{name}: hovering {thread} drew {drawn}, expected its waits {sorted(incident)}")
        skips.update(arrows_clear(page, want, name, failures, checked_geometry))
        seen.update(map(tuple, drawn))
        if skips and not lit_screenshot:
            page.screenshot(path=SHOTS / f"app-{name}-threads-lit.png")
            lit_screenshot = True
        page.mouse.move(0, 0)
        page.wait_for_function("() => document.querySelectorAll('.arrows g').length === 0", timeout=5_000)
    if seen != expected:
        failures.append(f"{name}: hovering endpoints exposed {sorted(seen)}, expected every wait {sorted(expected)}")
    if expected and not skips:
        failures.append(f"{name}: no selected connector skips a wave, so none was checked across it")


def arrows_clear(page, want, name, failures, checked_geometry):
    """Checks each distinct arrow path against tiles at 4 px intervals and verifies the view scrolls to all of it."""
    found = page.evaluate(
        """(checked) => {
            const view = document.querySelector('.waves');
            const origin = view.getBoundingClientRect();
            const tiles = [...document.querySelectorAll('.wave [data-thread-tile]')].map(tile => [tile.dataset.threadTile,
                tile.getBoundingClientRect()]);
            const seen = new Set(checked);
            const added = [];
            const addedPairs = [];
            const crossed = [];
            let outside = 0;
            for (const arrow of document.querySelectorAll('.arrows g')) {
                const line = arrow.querySelector('path:not(.head)');
                const matrix = line.getScreenCTM();
                const head = arrow.querySelector('path.head');
                const length = line.getTotalLength();
                const signature = JSON.stringify([
                    arrow.dataset.from, arrow.dataset.to, line.getAttribute('d'), head?.getAttribute('d'),
                    [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f],
                    [origin.left, origin.top, origin.width, origin.height, view.clientWidth, view.clientHeight,
                        view.scrollLeft, view.scrollTop, view.scrollWidth, view.scrollHeight]
                ]);
                if (seen.has(signature)) continue;
                seen.add(signature);
                added.push(signature);
                addedPairs.push([arrow.dataset.from, arrow.dataset.to]);
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
            return { crossed: [...new Set(crossed)], outside, added, addedPairs };
        }""",
        sorted(checked_geometry),
    )
    checked_geometry.update(found["added"])
    if found["crossed"]:
        failures.append(f"{name}: arrows cross tiles: {found['crossed'][:5]}")
    if found["outside"]:
        failures.append(f"{name}: {found['outside']} points of the arrows lie where the view doesn't scroll")
    skips = [
        (a, b)
        for a, b in found["addedPairs"]
        if want.get(a, "").isdigit() and want.get(b, "").isdigit() and int(want[b]) - int(want[a]) > 1
    ]
    return set(skips)


def item(env, id):
    """The item as the tracker shows it, or None while there is no such item."""
    out = subprocess.run(
        [ACYCLIC, "item", "show", id], env=env, capture_output=True, text=True, check=False
    )
    return json.loads(out.stdout)["data"][0] if out.returncode == 0 else None


def check_tracker(env, id, check, what, failures, name):
    """Checks that the tracker shows an edit the app has already shown: polling the CLI before then would contend with
    the server for the clone's lock."""
    if (row := item(env, id)) is None or not check(row):
        failures.append(f"{name}: the tracker doesn't show {what}")


def open_full(page, testid, full=True):
    """A phone's panel opens short, its title, status and main action in view: opens it full, as the user would to
    reach the rest, or with `full` false puts it back, so the Map is in view again. Wide, a panel is whole already."""
    if page.viewport_size["width"] >= 900:
        return
    panel = page.get_by_test_id(testid)
    if (panel.get_attribute("data-detent") == "full") != full:
        panel.locator(".sheet-handle").click()
        # Come to rest, not only switched: what is measured next is where it stands.
        page.wait_for_function(
            f"() => {{ const p = document.querySelector('[data-testid={testid}]');"
            f" return p?.dataset.detent === '{'full' if full else 'first'}' && p.getAnimations().length === 0; }}"
        )


def item_action(page, action):
    """Runs one of the item panel's actions from its menu, where all but Edit are."""
    page.get_by_test_id("item-menu").click()
    page.locator(f"[data-testid=inspector] [data-action={action}]").click()


def unlink(page, testid, held_alone=None):
    """Removes a panel's first link of a kind, its button shown as a pointer comes over its row. With `held_alone` a
    list, adds to it what was wrong while the write was out: only the pressed button may look held, no other control
    greyed or disabled (an observer sees the write's first draw, however soon it lands)."""
    open_full(page, "inspector")
    row = page.locator(f"[data-testid={testid}] .link").first
    row.hover()
    if held_alone is not None:
        page.evaluate(
            """() => {
              window.heldSeen = null;
              const panel = document.querySelector('[data-testid=inspector]');
              new MutationObserver((records, observer) => {
                const pressed = panel.querySelector('.unlink[aria-busy=true]');
                if (!pressed) return;
                observer.disconnect();
                const others = [...panel.querySelectorAll('.unlink, [data-action=edit]')].filter((b) => b !== pressed);
                window.heldSeen = {
                  others: others.length,
                  disabled: others.filter((b) => b.disabled).length,
                  dim: others.filter((b) => Number(getComputedStyle(b).opacity) < 0.9).length,
                  pressedDim: Number(getComputedStyle(pressed).opacity) < 0.9,
                };
              }).observe(panel, { attributes: true, subtree: true, attributeFilter: ['aria-busy'] });
            }"""
        )
    row.locator(".unlink").click()
    if held_alone is not None:
        try:
            page.wait_for_function("() => window.heldSeen !== null", timeout=10_000)
        except PlaywrightTimeout:
            held_alone.append(f"{testid}'s write held no button, the pressed one included")
            return
        seen = page.evaluate("() => window.heldSeen")
        # The Edit pencil is always one of the others, so none seen means the panel was not looked at.
        if seen["others"] < 1 or seen["disabled"] or seen["dim"] or not seen["pressedDim"]:
            held_alone.append(f"{testid}'s write held {seen}")
    open_full(page, "inspector", full=False)


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
    root = plan.chain(box)[-1]
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
    # Selecting frames the box about 100 ms after its inspector shows, which moves the camera under whatever the journey
    # centred meanwhile, so the tap below hit another leaf on a loaded machine. Taps wait for the camera to hold still.
    settled_corner(page, box)
    # Selecting shows the item's notes, as text. The leaf sits above the inspector, as a person would hold it, so the
    # Map need not pan to show what is added inside it.
    tap_middle(page, view, first, rise=0.2)
    notes = page.locator("[data-testid=inspector] [data-testid=notes]")
    notes.get_by_text(HOSTILE, exact=True).wait_for(timeout=10_000)
    if notes.locator("img").count() or page.evaluate("window.pwned === true"):
        failures.append(f"{name}: the inspector ran a note as markup")
    # A fingertip reaches a close button and a chip from 44 px apart, though the chip is drawn smaller.
    open_full(page, "inspector")
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
    open_full(page, "inspector", full=False)

    # Add inside the first leaf: the Map opens the leaf to show it, the leaf holds still, and the tracker has it.
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
        return None
    # The app made the new item's id.
    request = json.loads(waiting[0].request.post_data)
    command = request.get("command", request) if isinstance(request, dict) else request
    added = command["item"]["add"]["id"]
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
    # Once the tiles have stopped gliding, the new item is drawn at its final place, inside the box it was added to.
    if not page.get_by_test_id("map").evaluate(
        "(m, [child, parent]) => { const c = m.cy.getElementById(child).boundingBox();"
        " const p = m.cy.getElementById(parent).boundingBox();"
        " return c.x1 >= p.x1 && c.x2 <= p.x2 && c.y1 >= p.y1 && c.y2 <= p.y2; }",
        [added, first],
    ):
        failures.append(f"{name}: {added} isn't drawn inside {first} once the Map came to rest")
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
        # On an item clear of the open menu, below it, which the backdrop must stop the tap reaching: a tap on the menu
        # itself would only test the row it landed on.
        lambda: tap_middle(page, view, second, rise=-0.25),
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
    # The pick bar has the focus, so typing finds the target in the menu, starting with the key (M would otherwise
    # move), and Escape from the menu gives the focus back to the bar, the wait going on.
    bar_focused = "bar => bar === document.activeElement"
    if not page.get_by_test_id("prompt").evaluate(bar_focused):
        failures.append(f"{name}: the link's pick bar didn't take the focus")
    else:
        # The first key opens the find and the next goes into its field, so the two read as typed.
        page.keyboard.press("m")
        try:
            find = page.get_by_test_id("command-menu").get_by_role("combobox")
            find.wait_for(timeout=5_000)
            page.keyboard.press("o")
            if find.input_value() != "mo":
                failures.append(f"{name}: typing at the pick bar found {find.input_value()!r}, not 'mo'")
            page.keyboard.press("Escape")
            page.get_by_test_id("command-menu").wait_for(state="detached", timeout=5_000)
            if not page.get_by_test_id("prompt").evaluate(bar_focused):
                failures.append(f"{name}: leaving the find didn't give the pick bar the focus back")
        except PlaywrightTimeout:
            failures.append(f"{name}: typing at the pick bar didn't open and close the find")
    # A target refused, the box holding it, shows its whole reason however narrow the screen: the link waits on.
    tap_middle(page, view, second)
    refused = show_notice(page)
    try:
        refused.wait_for(timeout=5_000)
        whole = refused.locator("span").first.evaluate("s => s.scrollWidth <= s.clientWidth + 1")
        if not whole or not page.get_by_test_id("prompt").is_visible():
            failures.append(f"{name}: a refused target's reason was cut, or the link stopped waiting")
    except PlaywrightTimeout:
        failures.append(f"{name}: a refused target showed no notice")
    refused.get_by_role("button", name="Dismiss").click()
    page.get_by_test_id("notice").wait_for(state="detached", timeout=5_000)
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
    # Undoing the link addition is also a write: it restores the same root frame while removing the edge.
    frame = settled_corner(page, root)
    zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
    show_notice(page).get_by_role("button", name="Undo").click()
    page.wait_for_function(
        "edge => document.querySelector('[data-testid=map]').cy.getElementById(edge).empty()",
        arg=edge,
        timeout=10_000,
    )
    held(page, root, frame, "undoing a new wait", name, failures)
    after_undo_zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
    if abs(after_undo_zoom - zoom) > 0.001:
        failures.append(f"{name}: undoing a new wait changed the Map zoom from {zoom} to {after_undo_zoom}")
    check_tracker(
        env,
        added,
        lambda row: row["links"]["waits_for"] == [],
        f"{added} with the new wait undone",
        failures,
        name,
    )
    item_action(page, "waits-for")
    tap_middle(page, view, first)
    page.wait_for_function(
        "([edge, source]) => { const e = document.querySelector('[data-testid=map]').cy.getElementById(edge);"
        " return e.nonempty() && e.source().id() === source; }",
        arg=[edge, first],
        timeout=10_000,
    )
    # Unlinked from the inspector: gone from the Map and tracker, with Undo offered without moving the sheet or Map.
    show_notice(page).get_by_role("button", name="Dismiss").click()
    page.get_by_test_id("notice").wait_for(state="detached", timeout=5_000)
    frame = settled_corner(page, root)
    zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
    wrong = []
    unlink(page, "waits-for", wrong)
    failures.extend(f"{name}: {one}" for one in wrong)
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    notice = show_notice(page)
    wait_drawn(page, base + 1)
    held(page, root, frame, "unlinking a wait", name, failures)
    after_unlink_zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
    if abs(after_unlink_zoom - zoom) > 0.001:
        failures.append(f"{name}: unlinking a wait changed the Map zoom from {zoom} to {after_unlink_zoom}")
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
    notice = show_notice(page)
    frame = settled_corner(page, root)
    notice.get_by_role("button", name="Undo").click()
    wait_inspector(page, f"[data-testid=waits-for] [data-select='{first}']")
    wait_drawn(page, base + 1)
    held(page, root, frame, "undoing a wait", name, failures)
    after_undo_zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
    if abs(after_undo_zoom - zoom) > 0.001:
        failures.append(f"{name}: undoing a wait changed the Map zoom from {zoom} to {after_undo_zoom}")
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
    notice = show_notice(page)
    notice.get_by_role("button", name="Undo").wait_for(timeout=5_000)
    run(env, "item", "edit", added, "--waits-for", first)
    # Sent, the undo would add a link already there, which the tracker wouldn't show: the writes the app sends tell.
    sent = []
    listen = lambda request: (
        request.url.endswith("/api/command")
        and '"waits-for"' in (request.post_data or "")
        and sent.append(request.post_data)
    )
    page.on("request", listen)
    show_notice(page).get_by_role("button", name="Undo").click()
    show_notice(page, "Changed since, so it can't be undone").get_by_text("Changed since, so it can't be undone").wait_for(timeout=10_000)
    page.remove_listener("request", listen)
    if sent:
        failures.append(f"{name}: a refused undo sent {sent}")
    # Unlinked once more, as the journeys after expect.
    close_notice_details(page)
    unlink(page, "waits-for")
    wait_inspector(page, "[data-testid=waits-for]", state="detached")
    page.screenshot(path=SHOTS / f"app-{name}-edited.png")
    print(
        f"{name}: added, moved, linked and unlinked {added}, the pinned boxes holding still"
    )
    return added


@timed
def shrink_undo_journey(page, view, env, project, other, chain, name, failures):
    """Unlinks the last edge of a tall box: its size shrinks, the next root repacks, and Undo restores both."""
    plan = Plan(env)
    roots = {id for id in plan.items if plan.depth(id) == 0}

    def root_edges(plan):
        return sorted(
            {
                (plan.chain(id)[-1], plan.chain(target)[-1])
                for id, row in plan.items.items()
                for target in row["links"]["waits_for"]
                if target in plan.items and plan.chain(id)[-1] != plan.chain(target)[-1]
            }
        )

    edges = root_edges(plan)
    page.reload()
    to_map(page)
    count = len(plan.default())
    wait_drawn(page, count)
    centre(page, project)
    tap_middle(page, view, chain[-2])
    wait_inspector(page, f"[data-testid=waited-on-by] [data-select='{chain[-1]}']")
    # Establish a deliberate zoom after the panel opens. Keep the root and selected item inside the open Map area.
    page.get_by_test_id("map").evaluate(
        "(m, id) => { m.cy.zoom(0.55); const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " m.cy.panBy({ x: 24 - b.x1, y: 16 - b.y1 }); }",
        project,
    )
    if not in_open_view(page, project) or not in_open_view(page, chain[-2]):
        failures.append(f"{name}: the chosen-zoom frame does not show the root and selected item together")

    def geometry(ids):
        return page.get_by_test_id("map").evaluate(
            """(m, ids) => ({
                zoom: m.cy.zoom(), pan: m.cy.pan(),
                nodes: Object.fromEntries(ids.map(id => {
                    const n = m.cy.getElementById(id);
                    const b = n.boundingBox();
                    const r = n.renderedBoundingBox({ includeLabels: false });
                    return [id, { x: b.x1, y: b.y1, w: b.w, h: b.h, sx: r.x1, sy: r.y1 }];
                }))
            })""",
            ids,
        )

    before = geometry([project, other, chain[-2]])
    # A Notes-only write leaves layout inputs alone. Its save, Undo, and notice dismissal must leave both the selected
    # corner and chosen camera where they were.
    page.locator("[data-action=edit]").click()
    editor = page.get_by_test_id("item-form")
    editor.locator("textarea[name=notes]").fill("Frame proof notes")
    editor.get_by_role("button", name="Save").click()
    notice = show_notice(page)
    notice.get_by_role("button", name="Undo").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    wait_drawn(page, count)
    written_notes = geometry([project, other, chain[-2]])
    if written_notes["zoom"] != before["zoom"] or any(
        abs(written_notes["pan"][axis] - before["pan"][axis]) > 0.5 for axis in ("x", "y")
    ) or any(
        abs(written_notes["nodes"][id][axis] - before["nodes"][id][axis]) > 1
        for id in (project, chain[-2])
        for axis in ("sx", "sy")
    ):
        failures.append(f"{name}: a Notes-only save changed the selected frame: {before} -> {written_notes}")
    show_notice(page).get_by_role("button", name="Undo").click()
    # An Undo reads the plan before it writes, and is not busy while it reads, so busy alone is not the end of it: its
    # result is the notice that says so (news), where "Undoing…" and an offer are the notices that last until the next
    # write. Dismissing before then dismisses "Undoing…", and the write that follows raises its result again.
    page.locator("[data-testid=notice][data-lasts=news]").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    wait_drawn(page, count)
    undone_notes = geometry([project, other, chain[-2]])
    if undone_notes["zoom"] != before["zoom"] or any(
        abs(undone_notes["pan"][axis] - before["pan"][axis]) > 0.5 for axis in ("x", "y")
    ) or any(
        abs(undone_notes["nodes"][id][axis] - before["nodes"][id][axis]) > 1
        for id in (project, chain[-2])
        for axis in ("sx", "sy")
    ):
        failures.append(f"{name}: Undoing Notes changed the selected frame: {before} -> {undone_notes}")
    show_notice(page).get_by_role("button", name="Dismiss").click()
    page.get_by_test_id("notice").wait_for(state="detached", timeout=5_000)
    dismissed_notes = geometry([project, other, chain[-2]])
    if dismissed_notes["zoom"] != before["zoom"] or any(
        abs(dismissed_notes["pan"][axis] - before["pan"][axis]) > 0.5 for axis in ("x", "y")
    ) or any(
        abs(dismissed_notes["nodes"][id][axis] - before["nodes"][id][axis]) > 1
        for id in (project, chain[-2])
        for axis in ("sx", "sy")
    ):
        failures.append(f"{name}: dismissing the Notes notice changed the selected frame: {before} -> {dismissed_notes}")
    layouts_before = int(page.get_by_test_id("map").get_attribute("data-layouts") or 0)
    unlink(page, "waited-on-by")
    wait_inspector(page, "[data-testid=waited-on-by]", state="detached")
    show_notice(page)
    page.get_by_role("button", name="Undo").wait_for(timeout=10_000)
    page.wait_for_function(
        "([n, layouts, project, other, oldHeight, oldY]) => {"
        " const m = document.querySelector('[data-testid=map]');"
        " const p = m?.cy.getElementById(project).boundingBox();"
        " const o = m?.cy.getElementById(other).boundingBox();"
        " return m?.dataset.state === 'drawn' && Number(m.dataset.nodes) === n"
        " && !m.cy.elements().some((e) => e.animated())"
        " && Number(m.dataset.layouts) > layouts"
        " && p && p.h < oldHeight - 20 && o && o.y1 < oldY - 2; }",
        arg=[count, layouts_before, project, other, before["nodes"][project]["h"], before["nodes"][other]["y"]],
        timeout=20_000,
    )
    settled_map(page)
    after = geometry([project, other, chain[-2]])
    updated = Plan(env)

    if roots != {id for id in updated.items if updated.depth(id) == 0}:
        failures.append(f"{name}: the unlink changed the top-level item set")
    if edges != root_edges(updated):
        failures.append(f"{name}: the unlink changed the top-level wait edges")
    old, new = before["nodes"][project], after["nodes"][project]
    if new["w"] > old["w"] + 2 or new["h"] >= old["h"] - 20:
        failures.append(f"{name}: the unlink did not shrink only the tall box: {old} -> {new}")
    if after["nodes"][other]["y"] >= before["nodes"][other]["y"] - 2:
        failures.append(f"{name}: repacking did not move the other root up: {before['nodes'][other]} -> {after['nodes'][other]}")
    if abs(after["zoom"] - before["zoom"]) > 0.001 or any(
        abs(after["pan"][axis] - before["pan"][axis]) > 0.5 for axis in ("x", "y")
    ):
        failures.append(f"{name}: the shrinking write changed the camera: {before['zoom']}/{before['pan']} -> {after['zoom']}/{after['pan']}")
    if abs(after["nodes"][chain[-2]]["sx"] - before["nodes"][chain[-2]]["sx"]) > 2 or abs(
        after["nodes"][chain[-2]]["sy"] - before["nodes"][chain[-2]]["sy"]
    ) > 2:
        failures.append(f"{name}: the selected item moved on screen during the shrink")

    frame = settled_corner(page, project)
    show_notice(page)
    page.get_by_role("button", name="Undo").click()
    wait_inspector(page, f"[data-testid=waited-on-by] [data-select='{chain[-1]}']")
    wait_drawn(page, count)
    undone = geometry([project, other, chain[-2]])
    same_node = lambda left, right: all(
        abs(left[field] - right[field]) <= 1 for field in ("x", "y", "w", "h")
    )
    if not same_node(undone["nodes"][project], before["nodes"][project]) or not same_node(
        undone["nodes"][other], before["nodes"][other]
    ):
        failures.append(f"{name}: Undo did not restore the original root packing")
    held(page, project, frame, "Undoing the shrink", name, failures)
    if undone["zoom"] != before["zoom"]:
        failures.append(f"{name}: Undo changed the Map zoom from {before['zoom']} to {undone['zoom']}")

    # A chevron fold after Undo still holds the restored packing; reopening returns to the same root positions.
    centre(page, project)
    page.get_by_test_id("map").evaluate(
        "(m, id) => { const b = m.cy.getElementById(id).renderedBoundingBox({ includeLabels: false });"
        " m.cy.panBy({ x: 80 - b.x1, y: 80 - b.y1 }); }",
        project,
    )
    frame = settled_corner(page, project)
    tap_box(page, view, project)
    wait_drawn(page, count - len(plan.children(project)))
    folded = geometry([project, other])
    held(page, project, frame, "folding after Undo", name, failures)
    frame = settled_corner(page, project)
    tap_box(page, view, project)
    wait_drawn(page, count)
    reopened = geometry([project, other])
    held(page, project, frame, "reopening after Undo", name, failures)
    # "Slide into the gap" (fac642): the fold gives up room, and the other root may close into it, only up or left and
    # by no more than the box gave up; reopening returns it exactly.
    gave_w = before["nodes"][project]["w"] - folded["nodes"][project]["w"]
    gave_h = before["nodes"][project]["h"] - folded["nodes"][project]["h"]
    slid_x = before["nodes"][other]["x"] - folded["nodes"][other]["x"]
    slid_y = before["nodes"][other]["y"] - folded["nodes"][other]["y"]
    if not (-1 <= slid_x <= gave_w + 1 and -1 <= slid_y <= gave_h + 1):
        failures.append(
            f"{name}: folding after Undo moved the other root other than up or left by the room given up: "
            f"{before['nodes'][other]} -> {folded['nodes'][other]}, gave {gave_w}x{gave_h}"
        )
    if any(abs(folded["nodes"][other][field] - before["nodes"][other][field]) > 1 for field in ("w", "h")):
        failures.append(f"{name}: folding after Undo resized the other root")
    if not same_node(reopened["nodes"][other], before["nodes"][other]):
        failures.append(f"{name}: reopening after Undo did not restore the other root")
    hold_during_gesture(page, env, project, other, chain, name, failures)
    print(f"{name}: a phone shrink repacked the roots, Undo restored them, and a later fold held them")


def hold_during_gesture(page, env, project, other, chain, name, failures):
    """A write that repacks the top level while the pointer is down on the Map leaves the tiles where they are, and
    draws once it is released: the new item appears and the roots move, gliding to their places."""
    count = int(settled_map(page).get_attribute("data-nodes"))
    ids = [project, other]
    corners = """(m, ids) => Object.fromEntries(ids.map((id) => {
        const b = m.cy.getElementById(id).boundingBox({ includeLabels: false });
        return [id, [Math.round(b.x1), Math.round(b.y1), Math.round(b.w), Math.round(b.h)]];
    }))"""
    before = page.get_by_test_id("map").evaluate(corners, ids)
    box = page.get_by_test_id("map").bounding_box()
    # The Map's left edge, a tenth of the way down: clear of the Fit button and of the sheet that rises from below.
    x, y = box["x"] + 6, box["y"] + box["height"] * 0.1
    if not page.evaluate("([x, y]) => document.elementFromPoint(x, y)?.closest('[data-testid=map]') !== null", [x, y]):
        failures.append(f"{name}: the Map's edge is covered, so a press there isn't on the Map")
        return
    page.mouse.move(x, y)
    page.mouse.down()
    try:
        # A step added to the chain makes the tall root taller, so the other root has to move.
        step = json.loads(run(env, "item", "add", "--title", "Held while pressed", "--parent", project))["data"]["item"]
        run(env, "item", "edit", step, "--waits-for", chain[-1])
        # Coming back to the app reads what changed meanwhile; the layout's answer clears the mark, so the window below
        # starts once the repack is ready and waiting.
        page.get_by_test_id("map").evaluate("m => { delete m.dataset.layoutMs; }")
        page.evaluate("document.dispatchEvent(new Event('visibilitychange'))")
        page.wait_for_function(
            "() => document.querySelector('[data-testid=map]').dataset.layoutMs !== undefined", timeout=20_000
        )
        held = page.get_by_test_id("map").evaluate(
            """(m, [ids, step]) => new Promise((done) => {
                const read = () => JSON.stringify([m.cy.getElementById(step).nonempty(), m.dataset.nodes,
                    ids.map((id) => { const b = m.cy.getElementById(id).boundingBox({ includeLabels: false });
                        return [Math.round(b.x1), Math.round(b.y1), Math.round(b.w), Math.round(b.h)]; })]);
                const first = read(), start = performance.now(), seen = [];
                const frame = () => {
                    seen.push(read());
                    if (performance.now() - start < 400) requestAnimationFrame(frame); else done({ first, seen });
                };
                frame();
            })""",
            [ids, step],
        )
        if any(one != held["first"] for one in held["seen"]) or json.loads(held["first"])[0]:
            failures.append(f"{name}: the Map changed under a pressed pointer: {held['seen'][-1]}")
        if json.loads(held["first"])[1] != str(count):
            failures.append(f"{name}: the Map's tile count changed under a pressed pointer")
    finally:
        page.mouse.up()
    wait_drawn(page, count + 1)
    settled_map(page)
    after = page.get_by_test_id("map").evaluate(corners, ids)
    if not page.get_by_test_id("map").evaluate("(m, id) => m.cy.getElementById(id).nonempty()", step):
        failures.append(f"{name}: the new item wasn't drawn once the pointer was released")
    if after == before:
        failures.append(f"{name}: the roots didn't move once the pointer was released: {before}")


@timed
def guarded_undo_race(page, view, env, plan, name, failures, errors):
    """Changes an item after Undo's fresh read but before its guarded request reaches the server."""
    page.reload()
    to_map(page)
    wait_drawn(page, len(plan.default()))
    # The tile is tapped where it is read, so the Map must have stopped moving: drawn is not settled.
    settled_map(page)
    candidates = [id for id in plan.default() if not plan.children(id)]
    if not candidates:
        failures.append(f"{name}: no visible leaf was available for the guarded Undo race")
        return
    id = candidates[0]
    centre(page, id)
    tap_middle(page, view, id)
    inspector = page.get_by_test_id("inspector")
    inspector.wait_for(timeout=10_000)
    was = item(env, id)["title"]
    page.locator("[data-action=edit]").click()
    form = page.get_by_test_id("item-form")
    title = form.locator("input[name=title]")
    changed = f"{was} for guarded Undo"
    elsewhere = f"Concurrent edit {id[-6:]}"
    title.fill(changed)
    form.locator("button[type=submit]").click()
    notice = show_notice(page)
    notice.get_by_role("button", name="Undo").wait_for(timeout=10_000)

    requests = []
    responses = []
    errors_before = len(errors)

    def guarded_response(response):
        body = json.loads(response.request.post_data or "{}")
        command = body.get("command", body) if isinstance(body, dict) else body
        if command == {"item": {"edit": {"item": id, "title": was}}} and isinstance(body, dict) and body.get("expect"):
            responses.append(response.status)

    page.on("response", guarded_response)

    def interleave(route):
        body = json.loads(route.request.post_data or "{}")
        command = body.get("command") if isinstance(body, dict) else None
        item_command = command.get("item") if isinstance(command, dict) else None
        edit = item_command.get("edit", {}) if isinstance(item_command, dict) else {}
        if edit.get("item") == id and edit.get("title") == was:
            requests.append(body)
            run(env, "item", "edit", id, "--title", elsewhere)
        route.continue_()

    page.route("**/api/command", interleave)
    try:
        show_notice(page).get_by_role("button", name="Undo").click()
        show_notice(page, "Changed since, so it can't be undone").get_by_text("Changed since, so it can't be undone").wait_for(timeout=10_000)
    finally:
        page.unroute("**/api/command", interleave)
        page.remove_listener("response", guarded_response)

    if responses != [409]:
        failures.append(f"{name}: the stale guarded Undo response was {responses}, not one 409")
    else:
        for index in range(errors_before, len(errors)):
            if "409" in errors[index] and "Conflict" in errors[index]:
                del errors[index]
                break
    if len(requests) != 1 or not any(
        condition == {"kind": "item-field", "item": id, "field": "title", "value": changed}
        for condition in requests[0].get("expect", [])
    ):
        failures.append(f"{name}: Undo didn't send the checked title with its guarded request: {requests}")
    check_tracker(env, id, lambda row: row["title"] == elsewhere, f"{id} kept its concurrent title", failures, name)
    page.screenshot(path=SHOTS / f"app-{name}-undo-race.png")
    print(f"{name}: a title edit after Undo's read stayed intact when the guarded request was refused")


@timed
def failed_add_journey(page, errors, name, failures):
    """A form holding typing survives Escape and Back; a refused edit opens the editor again with what was typed, the
    tracker's smoke test in the browser. What else a write, a read and an Undo come to is tracker.test.ts's."""
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
        or not page.get_by_test_id("notice").get_by_text("Unsaved changes").count()
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
    # A refused edit opens the editor again with what was typed, over the item as it was: one added for it, from the
    # Overview, so the add goes to the Map to show where it landed, from a Map zoomed far out: at a zoom its title
    # reads at. Zoomed in braces, so nothing comes back: Cytoscape's zoom returns the whole graph, which takes the
    # browser seconds to serialise.
    page.get_by_test_id("map").evaluate("m => { m.cy.zoom(0.3); }")
    page.locator("[data-view=overview]").click()
    page.locator("[data-action=add-top]").click()
    form.locator("input[name=title]").fill("Edit refused")
    form.locator("button[type=submit]").click()
    inspector = page.get_by_test_id("inspector")
    inspector.get_by_role("heading", name="Edit refused").wait_for(timeout=10_000)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    try:
        page.locator("[data-view=map][aria-current=page]").wait_for(timeout=5_000)
        page.wait_for_function("() => document.querySelector('[data-testid=map]').cy.zoom() >= 0.85", timeout=5_000)
    except PlaywrightTimeout:
        zoom = page.get_by_test_id("map").evaluate("m => m.cy.zoom()")
        failures.append(f"{name}: an item added from the Overview came in on the Map at zoom {zoom:.2f}, past reading")
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
    show_notice(page)
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    page.unroute("**/api/command")
    if (
        not editor.count()
        or editor.locator("input[name=title]").input_value() != "Refused edit"
        or editor.locator("textarea[name=notes]").input_value()
        != "Notes typed on the phone"
        or inspector.get_by_role("heading", name="Edit refused").count() != 1
    ):
        failures.append(
            f"{name}: a refused edit didn't give back what was typed over the item as it was"
        )
    if editor.count():
        close_notice_details(page)
        editor.get_by_role("button", name="Cancel").click()
    print(f"{name}: a form kept its typing through Escape and Back, and a refused edit gave back what was typed")


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
    form.locator("textarea[name=notes]").fill("Plain notes")
    if view == VIEWS["phone"]:
        # A link offered while typing belongs to the notice, so Enter cannot submit the dirty editor.
        before = form.evaluate(
            "f => { const clipboard = navigator.clipboard;"
            " const descriptor = clipboard && Object.getOwnPropertyDescriptor(clipboard, 'writeText');"
            " if (clipboard) Object.defineProperty(clipboard, 'writeText', {configurable:true,"
            " value:() => Promise.reject(new Error('Clipboard unavailable'))});"
            " let submits = 0; const count = () => submits++;"
            " f.addEventListener('submit', count, {capture:true, passive:true});"
            " window.noticeSubmitProof = {form:f, count:() => submits, cleanup:() => {"
            " f.removeEventListener('submit', count, true);"
            " if (clipboard) { if (descriptor) Object.defineProperty(clipboard, 'writeText', descriptor);"
            " else delete clipboard.writeText; } }};"
            " return [f.elements.title.value, f.elements.notes.value]; }"
        )
        try:
            item_action(page, "copy-link")
            notice = show_notice(page)
            link = page.get_by_test_id("link-to-copy")
            link.wait_for(timeout=5_000)
            outside = link.evaluate("input => input.form === null")
            link.press("Enter")
            kept = page.evaluate(
                "() => { const p = window.noticeSubmitProof, f = p.form;"
                " return [p.count(), f.isConnected, f.elements.title.value, f.elements.notes.value]; }"
            )
            if not outside or kept != [0, True, *before]:
                failures.append(f"{name}: notice Link Enter reached the dirty editor: outside={outside}, kept={kept}")
        finally:
            try:
                notice = page.get_by_test_id("notice")
                if notice.count():
                    notice.get_by_role("button", name="Dismiss", exact=True).click()
            finally:
                page.evaluate("() => { window.noticeSubmitProof.cleanup(); delete window.noticeSubmitProof; }")
                if form.count():
                    form.locator("input[name=title]").focus()
        size = page.viewport_size
        drawn = page.get_by_test_id("map")
        before = drawn.evaluate("m => ({width: m.clientWidth, height: m.clientHeight, min: m.cy.minZoom()})")
        # Shrink both Map dimensions in the same portrait layout: its fit floor must follow whichever axis limits it.
        # A height-only shrink need not change the floor when width already limits how far out the plan can zoom.
        smaller = {axis: round(before[axis] * 0.75) for axis in ("width", "height")}
        page.set_viewport_size({axis: size[axis] - before[axis] + smaller[axis] for axis in smaller})
        try:
            page.wait_for_function(
                """([before, smaller]) => {
                    const m = document.querySelector('[data-testid=map]');
                    const sx = m.clientWidth / before.width, sy = m.clientHeight / before.height;
                    const min = m.cy.minZoom();
                    return Math.abs(m.clientWidth - smaller.width) <= 1 && Math.abs(m.clientHeight - smaller.height) <= 1
                        && before.min > 0 && before.min < 0.5 && sx > 0 && sx < 1 && sy > 0 && sy < 1
                        && min >= before.min * Math.min(sx, sy) - 1e-7
                        && min <= before.min * Math.max(sx, sy) + 1e-7;
                }""",
                arg=[before, smaller],
                timeout=5_000,
            )
        except PlaywrightTimeout:
            after = drawn.evaluate("m => ({width: m.clientWidth, height: m.clientHeight, min: m.cy.minZoom()})")
            failures.append(f"{name}: the Map's fit floor did not follow a proportional resize: {before} -> {after}")
        # A keyboard shrinks the page to a sliver over the title being typed: Save and Cancel, at the form's foot,
        # must stay in it.
        page.set_viewport_size({"width": size["width"], "height": 360})
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
    # Meanwhile, an agent parks the item, and coming back to the app reads it: saving sends only what the user changed.
    run(env, "item", "edit", id, "--state", "parked")
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
            (row["title"], row["notes"], row["state"])
            == ("Edited in the app", "Plain notes", "parked")
        ),
        f"{id}'s new fields, and the state set while the editor was open",
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
    run(other, "item", "edit", id, "--title", "Title from the other clone")
    run(env, "item", "edit", id, "--title", "Title from this clone")
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
    run(env, "item", "done", id, "--force", "--evidence", "finished for the journey")
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
    """Opens the thread's panel, which lists its items, and picks a Theme on the way."""
    panel = page.get_by_test_id("thread-panel")
    # Picking a thread already picked unpicks it.
    pick_thread(page, plan, thread)
    try:
        panel.wait_for(timeout=3_000)
    except PlaywrightTimeout:
        pick_thread(page, plan, thread)
    members = len(json.loads(run(env, "thread", "show", thread))["data"][0]["members"])
    # Its items, in the order their work runs, the first eight shown.
    page.locator("[data-testid=thread-panel]:not([aria-busy])").wait_for(timeout=10_000)
    listed = panel.locator(".thread-items li").count()
    if listed != min(members, 8):
        failures.append(f"{name}: the panel lists {listed} of {thread}'s {members} items")
    open_full(page, "thread-panel")
    # A Theme picked on the device shows at once, over the system's scheme, and System gives it back.
    ground = "() => getComputedStyle(document.documentElement).backgroundColor"
    lit = page.evaluate(ground)
    from_menu(page, lambda: page.get_by_test_id("theme").select_option("dark"))
    if lit == page.evaluate(ground) or page.evaluate("document.documentElement.dataset.theme") != "dark":
        failures.append(f"{name}: picking Dark over a light system left the page {lit}")
    from_menu(page, lambda: page.get_by_test_id("theme").select_option("system"))
    page.screenshot(path=SHOTS / f"app-{name}-thread-panel.png")
    print(f"{name}: {thread}'s panel listed its items, and a Theme showed at once")


@timed
def fold_journey(page, view, env, plan, name, failures):
    """A finished thread's items in a box fold into one box, named after it, on the Map; the command menu still finds
    one of them, opening every box above it."""
    box = min(id for id in plan.items if plan.depth(id) == 0 and plan.children(id))
    thread = json.loads(run(env, "thread", "add", "--title", "Folded away"))["data"]["thread"]
    items = []
    for title in ("Fold the first", "Fold the second"):
        added = json.loads(run(env, "item", "add", "--title", title, "--parent", box, "--thread", thread))
        items.append(added["data"]["item"])
        run(env, "item", "done", items[-1], "--evidence", "folded by a journey")
    to_map(page)
    refresh(page)
    folded = f"~thread {box} {thread}"
    # Folded as a box of its own, or with the box's other finished work in its Done box.
    page.wait_for_function(
        "([folded, done]) => { const m = document.querySelector('[data-testid=map]');"
        " return m?.dataset.state === 'drawn' && (m.cy.getElementById(folded).nonempty()"
        " || m.cy.getElementById(done).nonempty()); }",
        arg=[folded, f"~done {box}"],
        timeout=20_000,
    )
    drawn = page.get_by_test_id("map").evaluate(
        "(m, ids) => ids.filter(id => m.cy.getElementById(id).nonempty())", items
    )
    if drawn:
        failures.append(f"{name}: a finished thread's items {drawn} were drawn rather than folded")
    menu = page.get_by_test_id("command-menu")
    if view.get("has_touch"):
        page.locator("[data-action=search]").tap()
    else:
        page.keyboard.press("ControlOrMeta+k")
    menu.wait_for(timeout=5_000)
    page.keyboard.type("Fold the first")
    row = page.locator(f".command-row[data-command='item:{items[0]}']")
    row.wait_for(timeout=5_000)
    row.tap() if view.get("has_touch") else row.click()
    try:
        page.wait_for_function(
            "id => { const m = document.querySelector('[data-testid=map]');"
            " return m?.dataset.state === 'drawn' && m.cy.getElementById(id).nonempty(); }",
            arg=items[0],
            timeout=10_000,
        )
        page.screenshot(path=SHOTS / f"app-{name}-fold.png")
        print(f"{name}: a finished thread folded into one box, and the command menu unfolded it to {items[0]}")
    except PlaywrightTimeout:
        failures.append(f"{name}: the command menu's folded item {items[0]} wasn't unfolded into view")
    finally:
        # Reopened, so the views after, which count every item the plan holds, draw them as tiles.
        for id in items:
            run(env, "item", "reopen", id)


@timed
def needs_journey(page, env, plan, thread, name, failures):
    """Settles a conflict two clones wrote from Needs you's view, whose count follows, and names threads that wait on
    each other. A thread or an item an entry names goes to the Map, and closing it, by Escape or its ✕, comes back to
    the view."""
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
    # J focuses an entry, its ring rounded like the rest and drawn outside it, so the badge at its edge isn't crossed
    # and the ring isn't cut off at the view's left edge.
    page.keyboard.press("j")
    ring = page.evaluate(
        """() => {
            const e = document.activeElement;
            const s = getComputedStyle(e);
            return { entry: e.matches(".needs > li[data-row]"), radius: s.borderTopLeftRadius,
                     offset: parseFloat(s.outlineOffset), width: parseFloat(s.outlineWidth),
                     reach: e.getBoundingClientRect().left - parseFloat(s.outlineOffset) - parseFloat(s.outlineWidth) };
        }"""
    )
    if not ring["entry"] or ring["radius"] != "8px" or ring["offset"] <= 0 or ring["reach"] < 0:
        failures.append(f"{name}: J's focus ring on a Needs you entry was {ring}, not rounded, outside and uncut")
    # A grouped terminal-only entry opens its details with one tap; its member then returns to the same control.
    disclosure = needs.locator(".need-disclosure-toggle").first
    disclosure.wait_for(timeout=5_000)
    disclosure.click()
    disclosure.locator("..").locator(".need-details").wait_for(timeout=5_000)
    if disclosure.get_attribute("aria-expanded") != "true":
        failures.append(f"{name}: Needs you's grouped entry didn't reveal its details")
    # A thread picked from the list goes to the Map, as any thread does; closing it comes back to the view.
    viewed = "() => new URLSearchParams(location.search).get('view')"
    # One of a thread's own: a thread of one, a task alone, opens the task, as an item's row does.
    first_picker = needs.locator("[data-pick-thread]:not([data-pick-thread$='.one'])").first
    reveal_need_target(first_picker)
    first_picker.click()
    try:
        page.get_by_test_id("thread-panel").wait_for(timeout=5_000)
        if page.evaluate(viewed) != "map":
            failures.append(f"{name}: a thread picked from Needs you stayed off the Map, at {page.url}")
        page.keyboard.press("Escape")
        page.get_by_test_id("thread-panel").wait_for(state="detached", timeout=5_000)
        needs.wait_for(timeout=5_000)
        first_picker.wait_for(state="visible", timeout=5_000)
        if not first_picker.evaluate("e => e === document.activeElement"):
            failures.append(f"{name}: Back did not restore the chosen needs-you member without another expansion")
        if page.evaluate(viewed) != "needs":
            failures.append(f"{name}: closing a thread from Needs you left the view at {page.url}")
    except PlaywrightTimeout:
        failures.append(f"{name}: a thread picked from Needs you didn't open, or close back to the list")
    # An item likewise, selected on the Map; its panel's ✕ comes back.
    row = needs.locator("button.row[data-select]").first
    if row.count():
        reveal_need_target(row)
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
    # The queue has the name its push notification gives it.
    named = (page.locator("[data-view=needs]").get_attribute("aria-label"), page.locator(".bar h1").inner_text())
    if not named[0].startswith("Needs you:") or named[1] != "Needs you":
        failures.append(f"{name}: the queue's tab and title read {named}, not Needs you")
    # Threads that wait on each other are named, each a row that picks it.
    cycle = needs.locator("[data-need^='thread-cycle:']").first
    if not cycle.count() or cycle.locator("[data-pick-thread]").count() < 2:
        failures.append(f"{name}: Needs you names no threads waiting on each other in a cycle")
    # A conflict: the second clone form_journey made and this one write an item's title apart, and both sync.
    outside = next(
        id
        for id, row in sorted(plan.items.items())
        if row["thread"] not in (thread, None) and row["state"] != "done"
    )
    other = {**env, "ACYCLIC_TRACKER": str(Path(env["ACYCLIC_TRACKER"]).parent / "other")}
    run(env, "sync")
    run(other, "sync")
    run(other, "item", "edit", outside, "--title", "Needs you's title from the other clone")
    run(env, "item", "edit", outside, "--title", "Needs you's title from this clone")
    run(other, "sync")
    run(env, "sync")
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
    key = f"conflict:item:{outside}:title"
    conflict = needs.locator(f"[data-need='{key}']")
    conflict.locator("[data-action=use]").filter(has_text="from the other clone").click()
    conflict.wait_for(state="detached", timeout=10_000)
    # The keyboard stays in the list, on what took the entry's place, and what was used is said.
    page.locator("main.stage[data-busy=false]").wait_for(timeout=10_000)
    kept = page.evaluate(
        "() => document.querySelector('[data-testid=needs]').contains(document.activeElement)"
    )
    said = page.locator("p[role=status]").inner_text()
    if not kept or not said.startswith("Used "):
        failures.append(
            f"{name}: settling from the list left focus at {page.evaluate('document.activeElement.tagName')}"
            f" and said {said!r}"
        )
    if int(count.inner_text()) != before:
        failures.append(f"{name}: the needs count is {count.inner_text()}, not {before}, once the conflict was settled")
    if json.loads(run(env, "item", "show", outside))["data"][0]["title"] != "Needs you's title from the other clone":
        failures.append(f"{name}: settling the conflict from the list didn't keep the other clone's title")
    page.screenshot(path=SHOTS / f"app-{name}-needs.png")
    picker = needs.locator("[data-need] [data-pick-thread]:not([data-pick-thread$='.one'])").first
    if picker.count() == 0:
        failures.append(f"{name}: no entry picks a thread")
    else:
        picked = picker.get_attribute("data-pick-thread")
        reveal_need_target(picker)
        assert picker.get_attribute("data-pick-thread") == picked
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
            failures.append(f"{name}: an entry didn't pick {picked}, or its row is out of view")
        if sheet.count():
            page.get_by_test_id("projects").get_by_role("button", name="Close", exact=True).click()
        panel.get_by_role("button", name="Close").click()
        # Picked from the list, the thread showed on the Map: closing it comes back to the list.
        try:
            needs.wait_for(timeout=5_000)
        except PlaywrightTimeout:
            failures.append(f"{name}: closing a thread picked from the needs-you list didn't come back to it")
    print(f"{name}: settled a conflict in Needs you, named a cycle of threads, and came back from what it named")


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
        "body": "A conflict for the journey",
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


# Each shard builds its own tracker and server, so none waits on another: the desktop journeys and WebKit's desktop, the
# phone journeys with the notifying browser and WebKit's phone, the two small ones, and the 3,500-item one, whose tracker
# takes a third of its time to build.
SHARDS = ("desktop", "phone", "small", "large")


def relay(shard, stream):
    for line in stream:
        print(f"[{shard}] {line}", end="", flush=True)


def report(took, failures, walls=None):
    """Prints the run's slowest steps, adds them all to CI's summary, and holds the run to its budget."""
    steps = Counter()
    for label, seconds in TIMES:
        steps[label] += seconds
    rows = steps.most_common()
    print(f"took {took:.1f} s; slowest: " + ", ".join(f"{label} {s:.1f} s" for label, s in rows[:5]))
    for shard, wall in (walls or {}).items():
        print(f"shard {shard}: {wall:.1f} s")
    print("All step durations:")
    for label, seconds in rows:
        print(f"  {label}: {seconds:.3f} s")
    details, counts = Counter(), Counter()
    for label, seconds in DETAILS:
        details[label] += seconds
        counts[label] += 1
    captures = [entry for entry in DETAILS if entry[0].startswith("capture ")]
    print(f"Capture subset: {len(captures)} calls, {sum(seconds for _, seconds in captures):.3f} s")
    print("Detail durations (included in the whole run; overlap steps, so are not additive):")
    for label, seconds in details.most_common():
        print(f"  {label}: {seconds:.3f} s ({counts[label]} calls)")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as out:
            out.write(f"### The app's journeys: {took:.1f} s (budget {BUDGET_S} s, limit {LIMIT_S} s)\n\n")
            out.write("| Step | Seconds |\n|---|---|\n")
            out.writelines(f"| {label} | {s:.1f} |\n" for label, s in rows)
            out.writelines(f"| shard {shard} (wall) | {wall:.1f} |\n" for shard, wall in (walls or {}).items())
    if took > LIMIT_S:
        failures.append(f"the journeys took {took:.1f} s, past their {LIMIT_S} s limit")
    elif took > BUDGET_S:
        print(f"::warning title=The app's journeys::took {took:.1f} s, over their {BUDGET_S} s budget")


def server_origin(server):
    """Reads the server's own listening receipt within a bounded startup deadline."""
    deadline = time.monotonic() + 10
    line = b""
    while b"\n" not in line:
        left = deadline - time.monotonic()
        if left <= 0 or not select.select([server.stdout], [], [], left)[0]:
            raise TimeoutError(
                "the fixture server did not report its listening address"
            )
        chunk = os.read(server.stdout.fileno(), 4096)
        if not chunk:
            raise AssertionError(
                f"the fixture server exited before listening: {server.poll()}"
            )
        line += chunk
    return f"http://{json.loads(line.splitlines()[0])['data']['listening']}/"


@timed
def labels_journey(browser, link, ids, titles):
    """Measured labels fit their nodes; a failed font settles on the face that was measured."""
    origin = link.rstrip("/")

    def fresh(view, mode=None):
        context = browser.new_context(**view)
        blocked = []

        def local_only(route):
            if route.request.url.startswith(origin + "/"):
                route.continue_()
            else:
                blocked.append(route.request.url)
                route.abort()

        context.route(
            "**/*",
            local_only,
        )
        page = context.new_page()
        page.set_default_timeout(10_000)
        errors, messages, fonts = [], [], []
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.on(
            "console",
            lambda message: (
                messages.append({"text": message.text, "url": message.location.get("url", "")})
                if message.type in ("error", "warning")
                else None
            ),
        )
        if mode is not None:

            def font_route(route):
                fonts.append(route)
                if mode == "abort":
                    route.abort()

            context.route(re.compile(r"^" + re.escape(origin) + r"/[^?#]*\.woff2(?:\?[^#]*)?$"), font_route)
        return context, page, errors, messages, fonts, blocked

    def check_errors(errors, messages, fonts, blocked):
        assert not errors, errors
        assert not blocked, blocked
        font_urls = {route.request.url for route in fonts}
        unexpected = [
            message
            for message in messages
            if not (message["url"] in font_urls and message["text"].startswith("Failed to load resource:"))
        ]
        assert not unexpected, unexpected

    def read(page):
        return page.get_by_test_id("map").evaluate(
            r"""(m, ids) => {
                const core=m.cy, canvas=document.createElement('canvas'), measure=canvas.getContext('2d');
                return {zoom:core.zoom(),viewport:{width:m.clientWidth,height:m.clientHeight}, nodes:ids.map(id=>{
                    const n=core.getElementById(id), body=n.boundingBox({includeLabels:false}), ink=n.boundingBox({includeLabels:true});
                    const family=n.style('font-family');
                    measure.font=`${n.style('font-weight')} ${n.style('font-size')} ${family}`;
                    return {id,rendered:n.renderedBoundingBox({includeLabels:true}),label:n.data('label'),family,header:n.data('header'),body,ink,
                        textWidth:n.data('textWidth'), widths:n.data('label').split('\n').map(line=>measure.measureText(line.trimEnd()).width)};
                })};
            }""",
            ids,
        )

    def check_labels(found):
        for node, title in zip(found["nodes"], titles):
            assert re.sub(r"\s", "", title) in re.sub(r"\s", "", node["label"]), node
            assert all(width <= node["textWidth"] + 1 for width in node["widths"]), node
            body, ink = node["body"], node["ink"]
            assert ink["x1"] >= body["x1"] - 1 and ink["x2"] <= body["x2"] + 1, node
            assert ink["y1"] >= body["y1"] - 1 and ink["y2"] <= body["y2"] + 1, node
        root, long, ready = found["nodes"]
        assert "Ready" in long["label"] and "Ready" in ready["label"], found
        assert root["header"] > 72, root
        assert all(
            child["body"]["y1"] >= root["body"]["y1"] + root["header"] - 1
            for child in (long, ready)
        ), found

    for name, view in VIEWS.items():
        context, page, errors, messages, fonts, blocked = fresh(view)
        point = None
        try:
            page.goto(link + "?view=map")
            wait_drawn(page, 3)
            page.screenshot(path=SHOTS / f"app-{name}-full-labels.png")
            found = read(page)
            assert FIRST_ZOOM <= found["zoom"] <= 2, found
            ink = found["nodes"][1]["rendered"]
            assert ink["x1"] >= 0 and ink["y1"] >= 0, found
            assert ink["x2"] <= found["viewport"]["width"] and ink["y2"] <= found["viewport"]["height"], found
            assert all("Overpass" in node["family"] for node in found["nodes"]), found
            check_labels(found)
            point = page.get_by_test_id("map").evaluate(
                r"""(m,id)=>{
                    const n=m.cy.getElementById(id), box=n.renderedBoundingBox({includeLabels:false}), rect=m.getBoundingClientRect();
                    const svg=decodeURIComponent(n.data('mark').split(',').slice(1).join(','));
                    const transform=new DOMParser().parseFromString(svg,'image/svg+xml').querySelector('path').getAttribute('transform');
                    const [,x,y]=transform.match(/translate\(([-\d.]+)[ ,]+([-\d.]+)\)/);
                    return {x:rect.x+box.x1+(Number(x)+6)*m.cy.zoom(),y:rect.y+box.y1+(Number(y)+6)*m.cy.zoom()};
                }""",
                ids[0],
            )
            assert page.get_by_test_id("map").evaluate(
                """(m,p)=>{const r=m.getBoundingClientRect(), hit=document.elementFromPoint(p.x,p.y);
                return p.x>=r.left && p.x<r.right && p.y>=r.top && p.y<r.bottom
                    && hit && m.contains(hit) && !hit.closest('button');}""", point
            ), point
            if view.get("has_touch"):
                page.touchscreen.tap(point["x"], point["y"])
            else:
                page.mouse.click(point["x"], point["y"])
            wait_drawn(page, 1)
            assert page.get_by_test_id("map").evaluate(
                "(m,id)=>m.cy.getElementById(id).hasClass('collapsed')", ids[0]
            )
            assert page.get_by_test_id("inspector").count() == 0
            check_errors(errors, messages, fonts, blocked)
        except (AssertionError, PlaywrightTimeout):
            page.screenshot(path=SHOTS / f"app-{name}-labels-failure.png")
            state = page.evaluate(
                """id=>{const m=document.querySelector('[data-testid=map]'),core=m?.cy;
                return {dataset:m?{...m.dataset}:null,root:core?.getElementById(id).data(),
                nodes:core?.nodes().map(n=>({id:n.id(),kind:n.data('kind')})),font:document.fonts.status,
                inspector:document.querySelector('[data-testid=inspector] code.id')?.textContent};}""", ids[0]
            )
            (SHOTS / f"app-{name}-labels-failure.json").write_text(json.dumps({"point":point,"state":state}, indent=2))
            raise
        finally:
            context.close()

    context, page, errors, messages, fonts, blocked = fresh(VIEWS["desktop"], "abort")
    try:
        page.goto(link + "?view=map")
        wait_drawn(page, 3)
        page.screenshot(path=SHOTS / "app-font-fallback.png")
        assert fonts, "the font-failure case did not intercept a font"
        check_labels(read(page))
        assert all("Overpass" not in node["family"] for node in read(page)["nodes"])
        from_menu(page, lambda: page.get_by_test_id("theme").select_option("light"))
        page.wait_for_function("() => document.documentElement.dataset.theme === 'light'")
        colour = page.evaluate(r"""() => {
            const probe=document.body.appendChild(document.createElement('i'));probe.style.color='var(--box)';
            const colour=getComputedStyle(probe).color.replace(/\s/g,'');probe.remove();return colour;
        }""")
        page.wait_for_function(
            r"""([id,colour]) => {
            const n=document.querySelector('[data-testid=map]')?.cy?.getElementById(id);
            return n?.nonempty() && n.style('background-color').replace(/\s/g,'')===colour;
        }""",
            arg=[ids[0], colour],
        )
        assert all("Overpass" not in node["family"] for node in read(page)["nodes"])
        check_errors(errors, messages, fonts, blocked)
    finally:
        context.close()

    context, page, errors, messages, fonts, blocked = fresh(VIEWS["desktop"], "hold")
    try:
        with page.expect_request(
            lambda request: urlsplit(request.url).path.endswith(".woff2"),
            timeout=10_000,
        ):
            page.goto(link + "?view=map", wait_until="domcontentloaded", timeout=15_000)
        page.get_by_test_id("map").wait_for(state="attached")
        page.wait_for_function("() => document.fonts.status === 'loading'")
        assert fonts, "the font-wait case did not hold a font"
        assert page.get_by_test_id("map").evaluate(
            "m=>Number(m.dataset.layouts ?? 0)===0"
        )
        page.goto("about:blank", timeout=10_000)
        check_errors(errors, messages, fonts, blocked)
    finally:
        # The old document is gone; resolve the deliberately held routes before draining their handlers.
        # This proves lifetime cleanup, without inspecting the old document's cache.
        try:
            for route in fonts:
                route.abort()
            context.unroute_all(behavior="wait")
        finally:
            context.close()


def shard_run(shard):
    """Runs one shard of the journeys, and leaves its timings and failures where the driver reads them."""
    assert shard in SHARDS, f"unknown shard {shard}"
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
            if items < SMALL_TRACKER:
                remote = directory / f"origin-{items}.git"
                subprocess.run(["git", "init", "-q", "--bare", str(remote)], check=True)
            plan = tracker(directory, env, items, remote, cycles=items < SMALL_TRACKER)
            # A second initiative, so the switcher has a row to choose. The small tracker, which the Overview's journey runs on,
            # gives it ready work, so Up next has two populated initiatives.
            second = json.loads(run(env, "initiative", "add", "--title", "Second initiative"))["data"]["initiative"]
            if items < SMALL_TRACKER:
                epic = json.loads(run(env, "item", "add", "--title", "Second epic", "--parent", second))["data"]["item"]
                # In a thread of its own, as a task alone would be picked by the Threads journey, whose panel it opens no thread's.
                thread = json.loads(run(env, "thread", "add", "--title", "Second work"))["data"]["thread"]
                tasks = [
                    json.loads(run(env, "item", "add", "--title", f"Second task {number}", "--parent", epic, "--thread", thread))["data"]["item"]
                    for number in (1, 2)
                ]
                # One wait leaves the initiative, for the Map's stub: on an item of the fixture's that has work left and is
                # drawn in its default view, so following it opens no box.
                far = next(id for id in plan.default() if not plan.children(id) and plan.items[id]["stage"] == "ready")
                run(env, "item", "edit", tasks[0], "--waits-for", far)
                plan = Plan(env)
            server = subprocess.Popen(
                [ACYCLIC, "up", "--port", "0"],
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )
            servers.append(server)
            return plan, server_origin(server), env, remote

        @timed
        def serve_shrink():
            home = directory / "home-shrink"
            home.mkdir()
            env = {
                **os.environ,
                "HOME": str(home),
                "ACYCLIC_TRACKER": str(directory / "tracker-shrink"),
                "ACYCLIC_ACTOR": "session/browser",
            }
            run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "browser")
            run(env, "initiative", "add", "--title", "Home")
            run(env, "thread", "add", "--title", "Shrink proof")
            project = json.loads(run(env, "item", "add", "--title", "Tall chain"))["data"]["item"]
            other = json.loads(run(env, "item", "add", "--title", "Other root"))["data"]["item"]
            run(env, "item", "add", "--title", "Any time", "--parent", project)
            chain = [
                json.loads(
                    run(env, "item", "add", "--title", f"Chain step {index}", "--parent", project)
                )["data"]["item"]
                for index in range(7)
            ]
            for before, after in pairwise(chain):
                run(env, "item", "edit", after, "--waits-for", before)
            server = subprocess.Popen(
                [ACYCLIC, "up", "--port", "0"],
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )
            servers.append(server)
            return server_origin(server), env, project, other, chain

        @timed
        def serve_labels():
            home = directory / "home-labels"
            home.mkdir()
            env = {
                **os.environ,
                "HOME": str(home),
                "ACYCLIC_TRACKER": str(directory / "tracker-labels"),
                "ACYCLIC_ACTOR": "session/browser",
            }
            run(env, "init", env["ACYCLIC_TRACKER"], "--replica", "browser")
            run(env, "initiative", "add", "--title", "Home")
            thread = json.loads(run(env, "thread", "add", "--title", "Label proof"))[
                "data"
            ]["thread"]
            titles = [
                "Keep the whole project title readable while its header grows",
                "Read every word including 🧑🏽‍🚀 and "
                + "supercalifragilisticexpialidocious" * 2,
                "Ordinary item",
            ]
            # The Map orders boxes that nothing links by id, in natural order, which for random ids differs from run to run: the
            # phone's first view holds only the left of the two children, so these ids put the one with the long title there.
            ids = [f"00000000-0000-7000-8000-00000000000{number}" for number in (1, 2, 3)]
            root = json.loads(run(env, "item", "add", "--id", ids[0], "--title", titles[0]))["data"][
                "item"
            ]
            long = json.loads(
                run(env, "item", "add", "--id", ids[1], "--title", titles[1], "--parent", root)
            )["data"]["item"]
            ready = json.loads(
                run(
                    env,
                    "item",
                    "add",
                    "--id",
                    ids[2],
                    "--title",
                    titles[2],
                    "--parent",
                    root,
                    "--thread",
                    thread,
                )
            )["data"]["item"]
            server = subprocess.Popen(
                [ACYCLIC, "up", "--port", "0"],
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )
            servers.append(server)
            return server_origin(server), [root, long, ready], titles

        try:
            if shard == "small":
                chromium = playwright.chromium.launch()
                label_link, label_ids, label_titles = serve_labels()
                labels_journey(chromium, label_link, label_ids, label_titles)
                # Only these two roots share the packing, so a shrink can be measured without other roots obscuring it.
                shrink_link, shrink_env, project, other, chain = serve_shrink()
                context, page, _, errors = open_app(
                    chromium, VIEWS["phone"], shrink_link, failures, "phone shrink", glide=True
                )
                shrink_undo_journey(
                    page, VIEWS["phone"], shrink_env, project, other, chain, "phone shrink", failures
                )
                if errors:
                    failures.append(f"phone shrink: the page threw or logged errors: {errors}")
                context.close()

                chromium.close()
            elif shard == "large":
                chromium = playwright.chromium.launch()
                large, link, _, _ = serve(3500)
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
            else:
                plan, link, env, remote = serve(350)
                chromium = playwright.chromium.launch()
                print(f"Chromium default headless: {chromium.version}")
                hint = None
                for name, view in VIEWS.items():
                    if name != shard:
                        continue
                    # The desktop journeys run with the glide on: a chevron's anchor, a pick's reveal and the command
                    # menu's jump are held against tiles that are still moving. The phone's snap, for its time.
                    context, page, origin, errors = open_app(
                        chromium, view, link, failures, name, glide=name == "desktop"
                    )
                    # The sidebar counts the plan's items; a phone's bar leaves the count out.
                    counted = page.get_by_test_id("item-count")
                    total = len(plan.items) + len(plan.elsewhere)
                    count = counted.inner_text() if counted.count() else str(total)
                    if count != str(total):
                        failures.append(
                            f"{name}: the page lists {count} items, expected {total}"
                        )
                    installable(context, page, origin, name, failures)
                    overview_journey(page, view, name, failures)
                    to_map(page)
                    # Both sizes lay the plan out alike, so a box one finds the layout moves, the next tries first.
                    hint = map_journey(page, view, plan, name, failures, hint)
                    outline_journey(page, view, plan, name, failures)
                    threads_journey(page, view, env, plan, name, failures)
                    deep_link_journey(context, origin, view, plan, name, failures)
                    history_journey(context, origin, view, plan, name, failures)
                    command_journey(page, view, origin, plan, name, failures)
                    if name == "desktop":
                        cross_origin_journey(context, origin, env, name, failures)
                        keys_journey(page, plan, name, failures)
                    if name == "phone" and hint is not None:
                        # Edits run once, on the phone the app is used on; the plan is read again after them.
                        leaf = min(
                            id for id in plan.children(hint) if not plan.children(id)
                        )
                        # Read by the reload edit_journey starts with.
                        run(env, "item", "edit", leaf, "--notes", HOSTILE)
                        added = edit_journey(page, view, env, plan, hint, name, failures)
                        if added is not None:
                            form_journey(page, view, env, remote, added, name, failures)
                        failed_add_journey(page, errors, name, failures)
                        thread_journey(page, env, plan, plan.first_thread, name, failures)
                        needs_journey(page, env, plan, plan.first_thread, name, failures)
                        fold_journey(page, view, env, plan, name, failures)
                        plan = Plan(env)
                        guarded_undo_race(page, view, env, plan, name, failures, errors)
                        plan = Plan(env)
                    page.screenshot(path=SHOTS / f"app-{name}.png")
                    if errors:
                        failures.append(
                            f"{name}: the page threw or logged errors: {errors}"
                        )
                    context.close()
                if shard == "phone":
                    # The full build supports push delivery; the journey records display calls before sending a push.
                    notifying = playwright.chromium.launch(channel="chromium")
                    print(f"Chromium full headless: {notifying.version}")
                    context, page, origin, errors = open_app(
                        notifying, VIEWS["phone"], link, failures, "notifying phone"
                    )
                    push_journey(context, page, env, origin, "phone", failures)
                    if errors:
                        failures.append(
                            f"notifying phone: the page threw or logged errors: {errors}"
                        )
                    context.close()
                    notifying.close()
                webkit = playwright.webkit.launch()
                print(f"WebKit headless: {webkit.version}")
                if shard == "phone":
                    context, page, origin, errors = open_app(
                        webkit, VIEWS["phone"], link, failures, "webkit phone"
                    )
                    cross_origin_journey(context, origin, env, "webkit phone", failures)
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
                if shard == "desktop":
                    # WebKit draws the picked thread's Threads arrows where Chromium does, clear of the tiles.
                    context, page, _, errors = open_app(
                        webkit,
                        {**VIEWS["desktop"], "color_scheme": "light"},
                        link,
                        failures,
                        "webkit desktop",
                    )
                    listed = json.loads(run(env, "thread", "list"))["data"]
                    want = waves_in_context(plan, listed, anytime=False)
                    page.locator("[data-view=threads]").click()
                    verify_thread_arrows(page, listed, want, "webkit desktop", failures)
                    if errors:
                        failures.append(
                            f"webkit desktop: the page threw or logged errors: {errors}"
                        )
                    context.close()
                webkit.close()

                chromium.close()
        finally:
            for server in servers:
                server.terminate()
                server.wait(timeout=10)
    Path(os.environ["JOURNEYS_SHARD_OUT"]).write_text(
        json.dumps({"wall": time.monotonic() - began, "times": TIMES, "details": DETAILS, "failures": failures})
    )


def main():
    """Drives the four shards of the journeys at once, each in a process of its own with its own tracker, server and
    browsers, and reports them as one run. A shard's lines are prefixed with its name, so a failure and its traceback
    say which shard raised it; its screenshots are named by journey, as before."""
    if "--shard" in sys.argv:
        return shard_run(sys.argv[sys.argv.index("--shard") + 1])
    began = time.monotonic()
    print(f"Journeys optimization override: {os.environ.get('CARGO_PROFILE_JOURNEYS_OPT_LEVEL', 'unset (Cargo config)')}")
    failures, walls = [], {}
    with tempfile.TemporaryDirectory() as temp:
        running = {}
        for shard in SHARDS:
            out = Path(temp) / f"{shard}.json"
            proc = subprocess.Popen(
                [sys.executable, __file__, ACYCLIC, FIXTURES, "--shard", shard],
                env={**os.environ, "JOURNEYS_SHARD_OUT": str(out)},
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            pump = threading.Thread(target=relay, args=(shard, proc.stdout))
            pump.start()
            running[shard] = (proc, out, pump)
        for shard, (proc, out, pump) in running.items():
            proc.wait()
            pump.join()
            if not out.exists():
                failures.append(f"{shard}: the shard exited with {proc.returncode} before it reported")
                continue
            data = json.loads(out.read_text())
            walls[shard] = data["wall"]
            TIMES.extend((label, seconds) for label, seconds in data["times"])
            DETAILS.extend((label, seconds) for label, seconds in data["details"])
            failures.extend(data["failures"])
    report(time.monotonic() - began, failures, walls)
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    main()
