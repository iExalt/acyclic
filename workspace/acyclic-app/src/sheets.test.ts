import { describe, expect, it } from "vitest";
import { detentAfter, draggableFrom, firstHeight, PEEK, velocity } from "./sheets";

describe("velocity", () => {
  it("reads a drag's speed from its last 100 ms, not from its start", () => {
    // A slow pull for a second, then a flick down: the flick is what the user meant.
    const samples = [
      { t: 0, y: 500 },
      { t: 900, y: 510 },
      { t: 950, y: 540 },
      { t: 1000, y: 580 },
    ];
    expect(velocity(samples)).toBeCloseTo(70 / 100);
    expect(velocity([{ t: 5, y: 10 }])).toBe(0);
    // Fast, then held still for 300 ms before the release: it had stopped.
    expect(velocity([...samples, { t: 1300, y: 580 }])).toBe(0);
    expect(velocity([])).toBe(0);
  });
});

describe("firstHeight", () => {
  it("opens a panel short, as tall as its main action needs, and the outline at half the room, at least 320 px", () => {
    expect([firstHeight(700, false), firstHeight(400, false), firstHeight(100, false)]).toEqual([PEEK, PEEK, 100]);
    // A main action a long title pushed down still shows; one that fits leaves the panel short.
    expect([firstHeight(700, false, 170), firstHeight(700, false, 120), firstHeight(150, false, 170)]).toEqual([
      170,
      PEEK,
      150,
    ]);
    expect([firstHeight(700, true), firstHeight(500, true), firstHeight(300, true)]).toEqual([350, 320, 300]);
  });
});

describe("detentAfter", () => {
  // A 744 px stage, as a large phone's between its bars: a panel's first height, 600 px of travel to full.
  const land = (from: "first" | "full", dy: number, speed = 0) => detentAfter(from, dy, speed, 600, PEEK);
  it("opens a short sheet full past a quarter of the travel up, closes it past a third of its height down", () => {
    expect([land("first", -160), land("first", -140), land("first", 60), land("first", 40)]).toEqual([
      "full",
      "first",
      "close",
      "first",
    ]);
  });

  it("brings a full sheet back to its first height past a quarter down, closing it only past that and a third", () => {
    expect([land("full", 160), land("full", 140), land("full", 660), land("full", 640), land("full", -40)]).toEqual([
      "first",
      "full",
      "close",
      "first",
      "full",
    ]);
  });

  it("sends a flick the way it went whatever the distance", () => {
    expect([land("first", -10, -0.8), land("first", 10, 0.8), land("full", 10, 0.8)]).toEqual([
      "full",
      "close",
      "first",
    ]);
    // A drag up that ends falling fast lands by its end.
    expect(land("first", -100, 0.9)).toBe("close");
  });
});

describe("draggableFrom", () => {
  it("lets a press on the header drag, but never one on its controls", () => {
    // Elements as far as the rule reads them: what the press is inside of.
    const inside = (control: boolean) => ({ closest: () => (control ? {} : null) }) as unknown as Element;
    expect(draggableFrom(inside(false))).toBe(true);
    expect(draggableFrom(inside(true))).toBe(false);
    expect(draggableFrom(null)).toBe(false);
  });
});
