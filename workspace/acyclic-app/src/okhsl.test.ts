import { describe, expect, it } from "vitest";
import { okhsl, toe } from "./okhsl";

describe("okhsl", () => {
  it("gives the colours the reference implementation does, from grey to the gamut's edge", () => {
    // Converted by ColorAide's Okhsl, which follows Ottosson's reference.
    expect(okhsl(0, 0.5, 0.5)).toBe("#aa5a74");
    expect(okhsl(110, 0.8, 0.7)).toBe("#b1b337");
    expect(okhsl(256.7, 0.8, 0.65)).toBe("#659ff0");
    expect(okhsl(200, 1, 0.5)).toBe("#00888d");
    expect(okhsl(30, 0.3, 0.9)).toBe("#eedfdc");
    expect(okhsl(140, 0.9, 0.4)).toBe("#356e27");
    expect([okhsl(80, 0.5, 0), okhsl(80, 0.5, 1)]).toEqual(["#000000", "#ffffff"]);
  });

  it("lightens OKLab's lightness at the dark end, as eyes space it", () => {
    expect(toe(0.44)).toBeCloseTo(0.352971, 5);
    expect(toe(0.86)).toBeCloseTo(0.837041, 5);
  });
});
