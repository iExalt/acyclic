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
    const waiting = (id: string, title: string) =>
      needView({ kind: "contest", thread: { id, title }, summary: `${title} has a claim to sort out` }, lines);
    // The list's title wins over the one the entry carries, which may be older.
    // The badge-led text picks its thread, so the entry has no row repeating it.
    expect(waiting("t3", "Ship it")).toMatchObject({
      text: "Ship it now has a claim to sort out",
      line: lines.get("t3"),
      thread: "t3",
      threads: [],
    });
    // A thread the list lacks has no badge, and its row the title the entry gives.
    expect(waiting("t9", "Gone")).toMatchObject({
      text: "Gone has a claim to sort out",
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
      // Its words are the core's, as sent.
      expect(view.text).toBe(need.summary);
    }
    const states = conflicts.filter((need) => need.field === "state");
    const holding = (value: string) =>
      states.find((need) => keepsIn(need).some((kept) => kept.value === value && kept.command !== null)) as Need;
    const state = needView(states.find((need) => keepsIn(need).some((kept) => kept.value === "parked")) as Need, none);
    expect(state.actions.map((one) => one.label)).toEqual(["Use parked"]);
    expect(state.note).toMatch(/done: Keep done in a terminal: run acyclic item done/);
    expect(isCollapsible(state)).toBe(false);
    // A state stored as backlog reads, and is kept, as open.
    expect(needView(holding("open"), none).actions.map((one) => one.label)).toEqual(["Use open", "Use parked"]);
    // Canceled against done: canceled is kept here, and done in a terminal, where its evidence comes with it.
    const dropped = needView(holding("canceled"), none);
    expect(dropped.actions.map((one) => one.label)).toEqual(["Use canceled"]);
    expect(dropped.note).toMatch(/done: Keep done in a terminal/);
    // Named, and its values too where they are ids, while the command keeps the id.
    const parent = field("parent");
    const titles = (id: string) => (id === parent.id ? "First" : "Parent");
    const moved = needView(parent, none, titles);
    expect(moved.actions.map((one) => one.label)).toEqual(["Use Parent", "Use the top level"]);
    const said = "Parent: its parent names q, which doesn't exist";
    const gone = needView(
      { kind: "dangling", record: "item", id: "a", field: "parent", target: "q", summary: said },
      none,
      titles,
    );
    expect(gone.text).toBe(said);
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
        summary: "A waits for q, which doesn't exist",
      },
      none,
      titles,
    );
    expect(waits.text).toBe("A waits for q, which doesn't exist");
    expect(waits.note).toBe("acyclic item edit a --no-waits-for q");
    // The waiting item itself gone leaves the older shape, kept as it was.
    const orphan = needView(
      { kind: "dangling", record: "link", id: "x", field: "end", target: "q", summary: "x: its end names q" },
      none,
      titles,
    );
    expect(orphan.text).toBe("x: its end names q");
    // An item in a thread that is gone is opened from its row to choose another, as an item in none is.
    const ghost = needView(
      { kind: "dangling", record: "item", id: "a", field: "thread", target: "t", item: item("a") },
      none,
      titles,
    );
    expect(ghost).toMatchObject({ items: ["a"], note: "Open it to choose its thread in its editor" });
  });

  it("shows a kind it has no command for with what it names and the CLI's hint", () => {
    const cycle = needView(
      { kind: "parent-cycle", items: [item("a"), item("b")], summary: "Parents form a cycle: A, B" },
      none,
    );
    expect(cycle).toMatchObject({ text: "Parents form a cycle: A, B", items: ["a", "b"], actions: [] });
    const example = (kind: string) => (examples.needs as Need[]).find((need) => need.kind === kind) as Need;
    expect(needView(example("done-waiting"), none).text).toBe("Early is done but waits for Late");
    expect(needView(example("waits-on-canceled"), none)).toMatchObject({
      text: "Waiting waits for Dropped, which was canceled",
      actions: [],
    });
    // A thread an item has children over is ignored, and the CLI's command clears it; an empty thread names itself.
    const parent = example("thread-on-parent");
    expect(needView(parent, none)).toMatchObject({
      text: "Parent has children, so its thread Work is ignored",
      items: [(parent.item as { id: string }).id],
      actions: [],
      note: parent.fix,
    });
    expect(parent.notify).toBe(true);
    const empty = needView(example("empty-thread"), none);
    expect(empty).toMatchObject({ text: "Idle has no items", items: [], actions: [] });
    expect(empty.threads.map((one) => one.title)).toEqual(["Idle"]);
    expect(example("empty-thread").notify).toBe(false);
    // A stalled thread is named with its badge-led title, the last holder and when the lease ran out, in core's words.
    expect(needView(example("stalled"), none)).toMatchObject({
      text: "Stale has stalled: session/mac's claim ran out at 2026-10-14T23:00:00Z",
      kind: "stalled",
      actions: [],
      threads: [{ id: "01900000-0000-7000-8000-0000000000f3", title: "Stale" }],
    });
    // A busy agent can read as stalled, so it stays on the list without a notification.
    expect(example("stalled").notify).toBe(false);
    // The key is the list's, whatever the entry names.
    expect(needView({ kind: "contest", key: "contest:t", thread: thread("t"), holders: ["x"] }, none).key).toBe(
      "contest:t",
    );
  });

  it("names the threads a cycle holds, and a title written apart as its first value", () => {
    const cycle = needView(
      {
        kind: "thread-cycle",
        key: "thread-cycle:t:u",
        threads: [thread("t"), thread("u")],
        notify: true,
        summary: "Threads wait for each other in a cycle: T, U",
      },
      none,
    );
    expect(cycle).toMatchObject({
      key: "thread-cycle:t:u",
      kind: "thread-cycle",
      text: "Threads wait for each other in a cycle: T, U",
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
    const said = "Items wait for each other in a cycle: A, Bee";
    expect(needView({ kind: "waits-for-cycle", items: [item("a"), torn], summary: said }, none).text).toBe(said);
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
