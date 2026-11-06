import { describe, expect, it } from "vitest";
import { type Line, linesOf } from "./lines";
import { countText, isCollapsible, keepsIn, type Need, needView, structuralKey, threadLabel } from "./needs";
import examples from "./needs.examples.json";
import { PALETTES } from "./palette";

const item = (id: string) => ({ id, title: id.toUpperCase() });
const none = new Map<string, Line>();
const thread = (id: string) => ({ id, title: id.toUpperCase() });

describe("needView", () => {
  it("names a thread as the thread list does, with its badge, and as the entry gives it where the list lacks it", () => {
    const lines = linesOf(
      [
        { id: "t3", title: "Ship it now" },
        { id: "t7", title: "Build it" },
      ],
      PALETTES.dark,
    );
    const waiting = (id: string, title: string) => needView({ kind: "contest", thread: { id, title } }, lines);
    // The list's title wins over the one the entry carries, which may be older.
    // The badge-led text picks its thread, so the entry has no row repeating it.
    expect(waiting("t3", "Ship it")).toMatchObject({
      text: "Two holders claim one thread: Ship it now",
      line: lines.get("t3"),
      thread: "t3",
      threads: [],
    });
    // A thread the list lacks has no badge, and its row the title the entry gives.
    expect(waiting("t9", "Gone")).toMatchObject({
      text: "Two holders claim one thread: Gone",
      line: undefined,
      thread: null,
      threads: [{ id: "t9", title: "Gone" }],
    });
    expect(threadLabel({ id: "t9", title: "Gone" }, lines)).toEqual({ line: undefined, text: "Gone" });
    expect(threadLabel({ id: "t3", title: "Ship it" }, lines)).toEqual({
      line: lines.get("t3"),
      text: "Ship it now",
    });
    // Text that doesn't name its thread has no badge: the thread's row below already shows it.
    const torn = needView(
      {
        kind: "conflict",
        record: "thread",
        id: "t7",
        field: "notes",
        values: ["x", "y"],
        thread: { id: "t7", title: "Build it" },
      },
      lines,
    );
    expect(torn).toMatchObject({ line: undefined, thread: null, threads: [{ id: "t7", title: "Build it" }] });
  });

  it("settles a conflict with the command the list gives for each value it can keep, and says why for the rest", () => {
    const conflicts = (examples.needs as Need[]).filter((need) => need.kind === "conflict");
    const field = (name: string) => {
      const found = conflicts.find((need) => need.field === name);
      if (found === undefined) throw new Error(`no ${name} conflict in needs.examples.json`);
      return found;
    };
    for (const need of conflicts) {
      const view = needView(need, none);
      // Every command the list offers is an action, exactly as sent; every other value says why.
      expect(view.actions.map((one) => one.command)).toEqual(
        keepsIn(need)
          .map((kept) => kept.command)
          .filter((command) => command !== null),
      );
      expect(view.key).toBe(need.key);
    }
    const state = needView(field("state"), none);
    expect(state.actions.map((one) => one.label)).toEqual(["Use parked"]);
    expect(state.note).toMatch(/done: Keep done in a terminal: run acyclic item done/);
    expect(isCollapsible(state)).toBe(false);
    // A state stored as backlog reads, and is kept, as open.
    const open = conflicts.find((need) => need.field === "state" && need !== field("state"));
    expect(open === undefined ? [] : needView(open, none).actions.map((one) => one.label)).toEqual([
      "Use open",
      "Use parked",
    ]);
    // Named, and its values too where they are ids, while the command keeps the id.
    const parent = field("parent");
    const titles = (id: string) => (id === parent.id ? "First" : "Parent");
    const moved = needView(parent, none, titles);
    expect(moved.text).toBe("First: its parent was written differently on two machines");
    expect(moved.actions.map((one) => one.label)).toEqual(["Use Parent", "Use the top level"]);
    const gone = needView({ kind: "dangling", record: "item", id: "a", field: "parent", target: "q" }, none, titles);
    expect(gone.text).toBe("Parent: its parent names q, which doesn't exist");
  });

  it("says a dangling waits-for as the plan reads, with the CLI's fix, and any other dangling field as before", () => {
    const titles = (id: string) => id.toUpperCase();
    const waits = needView(
      {
        kind: "dangling",
        record: "item",
        id: "a",
        field: "waits-for",
        target: "q",
        fix: "acyclic item edit a --no-waits-for q",
      },
      none,
      titles,
    );
    expect(waits.text).toBe("A waits for q, which doesn't exist");
    expect(waits.note).toBe("acyclic item edit a --no-waits-for q");
    // The waiting item itself gone leaves the older shape, kept as it was.
    const orphan = needView({ kind: "dangling", record: "link", id: "x", field: "end", target: "q" }, none, titles);
    expect(orphan.text).toBe("x: its end names q, which doesn't exist");
    // An item in a thread that is gone is opened from its row to choose another, as an item in none is.
    const ghost = needView(
      { kind: "dangling", record: "item", id: "a", field: "thread", target: "t", item: item("a") },
      none,
      titles,
    );
    expect(ghost).toMatchObject({ items: ["a"], note: "Open it to choose its thread in its editor" });
  });

  it("shows a kind it has no command for with what it names and the CLI's hint", () => {
    const cycle = needView({ kind: "parent-cycle", items: [item("a"), item("b")] }, none);
    expect(cycle).toMatchObject({ text: "Parents form a cycle: A, B", items: ["a", "b"], actions: [] });
    // An item needing a thread says so in words, not the CLI's command, and names the item once, as its row.
    const loose = needView({ kind: "unthreaded", item: item("o"), fix: "acyclic item edit o --thread <THREAD>" }, none);
    expect(loose).toMatchObject({
      text: "Needs a thread",
      items: ["o"],
      note: "Open it to choose its thread in its editor",
    });
    expect(needView({ kind: "something-new" }, none).text).toBe("something-new");
    // The key is the list's, whatever the entry names.
    expect(needView({ kind: "contest", key: "contest:t", thread: thread("t"), holders: ["x"] }, none).key).toBe(
      "contest:t",
    );
  });

  it("names the threads a cycle holds, and a title written apart as its first value", () => {
    const cycle = needView(
      { kind: "thread-cycle", key: "thread-cycle:t:u", threads: [thread("t"), thread("u")], notify: true },
      none,
    );
    expect(cycle).toMatchObject({
      key: "thread-cycle:t:u",
      kind: "thread-cycle",
      text: "Threads wait on each other in a cycle: T, U",
      threads: [
        { id: "t", title: "T" },
        { id: "u", title: "U" },
      ],
      thread: null,
      actions: [],
    });
    expect(isCollapsible(cycle)).toBe(true);
    expect(isCollapsible(needView({ kind: "thread-cycle", threads: [thread("t")] }, none))).toBe(false);
    // A conflicted title reads as its first value, as the item's own panel shows it, not as its id.
    const torn = { id: "b", title: { conflict: ["Bee", "Bea"] } };
    expect(needView({ kind: "waits-for-cycle", items: [item("a"), torn] }, none).text).toBe(
      "Links form a cycle: A, Bee",
    );
  });
});

describe("structuralKey", () => {
  it("ignores object key order and keeps array order", () => {
    expect(structuralKey({ evidence: { commit: "abc", repository: "r" } })).toBe(
      structuralKey({ evidence: { repository: "r", commit: "abc" } }),
    );
    expect(structuralKey(["a", "b"])).not.toBe(structuralKey(["b", "a"]));
  });
});

describe("the needs-you list's count", () => {
  it("counts exactly to 99, then says 99+", () => {
    expect([0, 46, 99, 100, 120].map(countText)).toEqual(["0", "46", "99", "99+", "99+"]);
  });
});
