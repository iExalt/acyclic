import { describe, expect, it } from "vitest";
import { workOrder } from "./members";

const ids = (...list: string[]) => list.map((id) => ({ id }));

describe("workOrder", () => {
  it("puts each item after what it waits for in the thread, ties by id", () => {
    const waits: Record<string, string[]> = { sow: ["beds", "dig"], water: ["sow"], dig: ["elsewhere"] };
    expect(workOrder(ids("sow", "beds", "dig", "water"), (id) => waits[id] ?? [])).toEqual([
      "beds",
      "dig",
      "sow",
      "water",
    ]);
  });

  it("orders ties in natural id order, case folded, with numeric suffixes as numbers", () => {
    expect(workOrder(ids("item-10", "Item-3", "item-2", "b", "A"), () => [])).toEqual([
      "A",
      "b",
      "item-2",
      "Item-3",
      "item-10",
    ]);
  });

  it("lists isolated items before the graph, even when a linked item is ready", () => {
    expect(workOrder(ids("a", "b", "z"), (id) => (id === "b" ? ["a"] : []))).toEqual(["z", "a", "b"]);
  });

  it("ignores waits outside the thread when deciding which items are isolated", () => {
    expect(workOrder(ids("z", "a"), (id) => (id === "a" ? ["elsewhere"] : []))).toEqual(["a", "z"]);
  });

  it("still lists every item when waits loop", () => {
    const waits: Record<string, string[]> = { sow: ["water"], water: ["sow"] };
    expect(workOrder(ids("sow", "beds", "dig", "water"), (id) => waits[id] ?? [])).toEqual([
      "beds",
      "dig",
      "sow",
      "water",
    ]);
  });
});
