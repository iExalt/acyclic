import { describe, expect, it } from "vitest";
import { wheelZoom } from "./wheel";

describe("wheelZoom", () => {
  const scroll = (deltaY: number, deltaMode = 0) => ({ deltaY, deltaMode, ctrlKey: false });
  const run = (deltas: number[], ctrlKey = false) =>
    deltas.reduce((zoom, deltaY) => wheelZoom(zoom, { deltaY, deltaMode: 0, ctrlKey }, 0.01, 100, 800), 1);

  it("zooms 12% per notch of a Mac's mouse wheel, in and out alike, and a little more for larger notches", () => {
    expect(wheelZoom(1, scroll(-80), 0.1, 2, 800)).toBeCloseTo(1.12);
    expect(wheelZoom(1, scroll(80), 0.1, 2, 800)).toBeCloseTo(1 / 1.12);
    expect(wheelZoom(1, scroll(-100), 0.1, 2, 800)).toBeCloseTo(1.12 ** 1.25);
  });

  it("takes a wheel that counts lines or pages for a notched one", () => {
    expect(wheelZoom(1, scroll(-5, 1), 0.1, 2, 800)).toBeCloseTo(1.12);
    expect(wheelZoom(1, scroll(-1, 2), 0.1, 2, 80)).toBeCloseTo(1.12);
  });

  it("zooms a trackpad's scroll and pinch as Cytoscape did, a tenfold zoom per 250 pixels", () => {
    // A short two-finger swipe and a pinch out, as Chrome on a Mac sent them.
    expect(run([1, 2, 1, 4, 6, 6, 6, 8, 8, 13, 5, 12, 14, 10, 11])).toBeCloseTo(10 ** (-107 / 250));
    expect(run([-1.04, -0.89, -1.78, -1.78, -2.44], true)).toBeCloseTo(10 ** (7.93 / 250));
    // A pinch is never a notch, however far it moves at once.
    expect(run([-80], true)).toBeCloseTo(10 ** (80 / 250));
  });

  it("stays within the Map's zoom limits", () => {
    expect(wheelZoom(1.9, scroll(-1000), 0.1, 2, 800)).toBe(2);
    expect(wheelZoom(0.2, scroll(1000), 0.15, 2, 800)).toBe(0.15);
  });
});
