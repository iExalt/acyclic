import { describe, expect, it } from "vitest";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";
import { currentRows, outlineOf } from "./outline";

// Two phases and a task at the top level: t works in both phases, u only in the second, v only in the lone task, and
// w nowhere; the first phase's archived part holds x.
const model = buildModel([
  item("p1"),
  item("a", "p1", [], { thread: "t" }),
  item("old", "p1", [], { archived: true }),
  item("gone", "old", [], { thread: "x" }),
  item("p2", null, [], { thread: "u" }),
  item("b", "p2"),
  item("c", "b", [], { thread: "t" }),
  item("lone", null, [], { thread: "v" }),
]);
const threads = ["t", "u", "v", "w", "x"];

describe("outlineOf", () => {
  it("lists each phase with its threads, a thread in two under both, and threads in no phase apart", () => {
    expect(outlineOf(model, threads, false)).toEqual({
      phases: [
        { id: "p1", title: "P1", threads: [{ id: "t", also: ["p2"] }] },
        {
          id: "p2",
          title: "P2",
          threads: [
            { id: "t", also: ["p1"] },
            { id: "u", also: [] },
          ],
        },
      ],
      others: ["v", "w", "x"],
    });
  });

  it("counts what is archived only while it shows", () => {
    expect(outlineOf(model, threads, true).phases[0]?.threads.map((one) => one.id)).toEqual(["t", "x"]);
  });
});

describe("currentRows", () => {
  const outline = outlineOf(model, threads, false);
  it("marks a selected item's phase and thread, a phase selected itself, and no phase for a task at the top", () => {
    expect(currentRows(model, outline, "c")).toEqual({ phase: "p2", thread: "t" });
    expect(currentRows(model, outline, "p2")).toEqual({ phase: "p2", thread: "u" });
    expect(currentRows(model, outline, "lone")).toEqual({ phase: null, thread: "v" });
    expect(currentRows(model, outline, null)).toEqual({ phase: null, thread: null });
  });
});
