import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import {
  approveCommand,
  changes,
  draftOf,
  dropCommand,
  globalLevelCommand,
  itemEdit,
  keepCommand,
  pmCommand,
  reopenCommand,
  resolveCommand,
  revokeCommand,
  stateHint,
  threadLevelCommand,
  unsettledWhy,
} from "./forms";
import { item } from "./map/fixture";

const example = (name: string) => {
  const found = examples.find((one) => one.name === name);
  if (found === undefined) throw new Error(`no example ${name}`);
  return found.command;
};

describe("form commands", () => {
  it("build exactly the commands the server's tests run", () => {
    const c = { ...item("c"), title: "C" };
    expect(
      itemEdit("c", draftOf(c), { title: "C2", state: "in-progress", thread: "t", notes: "n", gate: "g", size: "m" }),
    ).toEqual(example("edit an item's fields"));
    const filled = { ...c, thread: "t", notes: "n", gate: "g", size: "m" };
    expect(itemEdit("c", draftOf(filled), { ...draftOf(filled), thread: "", notes: "", gate: "", size: "" })).toEqual(
      example("clear an item's fields"),
    );
    expect(resolveCommand("item", "d", "parent", null)).toEqual(example("settle an item's parent as a clear"));
    expect(resolveCommand("thread", "t", "title", "T")).toEqual(example("settle a thread's title"));
    expect(resolveCommand("thread", "t", "brief", null)).toEqual(example("settle a thread's brief as a clear"));
    expect(reopenCommand("e")).toEqual(example("reopen"));
    expect(revokeCommand("t")).toEqual(example("revoke"));
    expect(approveCommand("t")).toEqual(example("approve"));
    expect(keepCommand("$proposal-1")).toEqual(example("keep"));
    expect(dropCommand("$proposal-2")).toEqual(example("drop"));
    expect(threadLevelCommand("t", "everything")).toEqual(example("a thread's own level"));
    expect(threadLevelCommand("t", null)).toEqual(example("the global level for a thread"));
    expect(pmCommand("t", true)).toEqual(example("flag project management"));
    expect(pmCommand("t", false)).toEqual(example("unflag project management"));
    expect(globalLevelCommand("blockers")).toEqual(example("the global level"));
  });
});

describe("itemEdit", () => {
  it("sends nothing when nothing changed, and leaves a conflicted field the user didn't touch", () => {
    const a = { ...item("a"), conflicts: { title: ["A", "B"] } };
    expect(itemEdit("a", draftOf(a), draftOf(a))).toBeNull();
    expect(itemEdit("a", draftOf(a), { ...draftOf(a), gate: "g" })).toEqual({
      item: { edit: { item: "a", gate: "g" } },
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

describe("resolveCommand", () => {
  it("keeps a value the way the CLI's hint says, a clear as the no- form, and refuses what the app can't write", () => {
    expect(resolveCommand("item", "a", "title", "B")).toEqual({ item: { edit: { item: "a", title: "B" } } });
    expect(resolveCommand("item", "a", "thread", null)).toEqual({ item: { edit: { item: "a", "no-thread": true } } });
    expect(resolveCommand("item", "a", "state", "done")).toBeNull();
    expect(resolveCommand("item", "a", "title", null)).toBeNull();
    // One replica flagged the thread and another cleared the flag: either value settles it.
    expect(resolveCommand("thread", "t", "pm-flag", true)).toEqual({ thread: { edit: { thread: "t", pm: true } } });
    expect(resolveCommand("thread", "t", "pm-flag", null)).toEqual({
      thread: { edit: { thread: "t", "no-pm": true } },
    });
    expect(resolveCommand("thread", "t", "autonomy", "blockers")).toEqual({
      thread: { edit: { thread: "t", level: "blockers" } },
    });
    expect(resolveCommand("thread", "t", "autonomy", "sometimes")).toBeNull();
    expect(resolveCommand("thread", "t", "approved", false)).toEqual(revokeCommand("t"));
    expect(resolveCommand("thread", "t", "approved-members", ["a"])).toEqual(approveCommand("t"));
    expect(resolveCommand("thread", "t", "evidence", "x")).toBeNull();
    // The global level itself can be written apart.
    expect(resolveCommand("settings", "global", "autonomy", "blockers")).toEqual(example("the global level"));
    expect(resolveCommand("settings", "global", "autonomy", null)).toBeNull();
    expect(unsettledWhy("item", "state", "done")).toMatch(/acyclic done/);
  });
});

describe("stateHint", () => {
  it("says how a leaf in the backlog shows, by whether it is in a thread, and nothing for a parent or other states", () => {
    expect(stateHint({ state: "backlog", thread: "thread-1" }, true)).toBe(
      "Shows as Ready or Blocked, by what it waits on",
    );
    expect(stateHint({ state: "backlog", thread: "" }, true)).toBe("Shows as Needs a thread until it's in one");
    expect(stateHint({ state: "backlog", thread: "thread-1" }, false)).toBeNull();
    expect(stateHint({ state: "parked", thread: "thread-1" }, true)).toBeNull();
  });
});
