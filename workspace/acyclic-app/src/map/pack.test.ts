import { describe, expect, it } from "vitest";
import { pack } from "./pack";

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
