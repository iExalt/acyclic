import { describe, expect, it } from "vitest";
import type { Item } from "./api";
import { item } from "./map/fixture";
import { type Plan, undoOf } from "./undo";

const plan = (items: Item[], level = "blockers"): Plan => ({
  items: new Map(items.map((one) => [one.id, one])),
  level,
});
const before = plan([
  item("garden", null, [], { title: "Garden" }),
  item("beds", "garden", [], { title: "Beds" }),
  item("dig", "garden", [], { title: "Dig", notes: "deep", gate: null }),
  item("sow", "beds", ["dig"], { title: "Sow" }),
]);
/** The plan after changing one item as `change` says. */
const after = (id: string, change: Partial<Item>) =>
  plan([...before.items.values()].map((one) => (one.id === id ? { ...one, ...change } : one)));

describe("undo", () => {
  it("moves an item back where it was, while it is still where it went", () => {
    const undo = undoOf({ item: { edit: { item: "dig", parent: "beds" } } }, before);
    expect(undo?.text).toBe("Moved Dig into Beds");
    expect(undo?.command).toEqual({ item: { edit: { item: "dig", parent: "garden" } } });
    expect(undo?.holds(after("dig", { parent: "beds" }))).toBe(true);
    expect(undo?.holds(after("dig", { parent: null }))).toBe(false);
    const top = undoOf({ item: { edit: { item: "garden", parent: "beds" } } }, before);
    expect(top?.command).toEqual({ item: { edit: { item: "garden", "no-parent": true } } });
    expect(undoOf({ item: { edit: { item: "dig", "no-parent": true } } }, before)?.text).toBe(
      "Moved Dig to the top level",
    );
  });

  it("sets an item's fields back, clearing one that was empty, while each still holds what was set", () => {
    const undo = undoOf({ item: { edit: { item: "dig", title: "Dig deep", "no-notes": true, gate: "soil" } } }, before);
    expect(undo?.command).toEqual({ item: { edit: { item: "dig", title: "Dig", notes: "deep", "no-gate": true } } });
    expect(undo?.holds(after("dig", { title: "Dig deep", notes: null, gate: "soil" }))).toBe(true);
    // Someone else changed one of them since: the undo would take back their change too.
    expect(undo?.holds(after("dig", { title: "Dig deeper", notes: null, gate: "soil" }))).toBe(false);
  });

  it("takes back a link added or removed, while it is still so", () => {
    const added = undoOf({ link: { add: { from: "sow", kind: "relates-to", to: "garden" } } }, before);
    expect(added?.command).toEqual({ link: { remove: { from: "sow", kind: "relates-to", to: "garden" } } });
    expect(added?.text).toBe("Linked Sow and Garden");
    const removed = undoOf({ link: { remove: { from: "sow", kind: "waits-for", to: "dig" } } }, before);
    expect(removed?.command).toEqual({ link: { add: { from: "sow", kind: "waits-for", to: "dig" } } });
    expect(removed?.holds(before)).toBe(false);
    const gone = after("sow", { links: { waits_for: [], waited_on_by: [], relates_to: [], related_from: [] } });
    expect(removed?.holds(gone)).toBe(true);
  });

  it("sets the autonomy back, while it is still what was set", () => {
    const undo = undoOf({ level: { level: "all" } }, before);
    expect(undo?.command).toEqual({ level: { level: "blockers" } });
    expect([undo?.holds(plan([], "all")), undo?.holds(plan([], "none"))]).toEqual([true, false]);
  });

  it("offers nothing for a write without an exact inverse", () => {
    for (const command of [
      "list",
      { item: { add: { item: "hose", title: "Hose" } } },
      { item: { reopen: { item: "dig" } } },
      { thread: { approve: { thread: "t" } } },
      { proposal: { keep: { proposal: "p" } } },
      { item: { edit: { item: "gone", title: "X" } } },
      { item: { edit: { item: "dig", parent: "beds", title: "X" } } },
      { item: { edit: { item: "dig", flavour: "X" } } },
    ])
      expect(undoOf(command, before), JSON.stringify(command)).toBeNull();
  });

  it("leaves a thread changed as it is, undoing the rest of the edit, and offers nothing for a thread alone", () => {
    const undo = undoOf({ item: { edit: { item: "dig", title: "Dig deep", thread: "t" } } }, before);
    expect(undo?.command).toEqual({ item: { edit: { item: "dig", title: "Dig" } } });
    expect(undo?.holds(after("dig", { title: "Dig deep", thread: "t" }))).toBe(true);
    expect(undoOf({ item: { edit: { item: "dig", thread: "t" } } }, before)).toBeNull();
  });

  it("puts back a wait only while the work it held back hasn't started", () => {
    const removed = undoOf({ link: { remove: { from: "sow", kind: "waits-for", to: "dig" } } }, before);
    const unlinked = { waits_for: [], waited_on_by: [], relates_to: [], related_from: [] };
    expect(removed?.holds(after("sow", { links: unlinked, stage: "ready" }))).toBe(true);
    expect(removed?.holds(after("sow", { links: unlinked, stage: "in-progress" }))).toBe(false);
  });
});
