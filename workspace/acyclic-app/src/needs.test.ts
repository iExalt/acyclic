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
    const waiting = needView({ kind: "awaiting-approval", thread: thread("u", ["z"]), counts: { open_members: 3 } });
    expect(waiting.actions).toEqual([{ label: "Approve 3 items", command: approveCommand("u"), action: "approve" }]);
    expect(waiting.items).toEqual(["z"]);
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
});
