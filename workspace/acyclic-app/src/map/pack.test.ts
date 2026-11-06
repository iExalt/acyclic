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

  it("fills a vertical band above a box pushed down by what it waits on", () => {
    const kids = [box("p", 100, 100), box("a", 100, 300), box("b", 100, 100), box("c", 50, 100)];
    const waits = new Map([
      ["a", ["p"]],
      ["b", ["a"]],
      ["c", ["p"]],
    ]);
    expect(pack(kids, waits, 0.6)).toEqual({
      width: 248,
      height: 448,
      at: {
        p: { x: 12, y: 12 },
        a: { x: 136, y: 12 },
        b: { x: 12, y: 336 },
        c: { x: 12, y: 136 },
      },
    });
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
  const settled = (layout: ReturnType<typeof hold>) => layout ?? { width: NaN, height: NaN, at: {} };
  const sized = (sizes: Record<string, [number, number]>, by = waits) =>
    settled(
      hold(
        was,
        was.children.map((kid) => (kid.id in sizes ? box(kid.id, ...(sizes[kid.id] ?? [0, 0])) : kid)),
        by,
      ),
    );
  const grown = (id: string, width: number, height: number) => sized({ [id]: [width, height] });

  it("keeps every box where it was when none grew", () => {
    expect(sized({}).at).toEqual(was.layout.at);
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
    expect(sized({ e: [200, 300] }, new Map([["c", ["e"]]])).at).toEqual(was.layout.at);
    // Wider, e meets d, which stands on its right from higher up: d makes way, and e stays where it was.
    expect(grown("e", 300, 100).at).toEqual({ ...was.layout.at, d: { x: 560, y: 12 } });
    // b grows after a moves it right, and d makes way for b's new width.
    expect(sized({ a: [300, 100], b: [250, 100] }).at).toEqual({
      ...was.layout.at,
      b: { x: 336, y: 12 },
      d: { x: 610, y: 12 },
    });
  });
});

describe("hold, closing a box", () => {
  // a over b in one column, c beside a, d under c and beside a too, e far on the right, level with a.
  const was = {
    children: [box("a", 200, 300), box("b", 200, 100), box("c", 100, 100), box("d", 100, 100), box("e", 100, 100)],
    layout: {
      width: 0,
      height: 0,
      at: {
        a: { x: 12, y: 12 },
        b: { x: 12, y: 336 },
        c: { x: 236, y: 12 },
        d: { x: 236, y: 136 },
        e: { x: 600, y: 12 },
      },
    },
  };
  const shrunk = (id: string, width: number, height: number, waits = new Map<string, string[]>()) =>
    hold(
      was,
      was.children.map((kid) => (kid.id === id ? box(id, width, height) : kid)),
      waits,
    )?.at ?? {};

  it("moves the box under a closed one up into the room it gave up, and leaves the others", () => {
    // a gives up 200 of height: b, under it, follows to a's new bottom plus the gap.
    expect(shrunk("a", 200, 100)).toEqual({ ...was.layout.at, b: { x: 12, y: 136 } });
    // Never by more than it gave up.
    expect(shrunk("a", 200, 270).b).toEqual({ x: 12, y: 306 });
    // A sliver, under the gap, is no slide at all.
    expect(shrunk("a", 200, 280).b).toEqual(was.layout.at.b);
  });

  it("moves a box beside a closed one left into the width it gave up, not past what stands before it", () => {
    // c, d and e all stand level with a's old footprint, right of it: each follows a's right edge in, by 100.
    expect(shrunk("a", 100, 300)).toEqual({
      ...was.layout.at,
      c: { x: 136, y: 12 },
      d: { x: 136, y: 136 },
      e: { x: 500, y: 12 },
    });
    // d, under c, rises when c closes.
    expect(shrunk("c", 100, 40).d).toEqual({ x: 236, y: 76 });
  });

  it("keeps a box after what it waits on while it moves up", () => {
    // b waits on a, which it stood below: it comes up to a's new bottom and no further.
    expect(shrunk("a", 200, 100, new Map([["b", ["a"]]])).b).toEqual({ x: 12, y: 136 });
    // b waits on d, which stands beside it and lower: b stays after d, below it or level with it on its right.
    const late = shrunk("a", 200, 100, new Map([["b", ["d"]]])).b ?? { x: NaN, y: NaN };
    expect(late.y).toBeGreaterThanOrEqual(136);
    expect(late.x >= 236 + 100 + 24 || late.y >= 136 + 100 + 24).toBe(true);
  });

  it("returns to the packing when the box is its packed size again", () => {
    expect(shrunk("a", 200, 300)).toEqual(was.layout.at);
  });

  const random = (seed: number) => () => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed / 2147483648;
  };

  it("slides only the boxes under or beside what shrank, by no more than it gave up, and never into each other", () => {
    let slid = 0;
    for (let seed = 1; seed <= 300; seed++) {
      const next = random(seed);
      const size = () => 50 + Math.floor(next() * 400);
      const kids = Array.from({ length: 4 + Math.floor(next() * 9) }, (_, n) => box(`k${n}`, size(), size()));
      const waits = new Map(
        kids.map((kid, n) => [kid.id, kids.filter((_, m) => m < n && next() < 0.2).map((k) => k.id)]),
      );
      const packed = pack(kids, waits, 1);
      const shut = kids[Math.floor(next() * kids.length)] ?? box("none", 0, 0);
      const now = kids.map((kid) =>
        kid.id === shut.id ? box(kid.id, Math.ceil(kid.width / 2), Math.ceil(kid.height / 3)) : kid,
      );
      const held = hold({ children: kids, layout: packed }, now, waits);
      if (held === undefined) continue;
      const at = (layout: typeof packed, id: string) => layout.at[id] ?? { x: NaN, y: NaN };
      const [from, to] = [at(packed, shut.id), at(held, shut.id)];
      expect(to, `seed ${seed}: the closed box stays`).toEqual(from);
      const [gaveW, gaveH] = [shut.width - Math.ceil(shut.width / 2), shut.height - Math.ceil(shut.height / 3)];
      for (const [n, a] of now.entries())
        for (const b of now.slice(n + 1))
          expect(
            crowd(
              { ...at(held, a.id), width: a.width, height: a.height },
              { ...at(held, b.id), width: b.width, height: b.height },
            ),
            `seed ${seed}: ${a.id}, ${b.id}`,
          ).toBe(false);
      for (const kid of kids) {
        if (kid.id === shut.id) continue;
        const [was, is] = [at(packed, kid.id), at(held, kid.id)];
        const under = was.y >= from.y + shut.height && was.x < from.x + shut.width && from.x < was.x + kid.width;
        const aside = was.x >= from.x + shut.width && was.y < from.y + shut.height && from.y < was.y + kid.height;
        if (!under) expect(is.y, `seed ${seed}: ${kid.id} not under`).toBeGreaterThanOrEqual(was.y);
        if (!aside) expect(is.x, `seed ${seed}: ${kid.id} not beside`).toBeGreaterThanOrEqual(was.x);
        expect(was.y - is.y, `seed ${seed}: ${kid.id} up`).toBeLessThanOrEqual(gaveH);
        expect(was.x - is.x, `seed ${seed}: ${kid.id} left`).toBeLessThanOrEqual(gaveW);
        if (is.x !== was.x || is.y !== was.y) slid++;
        for (const first of waits.get(kid.id) ?? []) {
          const [a, b] = [at(packed, first), was];
          const wasAfter =
            b.y >= a.y + (kids.find((k) => k.id === first)?.height ?? 0) + 24 ||
            (b.y >= a.y && b.x >= a.x + (kids.find((k) => k.id === first)?.width ?? 0) + 24);
          if (!wasAfter) continue;
          const firstNow = now.find((k) => k.id === first) ?? box(first, 0, 0);
          const [fa, me] = [at(held, first), is];
          expect(
            me.y >= fa.y + firstNow.height + 24 || (me.y >= fa.y && me.x >= fa.x + firstNow.width + 24),
            `seed ${seed}: ${kid.id} after ${first}`,
          ).toBe(true);
        }
      }
      // Grown back, every box is where it was packed.
      expect(hold({ children: kids, layout: packed }, kids, waits)?.at, `seed ${seed}: reopened`).toEqual(
        packed.at,
      );
    }
    expect(slid).toBeGreaterThan(50);
  });
});

describe("hold, over random packings", () => {
  /** A small seeded generator, so a failure replays. */
  const random = (seed: number) => () => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed / 2147483648;
  };

  it("keeps boxes clear and after their waits when sizes shrink and grow together", () => {
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
      expect(hold({ children: kids, layout: packed }, kids, waits)?.at, `seed ${seed}: held as packed`).toEqual(
        packed.at,
      );
      const now = kids.map((kid, index) => {
        const delta = size();
        return index % 2 === 0
          ? box(kid.id, Math.max(20, kid.width - delta), Math.max(20, kid.height - delta))
          : box(kid.id, kid.width + delta, kid.height + delta);
      });
      const held = hold({ children: kids, layout: packed }, now, waits);
      // A plan whose moves don't settle packs afresh; none of these should come to that.
      expect(held, `seed ${seed}: settles`).toBeDefined();
      if (held === undefined) continue;
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
