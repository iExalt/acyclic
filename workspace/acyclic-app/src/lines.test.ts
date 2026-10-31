import { describe, expect, it } from "vitest";
import { byNumber, lineColour, lineOf, linesOf } from "./lines";
import { PALETTES } from "./palette";

const DARK = PALETTES.a.dark;

describe("lineOf", () => {
  it("numbers a thread by its id, and takes the same number off the front of its title", () => {
    expect(lineOf({ id: "thread-27", title: "Tracker-assigned ids" }, DARK)).toMatchObject({
      number: "27",
      name: "Tracker-assigned ids",
    });
    expect(lineOf({ id: "thread-28", title: "28. Phone checks" }, DARK)).toMatchObject({
      number: "28",
      name: "Phone checks",
    });
    expect(lineOf({ id: "thread-mac-3fa9c1-32", title: "Beds" }, DARK)).toMatchObject({ number: "32", name: "Beds" });
    expect(lineOf({ id: "thread-007", title: "Deploy" }, DARK)).toMatchObject({ number: "7" });
    // A number in the title that isn't the thread's own stays in its name.
    expect(lineOf({ id: "thread-5", title: "2.8 Draw the Map" }, DARK)).toMatchObject({ name: "2.8 Draw the Map" });
    expect(lineOf({ id: "thread-5", title: "7. Build it" }, DARK)).toMatchObject({ number: "5", name: "7. Build it" });
  });

  it("gives a thread whose id has no number a dot, and a colour from its id that holds as threads come and go", () => {
    const loose = { id: "errands", title: "3. Errands" };
    const alone = linesOf([loose], DARK).get(loose.id);
    const among = linesOf([{ id: "thread-1", title: "First" }, loose, { id: "y", title: "Other" }], DARK).get(loose.id);
    expect(alone).toMatchObject({ number: null, name: "3. Errands" });
    expect(among).toEqual(alone);
  });

  it("colours by number, so the same number is the same colour whichever clone gave it", () => {
    expect(lineOf({ id: "thread-3", title: "One" }, DARK).colour).toBe(
      lineOf({ id: "thread-pi-3", title: "Two" }, DARK).colour,
    );
    expect(lineOf({ id: "thread-3", title: "One" }, DARK).colour).not.toBe(
      lineOf({ id: "thread-4", title: "One" }, DARK).colour,
    );
  });

  it("tells clashing numbers apart by clone, by initial unless two clones share one", () => {
    const numbers = (ids: string[]) =>
      [
        ...linesOf(
          ids.map((id) => ({ id, title: id })),
          DARK,
        ).values(),
      ].map((line) => line.number);
    expect(numbers(["thread-mac-32", "thread-pi-32", "thread-mac-33"])).toEqual(["32m", "32p", "33"]);
    expect(numbers(["thread-27", "thread-x-27"])).toEqual(["27", "27x"]);
    expect(numbers(["thread-mac-9", "thread-mac-3fa9c1-9"])).toEqual(["9mac", "9mac-3fa9c1"]);
  });
});

describe("byNumber", () => {
  it("lists threads by number, then by title, with those without a number last", () => {
    const threads = ["thread-mac-32", "loose", "thread-10", "thread-9", "another"].map((id) => ({ id, title: id }));
    expect(threads.sort(byNumber).map((one) => one.id)).toEqual([
      "thread-9",
      "thread-10",
      "thread-mac-32",
      "another",
      "loose",
    ]);
  });
});

describe.each(["dark", "light"] as const)("lineColour in the %s scheme", (scheme) => {
  const palette = PALETTES.a[scheme];
  // Every line's colour: the spiral starts again after 108 threads, so these are all the colours a line has.
  const LINES = Array.from({ length: 108 }, (_, at) => lineColour(at, palette));
  it("walks the hue wheel by number, so neighbouring threads look alike but never the same", () => {
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
