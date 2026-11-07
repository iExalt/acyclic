import { describe, expect, it } from "vitest";
import {
  CALM_FRAMES,
  FRESH_WATCH,
  type FrameWatch,
  intentsPending,
  type SuppressionOwner,
  sameSuppressionOwner,
  stepFrame,
} from "./framed";

const none = { shown: null, parked: null, anchor: false, toFrame: false, tapReveals: 0 };

describe("stepFrame", () => {
  const run = (frames: { busy: boolean; animating?: number; viewport: string }[]) => {
    let watch: FrameWatch = FRESH_WATCH;
    return frames.map((frame) => {
      const step = stepFrame(watch, { animating: 0, ...frame });
      watch = step;
      return step.settled;
    });
  };

  it("settles once the viewport has held for the calm frames, and not before", () => {
    const still = { busy: false, viewport: "1,0,0" };
    expect(run(Array(CALM_FRAMES + 1).fill(still))).toEqual([false, false, true]);
  });

  it("starts again when the viewport moves or something is in flight", () => {
    const at = (viewport: string, busy = false) => ({ busy, viewport });
    expect(run([at("a"), at("a"), at("a"), at("b"), at("b"), at("b")])).toEqual([
      false,
      false,
      true,
      false,
      false,
      true,
    ]);
    expect(run([at("a"), at("a"), at("a", true), at("a"), at("a"), at("a")])).toEqual([
      false,
      false,
      false,
      false,
      true,
      true,
    ]);
  });
});

describe("animating tiles", () => {
  it("keep the Map from settling until they have ended, however still the viewport", () => {
    const still = { busy: false, viewport: "a" };
    expect(stepFrame({ viewport: "a", calm: 5 }, { ...still, animating: 1 }).settled).toBe(false);
    expect(stepFrame({ viewport: "a", calm: 5 }, { ...still, animating: 0 }).settled).toBe(true);
  });
});

describe("intentsPending", () => {
  it("holds for each intent that can still move the camera", () => {
    expect(intentsPending(none)).toBe(false);
    expect(intentsPending({ ...none, shown: {} })).toBe(true);
    expect(intentsPending({ ...none, anchor: true })).toBe(true);
    expect(intentsPending({ ...none, toFrame: true })).toBe(true);
    expect(intentsPending({ ...none, tapReveals: 1 })).toBe(true);
  });

  it("lets a reveal that cannot be satisfied, as on a fully covered Map, stop holding the camera", () => {
    const request = {};
    expect(intentsPending({ ...none, shown: request, parked: request })).toBe(false);
    expect(intentsPending({ ...none, shown: {}, parked: request })).toBe(true);
  });
});

describe("sameSuppressionOwner", () => {
  const owner: SuppressionOwner = {
    model: {},
    view: {},
    source: "source",
    thread: "thread",
    selected: "selected",
  };
  const cases: [string, SuppressionOwner | null, boolean][] = [
    ["same fields", { ...owner }, true],
    ["missing owner", null, false],
    ["different model", { ...owner, model: {} }, false],
    ["different view", { ...owner, view: {} }, false],
    ["different source", { ...owner, source: "other" }, false],
    ["different thread", { ...owner, thread: "other" }, false],
    ["different selection", { ...owner, selected: "other" }, false],
  ];

  it.each(cases)("compares %s", (_name, current, expected) => {
    expect(sameSuppressionOwner(current, owner)).toBe(expected);
  });
});
