import { describe, expect, it } from "vitest";
import { onChevron, TITLE } from "./chevron";

describe("onChevron", () => {
  const box = { x1: 100, y1: 100, x2: 500, y2: 400 };
  const at = (x: number, y: number) => ({ x: box.x1 + x, y: box.y1 + y });

  it("takes an open box's title strip at its left, and leaves the rest of the title and the box to select it", () => {
    expect(onChevron("box", box, at(10, 10), 1)).toBe(true);
    expect(onChevron("box", box, at(TITLE + 20, 10), 1)).toBe(false);
    expect(onChevron("box", box, at(10, TITLE + 20), 1)).toBe(false);
    expect(onChevron("leaf", box, at(10, 10), 1)).toBe(false);
  });

  it("takes a collapsed box's left edge from top to bottom, and leaves the rest of it to select it", () => {
    const tall = { ...box, y2: 164 };
    expect(onChevron("collapsed", tall, at(10, 60), 1)).toBe(true);
    expect(onChevron("collapsed", tall, at(150, 30), 1)).toBe(false);
  });

  it("widens to a fingertip when the Map is zoomed out, but never past half the node", () => {
    expect(onChevron("box", box, at(48, 48), 0.5)).toBe(true);
    expect(onChevron("box", box, at(48, 48), 1)).toBe(false);
    expect(onChevron("box", box, at(10, 80), 0.5)).toBe(true);
    expect(onChevron("box", box, at(10, 100), 0.5)).toBe(false);
    const narrow = { x1: 0, y1: 0, x2: 60, y2: 300 };
    expect(onChevron("box", narrow, { x: 29, y: 10 }, 0.25)).toBe(true);
    expect(onChevron("box", narrow, { x: 31, y: 10 }, 0.25)).toBe(false);
  });
});
