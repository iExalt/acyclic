/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { byNumber, contrast, inkOn, lineColour, lineOf, linesOf, TOKENS } from "./lines";

/** Every line's colour: the spiral starts again after 108 threads, so these are all the colours a line has. */
const LINES = Array.from({ length: 108 }, (_, at) => lineColour(at));

describe("lineOf", () => {
  it("numbers a thread by its id, and takes the same number off the front of its title", () => {
    expect(lineOf({ id: "thread-27", title: "Tracker-assigned ids" })).toMatchObject({
      number: "27",
      name: "Tracker-assigned ids",
    });
    expect(lineOf({ id: "thread-28", title: "28. Phone checks" })).toMatchObject({
      number: "28",
      name: "Phone checks",
    });
    expect(lineOf({ id: "thread-mac-3fa9c1-32", title: "Beds" })).toMatchObject({ number: "32", name: "Beds" });
    expect(lineOf({ id: "thread-007", title: "Deploy" })).toMatchObject({ number: "7" });
    // A number in the title that isn't the thread's own stays in its name.
    expect(lineOf({ id: "thread-5", title: "2.8 Draw the Map" })).toMatchObject({ name: "2.8 Draw the Map" });
    expect(lineOf({ id: "thread-5", title: "7. Build it" })).toMatchObject({ number: "5", name: "7. Build it" });
  });

  it("gives a thread whose id has no number a dot, and a colour from its id that holds as threads come and go", () => {
    const loose = { id: "errands", title: "3. Errands" };
    const alone = linesOf([loose]).get(loose.id);
    const among = linesOf([{ id: "thread-1", title: "First" }, loose, { id: "y", title: "Other" }]).get(loose.id);
    expect(alone).toMatchObject({ number: null, name: "3. Errands" });
    expect(among).toEqual(alone);
  });

  it("colours by number, so the same number is the same colour whichever clone gave it", () => {
    expect(lineOf({ id: "thread-3", title: "One" }).colour).toBe(lineOf({ id: "thread-pi-3", title: "Two" }).colour);
    expect(lineOf({ id: "thread-3", title: "One" }).colour).not.toBe(lineOf({ id: "thread-4", title: "One" }).colour);
  });

  it("tells clashing numbers apart by clone, by initial unless two clones share one", () => {
    const numbers = (ids: string[]) =>
      [...linesOf(ids.map((id) => ({ id, title: id }))).values()].map((line) => line.number);
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

describe("lineColour", () => {
  it("walks the hue wheel by number, so neighbouring threads look alike but never the same", () => {
    for (const [at, colour] of LINES.entries()) {
      expect(colour, String(at)).toMatch(/^#[0-9a-f]{6}$/);
      if (at > 0) expect(distance(colour, LINES[at - 1] ?? colour), String(at)).toBeGreaterThan(8);
    }
    expect(distance(lineColour(7), lineColour(8))).toBeLessThan(distance(lineColour(7), lineColour(20)));
  });

  it("spirals, so no two lines are as alike as neighbours may be until it starts again", () => {
    for (const [at, colour] of LINES.entries()) {
      for (const [other, again] of LINES.slice(at + 1).entries()) {
        expect(distance(colour, again), `${at} and ${at + 1 + other}`).toBeGreaterThan(8);
      }
    }
    expect(lineColour(108)).toBe(lineColour(0));
    expect(lineColour(2 ** 32 - 1)).toBe(lineColour((2 ** 32 - 1) % 108));
  });
});

/** The distance between two `#RRGGBB` colours in 8-bit RGB: a floor against two lines being the same colour. */
function distance(a: string, b: string): number {
  const channels = (hex: string) => [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16));
  const [x, y] = [channels(a), channels(b)];
  return Math.hypot(...x.map((value, at) => value - (y[at] ?? 0)));
}

describe("the palette", () => {
  it("is the one app.css uses", () => {
    const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
    const declared = Object.fromEntries(
      [...css.matchAll(/--([a-z]+):\s*(#[0-9A-Fa-f]{6});/g)].map((m) => [m[1], m[2]]),
    );
    for (const [name, value] of Object.entries(TOKENS)) expect(declared[name], name).toBe(value);
  });

  it("reads: text at 4.5:1 on every surface it sits on, and badges in their ink", () => {
    for (const surface of [TOKENS.ground, TOKENS.chrome, TOKENS.sheet, TOKENS.tile, TOKENS.box]) {
      for (const text of [TOKENS.ink, TOKENS.muted, TOKENS.signal]) {
        expect(contrast(text, surface), `${text} on ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
    for (const colour of LINES) {
      expect(contrast(colour, inkOn(colour)), `badge ${colour}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("draws a lit border at 3:1 against the tile, the box and the ground on either side of it", () => {
    for (const colour of LINES) {
      for (const surface of [TOKENS.tile, TOKENS.box, TOKENS.ground, TOKENS.chrome, TOKENS.sheet]) {
        expect(contrast(colour, surface), `${colour} on ${surface}`).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
