import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import { approveCommand, dropMemberCommand, keepCommand, keepMemberCommand } from "./forms";
import { needView } from "./needs";

const example = (name: string) => examples.find((one) => one.name === name)?.command;
const item = (id: string) => ({ id, title: id.toUpperCase() });
const thread = (id: string, heads: string[] = []) => ({ id, title: id.toUpperCase(), heads });

describe("needView", () => {
  it("keeps or drops a proposal, and a member the approval didn't see, with the server's own commands", () => {
    const proposal = needView({
      kind: "proposal",
      item: item("b"),
      proposal: { id: "mac-9", actor: "session/x", edit: { field: "thread", before: null, after: "t" } },
    });
    expect(proposal.text).toBe("session/x proposes thread none → t for B");
    expect(proposal.actions.map((one) => one.command)).toEqual([
      keepCommand("mac-9"),
      { proposal: { drop: { proposal: "mac-9" } } },
    ]);
    const late = needView({ kind: "unapproved", item: item("a"), thread: thread("t") });
    expect(late.actions.map((one) => one.command)).toEqual([
      example("keep a member the approval didn't see"),
      dropMemberCommand("a"),
    ]);
    expect(keepMemberCommand("a", "t")).toEqual(example("keep a member the approval didn't see"));
    expect(dropMemberCommand("b")).toEqual(example("drop a member the approval didn't see"));
  });

  it("approves a waiting thread naming how many items, with its heads to select", () => {
    // Its members all told, as the thread's panel counts them, not only the open ones.
    const waiting = needView({
      kind: "awaiting-approval",
      thread: { ...thread("u", ["z"]), counts: { members: 12 } },
      counts: { open_members: 3 },
    });
    expect(waiting.actions).toEqual([{ label: "Approve 12 items", command: approveCommand("u"), action: "approve" }]);
    expect(waiting.items).toEqual(["z"]);
    // Named as its line, since its badge shows the number beside the text.
    const numbered = needView({ kind: "awaiting-approval", thread: { id: "t7", title: "7. Ship it", heads: [] } });
    expect(numbered.text).toBe("Ship it waits for your approval");
  });

  it("settles a conflict with each value it can keep, and says why for one it can't", () => {
    const conflict = needView({
      kind: "conflict",
      entity: "item",
      id: "a",
      field: "state",
      values: ["done", "parked"],
      resolve: "acyclic item edit a --state <STATE>",
    });
    expect(conflict.actions.map((one) => one.label)).toEqual(["Use parked"]);
    expect(conflict.note).toMatch(/done: Keep done with `acyclic done`/);
    expect(conflict.key).toBe("conflict:item:a:state");
  });

  it("shows a kind it has no command for with what it names and the CLI's hint", () => {
    const cycle = needView({ kind: "parent-cycle", items: [item("a"), item("b")] });
    expect(cycle).toMatchObject({ text: "Parents form a cycle: A, B", items: ["a", "b"], actions: [] });
    const loose = needView({ kind: "unthreaded", item: item("o"), fix: "acyclic item edit o --thread <THREAD>" });
    expect(loose.note).toBe("acyclic item edit o --thread <THREAD>");
    expect(needView({ kind: "something-new" }).text).toBe("something-new");
  });

  it("shows each question's own text, keyed by its id, so two in one thread stay apart", () => {
    const asks = ["Which flag?", "Ship today?"].map((text, n) =>
      needView({ kind: "question", id: `q${n}`, thread: thread("t"), class: "scope", text }),
    );
    expect(asks.map((one) => [one.key, one.text, one.threads])).toEqual([
      ["question:q0", "Which flag?", ["t"]],
      ["question:q1", "Ship today?", ["t"]],
    ]);
  });

  it("names a new blocker and the threads a resource clash holds, to select", () => {
    const blocked = needView({ kind: "new-blocker", item: item("a"), thread: thread("t"), blocker: item("b") });
    expect(blocked).toMatchObject({ items: ["a", "b"], text: "A gained a new blocker since its approval: B" });
    const clash = needView({
      kind: "resource-clash",
      resource: "gpu",
      holders: [
        { thread: thread("t"), holder: "x" },
        { thread: thread("u"), holder: "y" },
      ],
    });
    expect(clash).toMatchObject({ threads: ["t", "u"], key: "resource-clash:gpu" });
    expect(clash.text).toMatch(/T, U$/);
  });
});
