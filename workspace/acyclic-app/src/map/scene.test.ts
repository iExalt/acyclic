import { describe, expect, it } from "vitest";
import { item } from "./fixture";
import { type Placed, ROOT } from "./layout";
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
const view = visible(model, { collapsed: new Set() });
/** Every node 10 wide and tall, placed by its index, so centres are easy to check. */
const placed: Placed = {
  width: 100,
  height: 100,
  laidOut: [],
  routes: new Map(),
  junctions: new Map(),
  dividers: new Map(),
  redundant: new Set(),
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

  it("draws a link another way implies only for its ends or its thread, and a routed link along its route", () => {
    const chain = buildModel([
      item("top"),
      item("a", "top", [], { thread: "t" }),
      item("b", "top", ["a"]),
      item("d", "top", ["a", "b"], { thread: "t" }),
    ]);
    const open = visible(chain, { collapsed: new Set() });
    const at = new Map(open.nodes.map((node, i) => [node.id, { x: 0, y: i * 100, width: 10, height: 10 }]));
    const routed: Placed = {
      ...placed,
      nodes: at,
      redundant: new Set(["a->d"]),
      routes: new Map([
        [
          "a->b",
          [
            { x: 5, y: 110 },
            { x: 5, y: 200 },
          ],
        ],
      ]),
    };
    const thread = (id: string) => chain.items.get(id)?.thread ?? null;
    const none = scene(open, routed, { item: null, thread: null }, thread);
    expect(none.edges.map((edge) => [edge.id, edge.route?.weights ?? null])).toEqual([
      ["a->b", "0.5"],
      ["b->d", null],
    ]);
    const picked = scene(open, routed, { item: "d", thread: "u" }, thread);
    expect(picked.edges.find((edge) => edge.id === "a->d")).toMatchObject({
      implied: true,
      crossing: false,
      route: null,
    });
    expect(scene(open, routed, { item: null, thread: "t" }, thread).edges.map((edge) => edge.id)).toContain("a->d");
  });

  it("draws the lit thread's links as its route, across boxes too", () => {
    // Inside one box: a solid route.
    const both = buildModel([
      item("top"),
      item("a", "top", [], { thread: "t" }),
      item("b", "top", ["a"], { thread: "t" }),
    ]);
    const open = visible(both, { collapsed: new Set() });
    const inside = scene(open, placed, { item: null, thread: "t" }, (id) => both.items.get(id)?.thread ?? null);
    expect(inside.edges).toMatchObject([{ id: "a->b", crossing: false, lit: true }]);
    const picked = scene(view, placed, { item: null, thread: "t" }, threadOf);
    expect(picked.edges.map((edge) => [edge.id, edge.crossing, edge.lit])).toEqual([
      ["a->b", false, false],
      ["a->c", true, true],
    ]);
    // A selected item's link stays visible outside its thread.
    const selected = scene(view, placed, { item: "b", thread: "t" }, threadOf);
    expect(selected.edges.find((edge) => edge.id === "a->b")).toMatchObject({ lit: false });
    // So does a crossing drawn for the selected item.
    const crossing = scene(view, placed, { item: "c", thread: "u" }, threadOf);
    expect(crossing.edges.find((edge) => edge.id === "a->c")).toMatchObject({ crossing: true, lit: false });
  });

  it("shows a hovered item's links, across boxes too", () => {
    const hovered = scene(view, placed, { item: null, thread: null, hovered: "c" }, threadOf);
    expect(hovered.edges.map((edge) => [edge.id, edge.hovered])).toEqual([
      ["a->b", false],
      ["a->c", true],
    ]);
    // The lit thread's route stays lit while another item is hovered.
    const lit = scene(view, placed, { item: null, thread: "t", hovered: "b" }, threadOf);
    expect(lit.edges.find((edge) => edge.id === "a->c")).toMatchObject({ lit: true });
    // A box opened under the pointer shows nothing of its own.
    const opened = scene(view, placed, { item: null, thread: null, hovered: "top" }, threadOf);
    expect(opened.edges.every((edge) => !edge.hovered)).toBe(true);
  });

  it("routes a thread through the collapsed box that hides a member", () => {
    const closed = visible(model, { collapsed: new Set(["top"]) });
    const drawn = scene(closed, placed, { item: null, thread: "t" }, threadOf);
    // Both now sit at the top level, so the link no longer crosses boxes.
    expect(drawn.edges).toMatchObject([{ id: "top->c", crossing: false, lit: true }]);
  });

  it("lights up the selected item's thread", () => {
    const lit = scene(view, placed, { item: "a", thread: null }, threadOf);
    const marks = lit.nodes.map((node) => `${node.id}:${node.lit ? "lit" : "-"}`);
    // Roots come in id order, so c before top.
    expect(marks).toEqual(["c:lit", "top:-", "a:lit", "b:-"]);
    expect(lit.thread).toBe("t");
    // A picked thread wins over the selected item's own.
    expect(scene(view, placed, { item: "b", thread: "t" }, threadOf).thread).toBe("t");
  });

  it("places nodes by their centres, a box under what it holds, and labels a collapsed box with what it hides", () => {
    const closed = visible(model, { collapsed: new Set(["top"]) });
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

describe("buildModel", () => {
  it("takes a parent's progress from its item, and none when it has no leaves", () => {
    const none = { in_progress: 0, ready: 0, stalled: 0, waiting: 0, parked: 0, canceled: 0 };
    const progress = (done: number, leaves: number) => ({ state: "done", counts: { ...none, done, leaves } });
    const model = buildModel([
      item("p", null, [], { progress: progress(2, 2) }),
      item("q", null, [], { progress: progress(0, 0) }),
    ]);
    expect(model.items.get("p")?.progress).toEqual({ ...none, done: 2, leaves: 2 });
    expect(model.items.get("q")?.progress).toBeNull();
  });
});

describe("a box's stage", () => {
  it("is the stage core gives its row, a collapsed one's too, and none without one, as with no leaves to count", () => {
    const rows = [
      item("p", null, [], { stage: "in-progress" }),
      item("p1", "p", [], { stage: "done" }),
      item("p2", "p", [], { stage: "ready" }),
      item("q", null, [], { stage: "waiting" }),
      item("q1", "q", [], { stage: "waiting" }),
      item("r"),
    ];
    const built = buildModel(rows);
    const shown = visible(built, { collapsed: new Set(["q"]) });
    const at = new Map(shown.nodes.map((node, i) => [node.id, { x: i * 20, y: 0, width: 10, height: 10 }]));
    const drawn = scene(shown, { ...placed, nodes: at }, { item: null, thread: null }, () => null);
    const stageOf = (id: string) => drawn.nodes.find((node) => node.id === id)?.stage;
    // One leaf in review beside a ready one: in progress, as core tells it, not in review.
    expect(stageOf("p")).toBe("in-progress");
    expect(stageOf("q")).toBe("waiting");
    expect(stageOf("r")).toBeNull();
  });
});

describe("scene stubs", () => {
  const gate = { id: "far", title: "Far", initiative: "Home lab" };
  const alone = buildModel([item("a", null, [], { stage: "ready" }), item("b")], new Map([["a", [gate]]]));
  const open = visible(alone, { collapsed: new Set() });
  const laid: Placed = {
    ...placed,
    nodes: new Map(open.nodes.map((node, i) => [node.id, { x: i * 300, y: 10, width: 200, height: 94 }])),
  };

  it("draws a stub on the foot of the tile it is for, over it, and only for it", () => {
    const drawn = scene(open, laid, { item: null, thread: null }, () => null);
    expect(drawn.stubs).toHaveLength(1);
    expect(drawn.stubs[0]).toMatchObject({ node: "a", target: "far", width: 184, height: 20, x: 100, layer: 1 });
    // Its foot: inside the tile, a margin up from its bottom edge.
    expect(drawn.stubs[0]?.y).toBe(10 + 94 - 8 - 10);
    expect(drawn.nodes.find((node) => node.id === "a")?.reserve).toBeGreaterThan(0);
    expect(drawn.nodes.find((node) => node.id === "b")?.reserve).toBe(0);
  });

  it("draws an open box's own wait in its reserved header row", () => {
    const boxModel = buildModel(
      [item("top"), item("box", "top", [], { stage: "waiting" }), item("in", "box")],
      new Map([["box", [gate]]]),
    );
    const opened = visible(boxModel, { collapsed: new Set() });
    const layout: Placed = {
      ...placed,
      nodes: new Map(
        opened.nodes.map((node, i) => [
          node.id,
          { x: i * 300, y: 10, width: 200, height: node.id === "box" ? 94 : 64 },
        ]),
      ),
    };
    const drawn = scene(opened, layout, { item: null, thread: null }, () => null);
    expect(drawn.stubs).toHaveLength(1);
    expect(drawn.stubs[0]).toMatchObject({ node: "box", target: "far", y: 10 + 52 + 30 - 8 - 10 });
    expect(drawn.nodes.find((node) => node.id === "box")).toMatchObject({ header: 82, reserve: 30 });
  });
});

it("draws the root lane divider without a visible root box", () => {
  const line = { x: 220, top: 12, bottom: 400 };
  const out = scene(view, { ...placed, dividers: new Map([[ROOT, line]]) }, { item: null, thread: null }, threadOf);
  expect(out.dividers).toEqual([{ id: `divider ${ROOT}`, owner: ROOT, ...line, layer: 0 }]);
});

it("previews only a visible reference without changing selection, geometry or links", () => {
  const selection = { item: "a", thread: null };
  const before = scene(view, placed, selection, threadOf);
  for (const id of ["c", "top", "a", "missing", null]) {
    const after = scene(view, placed, { ...selection, preview: id }, threadOf);
    expect(after.nodes.filter((node) => node.preview).map((node) => node.id)).toEqual(
      id !== null && id !== "missing" ? [id] : [],
    );
    expect({ ...after, nodes: after.nodes.map((node) => ({ ...node, preview: false })) }).toEqual(before);
  }
  const closed = visible(model, { collapsed: new Set(["top"]) });
  expect(scene(closed, placed, { ...selection, preview: "b" }, threadOf).nodes.some((node) => node.preview)).toBe(
    false,
  );
});
