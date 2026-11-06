import { describe, expect, it } from "vitest";
import { PALETTES } from "./palette";
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
      stalled: 1,
      waiting: 0,
      parked: 0,
      canceled: 2,
    };
    expect(stageCounts(counts)).toEqual([
      { stage: "done", count: 7 },
      { stage: "in-progress", count: 1 },
      { stage: "stalled", count: 1 },
      { stage: "ready", count: 3 },
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
    expect(stageCountWords("canceled")).toBe("canceled");
  });
});

describe("stalled", () => {
  it("has its word and icon, takes ready's tint so it draws no more attention, and is not drawn quiet", () => {
    expect(STAGE_WORDS.stalled).toBe("Stalled");
    expect(STAGE_ICONS.stalled).toContain("<circle");
    expect(QUIET.has("stalled")).toBe(false);
    expect(stageCountWords("stalled")).toBe("stalled");
    for (const palette of Object.values(PALETTES)) {
      expect(palette.tints.stalled).toBe(palette.tints.ready);
      expect(palette.tints.stalled).not.toBe(palette.tints["in-progress"]);
    }
  });
});

describe("stageCountWords", () => {
  it("says a stage in lower case, as a count follows it", () => {
    expect(stageCountWords("in-progress")).toBe("in progress");
  });
});
