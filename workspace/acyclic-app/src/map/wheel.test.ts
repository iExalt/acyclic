import { describe, expect, it } from "vitest";
import { wheelZoom } from "./wheel";

describe("wheelZoom", () => {
  const scroll = (deltaY: number, deltaMode = 0) => ({ deltaY, deltaMode, ctrlKey: false });

  it("zooms 12% per notch of a mouse wheel, in and out alike", () => {
    expect(wheelZoom(1, scroll(-100), 0.1, 2, 800)).toBeCloseTo(1.12);
    expect(wheelZoom(1, scroll(100), 0.1, 2, 800)).toBeCloseTo(1 / 1.12);
  });

  it("counts lines and pages as pixels, so every wheel zooms by the distance scrolled", () => {
    expect(wheelZoom(1, scroll(-100 / 16, 1), 0.1, 2, 800)).toBeCloseTo(1.12);
    expect(wheelZoom(1, scroll(-1, 2), 0.1, 2, 100)).toBeCloseTo(1.12);
  });

  it("gives a pinch a gain of its own", () => {
    expect(wheelZoom(1, { deltaY: -10, deltaMode: 0, ctrlKey: true }, 0.1, 2, 800)).toBeCloseTo(Math.exp(0.1));
  });

  it("stays within the Map's zoom limits", () => {
    expect(wheelZoom(1.9, scroll(-1000), 0.1, 2, 800)).toBe(2);
    expect(wheelZoom(0.2, scroll(1000), 0.15, 2, 800)).toBe(0.15);
  });
});
