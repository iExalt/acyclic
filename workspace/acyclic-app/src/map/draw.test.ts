import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import type { Item } from "../api";
import { apply, type Drawn, diff, elementsOf } from "./draw";
import { item } from "./fixture";
import type { Placed } from "./layout";
import { buildModel, visible } from "./model";
import { type Selection, scene } from "./scene";

// garden: beds (dig, sow waits for dig), plant (water); tools at the top level, waiting for dig; t holds dig and tools.
const base = [
  item("garden"),
  item("beds", "garden"),
  item("dig", "beds", [], { thread: "t" }),
  item("sow", "beds", ["dig"]),
  item("plant", "garden"),
  item("water", "plant"),
  item("tools", null, ["dig"], { thread: "t" }),
  item("old", null, [], { archived: true }),
];

interface Shot {
  items: Item[];
  collapsed?: string[];
  archived?: boolean;
  selection?: Selection;
  colour?: string;
}

/** The elements for a plan as the Map would draw it, with every node placed by its index. */
function elements({ items, collapsed = [], archived = false, selection, colour }: Shot): Drawn[] {
  const model = buildModel(items);
  const view = visible(model, { collapsed: new Set(collapsed), showArchived: archived });
  const placed: Placed = {
    width: 500,
    height: 500,
    laidOut: [],
    nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 30, y: i * 7, width: 20 + i, height: 10 }])),
  };
  const picked = selection ?? { item: null, thread: null };
  return elementsOf(
    scene(view, placed, picked, (id) => model.items.get(id)?.thread ?? null),
    colour,
  );
}

/** What a renderer holds, in a form to compare: every element's data, position and classes, by id. */
function held(core: cytoscape.Core) {
  return core
    .elements()
    .map((one) => ({
      data: one.data(),
      position: one.isNode() ? one.position() : null,
      classes: one.classes().sort(),
    }))
    .sort((a, b) => String(a.data.id).localeCompare(String(b.data.id)));
}

const fresh = (next: Drawn[]) => {
  const core = cytoscape({ headless: true });
  core.add(next);
  return core;
};

describe("draw", () => {
  const start: Shot = { items: base };
  const shots: [string, Shot][] = [
    ["expanding nothing", start],
    ["collapsing a box", { items: base, collapsed: ["beds"] }],
    ["selecting an item", { items: base, selection: { item: "dig", thread: null }, colour: "#ff0000" }],
    ["lighting a thread", { items: base, selection: { item: null, thread: "t" }, colour: "#00ff00" }],
    ["adding an item", { items: [...base, item("hose", "plant", ["water"])] }],
    ["retitling an item", { items: base.map((one) => (one.id === "sow" ? { ...one, title: "Sow seeds" } : one)) }],
    ["showing archived items", { items: base, archived: true }],
    ["dropping a link", { items: base.map((one) => (one.id === "sow" ? item("sow", "beds") : one)) }],
    ["dropping everything", { items: [] }],
  ];

  it.each(shots)("takes the renderer to what a fresh draw holds, after %s", (_, shot) => {
    for (const from of [start, ...shots.map(([, one]) => one)]) {
      const core = cytoscape({ headless: true });
      const was = apply(core, new Map(), elements(from));
      const now = apply(core, was, elements(shot));
      expect(held(core)).toEqual(held(fresh(elements(shot))));
      expect([...now.keys()].sort()).toEqual(
        core
          .elements()
          .map((one) => one.id())
          .sort(),
      );
    }
  });

  it("changes nothing when nothing changed, and only what did otherwise", () => {
    const before = elements(start);
    const drawn = new Map(before.map((one) => [one.data.id, one]));
    expect(diff(drawn, elements(start))).toEqual({ add: [], remove: [], update: [] });
    const retitled = diff(
      drawn,
      elements({ items: base.map((one) => (one.id === "sow" ? { ...one, title: "Sow seeds" } : one)) }),
    );
    expect(retitled).toEqual({
      add: [],
      remove: [],
      update: [{ id: "sow", data: { label: "Sow seeds" }, unset: [], position: null, classes: null }],
    });
  });
});
