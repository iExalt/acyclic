import { describe, expect, it } from "vitest";
import { threadInitiatives, threadsIn } from "./initiatives";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";
import { outlineOf } from "./outline";
import { outsideOf, projectLabels, scopeThreads, wavesView } from "./waves";

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
  canceled: 0,
};
const listed = {
  threads: [
    { id: "a", title: "A", task: null, waitsFor: [], counts: { ...none, leaves: 2, done: 2 } },
    {
      id: "b",
      title: "B",
      task: null,
      waitsFor: ["d"],
      counts: { ...none, leaves: 3, ready: 1, in_progress: 1, parked: 1 },
    },
    { id: "c", title: "C", task: null, waitsFor: ["b", "d"], counts: { ...none, leaves: 1, waiting: 1 } },
    { id: "d", title: "D", task: null, waitsFor: ["b"], counts: { ...none, leaves: 1, parked: 1 } },
    { id: "e", title: "E", task: null, waitsFor: [], counts: none },
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

describe("scopeThreads", () => {
  const some = {
    threads: [
      { id: "a", title: "A", task: null, waitsFor: [], counts: none },
      { id: "b", title: "B", task: null, waitsFor: ["a"], counts: none },
      { id: "c", title: "C", task: null, waitsFor: ["b"], counts: none },
      { id: "x", title: "X", task: null, waitsFor: [], counts: none },
    ],
    waves: [["a"], ["b"], ["c"]],
    anytime: ["x"],
  };

  it("keeps the threads asked for, closes the gap a wave leaves, and drops waits on those left out", () => {
    const kept = scopeThreads(some, new Set(["a", "c"]));
    expect(kept.waves).toEqual([["a"], ["c"]]);
    expect(kept.anytime).toEqual([]);
    expect(kept.threads.map((one) => [one.id, one.waitsFor])).toEqual([
      ["a", []],
      ["c", []],
    ]);
    expect(scopeThreads(some, new Set(["x"]))).toMatchObject({ waves: [], anytime: ["x"] });
  });

  it("leaves every thread as it was when all are kept", () => {
    expect(scopeThreads(some, new Set(["a", "b", "c", "x"]))).toEqual(some);
  });
});

// Two initiatives: x holds threads p and r, y holds q and s; r waits on q, which waits on p, and s waits on r, so the
// waves, which core computes across both, are [p], [q], [r], [s].
describe("waves across two initiatives", () => {
  const plan = [
    item("px", null, [], { initiative: "x", thread: "p", stage: "ready" }),
    item("qy", null, [], { initiative: "y", thread: "q", stage: "ready", title: "Q task" }),
    item("rx", null, [], { initiative: "x", thread: "r", stage: "waiting" }),
    item("ry", null, [], { initiative: "y", thread: "r", stage: "waiting", title: "Far task" }),
    item("rd", null, [], { initiative: "y", thread: "r", stage: "done" }),
    item("sy", null, [], { initiative: "y", thread: "s", stage: "waiting" }),
  ];
  const model = buildModel(plan);
  const all = {
    threads: [
      { id: "p", title: "P", task: null, waitsFor: [], counts: none },
      { id: "q", title: "Q", task: null, waitsFor: ["p"], counts: none },
      { id: "r", title: "R", task: null, waitsFor: ["q"], counts: none },
      { id: "s", title: "S", task: null, waitsFor: ["r"], counts: none },
    ],
    waves: [["p"], ["q"], ["r"], ["s"]],
    anytime: [],
  };
  const titles = new Map([
    ["x", "Ex"],
    ["y", "Why"],
  ]);
  const ofThread = threadInitiatives(model);
  const viewOf = (context: string) => {
    const keep = new Set(threadsIn(all.threads, ofThread, context).map((one) => one.id));
    const outside = outsideOf(
      all,
      keep,
      model,
      context,
      (id) => `in ${[...(ofThread.get(id) ?? [])].join("+")}`,
      titles,
    );
    return wavesView(scopeThreads(all, keep), outlineOf(model, [...keep]), outside);
  };

  it("numbers each view's waves without gaps, in the order every view shares", () => {
    // r is in both initiatives, so it is in both views; q is only in y, and p only in x.
    expect(viewOf("x").waves.map((wave) => wave.map((tile) => tile.id))).toEqual([["p"], ["r"]]);
    expect(viewOf("y").waves.map((wave) => wave.map((tile) => tile.id))).toEqual([["q"], ["r"], ["s"]]);
  });

  it("names a thread in an earlier wave in another initiative, which the view lacks, not one it shows", () => {
    const x = viewOf("x");
    // r waits on q, in y alone: named, and not an `after` of the view.
    expect(x.waves[1]?.[0]).toMatchObject({ id: "r", after: [], looped: [] });
    expect(x.waves[1]?.[0]?.outside.after).toEqual([{ id: "q", initiative: "in y" }]);
    const y = viewOf("y");
    // q waits on p, in x alone; r, in both, is shown as an after of s, not named as outside.
    expect(y.waves[0]?.[0]?.outside.after).toEqual([{ id: "p", initiative: "in x" }]);
    expect(y.waves[2]?.[0]).toMatchObject({ id: "s", after: ["r"] });
    expect(y.waves[2]?.[0]?.outside.after).toEqual([]);
  });

  it("names the tasks of a thread in other initiatives, unfinished ones, as the tile's own initiative has none of them", () => {
    expect(viewOf("x").waves[1]?.[0]?.outside.tasks).toEqual([{ id: "ry", title: "Far task", initiative: "Why" }]);
    expect(viewOf("y").waves[1]?.[0]?.outside.tasks).toEqual([{ id: "rx", title: "RX", initiative: "Ex" }]);
    expect(viewOf("y").waves[0]?.[0]?.outside.tasks).toEqual([]);
  });

  it("names a thread in the tile's own wave in another initiative as a cycle peer", () => {
    const cycle = {
      ...all,
      threads: all.threads.map((one) => (one.id === "q" ? { ...one, waitsFor: ["p", "r"] } : one)),
      waves: [["p"], ["q", "r"], ["s"]],
    };
    const keep = new Set(["p", "r", "s"]);
    const outside = outsideOf(cycle, keep, model, "x", () => "elsewhere", titles);
    expect(outside.get("r")).toMatchObject({ after: [], looped: [{ id: "q", initiative: "elsewhere" }] });
  });

  it("has nothing outside with no initiative in context", () => {
    expect(outsideOf(all, new Set(["p", "q", "r", "s"]), model, null, () => "", titles).size).toBe(0);
  });
});
