import { describe, expect, it } from "vitest";
import { hold, pack } from "./pack";

const box = (id: string, width: number, height: number) => ({ id, width, height });

/** Whether two placed boxes overlap or stand closer than the gap. */
function crowd(a: { x: number; y: number; width: number; height: number }, b: typeof a) {
  return a.x < b.x + b.width + 24 && b.x < a.x + a.width + 24 && a.y < b.y + b.height + 24 && b.y < a.y + a.height + 24;
}

describe("pack", () => {
  it("fills the space beside a wide box with narrow ones, where rows in order would leave it empty", () => {
    // In order, rows put the wide box alone in a row between the tall one and the narrow ones.
    const kids = [
      box("tall", 300, 600),
      box("wide", 1000, 200),
      box("n1", 300, 180),
      box("n2", 300, 180),
      box("n3", 300, 180),
    ];
    const packed = pack(kids, new Map(), 1);
    const placed = kids.map((kid) => ({ ...kid, ...(packed.at[kid.id] ?? { x: NaN, y: NaN }) }));
    for (const [i, a] of placed.entries()) {
      expect(a.x + a.width).toBeLessThanOrEqual(packed.width);
      expect(a.y + a.height).toBeLessThanOrEqual(packed.height);
      for (const b of placed.slice(i + 1)) expect(crowd(a, b)).toBe(false);
    }
    // The narrow boxes stand beside the tall one, under the wide one: the rectangle is the wide box and the tall one's
    // height, and no taller.
    expect(packed.height).toBe(12 + 200 + 24 + 600 + 12);
    expect(packed.width).toBe(12 + 1000 + 12);
  });

  it("never places a box before what it waits on: below it, or level with it or lower on its right", () => {
    const kids = [box("first", 200, 400), box("then", 900, 100), box("small", 100, 100)];
    const waits = new Map([
      ["then", ["first"]],
      ["small", ["then"]],
    ]);
    const { at } = pack(kids, waits, 1);
    const spot = (id: string) => at[id] ?? { x: NaN, y: NaN };
    const [first, then, small] = [spot("first"), spot("then"), spot("small")];
    const after = (a: typeof first, b: typeof first, wide: number, tall: number) =>
      b.y >= a.y + tall || (b.y >= a.y && b.x >= a.x + wide);
    expect([after(first, then, 200, 400), after(then, small, 900, 100)]).toEqual([true, true]);
  });
});

describe("hold", () => {
  // a, b and d in a row, c under a, and e beside c, waiting on it; at their packed places, 12 in from the corner.
  const was = {
    children: [box("a", 200, 100), box("b", 200, 100), box("c", 200, 100), box("d", 100, 300), box("e", 200, 100)],
    layout: {
      width: 0,
      height: 0,
      at: {
        a: { x: 12, y: 12 },
        b: { x: 236, y: 12 },
        c: { x: 12, y: 136 },
        d: { x: 460, y: 12 },
        e: { x: 236, y: 136 },
      },
    },
  };
  const waits = new Map([["e", ["c"]]]);
  const grown = (id: string, width: number, height: number) =>
    hold(
      was,
      was.children.map((kid) => (kid.id === id ? box(id, width, height) : kid)),
      waits,
    );

  it("keeps every box where it was when none grew", () => {
    expect(hold(was, was.children, waits).at).toEqual(was.layout.at);
    expect(grown("d", 50, 50).at).toEqual(was.layout.at);
  });

  it("moves only what a box that grew covers: right if it stood on its right, down if not, after what it waits on", () => {
    const wider = grown("a", 300, 100);
    // b moves right of a, and d right of b; c and e, under the row, stay.
    expect(wider.at).toEqual({ ...was.layout.at, b: { x: 336, y: 12 }, d: { x: 560, y: 12 } });
    expect(wider.width).toBe(12 + 648 + 12);
    // Taller, a moves c down; e, clear of a, follows c, which it waits on, rather than stand above it.
    expect(grown("a", 200, 200).at).toEqual({ ...was.layout.at, c: { x: 12, y: 236 }, e: { x: 236, y: 236 } });
    // c waiting on e, which it stood before, as packing leaves one of a cycle, isn't moved after it.
    expect(
      hold(
        was,
        was.children.map((kid) => (kid.id === "e" ? box("e", 200, 300) : kid)),
        new Map([["c", ["e"]]]),
      ).at,
    ).toEqual(was.layout.at);
    // Wider, e meets d, which stands on its right from higher up: d makes way, and e stays where it was.
    expect(grown("e", 300, 100).at).toEqual({ ...was.layout.at, d: { x: 560, y: 12 } });
  });
});

describe("hold, over random packings", () => {
  /** A small seeded generator, so a failure replays. */
  const random = (seed: number) => () => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed / 2147483648;
  };

  it("never leaves two boxes crowded, nor a box before what it stood after", () => {
    for (let seed = 1; seed <= 300; seed++) {
      const next = random(seed);
      const size = () => 50 + Math.floor(next() * 400);
      const kids = Array.from({ length: 3 + Math.floor(next() * 10) }, (_, n) => box(`k${n}`, size(), size()));
      // Mostly on earlier boxes, with now and then a later one, which makes a cycle.
      const waits = new Map(
        kids.map((kid, n) => [
          kid.id,
          kids.filter((_, m) => m !== n && next() < (m < n ? 0.2 : 0.05)).map((k) => k.id),
        ]),
      );
      const packed = pack(kids, waits, 1);
      const now = kids.map((kid) => (next() < 0.3 ? box(kid.id, kid.width + size(), kid.height + size()) : kid));
      const held = hold({ children: kids, layout: packed }, now, waits);
      const place = (layout: typeof packed, list: typeof kids, id: string) => {
        const at = layout.at[id] ?? { x: NaN, y: NaN };
        const kid = list.find((one) => one.id === id) ?? box(id, NaN, NaN);
        return { ...at, width: kid.width, height: kid.height };
      };
      const after = (a: ReturnType<typeof place>, b: typeof a) =>
        b.y >= a.y + a.height + 24 ? "below" : b.y >= a.y && b.x >= a.x + a.width + 24 ? "beside" : null;
      const ids = kids.map((kid) => kid.id);
      for (const [n, a] of ids.entries())
        for (const b of ids.slice(n + 1))
          expect(crowd(place(held, now, a), place(held, now, b)), `seed ${seed}: ${a} and ${b}`).toBe(false);
      for (const [id, firsts] of waits)
        for (const first of firsts)
          if (after(place(packed, kids, first), place(packed, kids, id)) !== null)
            expect(
              after(place(held, now, first), place(held, now, id)),
              `seed ${seed}: ${id} after ${first}`,
            ).not.toBeNull();
    }
  });
});
