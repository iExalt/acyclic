import type { ElkNode } from "elkjs/lib/elk-api.js";
import { describe, expect, it } from "vitest";
import { elkLayBox } from "./map/elk";
import { item } from "./map/fixture";
import { LayoutCache, place } from "./map/layout";
import { buildModel, defaultView, visible } from "./map/model";
import { currentRows, membersOf, outlineOf, pickedAfter } from "./outline";

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

  it("lists phases as the Map packs them: each after those it waits on, through what is inside, else by id", () => {
    const numbered = buildModel([item("phase-10"), item("a", "phase-10"), item("phase-2"), item("b", "phase-2")]);
    expect(outlineOf(numbered, [], false).phases.map((phase) => phase.id)).toEqual(["phase-2", "phase-10"]);
    // Inside phase-1, c waits on d, in side-1: side-1 comes first, as the Map draws it.
    const waiting = buildModel([
      item("phase-1"),
      item("c", "phase-1", ["d"]),
      item("phase-2"),
      item("e", "phase-2"),
      item("side-1"),
      item("d", "side-1"),
    ]);
    expect(outlineOf(waiting, [], false).phases.map((phase) => phase.id)).toEqual(["phase-2", "side-1", "phase-1"]);
  });

  it("lists phases in the order the Map packs its top level in, with its boxes collapsed", async () => {
    const plan = buildModel([
      item("phase-1"),
      item("a", "phase-1"),
      item("b", "a", ["d"]),
      item("lone", null, ["e"]),
      item("side-1"),
      item("c", "side-1"),
      item("d", "c"),
      item("phase-2"),
      item("e", "phase-2"),
    ]);
    let packed: string[] = [];
    // An engine that only notes what the top level is given, and puts everything at the origin.
    const engine = {
      layout: async (graph: ElkNode): Promise<ElkNode> => {
        if (graph.id === "root") packed = (graph.children ?? []).map((child) => child.id);
        const children = graph.children?.map((child) => ({ ...child, x: 0, y: 0 }));
        return { id: graph.id, width: 1, height: 1, ...(children === undefined ? {} : { children }) };
      },
    };
    await place(visible(plan, defaultView(plan)), new LayoutCache(), elkLayBox(engine), 1);
    expect(packed).toEqual(["phase-2", "lone", "side-1", "phase-1"]);
    expect(outlineOf(plan, [], false).phases.map((phase) => phase.id)).toEqual(packed.filter((id) => id !== "lone"));
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

describe("membersOf", () => {
  it("lists every item in a thread, archived too, each after those in the thread it waits for", () => {
    const plan = buildModel([
      item("a", null, ["b"], { thread: "t" }),
      item("b", null, [], { thread: "t" }),
      item("c", null, ["x"], { thread: "t", archived: true }),
      item("x", null, [], { thread: "u" }),
    ]);
    expect(membersOf(plan, "t")).toEqual(["b", "a", "c"]);
    expect(membersOf(plan, "none")).toEqual([]);
  });
});

describe("pickedAfter", () => {
  it("keeps the picked thread for an item in it, and unpicks it for any other, the phase above it included", () => {
    expect(pickedAfter(model, "t", "c")).toBe("t");
    expect(pickedAfter(model, "t", "p2")).toBeNull();
    expect(pickedAfter(model, "t", "lone")).toBeNull();
    expect(pickedAfter(model, null, "c")).toBeNull();
  });
});
