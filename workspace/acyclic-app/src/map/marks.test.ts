import { describe, expect, it } from "vitest";
import type { Stage } from "../api";
import { PALETTES } from "../palette";
import { onChevron } from "./chevron";
import { item } from "./fixture";
import { textWidth } from "./labelmetrics";
import { BOX_TEXT_LEFT, boxMarkSvg, MARKS_LEFT, MARKS_RIGHT, markOf, markSvg } from "./marks";
import { buildModel, fallbackStage, visible } from "./model";
import { textWidthOf } from "./scene";

const thread = { colour: "#4ec695", ink: "#141a1f" };
const DARK = PALETTES.dark;
const STAGES: Stage[] = ["done", "in-progress", "ready", "stalled", "waiting", "parked", "canceled"];

describe("markSvg", () => {
  it("draws a different icon for every stage, with the thread's stripe alone", () => {
    const icons = STAGES.map((stage) => markSvg(stage, thread, 200, 48, DARK).match(/<g [^>]*>(.*?)<\/g>/)?.[1]);
    expect(new Set(icons).size).toBe(STAGES.length);
    const mark = markSvg("ready", thread, 200, 48, DARK);
    expect(mark).toContain('<rect width="6" height="48" fill="#4ec695"/>');
    expect(mark).not.toContain('<circle cx="24"');
    expect(mark).toContain(`translate(${200 - 12 - 16} 16)`);
  });

  it("draws a box's chevron at its title strip's left and where its work stands at its right", () => {
    const open = boxMarkSvg("box", "waiting", 300, 400, DARK);
    // Over the strip alone, however tall the box.
    expect(open).toContain('height="52"');
    expect(open).toContain('d="M2 4l4 4 4-4"');
    expect(open).toContain("translate(272 18)");
    const closed = boxMarkSvg("collapsed", null, 300, 64, DARK);
    expect(closed).toContain('d="M4 2l4 4-4 4"');
    expect(closed).not.toContain("<g ");
  });

  it("keeps every box stage at the right and clear of reserved space", () => {
    for (const stage of STAGES) {
      for (const width of [200, 300]) {
        const closed = boxMarkSvg("collapsed", stage, width, 100, DARK, thread, 52, 20);
        expect(closed).toContain(`translate(${width - 28} 32)`);
        expect(closed).toContain('<rect width="6" height="100" fill="#4ec695"/>');
        const open = boxMarkSvg("box", stage, width, 400, DARK, null, 100);
        expect(open).toContain(`translate(${width - 28} 18)`);
      }
    }
  });

  it("keeps the chevron inside its hit area when a full box header grows", () => {
    const svg = boxMarkSvg("box", "waiting", 300, 400, DARK, null, 100);
    expect(svg).toContain('height="100"');
    expect(svg).toContain("translate(12 20)");
    expect(onChevron("box", { x1: 0, y1: 0, x2: 300, y2: 400, header: 100 }, { x: 18, y: 24 }, 1)).toBe(true);
  });

  it("draws a threaded item's stripe alone, in grey, for a tile in no thread, a task alone", () => {
    const none = markSvg("ready", null, 200, 48, DARK);
    // The same stripe, same size and place, in the quiet ink rather than a line's colour.
    const same = markSvg("ready", thread, 200, 48, DARK);
    expect(none).toBe(same.replaceAll(thread.colour, DARK.muted));
    expect(none).toContain(`<rect width="6" height="48" fill="${DARK.muted}"/>`);
    expect(none).not.toContain('<circle cx="24"');
    expect(none).not.toContain("stroke-dasharray");
    // The stage's icon still ends the tile: a task in no thread is a thread of one, ready like any other.
    expect(none).toContain("<g ");
  });

  it("keeps measured labels between the left marks and right stage icon", () => {
    for (const kind of ["box", "collapsed"] as const) {
      expect(onChevron(kind, { x1: 0, y1: 0, x2: 200, y2: 100 }, { x: BOX_TEXT_LEFT, y: 24 }, 1)).toBe(false);
    }
    for (const kind of ["leaf", "box", "collapsed"] as const) {
      for (const width of [200, 300]) {
        const left = kind === "leaf" ? MARKS_LEFT : BOX_TEXT_LEFT;
        expect(left + textWidth(kind, width)).toBe(width - MARKS_RIGHT);
        expect(textWidthOf(kind, width)).toBe(textWidth(kind, width));
      }
    }
  });

  it("makes each image once for each palette", () => {
    expect(markOf("done", thread, 200, 48, DARK)).toBe(markOf("done", { ...thread }, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("ready", thread, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("done", thread, 200, 48, PALETTES.light));
  });
});

describe("a tile's stage", () => {
  it("is core's, or for a parent drawn as a tile, what its own state says", () => {
    expect(["done", "parked", "in-progress", "open"].map(fallbackStage)).toEqual([
      "done",
      "parked",
      "in-progress",
      null,
    ]);
    const model = buildModel([
      item("live", null, [], { stage: "waiting" }),
      item("box", null, [], { state: "parked" }),
    ]);
    const stages = Object.fromEntries(
      visible(model, { collapsed: new Set() }).nodes.map((node) => [node.id, node.stage]),
    );
    expect(stages).toEqual({ live: "waiting", box: "parked" });
  });
});
