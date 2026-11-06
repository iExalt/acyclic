import { describe, expect, it } from "vitest";
import type { Counts } from "./api";
import { threadLists } from "./threadLists";

const NONE: Counts = {
  done: 0,
  leaves: 0,
  in_progress: 0,
  ready: 0,
  stalled: 0,
  waiting: 0,
  parked: 0,
  canceled: 0,
};
describe("the Overview's threads", () => {
  it("lists threads at work, and to start those with an item ready, each by id in natural order", () => {
    const counts = new Map([
      ["thread-10", { ...NONE, leaves: 3, in_progress: 1, done: 1 }],
      ["thread-2", { ...NONE, leaves: 2, in_progress: 1, ready: 1 }],
      ["Thread-8", { ...NONE, leaves: 3, in_progress: 1, done: 1, ready: 1 }],
      ["thread-3", { ...NONE, leaves: 2, ready: 2 }],
      ["thread-4", { ...NONE, leaves: 1, waiting: 1 }],
      ["thread-20", { ...NONE, leaves: 1, ready: 1 }],
      ["Thread-6", { ...NONE, leaves: 1, ready: 1 }],
      ["thread-5", { ...NONE, leaves: 1, ready: 1 }],
    ]);
    expect(threadLists(counts)).toEqual({
      working: ["thread-2", "Thread-8", "thread-10"],
      next: ["thread-3", "thread-5", "Thread-6", "thread-20"],
    });
  });

  it("keeps a stalled thread under work, whose holder's lease ran out, and not among those to start", () => {
    const counts = new Map([
      ["thread-1", { ...NONE, leaves: 2, stalled: 2 }],
      ["thread-2", { ...NONE, leaves: 2, stalled: 1, done: 1 }],
      ["thread-3", { ...NONE, leaves: 1, ready: 1 }],
    ]);
    expect(threadLists(counts)).toEqual({ working: ["thread-1", "thread-2"], next: ["thread-3"] });
  });

  it("has nothing to start when every thread is at work or waiting, or there are none", () => {
    const counts = new Map([
      ["thread-1", { ...NONE, leaves: 1, in_progress: 1 }],
      ["thread-2", { ...NONE, leaves: 1, waiting: 1 }],
    ]);
    expect(threadLists(counts)).toEqual({ working: ["thread-1"], next: [] });
    expect(threadLists(new Map())).toEqual({ working: [], next: [] });
  });
});
