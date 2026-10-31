import { describe, expect, it } from "vitest";
import { firstOf, firstView, nearest, READABLE, reveal, startOf, uncovered } from "./reveal";

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
