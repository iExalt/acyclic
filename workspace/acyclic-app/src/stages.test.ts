import { describe, expect, it } from "vitest";
import { stageCounts, stateWords } from "./stages";

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
