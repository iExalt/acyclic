import { describe, expect, it } from "vitest";
import type { Stage } from "../api";
import { PALETTES } from "../palette";
import { onChevron } from "./chevron";
import { item } from "./fixture";
import { boxMarkSvg, markOf, markSvg } from "./marks";
import { buildModel, fallbackStage, visible } from "./model";

const thread = { colour: "#4ec695", ink: "#141a1f" };
const DARK = PALETTES.dark;
const STAGES: Stage[] = ["done", "in-progress", "ready", "blocked", "parked", "unthreaded"];

describe("markSvg", () => {
  it("draws a different icon for every stage, with the thread's stripe and dot", () => {
    const icons = STAGES.map((stage) => markSvg(stage, thread, 200, 48, DARK).match(/<g [^>]*>(.*?)<\/g>/)?.[1]);
    expect(new Set(icons).size).toBe(STAGES.length);
    const mark = markSvg("ready", thread, 200, 48, DARK);
    expect(mark).toContain('<rect width="6" height="48" fill="#4ec695"/>');
    // The dot leads, after the stripe, as a thread's row in the sidebar does; the stage's icon ends the tile.
    expect(mark).toContain('<circle cx="24" cy="24" r="5" fill="#4ec695"/>');
    expect(mark).toContain(`translate(${200 - 12 - 16} 16)`);
  });

  it("draws a box's chevron at its title strip's left and where its work stands at its right", () => {
    const open = boxMarkSvg("box", "blocked", 300, 400, DARK);
    // Over the strip alone, however tall the box.
    expect(open).toContain('height="52"');
    expect(open).toContain('d="M2 4l4 4 4-4"');
    expect(open).toContain("translate(32 18)");
    const closed = boxMarkSvg("collapsed", null, 300, 64, DARK);
    expect(closed).toContain('d="M4 2l4 4-4 4"');
    expect(closed).not.toContain("<g ");
  });

  it("keeps the chevron inside its hit area when a full box header grows", () => {
    const svg = boxMarkSvg("box", "blocked", 300, 400, DARK, null, 100);
    expect(svg).toContain('height="100"');
    expect(svg).toContain("translate(12 20)");
    expect(onChevron("box", { x1: 0, y1: 0, x2: 300, y2: 400, header: 100 }, { x: 18, y: 24 }, 1)).toBe(true);
  });

  it("draws a ring where a tile with no thread would have its dot", () => {
    const none = markSvg("unthreaded", null, 200, 48, DARK);
    expect(none).toContain("stroke-dasharray");
    expect(none).not.toContain("<rect");
    // The ring says it once: no stage icon repeats it at the end; a done tile with no thread still ends with its own.
    expect(none).not.toContain("<g ");
    expect(markSvg("done", null, 200, 48, DARK)).toContain("<g ");
  });

  it("leads a box whose tiles all need a thread with the same dashed ring", () => {
    expect(boxMarkSvg("box", "unthreaded", 300, 400, DARK)).toContain('stroke-dasharray="3 2.4"');
  });

  it("makes each image once for each palette", () => {
    expect(markOf("done", thread, 200, 48, DARK)).toBe(markOf("done", { ...thread }, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("ready", thread, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("done", thread, 200, 48, PALETTES.light));
  });
});

describe("a tile's stage", () => {
  it("is core's, or for a parent drawn as a tile, what its own state says", () => {
    expect(["done", "parked", "in-progress", "backlog"].map(fallbackStage)).toEqual([
      "done",
      "parked",
      "in-progress",
      null,
    ]);
    const model = buildModel([
      item("live", null, [], { stage: "blocked" }),
      item("box", null, [], { state: "parked" }),
    ]);
    const stages = Object.fromEntries(
      visible(model, { collapsed: new Set() }).nodes.map((node) => [node.id, node.stage]),
    );
    expect(stages).toEqual({ live: "blocked", box: "parked" });
  });
});
