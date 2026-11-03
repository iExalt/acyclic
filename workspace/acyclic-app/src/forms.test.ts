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
    expect(itemEdit(ID.c, draftOf(c), { title: "C2", state: "parked", thread: ID.t, notes: "n", size: "m" })).toEqual(
      example("edit an item's fields"),
    );
    const filled = { ...c, thread: ID.t, notes: "n", size: "m" };
    expect(itemEdit(ID.c, draftOf(filled), { ...draftOf(filled), thread: "", notes: "", size: "" })).toEqual(
      example("clear an item's fields"),
    );
    expect(reopenCommand(ID.e)).toEqual(example("reopen"));
  });
});

describe("itemEdit", () => {
  it("sends nothing when nothing changed, and leaves a conflicted field the user didn't touch", () => {
    const a = { ...item("a"), conflicts: { title: ["A", "B"] } };
    expect(itemEdit("a", draftOf(a), draftOf(a))).toBeNull();
    expect(itemEdit("a", draftOf(a), { ...draftOf(a), size: "m" })).toEqual({
      item: { edit: { item: "a", size: "m" } },
    });
    // A blank title or a state only `done` sets is never sent.
    expect(itemEdit("a", draftOf(a), { ...draftOf(a), title: " ", state: "done" })).toBeNull();
  });

  it("sends only what the user changed since the editor opened, not what changed elsewhere meanwhile", () => {
    const opened = draftOf({ ...item("a"), notes: "old", size: "s" });
    // Notes and size change elsewhere; the user changes only the title, and the editor still holds the old notes.
    expect(itemEdit("a", opened, { ...opened, title: "New" })).toEqual({ item: { edit: { item: "a", title: "New" } } });
    expect(changes(opened, { ...opened, title: " New ", notes: "" })).toEqual({ title: "New", notes: "" });
  });
});

describe("stateHint", () => {
  it("says how a leaf in the backlog shows, by whether it is in a thread, and nothing for a parent or other states", () => {
    expect(stateHint({ state: "backlog", thread: "thread-1" }, true)).toBe(
      "Backlog shows as Ready or Blocked in Status, by what it waits on",
    );
    expect(stateHint({ state: "backlog", thread: "" }, true)).toBe(
      "Backlog shows as Needs a thread in Status until it's in one",
    );
    expect(stateHint({ state: "backlog", thread: "thread-1" }, false)).toBeNull();
    expect(stateHint({ state: "parked", thread: "thread-1" }, true)).toBeNull();
  });
});
