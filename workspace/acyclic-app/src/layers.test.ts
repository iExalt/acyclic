import { describe, expect, it } from "vitest";
import { arrived, depth, follow, openLayers, topLayer } from "./layers";

const none = { mode: false, adding: false, item: false, needs: false, phases: false, thread: false, away: false };

describe("topLayer", () => {
  it("puts a waiting move over a form, a form over the phases, those over an item, that over the needs list, and those over a thread", () => {
    const all = { mode: true, adding: true, item: true, needs: true, phases: true, thread: true, away: true };
    expect(topLayer(all)).toBe("mode");
    expect(topLayer({ ...all, mode: false })).toBe("add");
    expect(topLayer({ ...all, mode: false, adding: false })).toBe("phases");
    expect(topLayer({ ...all, mode: false, adding: false, phases: false })).toBe("item");
    expect(topLayer({ ...none, needs: true, thread: true })).toBe("needs");
    expect(topLayer({ ...none, thread: true })).toBe("thread");
    expect(topLayer(none)).toBeNull();
    expect(openLayers({ ...none, thread: true, item: true })).toEqual(["item", "thread"]);
    // Away from home, on a phone, is under everything: Back closes what's over the view before it goes home.
    expect(openLayers({ ...none, away: true, item: true, thread: true })).toEqual(["item", "thread", "away"]);
    expect(topLayer({ ...none, away: true })).toBe("away");
  });
});

describe("the history of layers", () => {
  const at = (pushed: number, waiting = false) => ({ pushed, waiting });

  it("keeps one entry per layer open: pushing as they open, going back over the extra as the app closes them", () => {
    expect(depth({ ...none, thread: true, item: true })).toBe(2);
    expect(follow(at(0), 2)).toEqual({ entries: at(2), action: { kind: "push", from: 0, to: 2 } });
    expect(follow(at(2), 1)).toEqual({ entries: at(1, true), action: { kind: "back", by: 1 } });
    expect(follow(at(1), 1).action).toBeNull();
  });

  it("holds a push until its own move back has landed, then brings the entries to what's open then", () => {
    // Closed in the app, then another opened before the move back lands.
    const waiting = follow(at(2), 1).entries;
    expect(follow(waiting, 2)).toEqual({ entries: waiting, action: null });
    expect(arrived(waiting, 1, 2)).toEqual({ entries: at(2), action: { kind: "push", from: 1, to: 2 } });
    expect(arrived(waiting, 1, 1)).toEqual({ entries: at(1), action: null });
  });

  it("closes the layer on top on Back, and ignores a move that lands on no fewer layers, such as forward", () => {
    expect(arrived(at(2), 1, 2)).toEqual({ entries: at(1), action: { kind: "close", by: 1 } });
    // A jump over several entries, as a long press on Back makes, or Chrome skipping one pushed without a tap.
    expect(arrived(at(3), 1, 3)).toEqual({ entries: at(1), action: { kind: "close", by: 2 } });
    expect(arrived(at(2), 2, 2).action).toBeNull();
    // A form left open by Back stays: it takes its entry again.
    expect(follow(arrived(at(2), 1, 2).entries, 2)).toEqual({
      entries: at(2),
      action: { kind: "push", from: 1, to: 2 },
    });
  });
});
