import ELK from "elkjs/lib/elk.bundled.js";
import type { ElkNode } from "elkjs/lib/elk-api.js";
import { describe, expect, it } from "vitest";
import { STRIP } from "./chevron";
import { beside, dependencyOrder, elkLayBox, PORTRAIT, shapeOf } from "./elk";
import { item } from "./fixture";
import { type LayBox, LayoutCache, LEAF, place, ROOT, scopes, withinOneBox } from "./layout";
import { buildModel, type ViewState, visible } from "./model";

/**
 * A layout engine whose box sizes depend on content: children in rows of two, each row as tall as its tallest child,
 * so a box grows with every second child and with any child that grows. Edges don't change a size.
 */
function grid() {
  const calls: string[] = [];
  const layBox: LayBox = async (input) => {
    calls.push(input.id);
    const at: Record<string, { x: number; y: number }> = {};
    let width = 0;
    let y = 36;
    for (let row = 0; row < input.children.length; row += 2) {
      const pair = input.children.slice(row, row + 2);
      pair.forEach((child, i) => {
        at[child.id] = { x: i * (child.width + 20), y };
        width = Math.max(width, child.width);
      });
      y += Math.max(...pair.map((child) => child.height)) + 32;
    }
    return { width: width * 2 + 44, height: y + 12, at };
  };
  return { calls, layBox };
}

// top: left (box: l1, l2, l3), right (box: r1); far
const base = [item("top"), item("left", "top"), item("l1", "left"), item("l2", "left"), item("l3", "left")];
const plan = [...base, item("right", "top"), item("r1", "right"), item("far")];
const open: ViewState = { collapsed: new Set(), showArchived: false };

/** The boxes laid out again after an edit, with the cache warm from the plan before it. */
async function relaid(before: ReturnType<typeof item>[], after: ReturnType<typeof item>[], state = open, next = open) {
  const engine = grid();
  const cache = new LayoutCache();
  await place(visible(buildModel(before), state), cache, engine.layBox, PORTRAIT);
  const placed = await place(visible(buildModel(after), next), cache, engine.layBox, PORTRAIT);
  return [...placed.laidOut].sort();
}

describe("place", () => {
  it("lays out again exactly the boxes whose content changed", async () => {
    // A fourth child fills the second row of two: right's size holds, so only right is laid out again.
    const three = [...plan, item("r2", "right"), item("r3", "right")];
    expect(await relaid(three, [...three, item("r4", "right")])).toEqual(["right"]);
    // A third child opens a row in right, but right stays no taller than left, its neighbour in top's row: top's
    // size holds, so the top level isn't laid out again.
    const grown = [...plan, item("r2", "right"), item("r3", "right")];
    expect(await relaid(plan, grown)).toEqual(["right", "top"]);
    // A fifth opens a third row: right outgrows left, so top grows, and the top level is laid out again.
    expect(await relaid(grown, [...grown, item("r4", "right"), item("r5", "right")])).toEqual([ROOT, "right", "top"]);
    // A move changes both boxes; left loses its second row, so top shrinks, and the top level follows.
    const moved = plan.map((one) => (one.id === "l3" ? item("l3", "right") : one));
    expect(await relaid(plan, moved)).toEqual([ROOT, "left", "right", "top"]);
    // A link between siblings changes only the box that holds both.
    const linked = plan.map((one) => (one.id === "l2" ? item("l2", "left", ["l1"]) : one));
    expect(await relaid(plan, linked)).toEqual(["left"]);
    // A link between items in different boxes lifts to the box holding both.
    const across = plan.map((one) => (one.id === "r1" ? item("r1", "right", ["l1"]) : one));
    expect(await relaid(plan, across)).toEqual(["top"]);
    // Collapsing a box changes its parent and everything above.
    expect(await relaid(plan, plan, open, { collapsed: new Set(["left"]), showArchived: false })).toEqual([
      ROOT,
      "top",
    ]);
  });

  it("finds every layout in the cache on returning to a view", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const model = buildModel(plan);
    const first = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    await place(visible(model, { collapsed: new Set(["left"]), showArchived: false }), cache, engine.layBox, PORTRAIT);
    const back = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    expect(back.laidOut).toEqual([]);
    expect(back.nodes).toEqual(first.nodes);
  });

  it("lays out only the top level again for another shape, which only it takes", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const view = visible(buildModel(plan), open);
    await place(view, cache, engine.layBox, PORTRAIT);
    expect((await place(view, cache, engine.layBox, 1.4)).laidOut).toEqual([ROOT]);
    expect((await place(view, cache, engine.layBox, PORTRAIT)).laidOut).toEqual([]);
  });

  it("places children at their box's position plus their own, with real ELK, the same every time", async () => {
    const elk = elkLayBox(new ELK());
    const linked = [...plan, item("r2", "right", ["l1"]), item("far2", null, ["far"])];
    const view = visible(buildModel(linked), open);
    const one = await place(view, new LayoutCache(), elk, PORTRAIT);
    const two = await place(view, new LayoutCache(), elk, PORTRAIT);
    expect(two.nodes).toEqual(one.nodes);
    const top = one.nodes.get("top");
    const left = one.nodes.get("left");
    const l1 = one.nodes.get("l1");
    expect(top && left && l1).toBeTruthy();
    // Every child lies within its box.
    for (const [child, box] of [
      [l1, left],
      [left, top],
    ] as const) {
      expect(child?.x).toBeGreaterThanOrEqual(box?.x ?? Infinity);
      expect((child?.x ?? 0) + (child?.width ?? 0)).toBeLessThanOrEqual((box?.x ?? 0) + (box?.width ?? 0));
      expect(child?.y).toBeGreaterThanOrEqual(box?.y ?? Infinity);
      expect((child?.y ?? 0) + (child?.height ?? 0)).toBeLessThanOrEqual((box?.y ?? 0) + (box?.height ?? 0));
    }
    // The edge from l1 to r2 runs between boxes, so ELK lays it out between left and right in top.
    expect(scopes(view).edges.get("top")).toEqual([{ source: "left", target: "right" }]);
    expect(one.width).toBeGreaterThan(0);
  });

  it("asks for a box only once every box inside it has answered, and for a level's boxes at once", async () => {
    const pending: { id: string; done: () => void }[] = [];
    const asked: string[][] = [];
    const layBox: LayBox = (input) =>
      new Promise((resolve) => {
        pending.push({ id: input.id, done: () => resolve({ width: 100, height: 100, at: {} }) });
      });
    const placing = place(visible(buildModel(plan), open), new LayoutCache(), layBox, PORTRAIT);
    // Let each level's requests go out, record them, then answer them all.
    for (let level = 0; level < 3; level++) {
      await new Promise((resolve) => setTimeout(resolve, 0));
      asked.push(pending.map((one) => one.id).sort());
      for (const one of pending.splice(0)) one.done();
    }
    await placing;
    expect(asked).toEqual([["left", "right"], ["top"], [ROOT]]);
  });
});

describe("elkLayBox", () => {
  it("packs the top level in dependency order and keeps every box's size", async () => {
    const order = dependencyOrder({
      id: ROOT,
      children: ["a", "b", "c"].map((id) => ({ id, width: 10, height: 10 })),
      edges: [{ source: "b", target: "a" }],
    });
    expect(order).toEqual(["b", "a", "c"]);
    // Without links, ids come in natural order, as the CLI lists them.
    const unlinked = ["i10", "i2", "phase-10", "phase-2"].map((id) => ({ id, width: 10, height: 10 }));
    expect(dependencyOrder({ id: ROOT, children: unlinked, edges: [] })).toEqual(["i2", "i10", "phase-2", "phase-10"]);
    const sizes = [
      { id: "a", width: 900, height: 120 },
      { id: "b", width: 200, height: 700 },
      { id: "c", width: 444, height: 48 },
      { id: "d", width: 300, height: 300 },
    ];
    const elk = new ELK();
    let answer: ElkNode | undefined;
    const watched = { layout: async (graph: ElkNode) => (answer = await elk.layout(graph)) };
    const packed = await elkLayBox(watched)({ id: ROOT, children: sizes, edges: [] });
    // Packing must not stretch a box to fill its row: what it holds was placed for its own size.
    expect(
      answer?.children
        ?.map(({ id, width, height }) => ({ id, width, height }))
        .sort((a, b) => a.id.localeCompare(b.id)),
    ).toEqual(sizes);
    for (const box of sizes) {
      const at = packed.at[box.id];
      expect((at?.x ?? Infinity) + box.width).toBeLessThanOrEqual(packed.width);
      expect((at?.y ?? Infinity) + box.height).toBeLessThanOrEqual(packed.height);
    }
  });

  it("shapes the top level like the Map, to a fifth, within portrait and three wide", () => {
    expect(shapeOf(1180, 850)).toBe(1.4);
    expect(shapeOf(1000, 1000)).toBe(1);
    expect(shapeOf(300, 900)).toBe(PORTRAIT);
    expect(shapeOf(5000, 400)).toBe(3);
  });

  it("breaks a dependency cycle at its least id", () => {
    const children = ["a", "b"].map((id) => ({ id, width: 1, height: 1 }));
    const edges = [
      { source: "a", target: "b" },
      { source: "b", target: "a" },
    ];
    expect(dependencyOrder({ id: ROOT, children, edges })).toEqual(["a", "b"]);
  });
});

describe("beside", () => {
  const tasks = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `t${i}`, ...LEAF }));
  const columns = (layout: { at: Record<string, { x: number; y: number }> }) =>
    new Set(Object.values(layout.at).map((at) => at.x)).size;

  it("leaves a box with every child linked as its graph lays it out", () => {
    const graph = { width: 224, height: 200, at: { a: { x: 12, y: STRIP } } };
    expect(beside(graph, [])).toBe(graph);
  });

  it("stands the unlinked beside the graph, in columns as tall as it, after a wider gap", () => {
    // A graph of three layers holds three tasks a column; five more take two columns.
    const graph = { width: 224, height: STRIP + 3 * 48 + 2 * 40 + 12, at: { a: { x: 12, y: STRIP } } };
    const out = beside(graph, tasks(5));
    expect(out.at.a).toEqual({ x: 12, y: STRIP });
    expect(out.at.t0).toEqual({ x: 224 - 12 + 40, y: STRIP });
    expect(out.at.t3).toEqual({ x: 224 - 12 + 40 + 200 + 24, y: STRIP });
    expect(out.width).toBe(224 - 12 + 40 + 2 * 200 + 24 + 12);
    expect(out.height).toBe(graph.height);
  });

  it("grids a box with nothing linked about as many tasks tall as wide, and grows past a short graph", () => {
    const alone = beside(null, tasks(9));
    expect(columns(alone)).toBe(3);
    expect(alone.height).toBe(STRIP + 3 * 48 + 2 * 24 + 12);
    expect(columns(beside(null, tasks(10)))).toBe(3);
    // A graph of one layer is shorter than four tasks' grid, so the column takes the grid's height and the box grows.
    const flat = beside({ width: 224, height: STRIP + 48 + 12, at: {} }, tasks(4));
    expect(columns(flat)).toBe(2);
    expect(flat.height).toBe(STRIP + 2 * 48 + 24 + 12);
  });

  it("shares the unlinked evenly among the columns they take, leaving none nearly empty", () => {
    // Four fit beside this graph, so nine take three columns: three each, not four, four and one.
    const graph = { width: 224, height: STRIP + 4 * 48 + 3 * 24 + 12, at: {} };
    const out = beside(graph, tasks(9));
    const xs = Object.values(out.at).map((at) => at.x);
    expect([...new Set(xs)].map((x) => xs.filter((one) => one === x).length)).toEqual([3, 3, 3]);
  });

  it("makes each column as wide as its widest child", () => {
    const out = beside(null, [
      { id: "box", width: 500, height: 300 },
      { id: "a", ...LEAF },
      { id: "b", ...LEAF },
    ]);
    expect(out.at.a).toEqual({ x: 12 + 500 + 24, y: STRIP });
    expect(out.width).toBe(12 + 500 + 24 + 200 + 12);
  });
});

describe("withinOneBox", () => {
  it("draws an edge always only when both drawn ends are in the same box", () => {
    const linked = [...plan, item("r2", "right", ["r1"]), item("far2", null, ["l1"])];
    const view = visible(buildModel(linked), { collapsed: new Set(["left"]), showArchived: false });
    const within = withinOneBox(view);
    const edges = view.edges.map((edge) => [`${edge.source}->${edge.target}`, within(edge)]);
    // r1 to r2 is inside right; left (standing in for l1) to far2 crosses from top to the top level.
    expect(edges).toEqual([
      ["left->far2", false],
      ["r1->r2", true],
    ]);
  });
});

describe("LayoutCache", () => {
  it("evicts the least recently used entry beyond its limit", () => {
    const cache = new LayoutCache(2);
    const layout = { width: 1, height: 1, at: {} };
    cache.set("a", layout);
    cache.set("b", layout);
    cache.get("a");
    cache.set("c", layout);
    expect([cache.get("a"), cache.get("b"), cache.get("c")].map(Boolean)).toEqual([true, false, true]);
    expect(cache.size).toBe(2);
  });
});
