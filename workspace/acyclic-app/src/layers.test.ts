import { describe, expect, it } from "vitest";
import { arrived, depth, follow, openLayers, restored, topLayer } from "./layers";

const none = {
  command: false,
  mode: false,
  adding: false,
  item: false,
  projects: false,
  thread: false,
  away: false,
};

describe("topLayer", () => {
  it("puts the command menu over all, a waiting move over a form, a form over the projects, those over an item, and that over a thread", () => {
    const all = { command: false, mode: true, adding: true, item: true, projects: true, thread: true, away: true };
    expect(topLayer({ ...all, command: true })).toBe("command");
    expect(topLayer(all)).toBe("mode");
    expect(topLayer({ ...all, mode: false })).toBe("add");
    expect(topLayer({ ...all, mode: false, adding: false })).toBe("projects");
    expect(topLayer({ ...all, mode: false, adding: false, projects: false })).toBe("item");
    expect(topLayer({ ...none, thread: true })).toBe("thread");
    expect(topLayer(none)).toBeNull();
    expect(openLayers({ ...none, thread: true, item: true })).toEqual(["item", "thread"]);
    // Away from home, on a phone, is under everything: Back closes what's over the view before it goes home, Needs
    // you's view as any other.
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

  it("closes the layer on top on Back, and ignores a move that lands on as many layers", () => {
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

  it("opens again on Forward what the entry's address names, then says how many layers that was, never moving", () => {
    const forward = arrived(at(1), 2, 1);
    expect(forward).toEqual({ entries: { pushed: 2, waiting: false, restoring: true }, action: { kind: "restore" } });
    // Over two entries at once, as a long press on Forward makes.
    expect(arrived(at(0), 2, 0).entries).toEqual({ pushed: 2, waiting: false, restoring: true });
    // All of it opened again: nothing to do.
    expect(follow(forward.entries, 2)).toEqual({ entries: at(2), action: null });
    // Fewer, as the command menu has no address, or a link's item is gone: the entry says so, and Back goes on from it.
    expect(follow(forward.entries, 1)).toEqual({ entries: at(1), action: { kind: "relabel", layers: 1 } });
    expect(follow(forward.entries, 0)).toEqual({ entries: at(0), action: { kind: "relabel", layers: 0 } });
    // More, as a thread named with an item opens a layer each: the one more takes its own entry.
    expect(follow(forward.entries, 3)).toEqual({ entries: at(3), action: { kind: "push", from: 2, to: 3 } });
    // Held behind a form holding typing: the entry stands for what is open meanwhile, so no later move goes back.
    const held = restored(forward.entries, 1);
    expect(held).toEqual({ entries: at(1), action: { kind: "relabel", layers: 1 } });
    expect(follow(held.entries, 2)).toEqual({ entries: at(2), action: { kind: "push", from: 1, to: 2 } });
  });
});
