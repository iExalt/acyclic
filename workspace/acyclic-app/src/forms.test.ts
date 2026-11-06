import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import { changes, draftOf, itemEdit, reopenCommand, stateHint } from "./forms";
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

describe("stateHint", () => {
  it("distinguishes backlog from a leaf's status, and says nothing for a parent or other states", () => {
    expect(stateHint({ state: "backlog", thread: "thread-1" }, true)).toBe(
      "Backlog is work still to do. Status also reflects its thread, waits and active claims",
    );
    expect(stateHint({ state: "backlog", thread: "" }, true)).toBe(
      stateHint({ state: "backlog", thread: "thread-1" }, true),
    );
    expect(stateHint({ state: "backlog", thread: "thread-1" }, false)).toBeNull();
    expect(stateHint({ state: "parked", thread: "thread-1" }, true)).toBeNull();
  });
});
