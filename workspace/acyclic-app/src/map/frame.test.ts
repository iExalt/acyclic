import cytoscape from "cytoscape";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PALETTES } from "../palette";
import { apply, type Drawn, land, MAX_GLIDES, settle, settledBox } from "./draw";
import { busy } from "./motion";
import { styleFor } from "./style";

const view = { reduced: false, extent: null };
const box = (id: string, width: number, height: number, owner: string | null = null): Drawn => ({
  group: "nodes",
  classes: width === 216 ? "collapsed" : "box one-line",
  data: {
    id,
    owner,
    width,
    height,
    layer: owner === null ? 0 : 1,
    header: 48,
    textWidth: 150,
    mark: "none",
    stageMark: "none",
    label: id,
  },
  position: { x: 100 + width / 2, y: 100 + height / 2 },
});
const child = (id = "child", owner = "epic"): Drawn => ({
  group: "nodes",
  classes: "leaf",
  position: { x: 240, y: 220 },
  data: { id, owner, width: 216, height: 48, layer: 2, label: id, textWidth: 150, mark: "none" },
});
const rested = () => vi.waitFor(() => expect(busy()).toBe(0), { timeout: 2000, interval: 5 });

describe("frame-first expansion", () => {
  let core: cytoscape.Core;
  beforeEach(() => {
    core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.light) });
  });
  afterEach(() => {
    settle(core);
    core.destroy();
  });

  it("holds the corner while size and centre tween, then shows contents at full opacity", async () => {
    const was = apply(core, new Map(), [box("epic", 216, 48)]);
    const next = [box("epic", 600, 360), child()];
    const now = apply(core, was, next, view);
    const frame = core.$id("epic");
    expect(core.$id("child").style("visibility")).toBe("hidden");
    await vi.waitFor(
      () => {
        expect(frame.width()).toBeGreaterThan(216);
        expect(frame.width()).toBeLessThan(600);
      },
      { interval: 5 },
    );
    expect(frame.position().x - frame.width() / 2).toBeCloseTo(100);
    expect(frame.position().y - frame.height() / 2).toBeCloseTo(100);
    const finalBounds = settledBox(core, "epic");
    // A selection/hover redraw must not reveal a child or restart its frame.
    apply(
      core,
      now,
      next.map((one) => ({ ...one, classes: `${one.classes} dim` })),
      view,
    );
    expect(core.$id("child").style("visibility")).toBe("hidden");
    await rested();
    expect(frame.width()).toBe(600);
    const landed = frame.renderedBoundingBox({ includeLabels: false });
    for (const key of ["x1", "x2", "y1", "y2", "w", "h"] as const) expect(landed[key]).toBeCloseTo(finalBounds[key]);
    expect(core.$id("child").style("visibility")).toBe("visible");
    expect(Number(core.$id("child").style("opacity"))).toBe(1);
  });

  it("collapses from the drawn size and reverses without jumping or leaving hidden contents", async () => {
    const open = [box("epic", 600, 360), child()];
    const was = apply(core, new Map(), open);
    const shut = apply(core, was, [box("epic", 216, 48)], view);
    expect(core.$id("child").empty()).toBe(true);
    const frame = core.$id("epic");
    await vi.waitFor(
      () => {
        expect(frame.width()).toBeLessThan(600);
        expect(frame.width()).toBeGreaterThan(216);
      },
      { interval: 5 },
    );
    expect(frame.position().x - frame.width() / 2).toBeCloseTo(100);
    expect(frame.position().y - frame.height() / 2).toBeCloseTo(100);
    const before = { position: { ...frame.position() }, width: frame.width(), height: frame.height() };
    apply(core, shut, open, view);
    expect(frame.position()).toEqual(before.position);
    expect(frame.width()).toBe(before.width);
    expect(frame.height()).toBe(before.height);
    await rested();
    expect(core.$id("child").style("visibility")).toBe("visible");
    expect(frame.width()).toBe(600);
  });

  it("waits for every growing ancestor and lands pending contents with the final geometry", async () => {
    const was = apply(core, new Map(), [box("epic", 216, 48)]);
    const opening = apply(core, was, [box("epic", 800, 600), box("inner", 216, 48, "epic")], view);
    await vi.waitFor(() => expect(core.$id("epic").width()).toBeGreaterThan(300), { interval: 5 });
    apply(core, opening, [box("epic", 800, 600), box("inner", 500, 300, "epic"), child("grandchild", "inner")], view);
    await vi.waitFor(
      () => {
        expect(core.$id("epic").width()).toBe(800);
        expect(core.$id("inner").width()).toBeLessThan(500);
      },
      { interval: 5 },
    );
    expect(core.$id("grandchild").style("visibility")).toBe("hidden");
    land(core);
    expect(core.$id("grandchild").style("visibility")).toBe("visible");
    expect(core.$id("inner").width()).toBe(500);
    expect(busy()).toBe(0);
  });

  it.each(["reduced", "offscreen", "cap"])("snaps %s frames with no hidden contents or size overrides", (fallback) => {
    const extras = fallback === "cap" ? Array.from({ length: MAX_GLIDES }, (_, i) => child(`leaf${i}`, "other")) : [];
    const was = apply(core, new Map(), [box("epic", 216, 48), ...extras]);
    apply(
      core,
      was,
      [box("epic", 600, 360), child(), ...extras.map((one) => ({ ...one, position: { x: 300, y: 400 } }))],
      {
        reduced: fallback === "reduced",
        extent: fallback === "offscreen" ? { x1: 5000, x2: 6000, y1: 5000, y2: 6000 } : null,
      },
    );
    expect(core.$id("epic").width()).toBe(600);
    expect(core.$id("child").style("visibility")).toBe("visible");
    expect(busy()).toBe(0);
  });
});
