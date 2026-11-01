import { describe, expect, it } from "vitest";
import { cut, wrap } from "./fit";

// Every character 1 wide, the ellipsis too.
const measure = (text: string) => text.length;

describe("cut", () => {
  it("keeps what fits and cuts the rest short with an ellipsis, never after a space", () => {
    expect(cut("Patch two", 9, measure)).toBe("Patch two");
    expect(cut("Patch two", 8, measure)).toBe("Patch t…");
    expect(cut("Patch two", 7, measure)).toBe("Patch…");
    expect(cut("Patch two", 0, measure)).toBe("…");
  });
});

describe("wrap", () => {
  it("wraps words while lines are left and cuts the last line short", () => {
    expect(wrap("Route the links inside a box", 12, 2, measure)).toEqual(["Route the", "links insid…"]);
    expect(wrap("Short", 12, 2, measure)).toEqual(["Short"]);
    expect(wrap("Route the links", 12, 1, measure)).toEqual(["Route the l…"]);
  });

  it("cuts a word too long for a line of its own where it stands", () => {
    expect(wrap("Unbreakablewordhere and more", 10, 2, measure)).toEqual(["Unbreakab…"]);
  });
});
