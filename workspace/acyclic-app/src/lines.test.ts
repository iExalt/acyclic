import { describe, expect, it } from "vitest";
import { lineStyle } from "./Badge";
import { byTitle, lineColour, lineOf, linesOf } from "./lines";
import { PALETTES } from "./palette";

const DARK = PALETTES.dark;

describe("lineOf", () => {
  it("colours a thread from its id, so the colour holds as threads come and go", () => {
    const loose = { id: "01900000-0000-7000-8000-0000000000f1", title: "Errands" };
    const alone = linesOf([loose], DARK).get(loose.id);
    const among = linesOf([{ id: "x", title: "First" }, loose, { id: "y", title: "Other" }], DARK).get(loose.id);
    expect(alone).toMatchObject({ name: "Errands" });
    expect(among).toEqual(alone);
    expect(lineOf({ id: "x", title: "Errands" }, DARK).colour).not.toBe(alone?.colour);
  });
});

describe("a thread of one", () => {
  it("carries its task, and a thread of its own none", () => {
    const lines = linesOf(
      [
        { id: "a.one", title: "Water the beds", task: "a" },
        { id: "t", title: "Garden", task: null },
        { id: "u", title: "Old shape" },
      ],
      DARK,
    );
    expect(lines.get("a.one")?.task).toBe("a");
    expect(lines.get("t")?.task).toBeNull();
    expect(lines.get("u")?.task).toBeNull();
  });

  it("is no thread's line, so it takes none of a line's colour into its row or tile", () => {
    const [task, real] = ["a.one", "t"].map((id) =>
      lineOf({ id, title: "X", task: id === "a.one" ? "a" : null }, DARK),
    );
    expect(lineStyle(task)).toBeUndefined();
    expect(lineStyle(real)).toEqual({ "--line": real?.colour, "--line-ink": real?.ink });
  });
});

describe("byTitle", () => {
  it("lists threads by title, numbers in it in order, then by id", () => {
    const threads = [
      { id: "c", title: "Beds 10" },
      { id: "b", title: "Beds 9" },
      { id: "z", title: "Alpha" },
      { id: "a", title: "Beds 10" },
    ];
    expect(threads.sort(byTitle).map((one) => one.id)).toEqual(["z", "b", "a", "c"]);
  });
});

describe.each(["dark", "light"] as const)("lineColour in the %s scheme", (scheme) => {
  const palette = PALETTES[scheme];
  // Every line's colour: the spiral starts again after 108 threads, so these are all the colours a line has.
  const LINES = Array.from({ length: 108 }, (_, at) => lineColour(at, palette));
  it("walks the hue wheel by position, so neighbouring threads look alike but never the same", () => {
    for (const [at, colour] of LINES.entries()) {
      expect(colour, String(at)).toMatch(/^#[0-9a-f]{6}$/);
      if (at > 0) expect(distance(colour, LINES[at - 1] ?? colour), String(at)).toBeGreaterThan(8);
    }
    expect(distance(lineColour(7, palette), lineColour(8, palette))).toBeLessThan(
      distance(lineColour(7, palette), lineColour(20, palette)),
    );
  });

  it("spirals, so no two lines are as alike as neighbours may be until it starts again", () => {
    for (const [at, colour] of LINES.entries()) {
      for (const [other, again] of LINES.slice(at + 1).entries()) {
        expect(distance(colour, again), `${at} and ${at + 1 + other}`).toBeGreaterThan(8);
      }
    }
    expect(lineColour(108, palette)).toBe(lineColour(0, palette));
    expect(lineColour(2 ** 32 - 1, palette)).toBe(lineColour((2 ** 32 - 1) % 108, palette));
  });
});

/** The distance between two `#RRGGBB` colours in 8-bit RGB: a floor against two lines being the same colour. */
function distance(a: string, b: string): number {
  const channels = (hex: string) => [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16));
  const [x, y] = [channels(a), channels(b)];
  return Math.hypot(...x.map((value, at) => value - (y[at] ?? 0)));
}
