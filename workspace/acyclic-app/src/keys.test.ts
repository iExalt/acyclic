import { describe, expect, it } from "vitest";
import {
  type KeyContext,
  type KeyPress,
  keyReference,
  nextRow,
  SEQUENCE_MS,
  SHORTCUTS,
  step,
  takesTyping,
} from "./keys";

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
const free: KeyContext = { enabled: true, panel: null, menu: false, dirty: false, typing: false, rows: 3 };
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
    // With no rows in view, J and K are the page's.
    expect([act("j", {}, { ...free, rows: 0 }), act("k", {}, { ...free, rows: 0 })]).toEqual([null, null]);
  });

  it("counts a field or a list to pick from as typing, never a box to tick or a button", () => {
    const typing: [string, string | null][] = [
      ["textarea", null],
      ["select", null],
      ["input", null],
      ["INPUT", "search"],
      ["input", "text"],
    ];
    expect(typing.map(([tag, type]) => takesTyping(tag, type))).toEqual([true, true, true, true, true]);
    const pressed: [string, string | null][] = [
      ["input", "checkbox"],
      ["input", "radio"],
      ["input", "range"],
      ["button", null],
      ["div", null],
    ];
    expect(pressed.map(([tag, type]) => takesTyping(tag, type))).toEqual([false, false, false, false, false]);
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

describe("keyReference", () => {
  it("lists every shortcut ? can't run now, so each is either runnable or described, with the platform's chords", () => {
    const runnable = new Set(["view:overview", "view:map", "view:threads", "needs", "add"]);
    const rows = keyReference(runnable, true);
    const said = rows.map((row) => row.keys).join(" | ");
    const keys = rows.flatMap((row) => row.keys.split(", "));
    for (const [key, chord] of Object.entries(SHORTCUTS))
      expect(runnable.has(key) || keys.some((one) => one === chord || one.startsWith(`${chord} `))).toBe(true);
    expect(rows[0]).toEqual({
      what: "With an item open",
      keys: "E edit, M move, W waits for, R relates to, A add inside",
    });
    expect(said).toContain("⌘K");
    expect(
      keyReference(runnable, false)
        .map((row) => row.keys)
        .join(" | "),
    ).toContain("Ctrl Z");
    // With an item open, its keys run, so the reference doesn't repeat them.
    expect(keyReference(new Set(Object.keys(SHORTCUTS)), true)[0]?.what).toBe("Next or previous row");
  });
});
