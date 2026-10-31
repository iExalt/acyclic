import { describe, expect, it } from "vitest";
import { item } from "./fixture";
import { buildModel, collapsedWith, defaultView, impliedWithin, inArchived, visible } from "./model";

// garden
//   beds: dig, edge
//   plant: sow (waits for dig), water (waits for sow)
// shed (archived): paint (waits for sow)
// tools (waits for paint)
const plan = [
  item("garden"),
  item("beds", "garden"),
  item("dig", "beds"),
  item("edge", "beds"),
  item("plant", "garden"),
  item("sow", "plant", ["dig"], { thread: "spring" }),
  item("water", "plant", ["sow"], { thread: "spring" }),
  item("shed", null, [], { archived: true }),
  item("paint", "shed", ["sow"], { thread: "spring" }),
  item("tools", null, ["paint"]),
];
const model = buildModel(plan);
const ids = (view: ReturnType<typeof visible>) => view.nodes.map((node) => `${node.id}:${node.kind}`);

describe("visible", () => {
  it("opens on the roots, with every box below them collapsed and archived subtrees hidden", () => {
    const view = visible(model, defaultView(model));
    expect(ids(view)).toEqual(["garden:box", "beds:collapsed", "plant:collapsed", "tools:leaf"]);
    expect(view.nodes.find((node) => node.id === "beds")?.hidden).toBe(2);
    // sow waits for dig: both are hidden, in different collapsed boxes, so the edge joins the boxes.
    expect(view.edges).toEqual([{ source: "beds", target: "plant" }]);
  });

  it("drops an edge inside one collapsed box, and one into a hidden archived subtree", () => {
    const view = visible(model, { collapsed: new Set(["garden"]), showArchived: false });
    expect(ids(view)).toEqual(["garden:collapsed", "tools:leaf"]);
    // tools waits for paint, which is archived and hidden; dig, sow and water all stand in as garden.
    expect(view.edges).toEqual([]);
  });

  it("shows archived subtrees when asked, with their edges", () => {
    const view = visible(model, { collapsed: new Set(), showArchived: true });
    expect(ids(view)).toContain("shed:box");
    expect(view.edges).toEqual([
      { source: "dig", target: "sow" },
      { source: "paint", target: "tools" },
      { source: "sow", target: "paint" },
      { source: "sow", target: "water" },
    ]);
    // Only the subtree's root is marked archived; what it holds is archived with it, closed or open.
    const archived = (one: ReturnType<typeof visible>) =>
      one.nodes.filter((node) => node.archived).map((node) => node.id);
    expect(archived(view)).toEqual(["shed", "paint"]);
    expect(archived(visible(model, { collapsed: new Set(["shed"]), showArchived: true }))).toEqual(["shed"]);
  });

  it("drops an edge between a box and an item inside it", () => {
    const nested = buildModel([item("a"), item("b", "a", ["a"])]);
    expect(visible(nested, { collapsed: new Set(), showArchived: false }).edges).toEqual([]);
  });

  it("highlights a thread's hidden members as the box that hides them, once", () => {
    const view = visible(model, defaultView(model));
    // sow and water are both inside collapsed plant; paint is archived and hidden.
    expect(view.threads.get("spring")).toEqual(["plant"]);
    const open = visible(model, { collapsed: new Set(), showArchived: true });
    expect(open.threads.get("spring")).toEqual(["paint", "sow", "water"]);
  });

  it("draws items whose parents form a cycle, from the first of them in id order", () => {
    const tangled = buildModel([item("a", "b"), item("b", "a"), item("c", "a")]);
    expect(tangled.roots).toEqual(["a"]);
    expect(ids(visible(tangled, { collapsed: new Set(), showArchived: false }))).toEqual(["a:box", "b:leaf", "c:leaf"]);
    expect([...tangled.depth.values()]).toEqual([0, 1, 1]);
    // An item hanging off a cycle stays under its parent, though it sorts before the cycle's items.
    const hanging = buildModel([item("x", "y"), item("y", "x"), item("a", "x")]);
    expect(hanging.roots).toEqual(["x"]);
    expect(hanging.children.get("x")).toEqual(["a", "y"]);
  });

  it("treats an item whose parent is missing as a root", () => {
    const orphan = buildModel([item("a", "gone")]);
    expect(orphan.roots).toEqual(["a"]);
  });
});

describe("inArchived", () => {
  it("holds for an archived item and what is under it, and ends on parents that loop", () => {
    expect([inArchived(model, "shed"), inArchived(model, "paint"), inArchived(model, "sow")]).toEqual([
      true,
      true,
      false,
    ]);
    expect(inArchived(buildModel([item("a", "b"), item("b", "a")]), "a")).toBe(false);
  });
});

describe("collapsedWith", () => {
  it("keeps the user's toggles across a refresh, and collapses a box new since the last one by default", () => {
    const toggles = new Map([
      ["beds", false],
      ["garden", true],
    ]);
    expect([...collapsedWith(model, toggles)].sort()).toEqual(["garden", "plant"]);
    // weeds is new, holding pull: collapsed, while beds stays open as the user left it.
    const refreshed = buildModel([...plan, item("weeds", "garden"), item("pull", "weeds")]);
    expect([...collapsedWith(refreshed, toggles)].sort()).toEqual(["garden", "plant", "weeds"]);
  });
});

describe("implied links", () => {
  const implied = (items: Parameters<typeof buildModel>[0], box: string | null, first: string, then: string) =>
    impliedWithin(buildModel(items), () => true)(first, then, box);

  it("finds a link a longer way through items inside the box implies, and none a way outside it does", () => {
    const chain = [item("p"), item("a", "p"), item("b", "p", ["a"]), item("c", "p", ["a", "b"]), item("x")];
    expect(implied(chain, "p", "a", "c")).toBe(true);
    expect(implied(chain, "p", "a", "b")).toBe(false);
    // Through x, outside p, the way isn't drawn in p.
    const around = [item("p"), item("a", "p"), item("x", null, ["a"]), item("c", "p", ["a", "x"])];
    expect(implied(around, "p", "a", "c")).toBe(false);
    expect(implied(around, null, "a", "c")).toBe(true);
  });

  it("leaves a link between boxes drawn unless every link it stands for is implied", () => {
    // A to B from a1 to b1, B to C from b2 to c1, and A to C from a2 to c2: nothing in A reaches C through B.
    const boxes = [
      item("p"),
      ...["A", "B", "C"].map((box) => item(box, "p")),
      item("a1", "A"),
      item("a2", "A"),
      item("b1", "B", ["a1"]),
      item("b2", "B"),
      item("c1", "C", ["b2"]),
      item("c2", "C", ["a2"]),
    ];
    const open = { collapsed: new Set(["A", "B", "C"]), showArchived: false };
    expect(visible(buildModel(boxes), open).edges).toEqual([
      { source: "A", target: "B" },
      { source: "A", target: "C" },
      { source: "B", target: "C" },
    ]);
    // With a2 reaching c2 through b1, b2 and c1, every link A to C stands for is implied.
    const changed: Record<string, string[]> = { b1: ["a1", "a2"], b2: ["b1"], c1: ["b2"], c2: ["a2", "c1"] };
    const linked = boxes.map((one) => (one.id in changed ? item(one.id, one.parent, changed[one.id]) : one));
    expect(visible(buildModel(linked), open).edges).toContainEqual({ source: "A", target: "C", implied: true });
  });
});
