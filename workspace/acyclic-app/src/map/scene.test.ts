import { describe, expect, it } from "vitest";
import { item } from "./fixture";
import type { Placed } from "./layout";
import { buildModel, visible } from "./model";
import { scene } from "./scene";

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
