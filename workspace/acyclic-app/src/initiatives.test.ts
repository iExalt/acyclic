import { describe, expect, it } from "vitest";
import {
  contextOf,
  contextTitle,
  crossingWaits,
  fitsFirstView,
  groupRows,
  groupTitle,
  inContext,
  needInitiatives,
  newParent,
  rowLabel,
  shownGroups,
  startProject,
  switcherRows,
  threadInitiatives,
  threadsIn,
} from "./initiatives";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";
import type { Need } from "./needs";
import { outlineOf } from "./outline";

// Two initiatives, "big" with two active leaves, a parked one, a done one and a canceled one, "small" with one, and a project in none.
const plan = [
  item("e1", "big", [], { initiative: "big" }),
  item("a", "e1", [], { initiative: "big", thread: "t1", stage: "ready" }),
  item("b", "e1", [], { initiative: "big", thread: "t1", stage: "in-progress" }),
  item("c", "e1", [], { initiative: "big", stage: "parked" }),
  item("d", "e1", [], { initiative: "big", thread: "t1", stage: "done" }),
  item("x", "e1", [], { initiative: "big", stage: "canceled" }),
  item("e2", "small", [], { initiative: "small" }),
  item("s", "e2", [], { initiative: "small", thread: "t2", stage: "ready" }),
  item("lost", null),
  item("l", "lost", [], { thread: "t2", stage: "ready" }),
];
const model = buildModel(plan);
const initiatives = [
  { id: "small", title: "Small", items: 1 },
  { id: "big", title: "Big", items: 1 },
  { id: "idle", title: "Idle", items: 0 },
];

describe("switcherRows", () => {
  it("counts the active leaves under each initiative, as a parent's stage is its leaves'", () => {
    // Parked, done and canceled leaves are excluded, as are epics, whose stage their leaves give.
    expect(switcherRows(initiatives, model)).toEqual([
      { id: "big", title: "Big", open: 2 },
      { id: "small", title: "Small", open: 1 },
      { id: "idle", title: "Idle", open: 0 },
    ]);
  });

  it("excludes descendants of manually parked parents, keeping other unfinished stages and their precedence", () => {
    const rows = [
      { id: "parked", title: "Parked", items: 3 },
      { id: "active", title: "Active", items: 5 },
    ];
    const model = buildModel([
      item("parked-epic", "parked", [], { initiative: "parked", state: "parked", stage: "parked" }),
      item("nested", "parked-epic", [], { initiative: "parked", stage: "ready" }),
      item("under-parked", "nested", [], { initiative: "parked", stage: "ready" }),
      item("parked-leaf", "parked", [], { initiative: "parked", stage: "parked" }),
      ...(["ready", "in-progress", "waiting", "stalled"] as const).map((stage, at) =>
        item(`active-${at}`, "active", [], { initiative: "active", stage }),
      ),
      item("unknown", "active", [], { initiative: "active" }),
      // A state conflict or cycle takes precedence over a leaf's stored parked state in core.
      item("conflicted", "active", [], { initiative: "active", state: "parked", stage: "waiting" }),
    ]);
    expect(switcherRows(rows, model)).toEqual([
      { id: "active", title: "Active", open: 6 },
      { id: "parked", title: "Parked", open: 0 },
    ]);
  });

  it("keeps children of conflicted parked parents, but excludes manually parked ancestry even with a canceled rollup", () => {
    const model = buildModel([
      item("conflicted-parent", "big", [], {
        initiative: "big",
        state: "parked",
        stage: "ready",
        conflicts: { state: ["parked", "open"] },
      }),
      item("active-child", "conflicted-parent", [], { initiative: "big", stage: "ready" }),
      item("parked-parent", "big", [], { initiative: "big", state: "parked", stage: "canceled" }),
      item("canceled-parent", "parked-parent", [], { initiative: "big", state: "canceled", stage: "canceled" }),
      item("conflicted-child", "canceled-parent", [], {
        initiative: "big",
        stage: "waiting",
        conflicts: { state: ["open", "done"] },
      }),
    ]);
    expect(switcherRows(initiatives, model).find((row) => row.id === "big")?.open).toBe(1);
  });

  it("orders the busiest first, then by title, whichever initiative is in context, and lists every one before the plan loads", () => {
    const tied = [
      { id: "z", title: "Beta", items: 0 },
      { id: "y", title: "Alpha", items: 0 },
    ];
    expect(switcherRows(tied, null).map((row) => row.id)).toEqual(["y", "z"]);
  });
});

describe("the initiative in context", () => {
  const rows = switcherRows(initiatives, model);

  it("is the one chosen, else the first of the switcher, so there is always somewhere for a new item to go", () => {
    expect(contextOf("small", rows)).toBe("small");
    expect(contextOf(null, rows)).toBe("big");
  });

  it("falls back to the first when the one chosen is gone, and is none only with no initiative at all", () => {
    expect(contextOf("deleted-elsewhere", rows)).toBe("big");
    expect(contextOf("big", [])).toBeNull();
  });

  it("is named for the phone's bar, which has no name for none", () => {
    expect(contextTitle("small", rows)).toBe("Small");
    expect(contextTitle(null, rows)).toBeNull();
  });
});

describe("where New item goes", () => {
  it("defaults to the initiative in context, with a project or a parent chosen taking precedence", () => {
    expect(newParent(null, null, "big")).toBe("big");
    expect(newParent(null, "e1", "big")).toBe("e1");
    expect(newParent("a", "e1", "big")).toBe("a");
    // With no initiative at all, none is named, and the server decides.
    expect(newParent(null, null, null)).toBeNull();
  });

  it("starts in the selected item's project only while that is in the initiative in context", () => {
    const of = (id: string) => model.items.get(id)?.initiative;
    expect(startProject("e1", of, "big")).toBe("e1");
    expect(startProject("e1", of, "small")).toBeNull();
    expect(startProject(null, of, "big")).toBeNull();
  });
});

describe("groupRows", () => {
  const order = ["big", "small", "idle"];
  const of = (row: string) =>
    row === "both" ? ["small", "big"] : row.startsWith("b") ? ["big"] : row.startsWith("s") ? ["small"] : [];

  it("puts the initiative in context first, the rest in the switcher's order, and rows in none last", () => {
    const rows = ["s1", "none", "b1", "s2", "b2"];
    expect(groupRows(rows, of, order, "small").map((group) => [group.initiative, group.rows])).toEqual([
      ["small", ["s1", "s2"]],
      ["big", ["b1", "b2"]],
      [null, ["none"]],
    ]);
    expect(groupRows(rows, of, order, null).map((group) => group.initiative)).toEqual(["big", "small", null]);
  });

  it("leaves out an initiative with no rows, and keeps each group's rows in the order given", () => {
    expect(groupRows(["b2", "b1"], of, order, "idle")).toEqual([{ initiative: "big", rows: ["b2", "b1"] }]);
    expect(groupRows([], of, order, "big")).toEqual([]);
  });

  it("lists a row in two initiatives under both, and one in an initiative the list doesn't know with those in none", () => {
    expect(groupRows(["both"], of, order, "big").map((group) => group.initiative)).toEqual(["big", "small"]);
    expect(groupRows(["row"], () => ["unknown"], order, "big")).toEqual([{ initiative: null, rows: ["row"] }]);
  });

  it("titles a group by its initiative, and None as No initiative", () => {
    const titles = new Map([["big", "Big"]]);
    expect(groupTitle("big", titles)).toBe("Big");
    expect(groupTitle(null, titles)).toBe("No initiative");
  });
});

describe("shownGroups", () => {
  const groups = [
    { initiative: "a", rows: [1, 2, 3] },
    { initiative: "b", rows: [4, 5] },
    { initiative: null, rows: [6] },
  ];

  it("gives each group a row first, then the rest in order up to the limit, dropping the heading of a group with none", () => {
    expect(shownGroups(groups, 4)).toEqual([
      { initiative: "a", rows: [1, 2] },
      { initiative: "b", rows: [4] },
      { initiative: null, rows: [6] },
    ]);
    expect(shownGroups(groups, 2)).toEqual([
      { initiative: "a", rows: [1] },
      { initiative: "b", rows: [4] },
    ]);
    expect(shownGroups(groups, 99)).toEqual(groups);
  });

  describe("within the first view, as Up next bounds it", () => {
    const group = (initiative: string, count: number) => ({
      initiative,
      rows: Array.from({ length: count }, (_, at) => `${initiative}${at}`),
    });
    const within = (groups: ReturnType<typeof group>[]) => {
      const shown = shownGroups(groups, 8, fitsFirstView);
      const rows = shown.reduce((sum, one) => sum + one.rows.length, 0);
      expect(fitsFirstView(rows, shown.length)).toBe(true);
      return shown;
    };

    it("shows a one-row initiative beside a busy first one, which takes the rest", () => {
      const shown = within([group("busy", 20), group("small", 1)]);
      expect(shown.map((one) => [one.initiative, one.rows.length])).toEqual([
        ["busy", 7],
        ["small", 1],
      ]);
    });

    it("shows all of six populated initiatives, one row each, and eight rows in two", () => {
      const six = within(Array.from({ length: 6 }, (_, at) => group(`i${at}`, 3)));
      expect(six.map((one) => one.rows.length)).toEqual([1, 1, 1, 1, 1, 1]);
      expect(within([group("a", 5), group("b", 3)]).map((one) => one.rows.length)).toEqual([5, 3]);
    });

    it("counts the initiatives that don't fit, the first ones shown and the bound held", () => {
      const eight = within(Array.from({ length: 8 }, (_, at) => group(`i${at}`, 2)));
      expect(eight.map((one) => one.initiative)).toEqual(["i0", "i1", "i2", "i3", "i4", "i5"]);
    });
  });
});

describe("rowLabel, a phone's trailing label for a row", () => {
  const titles = new Map([
    ["big", "Big"],
    ["small", "Small"],
  ]);

  it("is the initiative's title, and No initiative for a row in none", () => {
    expect(rowLabel(["big"], titles)).toBe("Big");
    expect(rowLabel([null], titles)).toBe("No initiative");
    expect(rowLabel([], titles)).toBe("No initiative");
  });

  it("reads No initiative for an initiative the titles lack, as the headings group it", () => {
    expect(rowLabel(["gone"], titles)).toBe("No initiative");
    expect(rowLabel(["big", "gone"], titles)).toBe("Big +1");
  });

  it("names the one in context first when a row is in several, counting the rest", () => {
    expect(rowLabel(new Set(["big", "small"]), titles)).toBe("Big +1");
    expect(rowLabel(new Set(["big", "small"]), titles, "small")).toBe("Small +1");
    expect(rowLabel(new Set(["big", null, "small"]), titles)).toBe("Big +2");
  });
});

describe("what a thread and an entry are in", () => {
  it("puts a thread under each initiative its tasks are in, and a task in no thread in its own", () => {
    const of = threadInitiatives(model);
    expect(of.get("t1")).toEqual(new Set(["big"]));
    // t2 has a task under small and one in no initiative.
    expect(of.get("t2")).toEqual(new Set(["small", null]));
    // c is a leaf in no thread: the thread of one it stands for.
    expect(of.get("c.one")).toEqual(new Set(["big"]));
    // An epic in no thread stands for no thread.
    expect(of.has("e1.one")).toBe(false);
  });

  it("puts an entry where its item is, as the entry says, else as the plan has it, else where its thread's tasks are", () => {
    const of = threadInitiatives(model);
    const need = (rest: Record<string, unknown>): Need => ({ kind: "x", ...rest });
    expect(needInitiatives(need({ item: { id: "a", initiative: "small" } }), model, of)).toEqual(new Set(["small"]));
    expect(needInitiatives(need({ item: { id: "l", initiative: null } }), model, of)).toEqual(new Set([null]));
    expect(needInitiatives(need({ item: { id: "s" } }), model, of)).toEqual(new Set(["small"]));
    expect(needInitiatives(need({ thread: { id: "t2" } }), model, of)).toEqual(new Set(["small", null]));
    expect(needInitiatives(need({ threads: [{ id: "t1" }, { id: "t2" }] }), model, of)).toEqual(
      new Set(["big", "small", null]),
    );
  });

  describe("an entry naming its items in an array", () => {
    const of = threadInitiatives(model);
    const need = (rest: Record<string, unknown>): Need => ({ kind: "x", ...rest });

    it("`items`, as a cycle does, goes under each item's initiative, the plan saying where they are", () => {
      expect(needInitiatives(need({ items: [{ id: "a" }, { id: "s" }] }), model, of)).toEqual(
        new Set(["big", "small"]),
      );
    });

    it("`dependents` and `open`, as done work waiting on open work does, go under each one's own", () => {
      expect(
        needInitiatives(
          need({ dependents: [{ id: "x", initiative: "big" }], open: [{ id: "l", initiative: null }] }),
          model,
          of,
        ),
      ).toEqual(new Set(["big", null]));
    });

    it("`items` of a duplicate title, which two initiatives may hold, go under the initiative they are in", () => {
      expect(needInitiatives(need({ items: [{ id: "s" }, { id: "e2" }] }), model, of)).toEqual(new Set(["small"]));
    });
  });

  it("puts an entry about nothing the plan holds with the rows in none", () => {
    const of = threadInitiatives(model);
    expect(needInitiatives({ kind: "x" }, model, of)).toEqual(new Set([null]));
    expect(needInitiatives({ kind: "x", thread: { id: "empty" } }, model, of)).toEqual(new Set([null]));
    expect(needInitiatives({ kind: "x", item: { id: "ghost" } }, null, of)).toEqual(new Set([null]));
  });
});

describe("what the Map and the Threads view draw", () => {
  const items = plan;

  it("draws the epics of the initiative in context as the roots, and nothing of the others", () => {
    const big = buildModel(inContext(items, "big"));
    expect(big.roots).toEqual(["e1"]);
    expect([...big.items.keys()].sort()).toEqual(["a", "b", "c", "d", "e1", "x"]);
    const small = buildModel(inContext(items, "small"));
    expect(small.roots).toEqual(["e2"]);
    expect([...small.items.keys()].sort()).toEqual(["e2", "s"]);
  });

  it("draws every item with no initiative in context, and items in none only then", () => {
    expect(inContext(items, null)).toHaveLength(items.length);
    expect(inContext(items, "big").some((one) => one.id === "lost")).toBe(false);
  });

  it("keeps threads with work in the initiative and truly empty threads, but not those only outside it", () => {
    const of = threadInitiatives(model);
    const threads = ["t1", "t2", "empty", "outside"].map((id) => ({ id }));
    const membership = new Map(of);
    membership.set("outside", new Set(["small", null]));
    expect(threadsIn(threads, membership, "big").map((one) => one.id)).toEqual(["t1", "empty"]);
    // t2 has a task in "small" and one in no initiative.
    expect(threadsIn(threads, membership, "small").map((one) => one.id)).toEqual(["t2", "empty", "outside"]);
    membership.set("unassigned", new Set([null]));
    expect(threadsIn([...threads, { id: "unassigned" }], membership, "big").map((one) => one.id)).toEqual([
      "t1",
      "empty",
    ]);
    const spanning = new Map<string, Set<string | null>>([["t", new Set(["big", "small"])]]);
    expect(threadsIn([{ id: "t" }], spanning, "big")).toHaveLength(1);
    expect(threadsIn([{ id: "t" }], spanning, "small")).toHaveLength(1);
    expect(threadsIn(threads, membership, null)).toHaveLength(4);
  });

  it("scopes the outline to the epics of the initiative, and the threads at work in them", () => {
    const small = buildModel(inContext(items, "small"));
    expect(outlineOf(small, ["t2"])).toEqual({
      projects: [{ id: "e2", title: "E2", initiative: "small", threads: [{ id: "t2", also: [] }] }],
      others: [],
    });
    const big = buildModel(inContext(items, "big"));
    expect(outlineOf(big, ["t1"]).projects.map((one) => one.id)).toEqual(["e1"]);
  });
});

describe("crossingWaits", () => {
  const titles = new Map([
    ["big", "Big"],
    ["small", "Small"],
  ]);
  const items = [
    item("a", null, ["s", "l", "d", "gone"], { initiative: "big", stage: "ready" }),
    item("d", null, [], { initiative: "big", stage: "done" }),
    item("b", null, ["s"], { initiative: "big", stage: "done" }),
    item("c", null, ["s"], { initiative: "big", state: "canceled" }),
    item("s", null, [], { initiative: "small", title: "Zed" }),
    item("l", null, [], { initiative: null, title: "Loose" }),
  ];

  it("names each wait an unfinished item in the initiative has on an item in another, in a stub's order", () => {
    const found = crossingWaits(items, "big", titles);
    expect([...found.keys()]).toEqual(["a"]);
    expect(found.get("a")).toEqual([
      { id: "l", title: "Loose", initiative: "No initiative" },
      { id: "s", title: "Zed", initiative: "Small" },
    ]);
  });

  it("leaves out a wait on done work, which is met, and keeps one on canceled work, which is stuck", () => {
    const waiting = [
      item("w", null, ["done", "gone", "cancelled"], { initiative: "big", stage: "ready" }),
      item("done", null, [], { initiative: "small", stage: "done" }),
      item("cancelled", null, [], { initiative: "small", stage: "canceled", state: "canceled" }),
    ];
    expect(
      crossingWaits(waiting, "big", titles)
        .get("w")
        ?.map((one) => one.id),
    ).toEqual(["cancelled"]);
    expect(
      crossingWaits(
        waiting.filter((one) => one.id !== "cancelled"),
        "big",
        titles,
      ).size,
    ).toBe(0);
  });

  it("leaves out waits inside the initiative, on what the plan lacks, and finished work's, and has none with no context", () => {
    expect(crossingWaits(items, "small", titles).size).toBe(0);
    expect(crossingWaits(items, null, titles).size).toBe(0);
  });
});
