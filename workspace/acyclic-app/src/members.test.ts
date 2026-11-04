import { describe, expect, it } from "vitest";
import { workOrder } from "./members";

const members = [
  { id: "sow", title: "Sow" },
  { id: "beds", title: "Build beds" },
  { id: "dig", title: "Dig" },
  { id: "water", title: "Water" },
];

describe("workOrder", () => {
  it("puts each item after what it waits for in the thread, ties by title", () => {
    const waits: Record<string, string[]> = { sow: ["beds", "dig"], water: ["sow"], dig: ["elsewhere"] };
    expect(workOrder(members, (id) => waits[id] ?? [])).toEqual(["beds", "dig", "sow", "water"]);
  });

  it("lists isolated items before the graph, even when a linked item is ready", () => {
    const rows = [
      { id: "a", title: "Upstream" },
      { id: "b", title: "Downstream" },
      { id: "z", title: "Independent" },
    ];
    expect(workOrder(rows, (id) => (id === "b" ? ["a"] : []))).toEqual(["z", "a", "b"]);
  });

  it("ignores waits outside the thread when deciding which items are isolated", () => {
    const rows = [
      { id: "z", title: "Zed" },
      { id: "a", title: "Alone" },
    ];
    expect(workOrder(rows, (id) => (id === "a" ? ["elsewhere"] : []))).toEqual(["a", "z"]);
  });

  it("still lists every item when waits loop", () => {
    const waits: Record<string, string[]> = { sow: ["water"], water: ["sow"] };
    expect(workOrder(members, (id) => waits[id] ?? [])).toEqual(["beds", "dig", "sow", "water"]);
  });
});
