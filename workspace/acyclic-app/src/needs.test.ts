import { describe, expect, it } from "vitest";
import { type Line, linesOf } from "./lines";
import { countText, needView, threadLabel, together } from "./needs";
import { PALETTES } from "./palette";

const item = (id: string) => ({ id, title: id.toUpperCase() });
const none = new Map<string, Line>();
const thread = (id: string, heads: string[] = []) => ({ id, title: id.toUpperCase(), heads });

describe("needView", () => {
  it("says which merge it can't confirm, and what to do about it", () => {
    const unverified = (reason: string) =>
      needView({ kind: "unverified", item: { id: "i1", title: "Move in" }, repository: "app", reason }, none);
    expect(unverified("no clone of the repository is mapped")).toMatchObject({
      text: "Can't confirm its merge into app",
      detail: "Move in",
      items: ["i1"],
      note: "This machine has no clone of app to look for merged commits in: add the clone's path to ~/.config/acyclic/clones.toml",
    });
    expect(unverified("the clone has no main branch").note).toBe(
      "Looking for merged commits in this machine's clone of app failed (the clone has no main branch): fetch in that clone, or check its path in ~/.config/acyclic/clones.toml",
    );
  });

  it("puts the merges one cause leaves unconfirmed in one repository in one entry, where the first was", () => {
    const unverified = (id: string, repository: string, reason: string) =>
      needView({ kind: "unverified", item: { id, title: id }, repository, reason }, none);
    const views = [
      unverified("i1", "app", "the clone has no main branch"),
      unverified("i2", "cli", "the clone has no main branch"),
      ...["i3", "i4", "i5", "i6", "i7", "i8"].map((id) => unverified(id, "app", "the clone has no main branch")),
      unverified("i9", "app", "not a commit id"),
    ];
    const shown = together(views);
    expect(shown[0]?.merged).toEqual(["i3", "i4", "i5", "i6", "i7", "i8"].map((id) => `unverified:${id}:app`));
    // One with no item names nothing to count, so two stay two.
    const bare = needView({ kind: "unverified", item: null, repository: "app", reason: "x" }, none);
    expect(together([bare, bare])).toHaveLength(2);
    expect(shown.map((view) => [view.key, view.text, view.items.length, view.more])).toEqual([
      ["unverified:i1:app", "Can't confirm 7 items' merges into app", 5, 2],
      ["unverified:i2:cli", "Can't confirm its merge into cli", 1, 0],
      ["unverified:i9:app", "Can't confirm its merge into app", 1, 0],
    ]);
  });

  it("leaves a thread's number out of the text only beside its badge, naming it as the thread list does", () => {
    const lines = linesOf(
      [
        { id: "thread-3", title: "3. Ship it now" },
        { id: "thread-7", title: "7. Build it" },
      ],
      PALETTES.dark,
    );
    const waiting = (id: string, title: string) =>
      needView({ kind: "contest", thread: { id, title, heads: [] } }, lines);
    // The list's title wins over the one the entry carries, which may be older.
    // The badge-led text picks its thread, so the entry has no row repeating it.
    expect(waiting("thread-3", "3. Ship it")).toMatchObject({
      text: "Two holders claim one thread: Ship it now",
      line: lines.get("thread-3"),
      thread: "thread-3",
      threads: [],
    });
    // A thread the list lacks has no badge, so it keeps its number, and its row the title the entry gives.
    expect(waiting("thread-9", "9. Gone")).toMatchObject({
      text: "Two holders claim one thread: 9. Gone",
      line: undefined,
      thread: null,
      threads: [{ id: "thread-9", title: "9. Gone" }],
    });
    expect(threadLabel({ id: "thread-9", title: "9. Gone" }, lines)).toEqual({ line: undefined, text: "9. Gone" });
    expect(threadLabel({ id: "thread-3", title: "3. Ship it" }, lines)).toEqual({
      line: lines.get("thread-3"),
      text: "Ship it now",
    });
    // Text that doesn't name its thread has no badge: the thread's row below already shows it.
    const torn = needView(
      {
        kind: "conflict",
        entity: "thread",
        id: "thread-7",
        field: "brief",
        values: ["x", "y"],
        thread: { id: "thread-7", title: "7. Build it" },
      },
      lines,
    );
    expect(torn).toMatchObject({ line: undefined, thread: null, threads: [{ id: "thread-7", title: "7. Build it" }] });
    const clash = needView(
      {
        kind: "resource-clash",
        resource: "gpu",
        holders: [
          { thread: { id: "thread-3", title: "3. Ship it now" } },
          { thread: { id: "thread-7", title: "7. Build it" } },
        ],
      },
      lines,
    );
    expect(clash).toMatchObject({
      line: undefined,
      thread: null,
      text: "Threads held by different holders need gpu: 3. Ship it now, 7. Build it",
    });
  });

  it("settles a conflict with each value it can keep, and says why for one it can't", () => {
    const conflict = needView(
      {
        kind: "conflict",
        entity: "item",
        id: "a",
        field: "state",
        values: ["done", "parked"],
        resolve: "acyclic item edit a --state <STATE>",
      },
      none,
    );
    expect(conflict.actions.map((one) => one.label)).toEqual(["Use parked"]);
    expect(conflict.note).toMatch(/done: Keep done in a terminal: run acyclic done/);
    expect(conflict.key).toBe("conflict:item:a:state");
    // Named, and its values too where they are ids, while the command keeps the id.
    const titles = (id: string) => id.toUpperCase();
    const moved = needView(
      { kind: "conflict", entity: "item", id: "a", field: "parent", values: ["p", null] },
      none,
      titles,
    );
    expect(moved.text).toBe("A: its parent was written differently on two machines");
    expect(moved.actions.map((one) => [one.label, one.command])).toEqual([
      ["Use P", { item: { edit: { item: "a", parent: "p" } } }],
      ["Use the top level", { item: { edit: { item: "a", "no-parent": true } } }],
    ]);
    const gone = needView({ kind: "dangling", entity: "item", id: "a", field: "parent", target: "q" }, none, titles);
    expect(gone.text).toBe("A: its parent names q, which doesn't exist");
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
    expect(needView({ kind: "contest", thread: thread("t") }, none).key).toBe("contest:t");
  });

  it("names the threads a cycle or a resource clash holds, and a title written apart as its first value", () => {
    const cycle = needView({ kind: "thread-cycle", threads: [thread("t"), thread("u")], notify: true }, none);
    expect(cycle).toMatchObject({
      key: "thread-cycle:t:u",
      text: "Threads wait on each other in a cycle: T, U",
      threads: [
        { id: "t", title: "T" },
        { id: "u", title: "U" },
      ],
      thread: null,
      actions: [],
    });
    // A resource clash names each holder's thread, to select.
    const clash = needView(
      {
        kind: "resource-clash",
        resource: "gpu",
        holders: [
          { thread: thread("t"), holder: "x" },
          { thread: thread("u"), holder: "y" },
        ],
      },
      none,
    );
    expect(clash).toMatchObject({ threads: cycle.threads, key: "resource-clash:gpu" });
    // A conflicted title reads as its first value, as the item's own panel shows it, not as its id.
    const torn = { id: "b", title: { conflict: ["Bee", "Bea"] } };
    expect(needView({ kind: "link-cycle", items: [item("a"), torn] }, none).text).toBe("Links form a cycle: A, Bee");
  });

  it("names the threads two machines numbered alike, with the command that renumbers one", () => {
    const clash = needView(
      {
        kind: "number-clash",
        number: 32,
        threads: [thread("thread-mac-32"), thread("thread-pi-32")],
        fresh: ["thread-pi-32"],
        fix: "acyclic thread renumber thread-pi-32",
      },
      none,
    );
    expect(clash).toMatchObject({
      key: "number-clash:32",
      threads: [{ id: "thread-mac-32" }, { id: "thread-pi-32" }],
      note: "acyclic thread renumber thread-pi-32",
    });
  });
});

describe("the needs-you list's count", () => {
  it("counts exactly to 99, then says 99+", () => {
    expect([0, 46, 99, 100, 120].map(countText)).toEqual(["0", "46", "99", "99+", "99+"]);
  });
});
