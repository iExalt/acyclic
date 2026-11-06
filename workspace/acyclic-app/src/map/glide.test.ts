import cytoscape from "cytoscape";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rested = () => vi.waitFor(() => expect(busy()).toBe(0), { timeout: 2000, interval: 10 });

import { apply, type Drawn, land, MAX_GLIDES, prefersReducedMotion, settle, settledBox, touchesView } from "./draw";
import { busy } from "./motion";

const node = (id: string, x: number, y: number, classes = "leaf"): Drawn => ({
  group: "nodes",
  data: { id },
  position: { x, y },
  classes,
});
const edge = (id: string, source: string, target: string): Drawn => ({
  group: "edges",
  data: { id, source, target },
  classes: "",
});

const view = { x1: -1000, y1: -1000, x2: 1000, y2: 1000 };
const glide = { reduced: false, extent: view };

describe("glide", () => {
  let core: cytoscape.Core;
  beforeEach(() => {
    core = cytoscape({ headless: true, styleEnabled: true });
  });
  afterEach(() => {
    settle(core);
    core.destroy();
  });

  const start = [node("a", 0, 0), node("b", 100, 0), edge("a->b", "a", "b")];
  const moved = [node("a", 0, 200), node("b", 100, 0), edge("a->b", "a", "b")];

  it("glides a moved tile to its target over the glide time, counting it busy, and returns the targets", async () => {
    const was = apply(core, new Map(), start);
    const now = apply(core, was, moved, glide);
    expect(now.get("a")?.position).toEqual({ x: 0, y: 200 });
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 0 });
    expect(busy()).toBeGreaterThan(0);
    await rested();
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 200 });
    // What the renderer was given is not what the scene records: a glide moves the node's own position, not the scene's.
    expect(now.get("a")?.position).toEqual({ x: 0, y: 200 });
    expect(start[0]?.position).toEqual({ x: 0, y: 0 });
    expect(busy()).toBe(0);
  });

  it("measures a gliding tile at its target", () => {
    const was = apply(core, new Map(), start);
    apply(core, was, moved, glide);
    const rest = settledBox(core, "a");
    land(core);
    const landed = core.getElementById("a").renderedBoundingBox({ includeLabels: false });
    expect(rest.y1).toBeCloseTo(landed.y1);
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 200 });
    expect(busy()).toBe(0);
  });

  it("starts a write that lands mid-glide from where the tile is drawn, and holds the new target", async () => {
    const was = apply(core, new Map(), start);
    const mid = apply(core, was, moved, glide);
    await vi.waitFor(() => expect(core.getElementById("a").position().y).toBeGreaterThan(0), { interval: 5 });
    const drawnAt = core.getElementById("a").position();
    expect(drawnAt.y).toBeGreaterThan(0);
    expect(drawnAt.y).toBeLessThan(200);
    const again = [node("a", 300, 400), node("b", 100, 0), edge("a->b", "a", "b")];
    const now = apply(core, mid, again, glide);
    expect(now.get("a")?.position).toEqual({ x: 300, y: 400 });
    expect(core.getElementById("a").position()).toEqual(drawnAt);
    expect(busy()).toBeGreaterThan(0);
    await rested();
    expect(core.getElementById("a").position()).toEqual({ x: 300, y: 400 });
    expect(busy()).toBe(0);
  });

  it("leaves a glide already on its way to a target alone when the next write keeps the target", async () => {
    const was = apply(core, new Map(), start);
    const mid = apply(core, was, moved, glide);
    await vi.waitFor(() => expect(core.getElementById("a").position().y).toBeGreaterThan(0), { interval: 5 });
    const drawnAt = core.getElementById("a").position();
    apply(core, mid, moved, glide);
    expect(core.getElementById("a").position()).toEqual(drawnAt);
    await rested();
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 200 });
    expect(busy()).toBe(0);
  });

  it("hides a link whose tile glides, and fades it back in once everything has settled", async () => {
    const routed = (to: number) => [
      node("a", 0, to),
      node("b", 100, 0),
      { ...edge("a->b", "a", "b"), data: { ...edge("a->b", "a", "b").data, bend: to } },
    ];
    const was = apply(core, new Map(), routed(0));
    apply(core, was, routed(200), glide);
    expect(Number(core.getElementById("a->b").style("opacity"))).toBe(0);
    await rested();
    expect(Number(core.getElementById("a->b").style("opacity"))).toBe(1);
    expect(core.getElementById("a->b").data("bend")).toBe(200);
    expect(busy()).toBe(0);
  });

  it("moves a tile whose move is off screen at once, with nothing animated", () => {
    const was = apply(core, new Map(), start);
    const far = { reduced: false, extent: { x1: 5000, y1: 5000, x2: 6000, y2: 6000 } };
    apply(core, was, moved, far);
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 200 });
    expect(busy()).toBe(0);
  });

  it("animates nothing under reduced motion", () => {
    const was = apply(core, new Map(), start);
    apply(core, was, moved, { ...glide, reduced: true });
    expect(core.getElementById("a").position()).toEqual({ x: 0, y: 200 });
    expect(core.getElementById("a").animated()).toBe(false);
    expect(busy()).toBe(0);
  });

  it("moves at once without a glide, and releases a glide for a tile that is removed", async () => {
    const was = apply(core, new Map(), start);
    const mid = apply(core, was, moved, glide);
    expect(busy()).toBeGreaterThan(0);
    apply(core, mid, [node("b", 100, 0)], glide);
    expect(core.getElementById("a").empty()).toBe(true);
    expect(busy()).toBe(0);
    const stop = apply(core, new Map(), [node("b", 100, 0)]);
    apply(core, stop, [node("b", 7, 7)]);
    expect(core.getElementById("b").position()).toEqual({ x: 7, y: 7 });
  });

  it("hides a link added in the same write as a glide, and fades it in with the rest", async () => {
    const was = apply(core, new Map(), start);
    apply(core, was, [...moved, edge("a->c", "a", "c"), node("c", 300, 300)], glide);
    expect(Number(core.getElementById("a->c").style("opacity"))).toBe(0);
    expect(core.getElementById("c").position()).toEqual({ x: 300, y: 300 });
    await rested();
    expect(Number(core.getElementById("a->c").style("opacity"))).toBe(1);
  });

  it("puts every tile in place at once when more than the cap would glide", () => {
    const row = (y: number) => Array.from({ length: MAX_GLIDES + 1 }, (_, i) => node(`n${i}`, i, y));
    const was = apply(core, new Map(), row(0));
    apply(core, was, row(50), glide);
    expect(core.getElementById("n0").position()).toEqual({ x: 0, y: 50 });
    expect(core.animated()).toBe(false);
    expect(busy()).toBe(0);
    const few = apply(core, new Map(row(50).map((one) => [one.data.id, one])), row(0).slice(0, MAX_GLIDES), glide);
    expect(few.size).toBe(MAX_GLIDES);
    expect(busy()).toBeGreaterThan(0);
  });

  it("never changes which box a tile is in: a node's box in its data stays what the scene says, mid-glide and at rest", async () => {
    // A seeded walk of writes that move tiles about without moving any across boxes.
    let seed = 7;
    const rand = (below: number) => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed % below;
    };
    const tile = (i: number, x: number, y: number): Drawn => ({
      group: "nodes",
      data: { id: `t${i}`, box: `box${i % 3}` },
      position: { x, y },
      classes: "leaf",
    });
    let held = apply(
      core,
      new Map(),
      Array.from({ length: 12 }, (_, i) => tile(i, rand(500), rand(500))),
    );
    for (let write = 0; write < 8; write++) {
      const next = Array.from({ length: 12 }, (_, i) => tile(i, rand(500), rand(500)));
      held = apply(core, held, next, glide);
      for (const one of core.nodes()) expect(one.data("box")).toBe(`box${Number(one.id().slice(1)) % 3}`);
      await new Promise((done) => setTimeout(done, rand(60)));
    }
    await rested();
    for (const [id, one] of held) expect(core.getElementById(id).data("box")).toBe(one.data.box);
  });

  it("tells a move in view from one out of it", () => {
    const at = (x: number, y: number) => ({ x, y });
    expect(touchesView(null, at(0, 0), at(9, 9), 10, 10)).toBe(true);
    const box = { x1: 0, y1: 0, x2: 100, y2: 100 };
    expect(touchesView(box, at(50, 50), at(500, 500), 10, 10)).toBe(true);
    expect(touchesView(box, at(500, 500), at(50, 50), 10, 10)).toBe(true);
    expect(touchesView(box, at(500, 500), at(600, 600), 10, 10)).toBe(false);
  });

  it("reads the reduced motion preference through the matchMedia it is given", () => {
    expect(prefersReducedMotion(() => ({ matches: true }))).toBe(true);
    expect(prefersReducedMotion(() => ({ matches: false }))).toBe(false);
    expect(prefersReducedMotion(undefined)).toBe(false);
  });
});
