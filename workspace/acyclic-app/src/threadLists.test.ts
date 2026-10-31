import { describe, expect, it } from "vitest";
import type { Counts } from "./api";
import { threadLists } from "./threadLists";
import type { Card } from "./waves";

const NONE: Counts = {
  done: 0,
  leaves: 0,
  in_review: 0,
  in_progress: 0,
  ready: 0,
  blocked: 0,
  parked: 0,
  unthreaded: 0,
};
const card = (id: string): Card => ({ id, phases: [], counts: NONE, after: [], looped: [] });
const waves = (...ids: string[][]) => ({ waves: ids.map((one) => one.map(card)), anytime: [] });

describe("the Overview's threads", () => {
  it("lists threads at work, and to start next the earliest wave's threads not at work, naming that wave", () => {
    const counts = new Map([
      ["thread-10", { ...NONE, leaves: 3, in_review: 1, done: 1 }],
      ["thread-2", { ...NONE, leaves: 2, in_progress: 1 }],
      ["thread-3", { ...NONE, leaves: 2, ready: 2 }],
    ]);
    // Wave 1 is all at work, so the next to start are wave 2's.
    expect(threadLists(counts, waves(["thread-2"], ["thread-10", "thread-3"]))).toEqual({
      working: ["thread-2", "thread-10"],
      next: ["thread-3"],
      wave: 1,
    });
  });

  it("has nothing to start when every wave's threads are at work, or there are no waves", () => {
    const counts = new Map([["thread-1", { ...NONE, leaves: 1, in_progress: 1 }]]);
    expect(threadLists(counts, waves(["thread-1"]))).toEqual({ working: ["thread-1"], next: [], wave: null });
    expect(threadLists(new Map(), waves())).toEqual({ working: [], next: [], wave: null });
    expect(threadLists(new Map(), null)).toEqual({ working: [], next: [], wave: null });
  });
});
