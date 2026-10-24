import { describe, expect, it } from "vitest";
import { reveal, uncovered } from "./reveal";

describe("reveal", () => {
  const open = { x1: 0, y1: 0, x2: 400, y2: 500 };

  it("leaves a node already in view, and pans the least that brings one in from any side", () => {
    expect(reveal({ x1: 50, y1: 50, x2: 150, y2: 90 }, open)).toEqual({ x: 0, y: 0 });
    expect(reveal({ x1: 50, y1: 600, x2: 150, y2: 640 }, open)).toEqual({ x: 0, y: -148 });
    expect(reveal({ x1: -100, y1: -30, x2: -20, y2: 10 }, open)).toEqual({ x: 108, y: 38 });
  });

  it("shows the start of a node too big to fit, where its title is", () => {
    expect(reveal({ x1: 100, y1: 100, x2: 900, y2: 1200 }, open)).toEqual({ x: -92, y: -92 });
  });
});

describe("uncovered", () => {
  it("leaves the part above a sheet on a phone, left of a pane on a laptop, or everything", () => {
    expect(uncovered(384, 700, { x1: 0, y1: 400, x2: 384, y2: 700 })).toEqual({ x1: 0, y1: 0, x2: 384, y2: 400 });
    expect(uncovered(1200, 800, { x1: 880, y1: 0, x2: 1200, y2: 800 })).toEqual({ x1: 0, y1: 0, x2: 880, y2: 800 });
    expect(uncovered(384, 700, null)).toEqual({ x1: 0, y1: 0, x2: 384, y2: 700 });
  });
});
