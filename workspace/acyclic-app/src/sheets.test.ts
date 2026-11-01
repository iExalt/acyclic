import { describe, expect, it } from "vitest";
import { detentAfter, draggableFrom, velocity } from "./sheets";

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

describe("detentAfter", () => {
  // A 600 px stage: the half sheet 320 px tall, 280 px of travel to full.
  const land = (from: "half" | "full", dy: number, speed = 0) => detentAfter(from, dy, speed, 280, 320);
  it("opens a half sheet full past a quarter of the travel up, closes it past a third of its height down", () => {
    expect([land("half", -80), land("half", -60), land("half", 120), land("half", 100)]).toEqual([
      "full",
      "half",
      "close",
      "half",
    ]);
  });

  it("brings a full sheet back to half past a quarter down, and closes it only past half and a third more", () => {
    expect([land("full", 80), land("full", 60), land("full", 400), land("full", -40)]).toEqual([
      "half",
      "full",
      "close",
      "full",
    ]);
  });

  it("sends a flick the way it went whatever the distance", () => {
    expect([land("half", -10, -0.8), land("half", 10, 0.8), land("full", 10, 0.8)]).toEqual(["full", "close", "half"]);
    // A drag up that ends falling fast lands by its end.
    expect(land("half", -100, 0.9)).toBe("close");
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
