import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import type { Stage } from "../api";
import { linesOf } from "../lines";
import { PALETTES, type Palette } from "../palette";
import { markOf } from "./marks";
import { closedOf, hatchOf, openOf, styleFor } from "./style";

const [OPEN, CLOSED, HATCH] = [openOf, closedOf, hatchOf].map((of) => of(PALETTES.dark));

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
        pstyle(name: string): { value: unknown };
      };
      return {
        images: one.pstyle("background-image").value,
        border: one.pstyle("border-style").value,
        align: one.pstyle("text-halign").value,
      };
    };
    expect(drawn("leaf")).toEqual({ images: [MARK], border: "solid", align: "left-inside" });
    expect(drawn("leaf archived")).toEqual({ images: [HATCH, MARK], border: "solid", align: "left-inside" });
    expect(drawn("box")).toEqual({ images: [OPEN], border: "solid", align: "left-inside" });
    expect(drawn("box archived")).toEqual({ images: [OPEN], border: "dashed", align: "left-inside" });
    expect(drawn("collapsed")).toEqual({ images: [CLOSED], border: "solid", align: "left-inside" });
    expect(drawn("collapsed archived")).toEqual({ images: [HATCH, CLOSED], border: "solid", align: "left-inside" });
    // Lit or selected, an open archived box keeps its dash.
    expect(drawn("box archived lit selected").border).toBe("dashed");
  });
});

describe("a scheme's drawing", () => {
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
      return {
        text:
          JSON.stringify(styleFor(p), (_, value) => (typeof value === "function" ? String(value) : value)) +
          marks.join("") +
          [openOf(p), closedOf(p), hatchOf(p)].map(decodeURIComponent).join(""),
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
