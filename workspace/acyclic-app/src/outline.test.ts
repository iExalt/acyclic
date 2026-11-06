import type { ElkNode } from "elkjs/lib/elk-api.js";
import { describe, expect, it } from "vitest";
import { dependencyOrder, elkLayBox } from "./map/elk";
import { item } from "./map/fixture";
import { type LayBox, LayoutCache, place, ROOT } from "./map/layout";
import { buildModel, defaultView, visible } from "./map/model";
import { currentRows, outlineOf, uniqueProjectThread } from "./outline";

// Two projects and a lone item at the top level: t works in both projects, u only in the second, v only in the lone
// item, and w and x nowhere.
const model = buildModel([
  item("p1"),
  item("a", "p1", [], { thread: "t" }),
  item("p2", null, [], { thread: "u" }),
  item("b", "p2"),
  item("c", "b", [], { thread: "t" }),
  item("lone", null, [], { thread: "v" }),
]);
const threads = ["t", "u", "v", "w", "x"];

describe("outlineOf", () => {
  it("lists each project with its threads, a thread in two under both, and threads in no project apart", () => {
    expect(outlineOf(model, threads)).toEqual({
      projects: [
        { id: "p1", title: "P1", noInitiative: true, threads: [{ id: "t", also: ["p2"] }] },
        {
          id: "p2",
          title: "P2",
          noInitiative: true,
          threads: [
            { id: "t", also: ["p1"] },
            { id: "u", also: [] },
          ],
        },
      ],
      others: ["v", "w", "x"],
    });
  });

  it("marks a project in no initiative, and not one under an initiative that the items don't include", () => {
    const tops = buildModel([
      item("epic", "init", [], { initiative: "init" }),
      item("a", "epic", [], { initiative: "init" }),
      item("lost"),
      item("b", "lost"),
    ]);
    expect(outlineOf(tops, []).projects.map((project) => [project.id, project.noInitiative])).toEqual([
      ["epic", false],
      ["lost", true],
    ]);
  });

  it("lists projects as the Map packs them: each after those it waits on, through what is inside, else by id", () => {
    const numbered = buildModel([
      item("project-10"),
      item("a", "project-10"),
      item("project-2"),
      item("b", "project-2"),
    ]);
    expect(outlineOf(numbered, []).projects.map((project) => project.id)).toEqual(["project-2", "project-10"]);
    // Inside project-1, c waits on d, in side-1: side-1 comes first, as the Map draws it.
    const waiting = buildModel([
      item("project-1"),
      item("c", "project-1", ["d"]),
      item("project-2"),
      item("e", "project-2"),
      item("side-1"),
      item("d", "side-1"),
    ]);
    expect(outlineOf(waiting, []).projects.map((project) => project.id)).toEqual(["project-2", "side-1", "project-1"]);
  });

  it("lists projects in the dependency order the Map packs its top level by, with its boxes collapsed", async () => {
    const plan = buildModel([
      item("project-1"),
      item("a", "project-1"),
      item("b", "a", ["d"]),
      item("lone", null, ["e"]),
      item("side-1"),
      item("c", "side-1"),
      item("d", "c"),
      item("project-2"),
      item("e", "project-2"),
    ]);
    let packed: string[] = [];
    // An engine that puts everything at the origin, and a layout that notes the order of what the top level is given.
    const engine = {
      layout: async (graph: ElkNode): Promise<ElkNode> => {
        const children = graph.children?.map((child) => ({ ...child, x: 0, y: 0 }));
        return { id: graph.id, width: 1, height: 1, ...(children === undefined ? {} : { children }) };
      },
    };
    const lay = elkLayBox(engine);
    const noted: LayBox = async (input) => {
      if (input.id === ROOT)
        packed = dependencyOrder(
          input.children.map((child) => child.id),
          input.edges,
        );
      return lay(input);
    };
    await place(visible(plan, defaultView(plan)), new LayoutCache(), noted, 1);
    expect(packed).toEqual(["project-2", "lone", "side-1", "project-1"]);
    expect(outlineOf(plan, []).projects.map((project) => project.id)).toEqual(packed.filter((id) => id !== "lone"));
  });
});

describe("currentRows", () => {
  const outline = outlineOf(model, threads);
  it("marks a selected item's project and thread, a project selected itself, and no project for a lone item at the top", () => {
    expect(currentRows(model, outline, "c")).toEqual({ project: "p2", thread: "t" });
    expect(currentRows(model, outline, "p2")).toEqual({ project: "p2", thread: "u" });
    expect(currentRows(model, outline, "lone")).toEqual({ project: null, thread: "v" });
    expect(currentRows(model, outline, null)).toEqual({ project: null, thread: null });
  });
});

describe("uniqueProjectThread", () => {
  const outline = outlineOf(model, threads);
  it("infers only a project's sole visible thread", () => {
    expect(uniqueProjectThread("p1", outline.projects)).toBe("t");
    expect(uniqueProjectThread("p2", outline.projects)).toBeNull();
    expect(uniqueProjectThread("empty", [{ id: "empty", threads: [] }])).toBeNull();
    expect(uniqueProjectThread("missing", outline.projects)).toBeNull();
    expect(uniqueProjectThread(null, outline.projects)).toBeNull();
  });
});

describe("a task in no thread, a thread of one", () => {
  const one = buildModel([
    item("p"),
    item("claimed", "p", [], { thread: "claimed.one" }),
    item("idle", "p"),
    item("alone"),
    item("a", "p", [], { thread: "t" }),
  ]);
  const listed = ["t", "claimed.one", "idle.one", "alone.one"];

  it("is in the projects its task is in, claimed or not, and else among the others", () => {
    expect(outlineOf(one, listed)).toEqual({
      projects: [
        {
          id: "p",
          title: "P",
          threads: [
            { id: "t", also: [] },
            { id: "claimed.one", also: [] },
            { id: "idle.one", also: [] },
          ],
        },
      ],
      others: ["alone.one"],
    });
  });

  it("marks its row for the task selected", () => {
    const outline = outlineOf(one, listed);
    expect(currentRows(one, outline, "idle")).toEqual({ project: "p", thread: "idle.one" });
    expect(currentRows(one, outline, "alone")).toEqual({ project: null, thread: "alone.one" });
    // One the plan doesn't list as a thread marks none.
    expect(currentRows(one, outlineOf(one, ["t"]), "idle")).toEqual({ project: "p", thread: null });
  });
});
