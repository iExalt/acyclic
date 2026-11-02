import { describe, expect, it } from "vitest";
import { noticeAfter as after, type Lasts, type Place } from "./notices";

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
    // Any other layer, such as the outline's sheet, is the same surface whatever view is under it.
    const sheet: Place = { ...map, layer: "projects" };
    expect(after(raised("refusal", sheet), moved({ ...sheet, view: "overview" }))).toEqual(raised("refusal", sheet));
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
