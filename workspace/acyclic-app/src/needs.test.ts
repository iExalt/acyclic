import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import { approveCommand, dropMemberCommand, keepCommand, keepMemberCommand } from "./forms";
import { type Line, linesOf } from "./lines";
import { countText, groupOf, needView, threadLabel } from "./needs";

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
    expect(proposal.text).toBe("session/x proposes thread none → t for B");
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
    expect(waiting.actions).toEqual([{ label: "Approve 12 items", command: approveCommand("u"), action: "approve" }]);
    expect(waiting.items).toEqual(["z"]);
  });

  it("leaves a thread's number out of the text only beside its badge, naming it as the thread list does", () => {
    const lines = linesOf([
      { id: "thread-3", title: "3. Ship it now" },
      { id: "thread-7", title: "7. Build it" },
    ]);
    const waiting = (id: string, title: string) =>
      needView({ kind: "awaiting-approval", thread: { id, title, heads: [] } }, lines);
    // The list's title wins over the one the entry carries, which may be older.
    // The badge-led text picks its thread, so the entry has no row repeating it.
    expect(waiting("thread-3", "3. Ship it")).toMatchObject({
      text: "Ship it now waits for your approval",
      line: lines.get("thread-3"),
      thread: "thread-3",
      threads: [],
    });
    // A thread the list lacks has no badge, so it keeps its number, and its row the title the entry gives.
    expect(waiting("thread-9", "9. Gone")).toMatchObject({
      text: "9. Gone waits for your approval",
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
  });

  it("shows a kind it has no command for with what it names and the CLI's hint", () => {
    const cycle = needView({ kind: "parent-cycle", items: [item("a"), item("b")] }, none);
    expect(cycle).toMatchObject({ text: "Parents form a cycle: A, B", items: ["a", "b"], actions: [] });
    const loose = needView({ kind: "unthreaded", item: item("o"), fix: "acyclic item edit o --thread <THREAD>" }, none);
    expect(loose.note).toBe("acyclic item edit o --thread <THREAD>");
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
  it("puts every kind in one group, a kind the app doesn't know among what to unblock", () => {
    expect(["proposal", "unapproved", "awaiting-approval", "new-blocker"].map(groupOf)).toEqual(
      Array(4).fill("approve"),
    );
    expect(groupOf("question")).toBe("answer");
    expect(
      ["conflict", "resource-clash", "number-clash", "dangling", "unverified", "link-cycle", "never-seen"].map(groupOf),
    ).toEqual(Array(7).fill("unblock"));
  });

  it("counts exactly to 99, then says 99+", () => {
    expect([0, 46, 99, 100, 120].map(countText)).toEqual(["0", "46", "99", "99+", "99+"]);
  });
});
