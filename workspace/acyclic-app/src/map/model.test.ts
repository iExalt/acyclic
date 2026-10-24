import { describe, expect, it } from "vitest";
import { item } from "./fixture";
import { buildModel, defaultView, visible } from "./model";

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

  it("treats an item whose parent is missing as a root", () => {
    const orphan = buildModel([item("a", "gone")]);
    expect(orphan.roots).toEqual(["a"]);
  });
});
