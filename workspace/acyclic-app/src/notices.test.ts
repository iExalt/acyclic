import { describe, expect, it } from "vitest";
import { noticeAfter as after, type Lasts, type Place, placed } from "./notices";

const map: Place = { layer: null, item: null, thread: null, view: "map" };
const item: Place = { ...map, layer: "item", item: "i1" };
const raised = (lasts: Lasts, at: Place | null = item) => ({ lasts, at });
const moved = (to: Place) => ({ kind: "moved" as const, to });

describe("a notice's life", () => {
  it("is shown where the user is when it first shows, whatever moved in the same step", () => {
    expect(after(raised("news", null), moved(map))).toEqual(raised("news", map));
  });

  it("keeps news while the user stays, and drops it once anything they see changes", () => {
    expect(after(raised("news"), moved(item))).toEqual(raised("news"));
    expect(after(raised("news"), moved({ ...item, item: "i2" }))).toBeNull();
    expect(after(raised("news"), moved({ ...item, thread: "t1" }))).toBeNull();
    expect(after(raised("news"), moved(map))).toBeNull();
    expect(after(raised("news", map), moved({ ...map, view: "threads" }))).toBeNull();
  });

  it("keeps a refusal on the surface that raised it, and drops it when that surface closes or shows another", () => {
    // A thread picked under the item's panel changes nothing the panel shows.
    expect(after(raised("refusal"), moved({ ...item, thread: "t1" }))).toEqual(raised("refusal"));
    expect(after(raised("refusal"), moved({ ...item, item: "i2" }))).toBeNull();
    expect(after(raised("refusal"), moved(map))).toBeNull();
    const thread: Place = { ...map, layer: "thread", thread: "t1" };
    expect(after(raised("refusal", thread), moved({ ...thread, thread: "t2" }))).toBeNull();
    const needs: Place = { ...map, layer: "needs" };
    expect(after(raised("refusal", needs), moved({ ...needs, view: "overview" }))).toEqual(raised("refusal", needs));
    expect(after(raised("refusal", map), moved({ ...map, view: "threads" }))).toBeNull();
  });

  it("keeps an offer and a stale plan wherever the user goes, until a write or Dismiss; a stale one until a read", () => {
    for (const lasts of ["offer", "stale"] as const) {
      expect(after(raised(lasts), moved(map))).toEqual(raised(lasts));
      expect(after(raised(lasts), { kind: "write" })).toBeNull();
      expect(after(raised(lasts), { kind: "dismiss" })).toBeNull();
    }
    expect(after(raised("stale"), { kind: "read" })).toBeNull();
    expect(after(raised("offer"), { kind: "read" })).toEqual(raised("offer"));
  });

  it("keeps a form's warning with the form, through moves and writes, until the form closes", () => {
    expect(after(raised("form"), moved(map))).toEqual(raised("form"));
    expect(after(raised("form"), { kind: "write" })).toEqual(raised("form"));
    expect(after(raised("form"), { kind: "form-closed" })).toBeNull();
    expect(after(raised("news"), { kind: "form-closed" })).toEqual(raised("news"));
  });
});

describe("where a notice shows", () => {
  it("shows under the panel on top, in the slot with no panel, and an offer or a stale plan always in the slot", () => {
    expect(placed(raised("refusal"), item)).toBe("panel");
    expect(placed(raised("news"), { ...map, layer: "needs" })).toBe("panel");
    expect(placed(raised("news"), map)).toBe("slot");
    expect(placed(raised("refusal"), { ...map, layer: "mode" })).toBe("slot");
    expect(placed(raised("offer"), item)).toBe("slot");
    expect(placed(raised("stale"), item)).toBe("slot");
  });
});
