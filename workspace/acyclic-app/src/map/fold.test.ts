import { describe, expect, it } from "vitest";
import type { Item } from "../api";
import { expandTo } from "../edits";
import { item } from "./fixture";
import { foldDone } from "./fold";
import { buildModel, collapsedWith, visible } from "./model";

const done = (id: string, parent: string, thread: string | null, waitsFor: string[] = [], extra: Partial<Item> = {}) =>
  item(id, parent, waitsFor, { state: "done", stage: "done", thread, ...extra });
const ready = (id: string, parent: string, thread: string | null, waitsFor: string[] = []) =>
  item(id, parent, waitsFor, { state: "backlog", stage: "ready", thread });
const fold = (plan: Item[]) => foldDone(buildModel(plan), (thread) => `Thread ${thread}`);
const kidsOf = (folded: ReturnType<typeof fold>, id: string) => folded.model.children.get(id) ?? [];

// p holds two finished threads, a and b, a finished item in no thread, r, after them, and x, still to do.
const chained = (xWaitsForR: boolean) => [
  item("p"),
  done("a1", "p", "a"),
  done("a2", "p", "a", ["a1"]),
  done("b1", "p", "b", ["a1"]),
  done("b2", "p", "b"),
  done("r", "p", null, ["b2"]),
  ready("x", "p", null, xWaitsForR ? ["r"] : []),
];

describe("foldDone", () => {
  it("folds a finished thread's leaves in a box into a group named after it, and those groups into the done pile", () => {
    const folded = fold(chained(false));
    expect(kidsOf(folded, "~thread p a")).toEqual(["a1", "a2"]);
    expect(folded.model.items.get("~thread p a")).toMatchObject({ title: "Thread a", thread: "a", parent: "~done p" });
    // b waits for a, and r for b: all finished, and nothing else is tied to them, so all go in the pile together.
    expect(kidsOf(folded, "~done p")).toEqual(["r", "~thread p a", "~thread p b"]);
    expect(kidsOf(folded, "p")).toEqual(["x", "~done p"]);
    expect(folded.model.items.get("~done p")?.progress).toMatchObject({ done: 5, leaves: 5 });
    expect(folded.groups.get("~done p")).toEqual({ kind: "done", threads: 2 });
    expect(folded.model.items.get("a1")?.parent).toBe("~thread p a");
    expect(folded.model.depth.get("a1")).toBe(3);
  });

  it("keeps finished work out of the pile when links tie it, through each other, to work left in the box", () => {
    const folded = fold(chained(true));
    expect(folded.model.children.has("~done p")).toBe(false);
    expect(kidsOf(folded, "p")).toEqual(["r", "x", "~thread p a", "~thread p b"]);
  });

  it("leaves a thread with work left anywhere, even in another box or in review, as it is", () => {
    const folded = fold([
      item("p"),
      item("q"),
      done("a1", "p", "a"),
      done("a2", "p", "a"),
      ready("a3", "q", "a"),
      done("b1", "p", "b"),
      done("b2", "p", "b"),
      item("b3", "q", [], { state: "in-progress", stage: "in-review", thread: "b" }),
      ready("x", "p", null),
    ]);
    expect(folded.groups.size).toBe(0);
    expect(kidsOf(folded, "p")).toEqual(["a1", "a2", "b1", "b2", "x"]);
  });

  it("counts an archived leaf as no work left, and folds no archived leaf", () => {
    const folded = fold([
      item("p"),
      done("a1", "p", "a"),
      done("a2", "p", "a"),
      ready("a3", "p", "a"),
      item("a3", "p", [], { stage: "ready", thread: "a", archived: true }),
      ready("x", "p", null),
    ]);
    expect(kidsOf(folded, "~thread p a")).toEqual(["a1", "a2"]);
  });

  it("makes no group of one: a finished thread's only leaf in a box stays a tile, and a pile needs two", () => {
    const one = fold([item("p"), done("a1", "p", "a"), ready("x", "p", null)]);
    expect(one.groups.size).toBe(0);
    const two = fold([item("p"), done("a1", "p", "a"), done("r", "p", null), ready("x", "p", null)]);
    expect(kidsOf(two, "~done p")).toEqual(["a1", "r"]);
  });

  it("makes no pile of a box's whole contents, whose own fold already does that", () => {
    const folded = fold([item("p"), done("a1", "p", "a"), done("a2", "p", "a"), done("r", "p", null)]);
    expect(kidsOf(folded, "p")).toEqual(["r", "~thread p a"]);
  });

  it("leaves a thread's leaves inside a child box to that box", () => {
    const folded = fold([
      item("p"),
      item("q", "p"),
      done("a1", "p", "a"),
      done("a2", "p", "a"),
      done("a3", "q", "a"),
      ready("x", "q", null),
    ]);
    expect(kidsOf(folded, "~thread p a")).toEqual(["a1", "a2"]);
    expect(folded.model.items.get("a3")?.parent).toBe("q");
  });
});

describe("a folded model on the Map", () => {
  it("opens with the groups folded, their links lifted to them and those inside dropped", () => {
    const folded = fold([...chained(true), item("far"), ready("y", "far", null, ["a2"])]);
    const view = visible(
      folded.model,
      { collapsed: collapsedWith(folded.model, new Map()), showArchived: false },
      folded.groups,
    );
    const node = view.nodes.find((one) => one.id === "~thread p a");
    expect(node).toMatchObject({
      kind: "collapsed",
      parent: "p",
      note: "2 done",
      group: { kind: "thread", thread: "a" },
    });
    expect(view.edges).toEqual([
      { source: "r", target: "x" },
      { source: "~thread p a", target: "y" },
      { source: "~thread p a", target: "~thread p b" },
      { source: "~thread p b", target: "r" },
    ]);
    expect(view.threads.get("a")).toEqual(["~thread p a"]);
  });

  it("opens every group above an item brought into view", () => {
    const folded = fold(chained(false));
    const toggles = expandTo(folded.model, "a1", new Map());
    expect([...toggles]).toEqual([
      ["~thread p a", false],
      ["~done p", false],
      ["p", false],
    ]);
    const view = visible(
      folded.model,
      { collapsed: collapsedWith(folded.model, toggles), showArchived: false },
      folded.groups,
    );
    expect(view.nodes.find((one) => one.id === "a1")?.parent).toBe("~thread p a");
    expect(view.nodes.find((one) => one.id === "~done p")).toMatchObject({ kind: "box", note: "5 done in 2 threads" });
  });
});
