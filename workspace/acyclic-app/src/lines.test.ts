/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { contrast, inkOn, LINE_COLOURS, lineOf, linesOf, TOKENS } from "./lines";

describe("lineOf", () => {
  it("takes the number a title starts with, in either style, and leaves the rest as the name", () => {
    expect(lineOf({ id: "a", title: "Thread 7: Build it" })).toMatchObject({ number: "7", name: "Build it" });
    expect(lineOf({ id: "b", title: "28. Phone checks" })).toMatchObject({ number: "28", name: "Phone checks" });
    expect(lineOf({ id: "c", title: "7" })).toMatchObject({ number: null, name: "7" });
    expect(lineOf({ id: "d", title: "2.8 Draw the Map" })).toMatchObject({ number: null, name: "2.8 Draw the Map" });
  });

  it("gives a thread without a number no number, and a colour from its id that holds as threads come and go", () => {
    const loose = { id: "thread-errands", title: "Errands" };
    const alone = linesOf([loose]).get(loose.id);
    const among = linesOf([{ id: "x", title: "1. First" }, loose, { id: "y", title: "Other" }]).get(loose.id);
    expect(alone).toMatchObject({ number: null, name: "Errands" });
    expect(among).toEqual(alone);
  });

  it("colours by number, so the same number is the same line whatever the id", () => {
    expect(lineOf({ id: "a", title: "3. One" }).colour).toBe(lineOf({ id: "b", title: "Thread 3: Two" }).colour);
    expect(lineOf({ id: "a", title: "3. One" }).colour).not.toBe(lineOf({ id: "a", title: "4. One" }).colour);
  });
});

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
    for (const colour of LINE_COLOURS) {
      expect(contrast(colour, inkOn(colour)), `badge ${colour}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("draws a lit border at 3:1 against the tile, the box and the ground on either side of it", () => {
    for (const colour of LINE_COLOURS) {
      for (const surface of [TOKENS.tile, TOKENS.box, TOKENS.ground, TOKENS.chrome, TOKENS.sheet]) {
        expect(contrast(colour, surface), `${colour} on ${surface}`).toBeGreaterThanOrEqual(3);
      }
    }
  });

  it("keeps the signal apart from every line, so a crossing link never reads as a thread", () => {
    // Hue, not contrast: amber against orange or yellow has little contrast either way, so neither is a line.
    const hue = (hex: string) => {
      const [r = 0, g = 0, b = 0] = [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16) / 255);
      const max = Math.max(r, g, b);
      const span = max - Math.min(r, g, b);
      const turn = max === r ? ((g - b) / span) % 6 : max === g ? (b - r) / span + 2 : (r - g) / span + 4;
      return (turn * 60 + 360) % 360;
    };
    const signal = hue(TOKENS.signal);
    for (const colour of LINE_COLOURS) {
      const apart = Math.abs(hue(colour) - signal);
      expect(Math.min(apart, 360 - apart), colour).toBeGreaterThanOrEqual(20);
    }
  });
});
