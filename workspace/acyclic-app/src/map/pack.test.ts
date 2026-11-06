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

  const rect = (layout: ReturnType<typeof pack>, list: { id: string; width: number; height: number }[], id: string) => {
    const at = layout.at[id] ?? { x: NaN, y: NaN };
    const kid = list.find((one) => one.id === id) ?? box(id, NaN, NaN);
    return { ...at, width: kid.width, height: kid.height };
  };
  const standsAfter = (a: ReturnType<typeof rect>, b: typeof a) =>
    b.y >= a.y + a.height + 24 || (b.y >= a.y && b.x >= a.x + a.width + 24);

  /** What changes the set of boxes or links, over a packed plan: sizes stay as they were. */
  it("keeps boxes clear, after their waits and, but for what waits on a new link, where they were, when boxes or links change", () => {
    let settled = 0;
    for (let seed = 1; seed <= 300; seed++) {
      const next = random(seed);
      const size = () => 50 + Math.floor(next() * 400);
      const kids = Array.from({ length: 4 + Math.floor(next() * 9) }, (_, n) => box(`k${n}`, size(), size()));
      const waits = new Map(
        kids.map((kid, n) => [kid.id, kids.filter((_, m) => m < n && next() < 0.2).map((k) => k.id)]),
      );
      const packed = pack(kids, waits, 1);
      const was = { children: kids, layout: packed, waits };
      const check = (what: string, now: typeof kids, by: Map<string, string[]>, moves: (id: string) => boolean) => {
        const held = hold(was, now, by);
        // A plan whose moves don't settle packs afresh, which is not held.
        if (held === undefined) return;
        settled++;
        for (const [n, a] of now.entries())
          for (const b of now.slice(n + 1))
            expect(crowd(rect(held, now, a.id), rect(held, now, b.id)), `seed ${seed} ${what}: ${a.id}, ${b.id}`).toBe(
              false,
            );
        for (const [id, firsts] of by)
          for (const first of firsts) {
            const wasAfter = standsAfter(rect(packed, kids, first), rect(packed, kids, id));
            const isNew = !(waits.get(id) ?? []).includes(first);
            if (wasAfter || isNew)
              expect(
                standsAfter(rect(held, now, first), rect(held, now, id)),
                `seed ${seed} ${what}: ${id} after ${first}`,
              ).toBe(true);
          }
        for (const kid of kids)
          if (now.some((one) => one.id === kid.id) && !moves(kid.id))
            expect(held.at[kid.id], `seed ${seed} ${what}: ${kid.id} stays`).toEqual(packed.at[kid.id]);
      };
      const downstream = (from: string[], by: Map<string, string[]>) => {
        const out = new Set(from);
        for (let grew = true; grew; ) {
          grew = false;
          for (const [id, ones] of by)
            if (!out.has(id) && ones.some((one) => out.has(one))) {
              out.add(id);
              grew = true;
            }
        }
        return out;
      };
      // A box added, waiting on some of the others, and now and then waited on by one.
      const added = box("new", size(), size());
      const waiters = kids.filter(() => next() < 0.1).map((k) => k.id);
      // Not on what waits on it, which would make a ring.
      const below = downstream(waiters, waits);
      const firsts = kids.filter((k) => next() < 0.3 && !below.has(k.id)).map((k) => k.id);
      const withNew = new Map(waits);
      withNew.set("new", firsts);
      for (const id of waiters) withNew.set(id, [...(withNew.get(id) ?? []), "new"]);
      const unmoved = downstream(waiters, withNew);
      check("add", [...kids, added], withNew, (id) => unmoved.has(id));
      // A box removed, with the waits through it dropped.
      const gone = kids[Math.floor(next() * kids.length)]?.id ?? "";
      const without = new Map(
        [...waits].filter(([id]) => id !== gone).map(([id, ones]) => [id, ones.filter((one) => one !== gone)]),
      );
      check(
        "remove",
        kids.filter((kid) => kid.id !== gone),
        without,
        () => false,
      );
      // A link added from a later box to an earlier one's dependent: the dependent, and what waits on it, may move.
      const [first, then] = [kids[1], kids[kids.length - 1]];
      if (first !== undefined && then !== undefined && first.id !== then.id) {
        const linked = new Map(waits);
        linked.set(then.id, [...(linked.get(then.id) ?? []).filter((one) => one !== first.id), first.id]);
        const moves = downstream([then.id], linked);
        check("link", kids, linked, (id) => moves.has(id));
      }
    }
    // Most plans hold; the rest fall back, so this doesn't pass on a hold that never returns.
    expect(settled).toBeGreaterThan(600);
  });
});
