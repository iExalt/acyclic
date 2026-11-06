import { describe, expect, it } from "vitest";
import type { Item } from "./api";
import { linkCommand, unlinkCommand } from "./edits";
import { item } from "./map/fixture";
import { keep, type Plan, undoOf, unkeep } from "./undo";

const plan = (items: Item[]): Plan => ({ items: new Map(items.map((one) => [one.id, one])) });
const before = plan([
  item("garden", null, [], { title: "Garden" }),
  item("beds", "garden", [], { title: "Beds" }),
  item("dig", "garden", [], { title: "Dig", notes: "deep" }),
  item("sow", "beds", ["dig"], { title: "Sow" }),
]);
/** `from` with one item changed as `change` says. */
const changed = (from: Plan, id: string, change: Partial<Item>) =>
  plan([...from.items.values()].map((one) => (one.id === id ? { ...one, ...change } : one)));
/** The plan after changing one item as `change` says. */
const after = (id: string, change: Partial<Item>) => changed(before, id, change);
const links = { waits_for: [], waited_on_by: [] };

describe("undo", () => {
  it("moves an item back where it was, while it is still where it went", () => {
    const undo = undoOf({ item: { edit: { item: "dig", parent: "beds" } } }, before);
    expect(undo?.text).toBe("Moved Dig into Beds");
    // Put back elsewhere on the Map, it is brought into view there, as the move brought it.
    expect(undo?.shows).toBe("dig");
    expect(undo?.command).toEqual({ item: { edit: { item: "dig", parent: "garden" } } });
    expect(undo?.expect).toEqual([{ kind: "item-field", item: "dig", field: "parent", value: "beds" }]);
    expect(undo?.holds(after("dig", { parent: "beds" }))).toBe(true);
    expect(undo?.holds(after("dig", { parent: null }))).toBe(false);
    const top = undoOf({ item: { edit: { item: "garden", parent: "beds" } } }, before);
    expect(top?.command).toEqual({ item: { edit: { item: "garden", "no-parent": true } } });
    expect(undoOf({ item: { edit: { item: "dig", "no-parent": true } } }, before)?.text).toBe(
      "Moved Dig to the top level",
    );
  });

  it("sets an item's fields back, clearing one that was empty, while each still holds what was set", () => {
    const undo = undoOf({ item: { edit: { item: "dig", title: "Dig deep", "no-notes": true } } }, before);
    expect(undo?.command).toEqual({ item: { edit: { item: "dig", title: "Dig", notes: "deep" } } });
    expect(undo?.expect).toEqual([
      { kind: "item-field", item: "dig", field: "title", value: "Dig deep" },
      { kind: "item-field", item: "dig", field: "notes", value: null },
    ]);
    expect(undo?.holds(after("dig", { title: "Dig deep", notes: null }))).toBe(true);
    // Someone else changed one of them since: the undo would take back their change too.
    expect(undo?.holds(after("dig", { title: "Dig deeper", notes: null }))).toBe(false);
    const filled = undoOf({ item: { edit: { item: "sow", notes: "rows" } } }, before);
    expect(filled?.command).toEqual({ item: { edit: { item: "sow", "no-notes": true } } });
  });

  it("takes back a waits-for added or removed, as item edits, while it is still so", () => {
    const added = undoOf(linkCommand("sow", "garden"), before);
    expect(added?.command).toEqual(unlinkCommand("sow", "garden"));
    expect(added?.expect).toEqual([{ kind: "waits-for", item: "sow", target: "garden", present: true }]);
    expect(added?.holds(before)).toBe(false);
    expect(added?.holds(after("sow", { links: { ...links, waits_for: ["dig", "garden"] } }))).toBe(true);
    // Said as the link reads, so which way it runs is plain.
    expect([added?.text, added?.undone]).toEqual(["Sow now waits for Garden", "Sow no longer waits for Garden"]);
    const removed = undoOf(unlinkCommand("sow", "dig"), before);
    expect(removed?.command).toEqual(linkCommand("sow", "dig"));
    expect(removed?.expect).toEqual([
      { kind: "waits-for", item: "sow", target: "dig", present: false },
      { kind: "started-at-most", item: "sow", count: 0 },
    ]);
    expect(removed?.holds(before)).toBe(false);
    const gone = after("sow", { links });
    expect(removed?.holds(gone)).toBe(true);
    expect([removed?.text, removed?.undone]).toEqual(["Sow no longer waits for Dig", "Sow waits for Dig again"]);
  });

  it("names an edited item as it now reads", () => {
    expect(undoOf({ item: { edit: { item: "dig", title: "Dig deep" } } }, before)?.text).toBe("Edited Dig deep");
  });

  it("offers nothing for a write without an exact inverse", () => {
    for (const command of [
      { item: "list" },
      { item: { add: { item: "hose", title: "Hose" } } },
      { item: { reopen: { item: "dig" } } },
      { thread: { edit: { thread: "t", title: "T" } } },
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
    const removed = undoOf(unlinkCommand("sow", "dig"), before);
    expect(removed?.holds(after("sow", { links, stage: "ready" }))).toBe(true);
    expect(removed?.holds(after("sow", { links, stage: "in-progress" }))).toBe(false);
  });

  it("sets a state back only while nothing waiting on the item, or on a box above it, has started since", () => {
    const done = undoOf({ item: { edit: { item: "dig", state: "done" } } }, before);
    expect(done?.command).toEqual({ item: { edit: { item: "dig", state: "open" } } });
    expect(done?.expect).toEqual([
      { kind: "item-field", item: "dig", field: "state", value: "done" },
      { kind: "waiters-started-at-most", item: "dig", baselines: [{ item: "sow", count: 0 }] },
    ]);
    const dug = after("dig", { state: "done" });
    expect(done?.holds(dug)).toBe(true);
    expect(done?.holds(changed(dug, "sow", { stage: "in-progress" }))).toBe(false);
    // Here Sow waits on Dig's box, Garden, which Dig being done may complete.
    const boxed = changed(before, "sow", { links: { ...links, waits_for: ["garden"] } });
    const outer = undoOf({ item: { edit: { item: "dig", state: "done" } } }, boxed);
    expect(outer?.expect).toEqual([
      { kind: "item-field", item: "dig", field: "state", value: "done" },
      { kind: "waiters-started-at-most", item: "dig", baselines: [{ item: "sow", count: 0 }] },
    ]);
    const boxDone = changed(boxed, "dig", { state: "done" });
    expect(outer?.holds(boxDone)).toBe(true);
    expect(outer?.holds(changed(boxDone, "sow", { stage: "in-progress" }))).toBe(false);
  });

  it("keeps the other end of a waits-for over a reload", () => {
    const kept = keep(linkCommand("sow", "garden"), before, "", "");
    expect(kept.items.map((one) => one.id).sort()).toEqual(["beds", "garden", "sow"]);
  });

  it("offers the same Undo after a reload, saying what it said, with its guard against the plan as it is then", () => {
    const command = { item: { edit: { item: "dig", parent: "beds" } } };
    const kept = unkeep(JSON.parse(JSON.stringify(keep(command, before, "Moved Dig into Beds", "Moved Dig back"))));
    expect([kept?.text, kept?.undone, kept?.command]).toEqual([
      "Moved Dig into Beds",
      "Moved Dig back",
      { item: { edit: { item: "dig", parent: "garden" } } },
    ]);
    expect([kept?.holds(after("dig", { parent: "beds" })), kept?.holds(before)]).toEqual([true, false]);
    expect(kept?.expect).toEqual([{ kind: "item-field", item: "dig", field: "parent", value: "beds" }]);
    // Only what an undo of an edit reads is kept, not the whole plan: the item, its boxes, what waits on them.
    expect(keep(command, before, "", "").items.map((one) => one.id)).toEqual(["beds", "dig", "garden", "sow"]);
  });

  it("keeps what a state's guard reads over a reload: what waits on the item or a box above it", () => {
    const command = { item: { edit: { item: "dig", state: "done" } } };
    const kept = unkeep(JSON.parse(JSON.stringify(keep(command, before, "Edited Dig", "Set Dig back"))));
    expect(kept?.holds(after("dig", { state: "done" }))).toBe(true);
    expect(kept?.holds(changed(after("dig", { state: "done" }), "sow", { stage: "in-progress" }))).toBe(false);
    expect(kept?.expect).toEqual([
      { kind: "item-field", item: "dig", field: "state", value: "done" },
      { kind: "waiters-started-at-most", item: "dig", baselines: [{ item: "sow", count: 0 }] },
    ]);
  });
});
