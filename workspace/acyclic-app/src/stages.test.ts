import { describe, expect, it } from "vitest";
import { overallStage, stageCounts, stageCountWords, stateWords } from "./stages";

describe("a state in words", () => {
  it("reads as a sentence starts it", () => {
    expect(["in-progress", "backlog", "done"].map(stateWords)).toEqual(["In progress", "Backlog", "Done"]);
  });
});

describe("stageCounts", () => {
  it("counts done apart from in review, in the order work moves, leaving out stages with none", () => {
    const counts = {
      done: 9,
      leaves: 14,
      in_review: 2,
      in_progress: 1,
      ready: 3,
      blocked: 0,
      parked: 0,
      unthreaded: 1,
    };
    expect(stageCounts(counts)).toEqual([
      { stage: "done", count: 7 },
      { stage: "in-review", count: 2 },
      { stage: "in-progress", count: 1 },
      { stage: "ready", count: 3 },
      { stage: "unthreaded", count: 1 },
    ]);
  });
});

describe("stageCountWords", () => {
  it("agrees with its count, so a count of tasks needing a thread reads as a sentence", () => {
    expect([1, 3].map((count) => `${count} ${stageCountWords("unthreaded", count)}`)).toEqual([
      "1 needs a thread",
      "3 need a thread",
    ]);
    expect(stageCountWords("in-review", 2)).toBe("in review");
  });
});

describe("overallStage", () => {
  const none = { done: 0, leaves: 0, in_review: 0, in_progress: 0, ready: 0, blocked: 0, parked: 0, unthreaded: 0 };
  it("tells a parent's work as one stage: moving first, then in review, done, ready, blocked and parked", () => {
    expect(overallStage({ ...none, leaves: 3, done: 1, in_progress: 1, blocked: 1 })).toBe("in-progress");
    expect(overallStage({ ...none, leaves: 2, done: 2, in_review: 1 })).toBe("in-review");
    expect(overallStage({ ...none, leaves: 2, done: 2 })).toBe("done");
    expect(overallStage({ ...none, leaves: 3, done: 1, ready: 1, blocked: 1 })).toBe("ready");
    expect(overallStage({ ...none, leaves: 2, done: 1, blocked: 1 })).toBe("blocked");
    expect(overallStage({ ...none, leaves: 1, parked: 1 })).toBe("parked");
  });
});
