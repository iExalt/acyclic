import type { ElkNode } from "elkjs/lib/elk-api.js";
import { describe, expect, it } from "vitest";
import { dependencyOrder, elkLayBox } from "./map/elk";
import { item } from "./map/fixture";
import { type LayBox, LayoutCache, place, ROOT } from "./map/layout";
import { buildModel, defaultView, visible } from "./map/model";
import { currentRows, membersOf, outlineOf, pickedAfter } from "./outline";

// Two projects and a lone item at the top level: t works in both projects, u only in the second, v only in the lone
// item, and w nowhere; the first project's archived part holds x.
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
  it("lists each project with its threads, a thread in two under both, and threads in no project apart", () => {
    expect(outlineOf(model, threads, false)).toEqual({
      projects: [
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

  it("lists projects as the Map packs them: each after those it waits on, through what is inside, else by id", () => {
    const numbered = buildModel([
      item("project-10"),
      item("a", "project-10"),
      item("project-2"),
      item("b", "project-2"),
    ]);
    expect(outlineOf(numbered, [], false).projects.map((project) => project.id)).toEqual(["project-2", "project-10"]);
    // Inside project-1, c waits on d, in side-1: side-1 comes first, as the Map draws it.
    const waiting = buildModel([
      item("project-1"),
      item("c", "project-1", ["d"]),
      item("project-2"),
      item("e", "project-2"),
      item("side-1"),
      item("d", "side-1"),
    ]);
    expect(outlineOf(waiting, [], false).projects.map((project) => project.id)).toEqual([
      "project-2",
      "side-1",
      "project-1",
    ]);
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
    expect(outlineOf(plan, [], false).projects.map((project) => project.id)).toEqual(
      packed.filter((id) => id !== "lone"),
    );
  });

  it("counts what is archived only while it shows", () => {
    expect(outlineOf(model, threads, true).projects[0]?.threads.map((one) => one.id)).toEqual(["t", "x"]);
  });
});

describe("currentRows", () => {
  const outline = outlineOf(model, threads, false);
  it("marks a selected item's project and thread, a project selected itself, and no project for a lone item at the top", () => {
    expect(currentRows(model, outline, "c")).toEqual({ project: "p2", thread: "t" });
    expect(currentRows(model, outline, "p2")).toEqual({ project: "p2", thread: "u" });
    expect(currentRows(model, outline, "lone")).toEqual({ project: null, thread: "v" });
    expect(currentRows(model, outline, null)).toEqual({ project: null, thread: null });
  });
});

describe("membersOf", () => {
  it("lists a thread's items not archived, each after those in the thread it waits for", () => {
    const plan = buildModel([
      item("a", null, ["b"], { thread: "t" }),
      item("b", null, [], { thread: "t" }),
      item("c", null, ["x"], { thread: "t", archived: true }),
      item("x", null, [], { thread: "u" }),
    ]);
    expect(membersOf(plan, "t")).toEqual(["b", "a"]);
    expect(membersOf(plan, "none")).toEqual([]);
  });
});

describe("pickedAfter", () => {
  it("keeps the picked thread for an item in it, and unpicks it for any other, the project above it included", () => {
    expect(pickedAfter(model, "t", "c")).toBe("t");
    expect(pickedAfter(model, "t", "p2")).toBeNull();
    expect(pickedAfter(model, "t", "lone")).toBeNull();
    expect(pickedAfter(model, null, "c")).toBeNull();
  });
});
