import { describe, expect, it } from "vitest";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";
import { outlineOf } from "./outline";
import { projectLabels, wavesView } from "./waves";

// Project p holds a's work and some of b's, project q the rest of b's; c waits on b, and b and d on each other.
const items = [
  item("p"),
  item("a1", "p", [], { thread: "a" }),
  item("a2", "p", [], { thread: "a" }),
  item("b1", "p", [], { thread: "b" }),
  item("q"),
  item("b2", "q", [], { thread: "b" }),
  item("c1", "q", [], { thread: "c" }),
  item("d1", "q", [], { thread: "d" }),
];
const none = {
  done: 0,
  leaves: 0,
  in_progress: 0,
  ready: 0,
  stalled: 0,
  waiting: 0,
  parked: 0,
  unthreaded: 0,
  canceled: 0,
};
const listed = {
  threads: [
    { id: "a", title: "A", waitsFor: [], counts: { ...none, leaves: 2, done: 2 } },
    { id: "b", title: "B", waitsFor: ["d"], counts: { ...none, leaves: 3, ready: 1, in_progress: 1, parked: 1 } },
    { id: "c", title: "C", waitsFor: ["b", "d"], counts: { ...none, leaves: 1, waiting: 1 } },
    { id: "d", title: "D", waitsFor: ["b"], counts: { ...none, leaves: 1, parked: 1 } },
    { id: "e", title: "E", waitsFor: [], counts: none },
  ],
  waves: [["b", "d"], ["c"]],
  anytime: ["a", "e"],
};

describe("wavesView", () => {
  const view = wavesView(listed, outlineOf(buildModel(items), ["a", "b", "c", "d", "e"]));

  it("lays out each wave's threads as tiles, with the projects they work in and what they wait on, earlier or in a cycle", () => {
    expect(view.waves.map((wave) => wave.map((tile) => tile.id))).toEqual([["b", "d"], ["c"]]);
    expect(view.waves[0]?.[0]).toMatchObject({ projects: ["P", "Q"], after: [], looped: ["d"] });
    expect(view.waves[1]?.[0]).toMatchObject({ projects: ["Q"], after: ["b", "d"], looped: [] });
    expect(view.anytime.map((tile) => tile.id)).toEqual(["a", "e"]);
  });

  it("takes each tile's counts from core's for its thread, whatever the plan's items say", () => {
    expect(view.anytime[0]?.counts).toBe(listed.threads[0]?.counts);
    expect(view.waves[0]?.[0]?.counts).toMatchObject({ leaves: 3, ready: 1, in_progress: 1, parked: 1 });
    expect(view.anytime[1]?.counts.leaves).toBe(0);
  });
});

describe("projectLabels", () => {
  it.each(["Alpha", "alpha", "  ALPHA  "])("omits a project matching the thread name: %s", (name) => {
    expect(projectLabels([name], "Alpha")).toEqual([]);
  });

  it("keeps other project labels in their original order", () => {
    expect(projectLabels(["Beta", " alpha ", "Gamma"], "Alpha")).toEqual(["Beta", "Gamma"]);
  });
});
