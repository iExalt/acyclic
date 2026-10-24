"""Times the Map at 3,500 items in headless Chromium with the CPU slowed 4x, as a stand-in for a phone.

Not part of any check: a measurement for the record. For ELK in its worker and ELK on the page's own thread (which
tells ELK's time from what the worker's messages add), it reports the default view's layout (time and boxes laid
out) and drawing, then an expand of a collapsed box, and the collapse back, which the cache should answer without ELK.

Usage: timing.py <acyclic binary> <acyclic-fixtures binary>
"""

import json
import os
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlsplit

from app import ACYCLIC, VIEWS, run, tracker, wait_drawn
from playwright.sync_api import sync_playwright

RUNS = 3
# Chrome slows only the page's own thread, not workers, so ELK in its worker runs at full speed even when throttled:
# the inline runs are the slowed ones. THROTTLE=1 runs everything at full speed, to compare the two engines directly.
THROTTLE = float(os.environ.get("THROTTLE", "4"))


def tap_node(page, id):
    """Brings a box to full size in the middle of the screen, as a person would before tapping it, then taps its
    title. At 3,500 items the whole plan fits only at a zoom where a box is a few pixels tall."""
    corner = page.get_by_test_id("map").evaluate(
        "(m, id) => { const n = m.cy.getElementById(id); m.cy.zoom(1); m.cy.center(n);"
        " const b = n.renderedBoundingBox({ includeLabels: false });"
        " const r = m.getBoundingClientRect(); return { x: r.x + b.x1 + b.w / 2, y: r.y + b.y1 + 10 }; }",
        id,
    )
    page.touchscreen.tap(corner["x"], corner["y"])


def numbers(data):
    return f"layout {data['layoutMs']} ms ({data['layoutCalls']} boxes), drawing {data['drawMs']} ms"


def main():
    with tempfile.TemporaryDirectory() as temp, sync_playwright() as playwright:
        directory = Path(temp)
        home = directory / "home"
        home.mkdir()
        env = {
            **os.environ,
            "HOME": str(home),
            "ACYCLIC_TRACKER": str(directory / "tracker"),
            "ACYCLIC_ACTOR": "session/timing",
        }
        plan = tracker(directory, env, 3500)
        server = subprocess.Popen(
            [ACYCLIC, "up", "--port", "0"], env=env, stdout=subprocess.PIPE, text=True
        )
        try:
            link = json.loads(server.stdout.readline())["data"]["link"]
            port = str(urlsplit(link).port)
            browser = playwright.chromium.launch()
            default = len(plan.default())
            box = min(
                id for id in plan.default() if plan.depth(id) == 1 and plan.children(id)
            )
            opened = default + len(plan.children(box))
            for engine in ["worker", "inline"]:
                for attempt in range(RUNS):
                    context = browser.new_context(**VIEWS["phone"])
                    page = context.new_page()
                    page.context.new_cdp_session(page).send(
                        "Emulation.setCPUThrottlingRate", {"rate": THROTTLE}
                    )
                    page.goto(link)
                    link = json.loads(run(env, "up", "--link", "--port", port))["data"][
                        "link"
                    ]
                    if engine == "inline":
                        page.goto(page.url.rstrip("/") + "/?engine=inline")
                    first = wait_drawn(page, default, timeout=120_000)
                    tap_node(page, box)
                    expand = wait_drawn(page, opened, timeout=120_000)
                    tap_node(page, box)
                    back = wait_drawn(page, default, timeout=120_000)
                    print(
                        f"{engine} run {attempt + 1}: default view {numbers(first)}; expand {numbers(expand)}; "
                        f"collapse back {numbers(back)}"
                    )
                    context.close()
            browser.close()
        finally:
            server.terminate()
            server.wait(timeout=10)


if __name__ == "__main__":
    main()
