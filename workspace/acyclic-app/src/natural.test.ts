import { describe, expect, it } from "vitest";
import { natural } from "./natural";

describe("natural", () => {
  it("sorts numbers by value and the rest as text, as the CLI does", () => {
    const titles = ["Step 10", "Step 2", "step 1", "Step 02", "Step", "10", "9", "Step 2b", "Step 2a"];
    expect(titles.sort(natural)).toEqual([
      "9",
      "10",
      "Step",
      "step 1",
      "Step 02",
      "Step 2",
      "Step 2a",
      "Step 2b",
      "Step 10",
    ]);
    expect(["phase-10", "phase-2", "i10", "i2"].sort(natural)).toEqual(["i2", "i10", "phase-2", "phase-10"]);
  });

  it("is total: strings equal only when the same, by UTF-8 bytes past case and zeros", () => {
    expect(natural("02", "2")).not.toBe(0);
    expect(Math.sign(natural("Step 1", "step 1"))).toBe(-Math.sign(natural("step 1", "Step 1")));
    // Past the ASCII letters, plain bytes: "é" (C3 A9) after "z", and "😀" (F0…) after "ｚ" (EF…), unlike UTF-16 order.
    expect(["é", "z"].sort(natural)).toEqual(["z", "é"]);
    expect(["😀", "ｚ"].sort(natural)).toEqual(["ｚ", "😀"]);
    expect(natural("a", "a")).toBe(0);
  });
});
