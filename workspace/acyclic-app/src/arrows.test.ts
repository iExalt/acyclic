import { describe, expect, it } from "vitest";
import {
  type Box,
  boundsOf,
  type Column,
  gapWidths,
  headOf,
  type Point,
  pathOf,
  type Route,
  routes,
  type Wait,
} from "./arrows";

/** Columns as the view lays them out when wide: 220 wide, `gaps` apart (64 by default), a heading, then cards 12
 * apart. */
function layout(heights: number[][], gaps: number[] = heights.map(() => 64)): Column[] {
  let left = 0;
  return heights.map((column, wave) => {
    if (wave > 0) left += 220 + nth(gaps, wave - 1);
    const heading = { left, top: 0, right: left + 180, bottom: 24 };
    let top = 32;
    const cards = column.map((height, at) => {
      const box = { left, top, right: left + 220, bottom: top + height };
      top += height + 12;
      return { id: `w${wave}c${at}`, box };
    });
    return { heading, cards };
  });
}

const forward = (from: string, to: string): Wait => ({ from, to, looped: false });

/** The `at`th of `all`, which the test knows is there. */
function nth<T>(all: readonly T[], at: number): T {
  const one = all[at < 0 ? all.length + at : at];
  if (one === undefined) throw new Error(`nothing at ${at}`);
  return one;
}

const segmentsOf = (points: readonly Point[]): [Point, Point][] =>
  points.slice(1).map((point, at) => [nth(points, at), point]);

/** Every way `found` could mislead: crossing a card or heading, two threads' arrows within 3 pixels along one line,
 * a head with no room, a segment neither level nor upright, two segments one way in a row, or a point outside the
 * bounds the view makes room for. */
function faults(columns: Column[], found: Route[]): string[] {
  const boxes = columns.flatMap((column, wave) => [
    { id: `heading ${wave}`, box: column.heading },
    ...column.cards.map((card) => ({ id: card.id, box: card.box })),
  ]);
  const problems: string[] = [];
  for (const route of found) {
    const segments = segmentsOf(route.points);
    segments.forEach(([a, b], at) => {
      if (a[0] !== b[0] && a[1] !== b[1]) problems.push(`${route.from}->${route.to} has a slanted segment`);
      for (const { id, box } of boxes) {
        const own = (id === route.from && at === 0) || (id === route.to && at === segments.length - 1);
        if (own ? meets(a, b, inside(box, 1), true) : meets(a, b, inflate(box, 2.5), false)) {
          problems.push(`${route.from}->${route.to} crosses ${id}`);
        }
      }
    });
    segments.slice(1).forEach(([a, b], at) => {
      const [c, d] = nth(segments, at);
      if ((a[0] === b[0]) === (c[0] === d[0])) problems.push(`${route.from}->${route.to} doesn't turn at a corner`);
    });
    const bounds = boundsOf(found) as Box;
    if (route.points.some((point) => meets(point, point, inside(bounds, -0.001), true) === false)) {
      problems.push(`${route.from}->${route.to} runs out of its bounds`);
    }
    const [last, end] = nth(segments, -1);
    if (Math.abs(last[0] - end[0]) + Math.abs(last[1] - end[1]) < 9) {
      problems.push(`${route.from}->${route.to} leaves its head no room`);
    }
  }
  for (const one of found) {
    for (const other of found) {
      if (one.from >= other.from) continue;
      if (overlap(one.points, other.points))
        problems.push(`${one.from}->${one.to} runs along ${other.from}->${other.to}`);
    }
  }
  return problems;
}

const inflate = (box: Box, by: number): Box => ({
  left: box.left - by,
  top: box.top - by,
  right: box.right + by,
  bottom: box.bottom + by,
});
const inside = (box: Box, by: number) => inflate(box, -by);

/** Whether the level or upright segment a–b meets `box`; with `open`, only its inside counts. */
function meets(a: Point, b: Point, box: Box, open: boolean): boolean {
  const [x1, x2] = [Math.min(a[0], b[0]), Math.max(a[0], b[0])];
  const [y1, y2] = [Math.min(a[1], b[1]), Math.max(a[1], b[1])];
  return open
    ? x2 > box.left && x1 < box.right && y2 > box.top && y1 < box.bottom
    : x2 >= box.left && x1 <= box.right && y2 >= box.top && y1 <= box.bottom;
}

/** Whether two polylines share a stretch of one line, longer than a point, nearer than 3 pixels apart. */
function overlap(one: Point[], other: Point[]): boolean {
  for (const [a, b] of segmentsOf(one)) {
    for (const [c, d] of segmentsOf(other)) {
      const level = a[1] === b[1] && c[1] === d[1];
      const upright = a[0] === b[0] && c[0] === d[0];
      if (!level && !upright) continue;
      const [axis, across] = level ? ([0, 1] as const) : ([1, 0] as const);
      if (Math.abs(a[across] - c[across]) >= 3) continue;
      const lo = Math.max(Math.min(a[axis], b[axis]), Math.min(c[axis], d[axis]));
      const hi = Math.min(Math.max(a[axis], b[axis]), Math.max(c[axis], d[axis]));
      if (hi - lo > 0.5) return true;
    }
  }
  return false;
}

const bends = (route: Route) => route.points.length - 2;

describe("the Threads view's arrows", () => {
  it("run from one wave to the next with at most two bends", () => {
    const columns = layout([[80, 80], [100]]);
    const found = routes(columns, [forward("w0c0", "w1c0"), forward("w0c1", "w1c0")], null);
    expect(faults(columns, found)).toEqual([]);
    expect(found.map(bends).every((count) => count <= 2)).toBe(true);
    // Each source enters the card at a point of its own.
    expect(new Set(found.map((route) => nth(route.points, -1)[1])).size).toBe(2);
  });

  it("cross a wave they skip between its cards, or below them once that gap is taken", () => {
    const columns = layout([
      [80, 80],
      [80, 80],
      [80, 80],
    ]);
    const one = routes(columns, [forward("w0c0", "w2c1")], null);
    expect(faults(columns, one)).toEqual([]);
    const only = nth(one, 0);
    const across = only.points.filter(([x]) => x > 220 + 64 && x < 2 * 220 + 64);
    expect(across.length).toBe(0);
    const level = segmentsOf(only.points).find(
      ([a, b]) => a[1] === b[1] && a[0] < 220 + 64 && b[0] > 220 + 64 + 220,
    )?.[0];
    expect(level?.[1]).toBeGreaterThan(32 + 80);
    expect(level?.[1]).toBeLessThan(32 + 80 + 12);
    const two = routes(columns, [forward("w0c0", "w2c1"), forward("w0c1", "w2c1")], null);
    expect(faults(columns, two)).toEqual([]);
  });

  it("between threads in one wave run through the gap to its right", () => {
    const columns = layout([[80, 80]]);
    const found = routes(
      columns,
      [
        { from: "w0c0", to: "w0c1", looped: true },
        { from: "w0c1", to: "w0c0", looped: true },
      ],
      220 + 64,
    );
    expect(faults(columns, found)).toEqual([]);
    for (const route of found) {
      expect(Math.max(...route.points.map(([x]) => x))).toBeLessThan(220 + 64 - 7);
      expect(nth(route.points, -1)[0]).toBe(220);
    }
    // The leftmost lane leaves a loop's head and a corner room.
    const crowded = layout([[80, 80, 80]]);
    const loops = routes(
      crowded,
      [
        { from: "w0c0", to: "w0c2", looped: true },
        { from: "w0c1", to: "w0c0", looped: true },
        { from: "w0c2", to: "w0c1", looped: true },
      ],
      220 + 64,
    );
    expect(faults(crowded, loops)).toEqual([]);
    expect(Math.min(...loops.map((route) => nth(route.points, 1)[0] - 220))).toBeGreaterThanOrEqual(11);
  });

  it("widen a gap as crowded as its lanes need, with every lane its full spacing from the next", () => {
    const waves = [["a", "b", "c", "d", "e", "f", "g", "h"], ["z"]];
    const waits = nth(waves, 0).map((from) => forward(from, "z"));
    expect(gapWidths(waves, waits, 64)).toEqual([12 + 12 + 6 * 9, 64]);
    expect(gapWidths(waves, waits.slice(0, 2), 64)).toEqual([64, 64]);
  });

  it("leave out waits between cards not drawn or against the waves' order", () => {
    const columns = layout([[80], [80]]);
    expect(routes(columns, [forward("w1c0", "w0c0"), forward("w0c0", "gone")], null)).toEqual([]);
  });

  it("never cross a card, overlap another thread's or crowd a head, over many layouts", () => {
    // ARROWS_SEED replays another run.
    const first = Number(process.env.ARROWS_SEED ?? 7);
    let seed = first;
    // mulberry32: a small seeded generator, so a failure replays.
    const random = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const pick = (lo: number, hi: number) => lo + Math.floor(random() * (hi - lo + 1));
    for (let trial = 0; trial < 300; trial++) {
      const heights = Array.from({ length: pick(2, 5) }, () => Array.from({ length: pick(1, 6) }, () => pick(60, 140)));
      const ids = layout(heights).flatMap((column, wave) => column.cards.map((card) => ({ id: card.id, wave })));
      const waits: Wait[] = [];
      for (const to of ids) {
        for (const from of ids) {
          if (from.wave < to.wave && random() < 0.25) waits.push(forward(from.id, to.id));
          if (from.wave === to.wave && from.id !== to.id && random() < 0.08) {
            waits.push({ from: from.id, to: to.id, looped: true });
          }
        }
      }
      const widths = gapWidths(
        heights.map((column, wave) => column.map((_, at) => `w${wave}c${at}`)),
        waits,
        64,
      );
      const columns = layout(heights, widths);
      const found = routes(columns, waits, null);
      expect(found.length).toBe(waits.length);
      expect(faults(columns, found), `seed ${first}, trial ${trial}`).toEqual([]);
      // Each gap's lanes stand their full spacing apart.
      for (let gap = 0; gap + 1 < columns.length; gap++) {
        const [left, right] = [nth(nth(columns, gap).cards, 0).box.right, nth(nth(columns, gap + 1).cards, 0).box.left];
        const lanes = [
          ...new Set(found.flatMap((route) => route.points.filter(([x]) => x > left && x < right).map(([x]) => x))),
        ].sort((a, b) => a - b);
        for (let at = 1; at < lanes.length; at++) expect(nth(lanes, at) - nth(lanes, at - 1)).toBeGreaterThanOrEqual(5);
      }
    }
  });
});

describe("an arrow's path and head", () => {
  it("round corners by no more than half a segment and point the head along the last one", () => {
    expect(
      pathOf([
        [0, 0],
        [10, 0],
        [10, 4],
        [20, 4],
      ]),
    ).toBe("M 0 0 L 8 0 Q 10 0 10 2 L 10 2 Q 10 4 12 4 L 20 4");
    expect(
      headOf([
        [0, 0],
        [20, 0],
      ]),
    ).toBe("M 20 0 L 13 4 L 13 -4 z");
    expect(
      headOf([
        [20, 0],
        [0, 0],
      ]),
    ).toBe("M 0 0 L 7 4 L 7 -4 z");
  });
});
