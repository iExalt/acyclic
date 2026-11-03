import { describe, expect, it } from "vitest";
import { type Tapped, tapAction } from "./tap";

const tap = (over: Partial<Tapped>) =>
  tapAction({ picking: false, doneBox: false, kind: "leaf", thread: null, chevron: false, ...over });

describe("tapAction", () => {
  it("never hands a done box to an edit or to selection, which take only items", () => {
    expect(tap({ picking: true })).toBe("pick");
    expect(tap({ picking: true, doneBox: true, kind: "collapsed" })).toBe("ignore");
    expect(tap({ doneBox: true, kind: "box" })).not.toBe("select");
  });

  it("folds and unfolds at a chevron, a done box's too", () => {
    expect(tap({ kind: "box", chevron: true })).toBe("toggle");
    expect(tap({ kind: "box", doneBox: true, thread: "t", chevron: true })).toBe("toggle");
  });

  it("picks a finished thread's box's thread, folded or open", () => {
    expect(tap({ kind: "collapsed", doneBox: true, thread: "t" })).toBe("thread");
    expect(tap({ kind: "box", doneBox: true, thread: "t" })).toBe("thread");
  });

  it("unfolds a folded Done box from anywhere on it, but folds an open one only at its chevron", () => {
    expect(tap({ kind: "collapsed", doneBox: true })).toBe("toggle");
    expect(tap({ kind: "box", doneBox: true })).toBe("ignore");
  });

  it("selects an item or a box", () => {
    expect(tap({})).toBe("select");
    expect(tap({ kind: "box" })).toBe("select");
  });
});
