import { describe, expect, it } from "vitest";
import { QUIET, STAGE_ICONS, STAGE_WORDS, stageCounts, stageCountWords, stateWords } from "./stages";

describe("a state in words", () => {
  it("reads as a sentence starts it", () => {
    expect(["in-progress", "open", "done"].map(stateWords)).toEqual(["In progress", "Open", "Done"]);
  });
});

describe("stageCounts", () => {
  it("counts each stage in the order work moves, leaving out stages with none", () => {
    const counts = {
      done: 7,
      leaves: 12,
      in_progress: 1,
      ready: 3,
      waiting: 0,
      parked: 0,
      unthreaded: 1,
      canceled: 2,
    };
    expect(stageCounts(counts)).toEqual([
      { stage: "done", count: 7 },
      { stage: "in-progress", count: 1 },
      { stage: "ready", count: 3 },
      { stage: "unthreaded", count: 1 },
      { stage: "canceled", count: 2 },
    ]);
  });
});

describe("canceled", () => {
  it("has its word, icon and a quiet look like the other work set aside", () => {
    expect(STAGE_WORDS.canceled).toBe("Canceled");
    expect(STAGE_ICONS.canceled).toContain("<circle");
    expect(QUIET.has("canceled")).toBe(true);
    expect(stateWords("canceled")).toBe("Canceled");
    expect(stageCountWords("canceled", 2)).toBe("canceled");
  });
});

describe("stageCountWords", () => {
  it("agrees with its count, so a count of items needing a thread reads as a sentence", () => {
    expect([1, 3].map((count) => `${count} ${stageCountWords("unthreaded", count)}`)).toEqual([
      "1 needs a thread",
      "3 need a thread",
    ]);
    expect(stageCountWords("in-progress", 2)).toBe("in progress");
  });
});
