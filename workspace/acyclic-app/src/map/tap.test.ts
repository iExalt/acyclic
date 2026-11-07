import { describe, expect, it } from "vitest";
import { stubTap, type Tapped, tapAction } from "./tap";

const tap = (over: Partial<Tapped>) => tapAction({ picking: false, chevron: false, ...over });
describe("tapAction", () => {
  it("picks an edit target before folding, folds at a chevron, and otherwise selects the item", () => {
    expect(tap({ picking: true, chevron: true })).toBe("pick");
    expect(tap({ chevron: true })).toBe("toggle");
    expect(tap({})).toBe("select");
  });
});

describe("stubTap", () => {
  const stub = { node: "tile", target: "far" };

  it("opens what the stub names", () => {
    expect(stubTap(false, stub)).toEqual({ action: "open", item: "far" });
  });

  it("passes a tap through to the stub's tile while an edit waits for a target", () => {
    expect(stubTap(true, stub)).toEqual({ action: "through", node: "tile" });
  });
});
