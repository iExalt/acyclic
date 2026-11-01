import { describe, expect, it } from "vitest";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";
import { outlineOf } from "./outline";
import { wavesView } from "./waves";

// Project p holds a's work and some of b's, project q the rest of b's; c waits on b, and b and d on each other.
const items = [
  item("p"),
  item("a1", "p", [], { thread: "a", stage: "done" }),
  item("a2", "p", [], { thread: "a", stage: "in-review" }),
  item("b1", "p", [], { thread: "b", stage: "ready" }),
  item("q"),
  item("b2", "q", [], { thread: "b", stage: "in-progress" }),
  item("b3", "q", [], { thread: "b", archived: true }),
  item("c1", "q", [], { thread: "c", stage: "blocked" }),
  item("d1", "q", [], { thread: "d", stage: "parked" }),
];
const listed = {
  threads: [
    { id: "a", title: "A", waitsFor: [] },
    { id: "b", title: "B", waitsFor: ["d"] },
    { id: "c", title: "C", waitsFor: ["b", "d"] },
    { id: "d", title: "D", waitsFor: ["b"] },
    { id: "e", title: "E", waitsFor: [] },
  ],
  waves: [["b", "d"], ["c"]],
  anytime: ["a", "e"],
};

describe("wavesView", () => {
  const view = wavesView(listed, items, outlineOf(buildModel(items), ["a", "b", "c", "d", "e"], false));

  it("cards each wave's threads, with the projects they work in and what they wait on, earlier or in a cycle", () => {
    expect(view.waves.map((wave) => wave.map((card) => card.id))).toEqual([["b", "d"], ["c"]]);
    expect(view.waves[0]?.[0]).toMatchObject({ projects: ["P", "Q"], after: [], looped: ["d"] });
    expect(view.waves[1]?.[0]).toMatchObject({ projects: ["Q"], after: ["b", "d"], looped: [] });
    expect(view.anytime.map((card) => card.id)).toEqual(["a", "e"]);
  });

  it("counts a thread's leaves by stage, done counting those in review, and none archived", () => {
    expect(view.anytime[0]?.counts).toMatchObject({ leaves: 2, done: 2, in_review: 1 });
    expect(view.waves[0]?.[0]?.counts).toMatchObject({ leaves: 2, ready: 1, in_progress: 1, done: 0 });
    expect(view.anytime[1]?.counts.leaves).toBe(0);
  });
});
