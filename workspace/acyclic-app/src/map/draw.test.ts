import cytoscape from "cytoscape";
import { describe, expect, it, vi } from "vitest";
import type { Item } from "../api";
import { linesOf } from "../lines";
import { PALETTES } from "../palette";
import { apply, type Drawn, diff, elementsOf } from "./draw";
import { item } from "./fixture";
import type { Placed } from "./layout";
import { buildModel, visible } from "./model";
import { type Selection, scene } from "./scene";
import { styleFor } from "./style";

// garden: beds (dig, sow waits for dig), plant (water); tools at the top level, waiting for dig; t holds dig and tools.
const base = [
  item("garden"),
  item("beds", "garden"),
  item("dig", "beds", [], { thread: "t" }),
  item("sow", "beds", ["dig"]),
  item("plant", "garden"),
  item("water", "plant"),
  item("tools", null, ["dig"], { thread: "t" }),
];

interface Shot {
  items: Item[];
  collapsed?: string[];
  selection?: Selection;
  colour?: string;
  routed?: boolean;
}

/** The elements for a plan as the Map would draw it, with every node placed by its index. */
function elements({ items, collapsed = [], selection, colour, routed = false }: Shot): Drawn[] {
  const model = buildModel(items);
  const view = visible(model, { collapsed: new Set(collapsed) });
  const placed: Placed = {
    width: 500,
    height: 500,
    laidOut: [],
    routes: routed
      ? new Map([
          [
            "dig->sow",
            [
              { x: 0, y: 0 },
              { x: 20, y: 10 },
              { x: 40, y: 20 },
            ],
          ],
        ])
      : new Map(),
    junctions: routed ? new Map([["dig->sow", [{ x: 20, y: 10 }]]]) : new Map(),
    dividers: new Map(),
    redundant: new Set(),
    nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 30, y: i * 7, width: 20 + i, height: 10 }])),
  };
  const picked = selection ?? { item: null, thread: null };
  return elementsOf(
    scene(view, placed, picked, (id) => model.items.get(id)?.thread ?? null),
    colour,
    linesOf([{ id: "t", title: "Tools" }], PALETTES.dark),
    PALETTES.dark,
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

describe("a task in no thread of its own", () => {
  const marks = (thread: string | null) => {
    const items = [item("solo", null, [], { thread }), item("other", null, [], { thread: "t" })];
    const lines = linesOf(
      [
        { id: "s.one", title: "Solo", task: "solo" },
        { id: "t", title: "Tools" },
      ],
      PALETTES.dark,
    );
    const model = buildModel(items);
    const view = visible(model, { collapsed: new Set() });
    const placed: Placed = {
      width: 500,
      height: 500,
      laidOut: [],
      routes: new Map(),
      junctions: new Map(),
      dividers: new Map(),
      redundant: new Set(),
      nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 30, y: 0, width: 200, height: 48 }])),
    };
    const drawn = elementsOf(
      scene(view, placed, { item: null, thread: null }, (id) => model.items.get(id)?.thread ?? null),
      undefined,
      lines,
      PALETTES.dark,
    );
    const mark = (id: string) => decodeURIComponent(String(drawn.find((one) => one.data.id === id)?.data.mark));
    return { solo: mark("solo"), other: mark("other") };
  };

  it("draws a stripe and dot like a threaded item's, in grey, once claimed too", () => {
    const grey = PALETTES.dark.muted;
    for (const thread of [null, "s.one"]) {
      const { solo, other } = marks(thread);
      expect(solo).toContain(`<rect width="6" height="48" fill="${grey}"/>`);
      expect(solo).toContain(`<circle cx="24" cy="24" r="5" fill="${grey}"/>`);
      expect(solo).not.toContain("stroke-dasharray");
      // A real thread's item has the same shapes, in its line's colour.
      expect(other).toMatch(new RegExp(`<rect width="6" height="48" fill="#(?!${grey.slice(1)})`));
      expect(other).toMatch(new RegExp(`<circle cx="24" cy="24" r="5" fill="#(?!${grey.slice(1)})`));
    }
  });
});

describe("draw", () => {
  const start: Shot = { items: base };
  const shots: [string, Shot][] = [
    ["expanding nothing", start],
    ["collapsing a box", { items: base, collapsed: ["beds"] }],
    ["selecting an item", { items: base, selection: { item: "dig", thread: null }, colour: "#ff0000" }],
    ["lighting a thread", { items: base, selection: { item: null, thread: "t" }, colour: "#00ff00" }],
    ["adding an item", { items: [...base, item("hose", "plant", ["water"])] }],
    ["retitling an item", { items: base.map((one) => (one.id === "sow" ? { ...one, title: "Sow seeds" } : one)) }],
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

  it("sets a box's title alone, with no counts under it, a line lower", () => {
    const counts = {
      done: 0,
      leaves: 2,
      in_progress: 0,
      ready: 1,
      stalled: 0,
      waiting: 1,
      parked: 0,
      canceled: 0,
    };
    const counted = base.map((one) => (one.id === "beds" ? { ...one, progress: { counts } } : one));
    const oneLine = elements({ items: counted })
      .filter((one) => one.classes.split(" ").includes("one-line"))
      .map((one) => one.data.id);
    expect(oneLine).toEqual(["garden", "plant"]);
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

  it("changes a selection of no thread in classes only, and leaves unlit color out of data", () => {
    const before = elements(start);
    const selected = elements({ items: base, selection: { item: "sow", thread: null } });
    for (const element of selected) {
      expect(element.data).not.toHaveProperty("lit");
      expect(element.data).not.toHaveProperty("dim");
      expect(element.data).not.toHaveProperty("selected");
    }
    const changes = diff(new Map(before.map((element) => [element.data.id, element])), selected);
    expect(changes.add).toEqual([]);
    expect(changes.remove).toEqual([]);
    expect(changes.update).toEqual([
      {
        id: "sow",
        data: null,
        unset: [],
        position: null,
        classes: selected.find((element) => element.data.id === "sow")?.classes,
      },
    ]);

    const recoloured = elements({ items: base, colour: "#123456" });
    expect(diff(new Map(before.map((element) => [element.data.id, element])), recoloured)).toEqual({
      add: [],
      remove: [],
      update: [],
    });
  });

  it("keeps computed lit styles through lit, unlit and recoloured transitions", () => {
    const items = base.map((one) => (one.id === "sow" ? { ...one, thread: "t" } : one));
    const shots: Shot[] = [
      { items, routed: true, selection: { item: null, thread: "t" }, colour: "#ff0000" },
      { items, routed: true, selection: { item: null, thread: null }, colour: "#123456" },
      { items, routed: true, selection: { item: null, thread: "t" }, colour: "#00ff00" },
    ];
    const core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.dark) });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    let drawn = new Map<string, Drawn>();
    const computed = (renderer: cytoscape.Core) => {
      const pstyle = (id: string, property: string) =>
        (
          renderer.getElementById(id) as unknown as {
            pstyle(name: string): { strValue: string };
          }
        ).pstyle(property).strValue;
      return {
        node: [pstyle("dig", "border-color"), pstyle("dig", "border-width")],
        edge: [pstyle("dig->sow", "line-color"), pstyle("dig->sow", "width")],
        junction: [pstyle("junction 20,10", "background-color"), pstyle("junction 20,10", "width")],
      };
    };
    const snapshots: ReturnType<typeof computed>[] = [];
    for (const shot of shots) {
      const next = elements(shot);
      for (const element of next) {
        const stylesLine = element.classes.split(" ").some((one) => one === "lit" || one === "junction-lit");
        expect("line" in element.data).toBe(stylesLine);
      }
      drawn = apply(core, drawn, next);
      snapshots.push(computed(core));
      const reference = cytoscape({
        headless: true,
        styleEnabled: true,
        style: styleFor(PALETTES.dark),
        elements: next,
      });
      expect(computed(core)).toEqual(computed(reference));
      reference.destroy();
    }
    expect(snapshots[0]?.node[0]).not.toEqual(snapshots[2]?.node[0]);
    expect(snapshots[0]?.edge[0]).not.toEqual(snapshots[2]?.edge[0]);
    expect(snapshots[0]?.junction[0]).not.toEqual(snapshots[2]?.junction[0]);
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
    core.destroy();
  });

  it("measures a node it gives a new title at once, so its label isn't kept at the old one's height", () => {
    const core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.dark) });
    const drawn = apply(core, new Map(), elements(start));
    const measured: string[][] = [];
    const prototype = Object.getPrototypeOf(core.collection()) as { recalculateRenderedStyle(useCache: boolean): void };
    const spy = vi.spyOn(prototype, "recalculateRenderedStyle").mockImplementation(function (
      this: cytoscape.Collection,
    ) {
      measured.push(this.map((one) => one.id()));
    });
    apply(
      core,
      drawn,
      elements({ items: base.map((one) => (one.id === "sow" ? { ...one, title: "Sow seeds" } : one)) }),
    );
    spy.mockRestore();
    expect(measured).toEqual([["sow"]]);
  });
});

describe("a junction's dot", () => {
  it("carries the data every node's style reads, so the renderer warns of nothing", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const model = buildModel(base);
    const view = visible(model, { collapsed: new Set() });
    const placed: Placed = {
      width: 500,
      height: 500,
      laidOut: [],
      routes: new Map([
        [
          "dig->sow",
          [
            { x: 0, y: 0 },
            { x: 10, y: 10 },
          ],
        ],
      ]),
      junctions: new Map([["dig->sow", [{ x: 5, y: 5 }]]]),
      dividers: new Map(),
      redundant: new Set(),
      nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 30, y: i * 7, width: 20 + i, height: 10 }])),
    };
    const drawn = elementsOf(
      scene(view, placed, { item: null, thread: null }, () => null),
      undefined,
      linesOf([], PALETTES.dark),
      PALETTES.dark,
    );
    const core = cytoscape({ headless: true, styleEnabled: true, style: styleFor(PALETTES.dark), elements: drawn });
    const dots = core.nodes(".junction");
    for (const dot of dots) (dot as unknown as { pstyle(name: string): unknown }).pstyle("text-max-width");
    expect(dots.length).toBeGreaterThan(0);
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});

describe("a stub", () => {
  const gate = { id: "far", title: "Far", initiative: "Home lab" };
  const shot = (outside: boolean) => {
    const model = buildModel([item("a"), item("b")], outside ? new Map([["a", [gate]]]) : new Map());
    const view = visible(model, { collapsed: new Set() });
    const placed: Placed = {
      width: 500,
      height: 500,
      laidOut: [],
      routes: new Map(),
      junctions: new Map(),
      dividers: new Map(),
      redundant: new Set(),
      nodes: new Map(view.nodes.map((node, i) => [node.id, { x: i * 300, y: 0, width: 200, height: 94 }])),
    };
    return elementsOf(
      scene(view, placed, { item: null, thread: null }, () => null),
      undefined,
      new Map(),
      PALETTES.dark,
    );
  };

  it("is a node of its own that says where a tap goes and what it names in full, drawn over its tile", () => {
    const stub = shot(true).find((one) => one.classes.includes("stub"));
    expect(stub?.data).toMatchObject({
      id: "stub a",
      label: "↗ Home lab: Far",
      target: "far",
      full: "Home lab: Far",
      layer: 1,
    });
    expect(shot(false).some((one) => one.classes.includes("stub"))).toBe(false);
  });

  it("is added and removed with the wait, and the tile keeps the room for it", () => {
    const next = shot(true);
    const change = diff(new Map(shot(false).map((one) => [one.data.id, one])), next);
    expect(change.add.map((one) => one.data.id)).toEqual(["stub a"]);
    expect(change.update.find((one) => one.id === "a")?.data).toMatchObject({ reserve: 30 });
    expect(diff(new Map(next.map((one) => [one.data.id, one])), shot(false)).remove).toEqual(["stub a"]);
  });
});
