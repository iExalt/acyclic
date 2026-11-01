import { describe, expect, it } from "vitest";
import {
  firstOf,
  firstView,
  frame,
  framesAll,
  headed,
  nearest,
  newlyCovered,
  READABLE,
  reveal,
  startOf,
  uncovered,
} from "./reveal";

describe("reveal", () => {
  const open = { x1: 0, y1: 0, x2: 400, y2: 500 };

  it("leaves a node already in view, and pans the least that brings one in from any side", () => {
    expect(reveal({ x1: 50, y1: 50, x2: 150, y2: 90 }, open)).toEqual({ x: 0, y: 0 });
    expect(reveal({ x1: 50, y1: 600, x2: 150, y2: 640 }, open)).toEqual({ x: 0, y: -148 });
    expect(reveal({ x1: -100, y1: -30, x2: -20, y2: 10 }, open)).toEqual({ x: 108, y: 38 });
  });

  it("shows the start of a node too big to fit, where its title is", () => {
    expect(reveal({ x1: 100, y1: 100, x2: 900, y2: 1200 }, open)).toEqual({ x: -92, y: -92 });
  });
});

describe("newlyCovered", () => {
  // A phone's Map 700 tall, its sheet from 400; a slot of 70 px takes the Map to 630 and the sheet up to 330.
  const was = { width: 384, height: 700 };
  const now = { width: 384, height: 630 };
  const sheet = { x1: 0, y1: 330, x2: 384, y2: 630 };

  it("is a selection the resize hides that was clear before", () => {
    expect(newlyCovered({ x1: 20, y1: 340, x2: 200, y2: 380 }, was, now, sheet)).toBe(true);
  });

  it("isn't one still clear, nor one hidden already, nor one panned away", () => {
    expect(newlyCovered({ x1: 20, y1: 100, x2: 200, y2: 140 }, was, now, sheet)).toBe(false);
    expect(newlyCovered({ x1: 20, y1: 450, x2: 200, y2: 490 }, was, now, sheet)).toBe(false);
    expect(newlyCovered({ x1: -400, y1: 100, x2: -200, y2: 140 }, was, now, sheet)).toBe(false);
  });

  it("is one a window turned hides, with no panel open", () => {
    expect(
      newlyCovered(
        { x1: 500, y1: 100, x2: 600, y2: 140 },
        { width: 900, height: 400 },
        { width: 400, height: 900 },
        null,
      ),
    ).toBe(true);
  });
});

describe("uncovered", () => {
  it("leaves the part above a sheet on a phone, left of a pane on a laptop, or everything", () => {
    expect(uncovered(384, 700, { x1: 0, y1: 400, x2: 384, y2: 700 })).toEqual({ x1: 0, y1: 0, x2: 384, y2: 400 });
    expect(uncovered(1200, 800, { x1: 880, y1: 0, x2: 1200, y2: 800 })).toEqual({ x1: 0, y1: 0, x2: 880, y2: 800 });
    expect(uncovered(384, 700, null)).toEqual({ x1: 0, y1: 0, x2: 384, y2: 700 });
  });
});

describe("firstView", () => {
  const phone = { width: 384, height: 600 };

  it("shows a plan that fits at a readable zoom whole and centred, zooming in no further than the most", () => {
    expect(firstView({ width: 352, height: 284 }, phone, 16, 2)).toEqual({ zoom: 1, pan: { x: 16, y: 158 } });
    expect(firstView({ width: 100, height: 100 }, phone, 16, 2)).toEqual({ zoom: 2, pan: { x: 92, y: 200 } });
  });

  it("opens a plan too big to read whole at its top-left corner, at the furthest readable zoom", () => {
    expect(firstView({ width: 4000, height: 3000 }, phone, 16, 2)).toEqual({ zoom: READABLE, pan: { x: 16, y: 16 } });
    // Just readable whole: still whole.
    const edge = (phone.width - 32) / READABLE;
    expect(firstView({ width: edge, height: 10 }, phone, 16, 2).zoom).toBeCloseTo(READABLE);
  });

  it("opens no further out than the least zoom asked for, as 1:1 on a wide screen", () => {
    const desk = { width: 1200, height: 800 };
    expect(firstView({ width: 4000, height: 3000 }, desk, 16, 2, { x: 100, y: 0 }, 1)).toEqual({
      zoom: 1,
      pan: { x: -84, y: 16 },
    });
    // One that fits at 0.9 would open whole at a readable zoom, but not at 1:1.
    expect(firstView({ width: 1168 / 0.9, height: 100 }, desk, 16, 2, { x: 0, y: 0 }, 1).zoom).toBe(1);
  });

  it("opens a plan too big to read whole on its first item, not on the empty corner of a box", () => {
    const start = firstOf([
      { x: 300, y: 900 },
      { x: 500, y: 400 },
      { x: 200, y: 400 },
    ]);
    expect(start).toEqual({ x: 200, y: 400 });
    // A node a few pixels higher, to the right, is in the same row.
    expect(
      firstOf([
        { x: 500, y: 396 },
        { x: 200, y: 400 },
      ]),
    ).toEqual({ x: 200, y: 400 });
    expect(firstView({ width: 4000, height: 3000 }, phone, 16, 2, start)).toEqual({
      zoom: READABLE,
      pan: { x: 16 - 200 * READABLE, y: 16 - 400 * READABLE },
    });
    expect(firstOf([])).toBeUndefined();
  });
});

describe("startOf", () => {
  const box = (x: number, y: number, width: number, height: number) => ({ x, y, width, height, box: true, top: true });
  const item = (x: number, y: number, width = 200) => ({ x, y, width, height: 48, box: false, top: false });
  // 400 wide, 16 aside, at zoom 1: 368 of the plan's width shows.
  const start = (...nodes: Parameters<typeof startOf>[0]) => startOf(nodes, 400, 16, 1);

  it("starts at the first box's corner, with its title, while its first item shows whole there", () => {
    expect(start(box(0, 0, 1000, 800), item(12, 40), box(1100, 0, 400, 400), item(1110, 20))).toEqual({ x: 0, y: 0 });
  });

  it("moves right only as far as shows the first item whole, or to the item when it is wider than the screen", () => {
    expect(start(box(0, 0, 1000, 800), item(500, 40))).toEqual({ x: 332, y: 0 });
    expect(start(box(0, 0, 1200, 800), item(500, 40, 600))).toEqual({ x: 500, y: 0 });
  });

  it("starts at the top of a first box taller than the screen, not partway down it", () => {
    expect(start(box(0, 0, 300, 5000), item(12, 40), box(400, 0, 300, 200))).toEqual({ x: 0, y: 0 });
  });

  it("starts at a top-level task's own corner, and nowhere without nodes", () => {
    expect(start({ ...item(0, 0), top: true })).toEqual({ x: 0, y: 0 });
    expect(start()).toBeUndefined();
  });
});

describe("nearest", () => {
  const open = { x1: 0, y1: 0, x2: 400, y2: 400 };
  const at = (x: number, y: number) => ({ x1: x, y1: y, x2: x + 50, y2: y + 20 });

  it("brings in nothing when one of the nodes shows, partly or whole, nor when there are none", () => {
    expect(nearest([at(380, 10), at(900, 900)], open)).toBeNull();
    expect(nearest([], open)).toBeNull();
  });

  it("else brings in the one nearest the middle of what shows, which moves the Map least", () => {
    expect(nearest([at(-900, 180), at(500, 180), at(180, 2000)], open)).toEqual(at(500, 180));
  });
});

describe("frame", () => {
  const laptop = { x1: 0, y1: 0, x2: 880, y2: 800 };
  const big = { x1: 0, y1: 0, x2: 4000, y2: 3000 };

  it("centres a thread that fits in the part left open, zooming in no closer than the most", () => {
    const boxes = [
      { x1: 1100, y1: 1100, x2: 1300, y2: 1148 },
      { x1: 1500, y1: 1300, x2: 1700, y2: 1348 },
    ];
    // 600 by 248 fits 880 by 800 at more than 1, so it stays at 1, centred in the part left of the pane.
    expect(frame(boxes, big, laptop, 1)).toEqual({ zoom: 1, pan: { x: 140 - 1100, y: 276 - 1100 } });
    // Wider than the open part at 1: zoomed out to fit, still readable.
    const wide = [{ x1: 1000, y1: 1000, x2: 1920, y2: 1048 }];
    expect(frame(wide, big, laptop, 1)?.zoom).toBeCloseTo((880 - 48) / 920);
  });

  it("moves a thread near the plan's edge off the centre, rather than show blank canvas beyond the plan", () => {
    // Near the top-left corner, centring would leave 176 px of nothing above the plan and 40 px left of it.
    const corner = [{ x1: 100, y1: 100, x2: 700, y2: 348 }];
    expect(frame(corner, big, laptop, 1)).toEqual({ zoom: 1, pan: { x: 24, y: 24 } });
    // Near the bottom, the plan's bottom edge stops at the open part's.
    const bottom = [{ x1: 1100, y1: 2900, x2: 1700, y2: 2948 }];
    expect(frame(bottom, big, laptop, 1)?.pan.y).toBe(800 - 24 - 3000);
    // A plan smaller than the open part stays centred on the thread.
    expect(frame(corner, { x1: 0, y1: 0, x2: 720, y2: 400 }, laptop, 1)?.pan).toEqual({ x: 40, y: 176 });
  });

  it("starts at the first in reading order, at a readable zoom, when the thread is too spread out to fit", () => {
    const boxes = [
      { x1: 3000, y1: 900, x2: 3200, y2: 948 },
      { x1: 400, y1: 904, x2: 600, y2: 952 },
      { x1: 100, y1: 2000, x2: 300, y2: 2048 },
    ];
    // The tops at 900 and 904 read as one row, so the one at 400, its leftmost, comes first.
    expect(frame(boxes, big, laptop, 1)).toEqual({
      zoom: READABLE,
      pan: { x: 24 - 400 * READABLE, y: 24 - 904 * READABLE },
    });
    // Starting at a first item near the plan's right edge, it moves only as far as the plan goes, the item in view.
    const edge = [
      { x1: 1900, y1: 100, x2: 2000, y2: 148 },
      { x1: 100, y1: 2900, x2: 300, y2: 2948 },
    ];
    const view = frame(edge, { x1: 0, y1: 0, x2: 2000, y2: 3000 }, laptop, 1);
    expect(view?.pan.x).toBeCloseTo(880 - 24 - 2000 * READABLE);
    expect(1900 * READABLE + (view?.pan.x ?? 0)).toBeGreaterThan(0);
  });

  it("frames in the part above a phone's sheet, and frames nothing when the thread has nothing drawn", () => {
    const phone = uncovered(384, 700, { x1: 0, y1: 400, x2: 384, y2: 700 });
    const view = frame([{ x1: 1000, y1: 1000, x2: 1200, y2: 1048 }], big, phone, 1);
    expect(view).toEqual({ zoom: 1, pan: { x: 92 - 1000, y: 176 - 1000 } });
    expect(frame([], big, phone, 1)).toBeNull();
  });
});

describe("headed", () => {
  const outer = { x1: 0, y1: 0, x2: 1000, y2: 800 };
  const inner = { x1: 100, y1: 100, x2: 600, y2: 500 };
  const item = { x1: 200, y1: 300, x2: 300, y2: 340 };

  it("adds the title corner of the innermost box an item is in", () => {
    expect(headed([item], [outer, inner], 52)).toEqual([item, { x1: 100, y1: 100, x2: 100, y2: 152 }]);
  });

  it("adds nothing for an item in no box, and doesn't take a box for its own holder", () => {
    expect(headed([inner], [inner], 52)).toEqual([inner]);
    expect(headed([{ x1: 2000, y1: 0, x2: 2100, y2: 40 }], [outer], 52)).toHaveLength(1);
  });
});

describe("framesAll", () => {
  const open = { x1: 0, y1: 0, x2: 400, y2: 600 };

  it("holds when everything fits at a readable zoom, and not when it would need to zoom out further", () => {
    expect(framesAll([{ x1: 0, y1: 0, x2: 300, y2: 300 }], open)).toBe(true);
    expect(framesAll([{ x1: 0, y1: 0, x2: 1000, y2: 300 }], open)).toBe(false);
    expect(framesAll([], open)).toBe(false);
  });
});
