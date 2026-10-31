/// <reference types="node" />
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { contrast, inkOn, lineColour } from "./lines";
import { PALETTES, type Palette } from "./palette";

const SCHEMES = ["dark", "light"] as const;
const linesOf = (palette: Palette) => Array.from({ length: 108 }, (_, at) => lineColour(at, palette));
/** Every surface something is drawn on, the stages' tints among them. */
const surfaces = (p: Palette) => [p.ground, p.chrome, p.sheet, p.tile, p.box, ...Object.values(p.tints)];

describe.each(SCHEMES)("the %s palette", (scheme) => {
  const p = PALETTES[scheme];
  const LINES = linesOf(p);

  it("is the one app.css declares for its scheme", () => {
    const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");
    // The dark scheme is the first :root block; the light one is inside its media query.
    const block =
      scheme === "dark"
        ? css.slice(css.indexOf(":root {"), css.indexOf("}", css.indexOf(":root {")))
        : css.slice(
            css.indexOf("(prefers-color-scheme: light)"),
            css.indexOf("}", css.indexOf("(prefers-color-scheme: light)")),
          );
    const declared = Object.fromEntries(
      [...block.matchAll(/--([a-z-]+):\s*(#[0-9A-Fa-f]{6});/g)].map((m) => [m[1], m[2]]),
    );
    const named = { ...p, "signal-ink": p.signalInk } as Record<string, unknown>;
    expect(Object.keys(declared).sort()).toEqual(
      [
        "ground",
        "chrome",
        "box",
        "tile",
        "sheet",
        "rule",
        "edge",
        "ink",
        "muted",
        "signal",
        "signal-ink",
        "danger",
      ].sort(),
    );
    for (const [name, value] of Object.entries(declared)) expect(value, name).toBe(named[name]);
    expect(block).toContain(`color-scheme: ${scheme}`);
    const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
    expect(html, "the bar's colour").toContain(
      `<meta name="theme-color" content="${p.chrome}" media="(prefers-color-scheme: ${scheme})" />`,
    );
  });

  it("reads: text at 4.5:1 on every surface, the signal and danger as text too, and badges in their ink", () => {
    for (const surface of surfaces(p)) {
      for (const text of [p.ink, p.muted, p.signal, p.danger]) {
        expect(contrast(text, surface), `${text} on ${surface}`).toBeGreaterThanOrEqual(4.5);
      }
    }
    expect(contrast(p.signalInk, p.signal), "the needs badge's !").toBeGreaterThanOrEqual(4.5);
    for (const colour of LINES) {
      expect(contrast(colour, inkOn(colour, p)), `badge ${colour}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("draws a lit border, a crossing link and the selection at 3:1 against every surface beside them", () => {
    for (const surface of surfaces(p)) {
      for (const colour of [...LINES, p.signal, p.ink]) {
        expect(contrast(colour, surface), `${colour} on ${surface}`).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
