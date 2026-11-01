import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import type { Stage } from "../api";
import { linesOf } from "../lines";
import { PALETTES, type Palette } from "../palette";
import { boxMarkOf, markOf } from "./marks";
import { faded, hatchOf, styleFor } from "./style";

/** A hex colour as Cytoscape gives a parsed one back. */
const rgb = (hex: string) => `rgb(${[1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16)).join(",")})`;
const HATCH = hatchOf(PALETTES.dark);

/** A task's marks, as an image its data carries. */
const MARK = "data:image/svg+xml;utf8,%3Csvg%2F%3E";

describe("style", () => {
  it("draws each kind of box with its chevron, a task with its marks, crosses out what is archived, and dashes an open archived box", () => {
    const core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.dark) });
    const node = (id: string, classes: string) =>
      core.add({
        group: "nodes",
        data: { id, label: id, width: 200, height: 64, textWidth: 150, layer: 0, mark: MARK },
        classes,
      });
    const drawn = (classes: string) => {
      // The parsed style, which Cytoscape's types leave out.
      const one = node(classes.replaceAll(" ", "-"), classes) as unknown as {
        pstyle(name: string): { value: unknown; strValue: string };
      };
      return {
        images: one.pstyle("background-image").value,
        border: one.pstyle("border-style").value,
        align: one.pstyle("text-halign").value,
        opacity: one.pstyle("opacity").value,
        fill: one.pstyle("background-opacity").value,
        colour: one.pstyle("color").strValue,
      };
    };
    const plain = { border: "solid", align: "left-inside", opacity: 1, fill: 1, colour: expect.any(String) };
    expect(drawn("leaf")).toEqual({ ...plain, images: [MARK] });
    expect(drawn("leaf archived")).toEqual({ ...plain, images: [HATCH, MARK] });
    expect(drawn("box")).toEqual({ ...plain, images: [MARK] });
    expect(drawn("box archived")).toEqual({ ...plain, images: [MARK], border: "dashed" });
    expect(drawn("collapsed")).toEqual({ ...plain, images: [MARK] });
    expect(drawn("collapsed archived")).toEqual({ ...plain, images: [HATCH, MARK] });
    // Lit or selected, an open archived box keeps its dash.
    expect(drawn("box archived lit selected").border).toBe("dashed");
    // Outside the thread lit, a task fades, but its title stays at full strength, muted.
    expect(drawn("leaf dim")).toMatchObject({ opacity: 1, fill: 0.35, colour: rgb(PALETTES.dark.muted) });
  });
});

describe("a scheme's drawing", () => {
  it("doesn't darken where a press lands, on the background or on a node", () => {
    const core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.dark) });
    const node = core.add({
      group: "nodes",
      data: { id: "a", label: "a", width: 200, height: 48, textWidth: 150, layer: 0 },
    });
    // Pressed, as a tap marks it; the parsed style too, which Cytoscape's types leave out.
    (node as unknown as { activate(): void }).activate();
    const style = core.style() as unknown as { core(name: string): { value: unknown } };
    expect(style.core("active-bg-opacity").value).toBe(0);
    expect((node as unknown as { pstyle(name: string): { value: unknown } }).pstyle("overlay-opacity").value).toBe(0);
  });

  it("holds only its own palette's colours and lines, whichever scheme was drawn before it", () => {
    const stages = Object.keys(PALETTES.dark.tints) as Stage[];
    const drawing = (p: Palette) => {
      const lines = linesOf(
        [
          { id: "thread-3", title: "Numbered" },
          { id: "thread-40", title: "Another" },
          { id: "loose", title: "Dotted" },
        ],
        p,
      );
      const threads = [...lines.values()].map((line) => ({ colour: line.colour, number: line.number, ink: line.ink }));
      const marks = [null, ...threads].flatMap((thread) =>
        [null, ...stages].map((stage) => decodeURIComponent(markOf(stage, thread, 200, 48, p))),
      );
      const boxes = (["box", "collapsed"] as const).flatMap((kind) =>
        [null, ...stages].map((stage) => decodeURIComponent(boxMarkOf(kind, stage, 200, 120, p))),
      );
      return {
        text:
          JSON.stringify(styleFor(p), (_, value) => (typeof value === "function" ? String(value) : value)) +
          marks.join("") +
          boxes.join("") +
          decodeURIComponent(hatchOf(p)),
        lines,
      };
    };
    const all = [PALETTES.dark, PALETTES.light];
    for (const [at, p] of all.entries()) {
      const then = `${at}`;
      drawing(all[(at + all.length - 1) % all.length] as Palette);
      const { text, lines } = drawing(p);
      const allowed = new Set(
        [
          ...Object.values(p),
          ...Object.values(p.tints),
          ...p.badgeInks,
          ...[...lines.values()].map((line) => line.colour),
          // A dimmed tile's colour, its own faded over its box's.
          ...[...Object.values(p.tints), p.tile].map((colour) => faded(colour, p.box)),
        ]
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.toLowerCase()),
      );
      const found = new Set([...text.matchAll(/#[0-9a-fA-F]{6}\b/g)].map((m) => m[0].toLowerCase()));
      expect(found.size, then).toBeGreaterThan(5);
      for (const colour of found) expect(allowed.has(colour), `${colour} in the ${then} drawing`).toBe(true);
    }
  });
});

describe("faded", () => {
  it("is a colour at 35 % over what is under it, as a dimmed node is drawn", () => {
    expect(faded("#ffffff", "#000000")).toBe("#595959");
    expect(faded("#23282e", "#23282e")).toBe("#23282e");
  });
});
