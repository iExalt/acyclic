import { describe, expect, it } from "vitest";
import { settle } from "./api";
import { examples } from "./commands.examples.json";
import { deleteCommand, deletionImpact } from "./deletion";
import { undoOf } from "./undo";

describe("permanent deletion", () => {
  it("sends the existing command for each record type and never offers Undo", () => {
    for (const kind of ["item", "thread", "initiative"] as const) {
      const example = examples.find((one) => one.name === `delete ${kind}`);
      expect(example).toBeDefined();
      const id =
        kind === "item"
          ? "01900000-0000-7000-8000-0000000000c1"
          : kind === "thread"
            ? "01900000-0000-7000-8000-0000000000f1"
            : "01900000-0000-7000-8000-0000000000a2";
      const command = deleteCommand({ kind, id, title: "Record" });
      expect(command).toEqual(example?.command);
      expect(undoOf(command, { items: new Map() })).toBeNull();
    }
  });
  it("counts detached children including conflicting parents, and links in both directions", () => {
    const make = (id: string, parent: string | null) =>
      settle({
        id,
        title: id,
        parent,
        state: "open",
        thread: null,
        notes: null,
        initiative: null,
        links: { waits_for: [], waited_on_by: [] },
      });
    const parent = make("parent", null);
    parent.links = { waits_for: ["before"], waited_on_by: ["after", "other"] };
    const conflicted = make("child", "elsewhere");
    conflicted.conflicts.parent = ["elsewhere", "parent"];
    expect(deletionImpact("parent", [parent, make("child2", "parent"), conflicted, make("other", null)])).toEqual({
      children: 2,
      links: 3,
    });
    expect(deletionImpact("missing", [])).toEqual({ children: 0, links: 0 });
  });
});
