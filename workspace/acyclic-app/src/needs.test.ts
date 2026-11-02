import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import { approveCommand, dropMemberCommand, keepCommand, keepMemberCommand } from "./forms";
import { type Line, linesOf } from "./lines";
import { awaitingApproval, countText, groupOf, needView, threadLabel, together } from "./needs";
import { PALETTES } from "./palette";

const example = (name: string) => examples.find((one) => one.name === name)?.command;
const item = (id: string) => ({ id, title: id.toUpperCase() });
const none = new Map<string, Line>();
const thread = (id: string, heads: string[] = []) => ({ id, title: id.toUpperCase(), heads });

describe("needView", () => {
  it("keeps or drops a proposal, and a member the approval didn't see, with the server's own commands", () => {
    const proposal = needView(
      {
        kind: "proposal",
        item: item("b"),
        proposal: { id: "mac-9", actor: "session/x", edit: { field: "thread", before: null, after: "t" } },
      },
      none,
    );
    expect(proposal.text).toBe("session/x proposes thread: no thread → t, for B");
    // Threads by their names, and repositories as a list, for the item or the thread it changes.
    const lines = linesOf([{ id: "t", title: "Tend" }], PALETTES.light);
    const moved = needView(
      {
        kind: "proposal",
        item: item("b"),
        proposal: { id: "mac-8", actor: "session/x", edit: { field: "thread", before: "t", after: null } },
      },
      lines,
    );
    expect(moved.text).toBe(`session/x proposes thread: ${lines.get("t")?.name} → no thread, for B`);
    const repositories = needView(
      {
        kind: "proposal",
        proposal: {
          id: "mac-7",
          actor: "session/x",
          edit: { kind: "thread-repository", thread: "t", field: "repositories", before: [], after: ["app", "cli"] },
        },
      },
      lines,
    );
    expect(repositories.text).toBe(`session/x proposes repositories: none → app, cli, for ${lines.get("t")?.name}`);
    expect(proposal.actions.map((one) => one.command)).toEqual([
      keepCommand("mac-9"),
      { proposal: { drop: { proposal: "mac-9" } } },
    ]);
    const late = needView({ kind: "unapproved", item: item("a"), thread: thread("t") }, none);
    expect(late.actions.map((one) => one.command)).toEqual([
      example("keep a member the approval didn't see"),
      dropMemberCommand("a"),
    ]);
    expect(keepMemberCommand("a", "t")).toEqual(example("keep a member the approval didn't see"));
    expect(dropMemberCommand("b")).toEqual(example("drop a member the approval didn't see"));
  });

  it("approves a waiting thread naming how many items, with its heads to select", () => {
    // Its members all told, as the thread's panel counts them, not only the open ones.
    const waiting = needView(
      {
        kind: "awaiting-approval",
        thread: { ...thread("u", ["z"]), counts: { members: 12 } },
        counts: { open_members: 3 },
      },
      none,
    );
    expect(waiting.actions).toEqual([
      {
        label: "Approve 12 items",
        command: approveCommand("u"),
        action: "approve",
        done: "Approved U: 12 items",
        undo: true,
      },
    ]);
    expect(waiting.items).toEqual(["z"]);
    expect(waiting.more).toBe(11);
    // Where the plan holds its items, it lists them, the first few, and says how many more the count covers.
    const ten = Array.from({ length: 10 }, (_, n) => `i${n}`);
    const all = needView(
      { kind: "awaiting-approval", thread: { ...thread("u", ["z"]), counts: { members: 12 } } },
      none,
      (id) => (id === "u" ? ten : []),
    );
    expect(all.items).toEqual(ten.slice(0, 5));
    expect(all.more).toBe(7);
  });

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
      needView({ kind: "awaiting-approval", thread: { id, title, heads: [] } }, lines);
    // The list's title wins over the one the entry carries, which may be older.
    // The badge-led text picks its thread, so the entry has no row repeating it.
    expect(waiting("thread-3", "3. Ship it")).toMatchObject({
      text: "Ship it now",
      line: lines.get("thread-3"),
      thread: "thread-3",
      threads: [],
    });
    // A thread the list lacks has no badge, so it keeps its number, and its row the title the entry gives.
    expect(waiting("thread-9", "9. Gone")).toMatchObject({
      text: "9. Gone",
      line: undefined,
      thread: null,
      threads: [{ id: "thread-9", title: "9. Gone" }],
    });
    // What it asks, for a list without its button, gives the button's own count.
    const covering = needView(
      { kind: "awaiting-approval", thread: { id: "thread-3", title: "Ship", counts: { members: 4 } } },
      lines,
    );
    expect(covering.detail).toBe("4 items to approve");
    expect(covering.actions[0]?.label).toBe("Approve 4 items");
    expect(threadLabel({ id: "thread-9", title: "9. Gone" }, lines)).toEqual({ line: undefined, text: "9. Gone" });
    expect(threadLabel({ id: "thread-3", title: "3. Ship it" }, lines)).toEqual({
      line: lines.get("thread-3"),
      text: "Ship it now",
    });
    // Text that doesn't name its thread has no badge: the thread's row below already shows it.
    const ask = needView(
      { kind: "question", id: "q", thread: { id: "thread-7", title: "7. Build it" }, text: "Which?" },
      lines,
    );
    expect(ask).toMatchObject({ line: undefined, thread: null, threads: [{ id: "thread-7", title: "7. Build it" }] });
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
      undefined,
      titles,
    );
    expect(moved.text).toBe("A: its parent was written differently on two machines");
    expect(moved.actions.map((one) => [one.label, one.command])).toEqual([
      ["Use P", { item: { edit: { item: "a", parent: "p" } } }],
      ["Use the top level", { item: { edit: { item: "a", "no-parent": true } } }],
    ]);
    const gone = needView(
      { kind: "dangling", entity: "item", id: "a", field: "parent", target: "q" },
      none,
      undefined,
      titles,
    );
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

  it("shows each question's own text, keyed by its id, so two in one thread stay apart", () => {
    const asks = ["Which flag?", "Ship today?"].map((text, n) =>
      needView({ kind: "question", id: `q${n}`, thread: thread("t"), class: "scope", text }, none),
    );
    expect(asks.map((one) => [one.key, one.text, one.threads.map((t) => t.id)])).toEqual([
      ["question:q0", "Which flag?", ["t"]],
      ["question:q1", "Ship today?", ["t"]],
    ]);
  });

  it("names a new blocker and the threads a resource clash holds, to select", () => {
    const blocked = needView({ kind: "new-blocker", item: item("a"), thread: thread("t"), blocker: item("b") }, none);
    expect(blocked).toMatchObject({ items: ["a", "b"], text: "A gained a new blocker since its approval: B" });
    // A conflicted title reads as its first value, as the item's own panel shows it, not as its id.
    const torn = { id: "b", title: { conflict: ["Bee", "Bea"] } };
    expect(needView({ kind: "new-blocker", item: item("a"), thread: thread("t"), blocker: torn }, none).text).toBe(
      "A gained a new blocker since its approval: Bee",
    );
    // Approving the thread again acknowledges it.
    expect(blocked.actions).toEqual([
      { label: "Approve again", command: approveCommand("t"), action: "approve", done: "Approved t again" },
    ]);
    // The thread it approves keeps its row, named, since the text doesn't name it.
    const lines = linesOf([{ id: "t", title: "Tend" }], PALETTES.light);
    const lined = needView({ kind: "new-blocker", item: item("a"), thread: thread("t"), blocker: item("b") }, lines);
    expect(lined).toMatchObject({ thread: null, threads: [{ id: "t" }], line: undefined });
    expect(lined.actions[0]?.done).toBe(`Approved ${lines.get("t")?.name} again`);
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
    expect(clash).toMatchObject({
      threads: [
        { id: "t", title: "T" },
        { id: "u", title: "U" },
      ],
      key: "resource-clash:gpu",
    });
    expect(clash.text).toMatch(/T, U$/);
  });

  it("names the threads two machines numbered alike, with the command that renumbers one", () => {
    const clash = needView(
      {
        kind: "number-clash",
        number: 32,
        threads: [thread("thread-mac-32"), thread("thread-pi-32")],
        fresh: ["thread-pi-32"],
        fix: "acyclic thread renumber thread-pi-32 --as-user",
      },
      none,
    );
    expect(clash).toMatchObject({
      key: "number-clash:32",
      threads: [{ id: "thread-mac-32" }, { id: "thread-pi-32" }],
      note: "acyclic thread renumber thread-pi-32 --as-user",
    });
  });
});

describe("the needs-you list's groups and count", () => {
  it("puts every kind in one group, a kind the app doesn't know among what to resolve", () => {
    expect(["proposal", "unapproved", "awaiting-approval", "new-blocker"].map(groupOf)).toEqual(
      Array(4).fill("approve"),
    );
    expect(groupOf("question")).toBe("answer");
    expect(
      ["conflict", "resource-clash", "number-clash", "dangling", "unverified", "link-cycle", "never-seen"].map(groupOf),
    ).toEqual(Array(7).fill("resolve"));
  });

  it("names the threads waiting for approval, and no thread another kind names", () => {
    const needs = [
      { kind: "awaiting-approval", thread: { id: "thread-1", title: "One" } },
      { kind: "unapproved", thread: { id: "thread-2" }, item: { id: "a" } },
      { kind: "awaiting-approval", thread: null },
    ];
    expect(awaitingApproval(needs)).toEqual(new Set(["thread-1"]));
  });

  it("counts exactly to 99, then says 99+", () => {
    expect([0, 46, 99, 100, 120].map(countText)).toEqual(["0", "46", "99", "99+", "99+"]);
  });
});
