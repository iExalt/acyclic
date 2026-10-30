import { describe, expect, it } from "vitest";
import { segmentsOf } from "./route";

describe("segmentsOf", () => {
  it("places each bend by its weight along the ends' line and its distance off it, to the left of the way it runs", () => {
    // Down from (100, 24) to (20, 104), the target to the left: bends at (100, 64) and (20, 64). The line runs
    // (−80, 80), length 80√2, normal (−80, −80)/80√2. (100, 64) is (0, 40) from the start: weight 3200/12800 = 0.25,
    // distance −3200/(80√2) = −20√2. (20, 64) is (−80, 40): weight 9600/12800 = 0.75, distance 3200/(80√2) = 20√2.
    const out = segmentsOf(
      [
        { x: 100, y: 24 },
        { x: 100, y: 64 },
        { x: 20, y: 64 },
        { x: 20, y: 104 },
      ],
      { x: 110, y: 0 },
      { x: 10, y: 128 },
    );
    const root = 20 * Math.SQRT2;
    expect(out).toEqual({
      sourceEndpoint: "-10px 24px",
      targetEndpoint: "10px -24px",
      weights: "0.25 0.75",
      distances: `${Math.round(-root * 1000) / 1000} ${Math.round(root * 1000) / 1000}`,
    });
  });

  it("gives a straight route one bend, at its middle, and draws no route without two distinct points", () => {
    const straight = segmentsOf(
      [
        { x: 0, y: 24 },
        { x: 0, y: 104 },
      ],
      { x: 0, y: 0 },
      { x: 0, y: 128 },
    );
    expect(straight).toMatchObject({ weights: "0.5", distances: "0" });
    expect(segmentsOf([{ x: 0, y: 0 }], { x: 0, y: 0 }, { x: 0, y: 1 })).toBeNull();
    const still = { x: 5, y: 5 };
    expect(segmentsOf([still, still], still, still)).toBeNull();
  });
});
