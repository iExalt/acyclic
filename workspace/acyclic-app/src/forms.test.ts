import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import { changes, draftOf, itemEdit, reopenCommand, STATES, stateHint } from "./forms";
import { ID, item } from "./map/fixture";

const example = (name: string) => {
  const found = examples.find((one) => one.name === name);
  if (found === undefined) throw new Error(`no example ${name}`);
  return found.command;
};

describe("form commands", () => {
  it("build exactly the commands the server's tests run", () => {
    const c = { ...item(ID.c), title: "C" };
    expect(itemEdit(ID.c, draftOf(c), { title: "C2", state: "parked", thread: ID.t, notes: "n" })).toEqual(
      example("edit an item's fields"),
    );
    const filled = { ...c, thread: ID.t, notes: "n" };
    expect(itemEdit(ID.c, draftOf(filled), { ...draftOf(filled), thread: "", notes: "" })).toEqual(
      example("clear an item's fields"),
    );
    expect(reopenCommand(ID.e)).toEqual(example("reopen"));
  });
});

describe("itemEdit", () => {
  it("sends nothing when nothing changed, and leaves a conflicted field the user didn't touch", () => {
    const a = { ...item("a"), conflicts: { title: ["A", "B"] } };
    expect(itemEdit("a", draftOf(a), draftOf(a))).toBeNull();
    expect(itemEdit("a", draftOf(a), { ...draftOf(a), notes: "n" })).toEqual({
      item: { edit: { item: "a", notes: "n" } },
    });
    // A blank title or a state only `done` sets is never sent.
    expect(itemEdit("a", draftOf(a), { ...draftOf(a), title: " ", state: "done" })).toBeNull();
  });

  it("sends only what the user changed since the editor opened, not what changed elsewhere meanwhile", () => {
    const opened = draftOf({ ...item("a"), notes: "old", thread: "t" });
    // Notes and thread change elsewhere; the user changes only the title, and the editor still holds the old notes.
    expect(itemEdit("a", opened, { ...opened, title: "New" })).toEqual({ item: { edit: { item: "a", title: "New" } } });
    expect(changes(opened, { ...opened, title: " New ", notes: "" })).toEqual({ title: "New", notes: "" });
  });
});

describe("canceling", () => {
  it("is a state a command may set, saved as one edit and said plainly", () => {
    expect(STATES).toContain("canceled");
    const base = { title: "T", state: "open", thread: "", notes: "" };
    expect(itemEdit("a", base, { ...base, state: "canceled" })).toEqual({
      item: { edit: { item: "a", state: "canceled" } },
    });
    expect(stateHint({ state: "canceled", thread: "" }, true)).toMatch(/^Canceled work is dropped/);
  });
});

describe("stateHint", () => {
  it("distinguishes open from a leaf's status, and says nothing for a parent or other states", () => {
    expect(stateHint({ state: "open", thread: "thread-1" }, true)).toBe(
      "Open is work still to do. Status also reflects its thread, waits and active claims",
    );
    expect(stateHint({ state: "open", thread: "" }, true)).toBe(stateHint({ state: "open", thread: "thread-1" }, true));
    expect(stateHint({ state: "open", thread: "thread-1" }, false)).toBeNull();
    expect(stateHint({ state: "parked", thread: "thread-1" }, true)).toBeNull();
  });
});

describe("completion", () => {
  const base = draftOf(item("a"));
  it("uses the completion command with trimmed evidence and no force", () => {
    expect(itemEdit("a", base, { ...base, state: "done", evidence: "  Finished the work  " })).toEqual({
      item: { done: { item: "a", evidence: "Finished the work" } },
    });
  });
  it("refuses blank evidence and combined field changes instead of partially saving", () => {
    expect(itemEdit("a", base, { ...base, state: "done", evidence: "  " })).toBeNull();
    for (const field of ["title", "thread", "notes"] as const)
      expect(itemEdit("a", base, { ...base, [field]: "Changed", state: "done", evidence: "Finished" })).toBeNull();
    expect(
      itemEdit("a", base, { ...base, title: ` ${base.title} `, state: "done", evidence: "Finished" }),
    ).not.toBeNull();
  });
});
