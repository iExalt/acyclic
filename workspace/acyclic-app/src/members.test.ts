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

  it("still lists every item when waits loop", () => {
    const waits: Record<string, string[]> = { sow: ["water"], water: ["sow"] };
    expect(workOrder(members, (id) => waits[id] ?? [])).toEqual(["beds", "dig", "sow", "water"]);
  });
});
