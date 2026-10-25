import { describe, expect, it } from "vitest";
import { examples } from "./commands.examples.json";
import {
  addCommand,
  expandTo,
  landed,
  linkCommand,
  linkRefusal,
  type Mode,
  moveCommand,
  newId,
  pick,
  recovery,
  told,
  unanswered,
  unlinkCommand,
  withPending,
} from "./edits";
import { draftOf } from "./forms";
import { item } from "./map/fixture";
import { buildModel } from "./map/model";

// garden: beds (dig), plant (sow); tools at the top level.
const model = buildModel([
  item("garden"),
  item("beds", "garden"),
  item("dig", "beds"),
  item("plant", "garden"),
  item("sow", "plant"),
  item("tools"),
]);

describe("commands", () => {
  it("builds exactly the commands the server's tests run", () => {
    const built = [
      ["list", "list"],
      ["show", { show: { items: ["a", "b"] } }],
      ["threads", { thread: "list" }],
      ["add under a parent, in a thread", addCommand({ item: "c", title: "C", parent: "a", thread: "t" })],
      ["add at the top level", addCommand({ item: "d", title: "D", parent: null, thread: null })],
      ["move to the top level", moveCommand("c", null)],
      ["move under a parent", moveCommand("d", "a")],
      ["c waits for d", linkCommand("c", "waits-for", "d")],
      ["c relates to b", linkCommand("c", "relates-to", "b")],
      ["c no longer waits for d", unlinkCommand("c", "waits-for", "d")],
    ];
    const named = new Map(examples.map((one) => [one.name, one.command]));
    expect(built).toEqual(built.map(([name]) => [name, named.get(name as string)]));
  });
});

describe("newId", () => {
  it("slugs the title, numbers it past taken ids, and keeps the number within the tracker's 64 characters", () => {
    expect(newId("Sow the Beans!", new Set())).toBe("sow-the-beans");
    expect(newId("Crème brûlée", new Set())).toBe("creme-brulee");
    expect(newId("Sow", new Set(["sow", "sow-2"]))).toBe("sow-3");
    expect(newId("日本", new Set())).toBe("item");
    const long = "a".repeat(70);
    expect(newId(long, new Set())).toBe("a".repeat(64));
    expect(newId(long, new Set(["a".repeat(64)]))).toBe(`${"a".repeat(62)}-2`);
    // A cut that ends between words leaves no trailing dash before the number.
    expect(newId(`${"a".repeat(61)} b`, new Set([`${"a".repeat(61)}-b`]))).toBe(`${"a".repeat(61)}-2`);
  });
});

describe("pick", () => {
  const move: Mode = { kind: "move", item: "plant" };

  it("moves under a box or to the top level, and refuses the item, its descendants, its own parent and the background", () => {
    expect(pick(model, move, { kind: "item", id: "beds" })).toEqual({
      mode: { kind: "idle" },
      command: moveCommand("plant", "beds"),
      refusal: null,
    });
    expect(pick(model, move, { kind: "top" }).command).toEqual(moveCommand("plant", null));
    for (const target of [
      { kind: "item", id: "plant" },
      { kind: "item", id: "sow" },
      { kind: "item", id: "garden" },
      { kind: "background" },
    ] as const) {
      const step = pick(model, move, target);
      expect(step.command).toBeNull();
      expect(step.mode).toBe(move);
      expect(step.refusal).not.toBeNull();
    }
    expect(pick(model, { kind: "move", item: "tools" }, { kind: "top" }).refusal).toMatch(/already at the top/);
  });

  it("links the selected item to the one tapped, which comes first, and refuses links the Map couldn't draw", () => {
    const link: Mode = { kind: "link", item: "sow", link: "waits-for" };
    expect(pick(model, link, { kind: "item", id: "dig" }).command).toEqual({
      link: { add: { from: "sow", kind: "waits-for", to: "dig" } },
    });
    for (const id of ["sow", "plant", "garden"]) expect(pick(model, link, { kind: "item", id }).command).toBeNull();
    expect(linkRefusal(model, "garden", "dig")).toMatch(/holds/);
    expect(pick(model, link, { kind: "background" }).mode).toBe(link);
  });

  it("returns to idle on cancel from every mode", () => {
    const modes: Mode[] = [{ kind: "idle" }, move, { kind: "link", item: "sow", link: "relates-to" }];
    for (const mode of modes)
      expect(pick(model, mode, { kind: "cancel" })).toEqual({
        mode: { kind: "idle" },
        command: null,
        refusal: null,
      });
  });
});

describe("expandTo", () => {
  it("expands every box above an item and leaves the rest as the user left it", () => {
    const toggles = new Map([
      ["tools", true],
      ["beds", true],
    ]);
    expect([...expandTo(model, "sow", toggles)].sort()).toEqual([
      ["beds", true],
      ["garden", false],
      ["plant", false],
      ["tools", true],
    ]);
    const open = new Map([["garden", false]]);
    expect(expandTo(model, "beds", open)).toBe(open);
  });
});

describe("recovery", () => {
  it("offers a retry when the server is busy, and a fix to the id when the id is refused", () => {
    expect(recovery("busy")).toEqual({ retry: true, fixId: false });
    for (const code of ["already-exists", "deleted", "invalid-name"]) expect(recovery(code).fixId).toBe(true);
    expect(recovery("parent-cycle")).toEqual({ retry: false, fixId: false });
  });
});

describe("told", () => {
  it("offers the write again only when it failed busy, and a refresh when it saved but the reload failed", () => {
    expect(told({ write: "failed", code: "busy", message: "busy" })).toEqual({
      text: "busy",
      offer: "retry",
      fixId: false,
    });
    expect(told({ write: "failed", code: "already-exists", message: "taken" })).toEqual({
      text: "taken",
      offer: null,
      fixId: true,
    });
    // A proposal decided elsewhere: the app's view is stale, so it offers a refresh.
    expect(told({ write: "failed", code: "decided", message: "decided" }).offer).toBe("refresh");
    expect(told({ write: "saved", proposals: 0, reload: { failed: "offline" } })).toEqual({
      text: "Saved, but the refresh failed: offline",
      offer: "refresh",
      fixId: false,
    });
    expect(told({ write: "saved", proposals: 1, reload: "done" }).text).toMatch(/proposal/);
    expect(told({ write: "saved", proposals: 0, reload: "done" })).toEqual({ text: null, offer: null, fixId: false });
  });
});

describe("a write drawn before the server answers", () => {
  const items = [item("garden"), item("beds", "garden", [], { notes: "old", size: "s" })];

  it("draws an added item where it goes, once, and says when the plan holds it", () => {
    const add = { kind: "add" as const, item: { item: "pond", title: "Pond", parent: "garden", thread: "t" } };
    const drawn = withPending(items, add);
    expect(drawn.at(-1)).toMatchObject({ id: "pond", title: "Pond", parent: "garden", thread: "t", state: "backlog" });
    expect(landed(add, items)).toBe(false);
    // Read back from the server, it is drawn once, as the server has it.
    const read = [...items, item("pond", "garden")];
    expect(withPending(read, add)).toBe(read);
    expect(landed(add, read)).toBe(true);
    expect(withPending(items, null)).toBe(items);
  });

  it("draws an edit's fields, clearing what the draft empties, and says when the plan holds them", () => {
    const beds = items[1] ?? item("beds");
    const base = draftOf(beds);
    const edit = {
      kind: "edit" as const,
      id: "beds",
      edited: { base, draft: { ...base, title: " Raised beds ", notes: "", state: "parked" } },
    };
    const drawn = withPending(items, edit);
    expect(drawn[1]).toMatchObject({ title: "Raised beds", notes: null, size: "s", state: "parked" });
    expect(drawn[0]).toBe(items[0]);
    expect(landed(edit, items)).toBe(false);
    expect(landed(edit, drawn)).toBe(true);
    // A done item's state isn't the editor's to change.
    const done = item("x", null, [], { state: "done" });
    const finished = { base: draftOf(done), draft: { ...draftOf(done), title: "Y", state: "parked" } };
    expect(withPending([done], { kind: "edit", id: "x", edited: finished })[0]?.state).toBe("done");
  });

  it("draws and checks only the fields the user changed, not ones changed elsewhere while the editor was open", () => {
    const beds = items[1] ?? item("beds");
    const base = draftOf(beds);
    const edit = { kind: "edit" as const, id: "beds", edited: { base, draft: { ...base, title: "Beds 2" } } };
    // Elsewhere, the notes changed meanwhile; the server holds the title too.
    const read = [{ ...beds, title: "Beds 2", notes: "from an agent" }];
    expect(withPending(read, edit)[0]?.notes).toBe("from an agent");
    expect(landed(edit, read)).toBe(true);
  });

  it("knows only a missing or foreign answer leaves the write's outcome unknown", () => {
    expect(["network", "http"].map(unanswered)).toEqual([true, true]);
    expect(["busy", "already-exists", "invalid"].map(unanswered)).toEqual([false, false, false]);
  });
});
