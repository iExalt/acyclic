import { describe, expect, it } from "vitest";
import { item } from "./fixture";
import type { Placed } from "./layout";
import { buildModel, visible } from "./model";
import { labelOf, scene } from "./scene";

// top: a, b (waits for a); c (waits for a), in thread t with a
const plan = [
  item("top"),
  item("a", "top", [], { thread: "t" }),
  item("b", "top", ["a"]),
  item("c", null, ["a"], { thread: "t" }),
];
const model = buildModel(plan);
const view = visible(model, { collapsed: new Set(), showArchived: false });
/** Every node 10 wide and tall, placed by its index, so centres are easy to check. */
const placed: Placed = {
  width: 100,
  height: 100,
  laidOut: [],
  nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 20, y: 0, width: 10, height: 10 }])),
};
const threadOf = (id: string) => model.items.get(id)?.thread ?? null;

describe("scene", () => {
  it("draws edges inside a box always, and edges across boxes only for the selected item", () => {
    const none = scene(view, placed, { item: null, thread: null }, threadOf);
    expect(none.edges.map((edge) => edge.id)).toEqual(["a->b"]);
    const selected = scene(view, placed, { item: "c", thread: null }, threadOf);
    expect(selected.edges.map((edge) => [edge.id, edge.crossing])).toEqual([
      ["a->b", false],
      ["a->c", true],
    ]);
  });

  it("draws the lit thread's links as its route, across boxes too, and dims the rest", () => {
    // Inside one box: a solid route.
    const both = buildModel([
      item("top"),
      item("a", "top", [], { thread: "t" }),
      item("b", "top", ["a"], { thread: "t" }),
    ]);
    const open = visible(both, { collapsed: new Set(), showArchived: false });
    const inside = scene(open, placed, { item: null, thread: "t" }, (id) => both.items.get(id)?.thread ?? null);
    expect(inside.edges).toMatchObject([{ id: "a->b", crossing: false, lit: true, dim: false }]);
    const picked = scene(view, placed, { item: null, thread: "t" }, threadOf);
    expect(picked.edges.map((edge) => [edge.id, edge.crossing, edge.lit, edge.dim])).toEqual([
      ["a->b", false, false, true],
      ["a->c", true, true, false],
    ]);
    // A link of the selected item's stays undimmed, though outside its thread.
    const selected = scene(view, placed, { item: "b", thread: "t" }, threadOf);
    expect(selected.edges.find((edge) => edge.id === "a->b")).toMatchObject({ lit: false, dim: false });
    // So does a crossing drawn for the selected item.
    const crossing = scene(view, placed, { item: "c", thread: "u" }, threadOf);
    expect(crossing.edges.find((edge) => edge.id === "a->c")).toMatchObject({ crossing: true, lit: false, dim: false });
    expect(scene(view, placed, { item: null, thread: null }, threadOf).edges.every((edge) => !edge.dim)).toBe(true);
  });

  it("routes a thread through the collapsed box that hides a member", () => {
    const closed = visible(model, { collapsed: new Set(["top"]), showArchived: false });
    const drawn = scene(closed, placed, { item: null, thread: "t" }, threadOf);
    // Both now sit at the top level, so the link no longer crosses boxes.
    expect(drawn.edges).toMatchObject([{ id: "top->c", crossing: false, lit: true, dim: false }]);
  });

  it("lights up the selected item's thread and dims other items, but not the boxes around them", () => {
    const lit = scene(view, placed, { item: "a", thread: null }, threadOf);
    const marks = lit.nodes.map((node) => `${node.id}:${node.lit ? "lit" : node.dim ? "dim" : "-"}`);
    // Roots come in id order, so c before top.
    expect(marks).toEqual(["c:lit", "top:-", "a:lit", "b:dim"]);
    expect(lit.thread).toBe("t");
    // A picked thread wins over the selected item's own.
    expect(scene(view, placed, { item: "b", thread: "t" }, threadOf).thread).toBe("t");
    expect(scene(view, placed, { item: "b", thread: null }, threadOf).nodes.some((node) => node.dim)).toBe(false);
  });

  it("places nodes by their centres, a box under what it holds, and labels a collapsed box with what it hides", () => {
    const closed = visible(model, { collapsed: new Set(["top"]), showArchived: false });
    const drawn = scene(closed, placed, { item: null, thread: null }, threadOf);
    expect(drawn.nodes.find((node) => node.id === "top")?.label).toBe("TOP (+2)");
    const open = scene(view, placed, { item: null, thread: null }, threadOf).nodes;
    expect(open.map((node) => [node.id, node.layer, node.x])).toEqual([
      ["c", 0, 5],
      ["top", 0, 25],
      ["a", 1, 45],
      ["b", 1, 65],
    ]);
  });
});

describe("labelOf", () => {
  // Every character 1 wide.
  const fonts = { regular: (text: string) => text.length, bold: (text: string) => text.length };
  const counts = { done: 9, leaves: 15, in_review: 0, in_progress: 2, ready: 3, blocked: 1, parked: 0, unthreaded: 0 };

  it("titles a box on one line, its leaves by stage on the next, and ends a collapsed one with what it hides", () => {
    const node = { title: "Phase two of the plan", kind: "box" as const, hidden: 0, progress: counts };
    expect(labelOf(node, 60, fonts)).toBe("Phase two of the plan\n9 done, 2 in progress, 3 ready, 1 blocked");
    expect(labelOf({ ...node, kind: "collapsed", hidden: 20 }, 16, fonts)).toBe("Phase two… (+20)\n9 done, 2 in pr…");
    expect(labelOf({ ...node, progress: null }, 60, fonts)).toBe("Phase two of the plan");
  });

  it("wraps a task's title over two lines and cuts the second short", () => {
    const leaf = { title: "Route the links inside a box", kind: "leaf" as const, hidden: 0, progress: null };
    expect(labelOf(leaf, 12, fonts)).toBe("Route the\nlinks insid…");
  });
});

describe("buildModel", () => {
  it("takes a parent's progress from its item, and none when every leaf under it is archived", () => {
    const none = { in_review: 0, in_progress: 0, ready: 0, blocked: 0, parked: 0, unthreaded: 0 };
    const progress = (done: number, leaves: number) => ({ state: "done", counts: { ...none, done, leaves } });
    const model = buildModel([
      item("p", null, [], { progress: progress(2, 2) }),
      item("q", null, [], { progress: progress(0, 0) }),
    ]);
    expect(model.items.get("p")?.progress).toEqual({ ...none, done: 2, leaves: 2 });
    expect(model.items.get("q")?.progress).toBeNull();
  });
});
