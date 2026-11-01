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

const lists = (...ids: string[][]) => ({ waves: ids.map((one) => one.map(card)), anytime: [] as Card[] });

describe("the Overview's threads", () => {
  it("lists threads at work, and to start the approved ones with a task ready, Any time first, then by wave", () => {
    const counts = new Map([
      ["thread-10", { ...NONE, leaves: 3, in_review: 1, done: 1 }],
      ["thread-2", { ...NONE, leaves: 2, in_progress: 1, ready: 1 }],
      ["thread-8", { ...NONE, leaves: 2, in_review: 1, done: 1, ready: 1 }],
      ["thread-3", { ...NONE, leaves: 2, ready: 2 }],
      ["thread-4", { ...NONE, leaves: 1, blocked: 1 }],
      ["thread-5", { ...NONE, leaves: 1, ready: 1 }],
      ["thread-6", { ...NONE, leaves: 1, ready: 1 }],
      ["thread-7", { ...NONE, leaves: 1, ready: 1 }],
    ]);
    // Wave 1 is at work (in progress, or in review with a task ready) or blocked, so nothing in it is to start;
    // thread-6 waits for approval, so it is in Needs you instead; thread-7, any time, comes first.
    const view = { ...lists(["thread-2", "thread-8", "thread-4"], ["thread-10", "thread-6", "thread-3", "thread-5"]) };
    view.anytime = [card("thread-7")];
    expect(threadLists(counts, view, new Set(["thread-6"]))).toEqual({
      working: ["thread-2", "thread-8", "thread-10"],
      next: ["thread-7", "thread-3", "thread-5"],
    });
  });

  it("has nothing to start when every thread is at work or blocked, or there are no waves", () => {
    const counts = new Map([
      ["thread-1", { ...NONE, leaves: 1, in_progress: 1 }],
      ["thread-2", { ...NONE, leaves: 1, blocked: 1 }],
    ]);
    expect(threadLists(counts, lists(["thread-1"], ["thread-2"]), new Set())).toEqual({
      working: ["thread-1"],
      next: [],
    });
    expect(threadLists(new Map(), lists(), new Set())).toEqual({ working: [], next: [] });
    expect(threadLists(new Map(), null, new Set())).toEqual({ working: [], next: [] });
  });
});
