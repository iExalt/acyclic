import { describe, expect, it } from "vitest";
import { type KeyContext, type KeyPress, nextRow, SEQUENCE_MS, step } from "./keys";

const press = (key: string, fields: Partial<KeyPress> = {}): KeyPress => ({
  key,
  code: /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : "",
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  isComposing: false,
  keyCode: 0,
  repeat: false,
  timeStamp: 0,
  ...fields,
});
const free: KeyContext = { enabled: true, panel: null, menu: false, dirty: false, typing: false };
const act = (key: string, fields: Partial<KeyPress> = {}, context = free) =>
  step(null, press(key, fields), context).action;

describe("single-key shortcuts", () => {
  it("go to a view with G then its letter, within a second, and nowhere with any other letter or too late", () => {
    const g = step(null, press("g", { timeStamp: 100 }), free);
    expect(g).toEqual({ pending: 100, action: null });
    expect(step(g.pending, press("t", { timeStamp: 600 }), free).action).toEqual({ kind: "go", view: "threads" });
    expect(step(g.pending, press("n", { timeStamp: 600 }), free).action).toEqual({ kind: "needs" });
    expect(step(g.pending, press("x", { timeStamp: 600 }), free)).toEqual({ pending: null, action: null });
    // Too late, the letter is itself again: C adds.
    expect(step(g.pending, press("c", { timeStamp: 100 + SEQUENCE_MS + 1 }), free).action).toEqual({ kind: "add" });
  });

  it("add with C, search with /, list the keys with ? however the layout types it, and move through rows with J and K", () => {
    expect([act("c"), act("/"), act("?", { shiftKey: true }), act("j"), act("k")]).toEqual([
      { kind: "add" },
      { kind: "search" },
      { kind: "help" },
      { kind: "row", by: 1 },
      { kind: "row", by: -1 },
    ]);
  });

  it("act on an item only while its panel shows, and never approve, keep or drop anything", () => {
    const item = { ...free, panel: "item" as const };
    expect(["e", "m", "w", "r", "a"].map((key) => act(key, {}, item))).toEqual([
      { kind: "edit" },
      { kind: "move" },
      { kind: "link", link: "waits-for" },
      { kind: "link", link: "relates-to" },
      { kind: "addInside" },
    ]);
    expect(act("e")).toBeNull();
    expect(["y", "d", "x", "Enter", " "].map((key) => act(key, {}, { ...free, panel: "needs" }))).toEqual([
      null,
      null,
      null,
      null,
      null,
    ]);
  });

  it("read a letter from where it sits on a layout without Latin letters", () => {
    expect(act("е", { code: "KeyE" }, { ...free, panel: "item" })).toEqual({ kind: "edit" });
  });

  it("ignore keys while typing, composing, held down, with a modifier, with a form changed, the menu open or switched off", () => {
    for (const context of [
      { ...free, typing: true },
      { ...free, dirty: true },
      { ...free, menu: true },
      { ...free, enabled: false },
    ])
      expect(act("c", {}, context)).toBeNull();
    for (const fields of [
      { isComposing: true },
      { keyCode: 229 },
      { repeat: true },
      { ctrlKey: true },
      { metaKey: true },
      { altKey: true },
      { shiftKey: true },
    ])
      expect(act("c", fields)).toBeNull();
  });
});

describe("nextRow", () => {
  it("starts at the first or last row with none chosen, and stops at the ends", () => {
    expect([nextRow(3, -1, 1), nextRow(3, -1, -1), nextRow(3, 0, 1), nextRow(3, 2, 1), nextRow(3, 0, -1)]).toEqual([
      0, 2, 1, 2, 0,
    ]);
    expect(nextRow(0, -1, 1)).toBe(-1);
  });
});
