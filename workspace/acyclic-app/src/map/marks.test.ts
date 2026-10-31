import { describe, expect, it } from "vitest";
import type { Stage } from "../api";
import { PALETTES } from "../palette";
import { item } from "./fixture";
import { boxMarkSvg, markOf, markSvg } from "./marks";
import { buildModel, fallbackStage, visible } from "./model";

const thread = { colour: "#4ec695", number: "31", ink: "#141a1f" };
const DARK = PALETTES.dark;
const STAGES: Stage[] = ["done", "in-review", "in-progress", "ready", "blocked", "parked", "unthreaded"];

describe("markSvg", () => {
  it("draws a different icon for every stage, with the thread's stripe and numbered badge", () => {
    const icons = STAGES.map((stage) => markSvg(stage, thread, 200, 48, DARK).match(/<g [^>]*>(.*?)<\/g>/)?.[1]);
    expect(new Set(icons).size).toBe(STAGES.length);
    const mark = markSvg("ready", thread, 200, 48, DARK);
    expect(mark).toContain('<rect width="6" height="48" fill="#4ec695"/>');
    expect(mark).toContain(">31</text>");
    // The badge leads, after the stripe, as a thread's row in the sidebar does; the stage's icon ends the task.
    expect(mark.indexOf('<rect x="14"')).toBeGreaterThan(-1);
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

  it("widens the badge for a clone's initial, draws a dot for a thread with no number, and a ring for none", () => {
    const pill = (number: string) =>
      Number(markSvg("ready", { ...thread, number }, 200, 48, DARK).match(/width="(\d+)" height="20"/)?.[1]);
    expect(pill("32m")).toBeGreaterThan(pill("3"));
    expect(markSvg("ready", { ...thread, number: null }, 200, 48, DARK)).toContain('r="5" fill="#4ec695"');
    const none = markSvg("unthreaded", null, 200, 48, DARK);
    expect(none).toContain("stroke-dasharray");
    expect(none).not.toContain("<rect");
  });

  it("makes each image once for each palette", () => {
    expect(markOf("done", thread, 200, 48, DARK)).toBe(markOf("done", { ...thread }, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("ready", thread, 200, 48, DARK));
    expect(markOf("done", thread, 200, 48, DARK)).not.toBe(markOf("done", thread, 200, 48, PALETTES.light));
  });
});

describe("a task's stage", () => {
  it("is core's, or for an archived task or a parent drawn as one, what its own state says", () => {
    expect(["done", "parked", "in-progress", "backlog"].map(fallbackStage)).toEqual([
      "done",
      "parked",
      "in-progress",
      null,
    ]);
    const model = buildModel([
      item("live", null, [], { stage: "blocked" }),
      item("old", null, [], { archived: true, state: "done" }),
      item("box", null, [], { state: "parked" }),
      item("gone", "box", [], { archived: true }),
    ]);
    const stages = Object.fromEntries(
      visible(model, { collapsed: new Set(), showArchived: false }).nodes.map((node) => [node.id, node.stage]),
    );
    expect(stages).toEqual({ live: "blocked", box: "parked" });
    const shown = visible(model, { collapsed: new Set(), showArchived: true }).nodes;
    expect(Object.fromEntries(shown.map((node) => [node.id, node.stage]))).toEqual({
      live: "blocked",
      old: "done",
      box: null,
      gone: null,
    });
  });
});
