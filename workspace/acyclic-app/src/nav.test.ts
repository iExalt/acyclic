import { describe, expect, it } from "vitest";
import {
  addressOf,
  arrived,
  depth,
  type Found,
  follow,
  type Intent,
  landing,
  landingPlace,
  NOTHING,
  navigate,
  openLayers,
  openOf,
  type Place,
  parse,
  placeIn,
  restored,
  START,
  type Step,
  stored,
  topLayer,
  type Wanted,
  type World,
} from "./nav";

// Items a, b in thread t; c in none; d not drawn; e archived, hidden.
const world: World = {
  has: (id) => id !== "d",
  archived: (id) => id === "e",
  threadOf: (id) => (id === "a" || id === "b" ? "t" : null),
  wide: true,
  dirty: false,
};
const go = (place: Place, intent: Intent, over: Partial<World> = {}) => navigate(place, intent, { ...world, ...over });
const at = (change: Partial<Place>): Place => ({ ...START, ...change });
const overview = at({ view: "overview" });
const onMap = at({ view: "map" });

describe("select", () => {
  it("goes to the Map for an item chosen on another view, brought in, so closing it comes back", () => {
    const step = go(overview, { kind: "select", id: "c", pin: "p" });
    expect(step.place).toEqual(at({ view: "map", item: "c", cameFrom: { view: "overview", by: "item" } }));
    expect(step.requests).toEqual({ focus: { id: "c", pin: "p" } });
    // Each ask a fresh object, so the same item asked twice is brought in twice.
    expect(go(overview, { kind: "select", id: "c", pin: "p" }).requests.focus).not.toBe(step.requests.focus);
  });

  it("keeps the thread picked for an item in it, keeping where the thread came from, and unpicks it for any other", () => {
    const fromThreads = at({ view: "map", thread: "t", cameFrom: { view: "threads", by: "thread" } });
    expect(go(fromThreads, { kind: "select", id: "a", pin: null }).place).toMatchObject({
      thread: "t",
      cameFrom: { view: "threads", by: "thread" },
    });
    expect(go(fromThreads, { kind: "select", id: "c", pin: null }).place.thread).toBeNull();
    expect(go(onMap, { kind: "select", id: "a", pin: null }).place.thread).toBeNull();
  });
});

describe("tap", () => {
  it("makes the Map where the user is: the background keeps the thread, an item outside it unpicks it", () => {
    const opened = at({ view: "map", item: "a", thread: "t", cameFrom: { view: "threads", by: "thread" } });
    expect(go(opened, { kind: "tap", id: null }).place).toEqual(at({ view: "map", thread: "t" }));
    expect(go(opened, { kind: "tap", id: "c" }).place).toEqual(at({ view: "map", item: "c" }));
    expect(go(opened, { kind: "tap", id: "b" }).requests).toEqual({});
  });
});

describe("gone", () => {
  it("drops the selection, and where it came from once nothing is open", () => {
    const opened = at({ view: "map", item: "c", cameFrom: { view: "overview", by: "item" } });
    expect(go(opened, { kind: "gone" }).place).toEqual(onMap);
  });
});

describe("thread", () => {
  it("goes to the Map for a thread, picked and framed, closing the item; from the Map it came from nowhere else", () => {
    const step = go(at({ view: "threads", item: "c" }), { kind: "thread", id: "t" });
    expect(step.place).toEqual(at({ view: "map", thread: "t", cameFrom: { view: "threads", by: "thread" } }));
    expect(step.requests).toEqual({ frame: { thread: "t" } });
    expect(go(onMap, { kind: "thread", id: "t" }).place.cameFrom).toBeNull();
  });
});

describe("outline", () => {
  it("unpicks the thread picked on the Map with nothing over it, and otherwise goes to it, framed", () => {
    expect(go(at({ view: "map", thread: "t" }), { kind: "outline", id: "t" })).toEqual({
      place: onMap,
      requests: {},
    });
    const withItem = go(at({ view: "map", thread: "t", item: "a" }), { kind: "outline", id: "t" });
    expect(withItem).toEqual({ place: at({ view: "map", thread: "t" }), requests: { frame: { thread: "t" } } });
  });
});

describe("itemThread", () => {
  it("goes to the thread framed, the item under its panel, keeping where the item came from", () => {
    const opened = at({ view: "map", item: "a", cameFrom: { view: "overview", by: "item" } });
    const step = go(opened, { kind: "itemThread", id: "t" });
    expect(step).toEqual({
      place: { ...opened, item: null, thread: "t", cameFrom: { view: "overview", by: "thread", item: "a" } },
      requests: { frame: { thread: "t" } },
    });
    // A layer of its own under the thread's, so Back closes the thread before the item.
    expect(openLayers(openOf(step.place, world))).toEqual(["thread", "under"]);
    // Gone or hidden since, it is no layer.
    expect(openLayers(openOf({ ...step.place, cameFrom: { view: "map", by: "thread", item: "e" } }, world))).toEqual([
      "thread",
    ]);
  });
});

describe("goTo", () => {
  const open = at({ view: "map", item: "a", thread: "t", needsLeft: { top: 1, entry: "e", control: "c" } });

  it("leaves the item off the Map, and the thread too for a list, keeping it picked on the Threads view", () => {
    expect(go(open, { kind: "goTo", view: "threads", at: null }).place).toMatchObject({ item: null, thread: "t" });
    expect(go(open, { kind: "goTo", view: "overview", at: null }).place).toMatchObject({ item: null, thread: null });
    expect(go(open, { kind: "goTo", view: "map", at: null }).place).toEqual(open);
  });

  it("opens Needs you from its top or at the entry asked for, never where it was left", () => {
    expect(go(open, { kind: "goTo", view: "needs", at: "e2" })).toEqual({
      place: at({ view: "needs", needsAt: "e2" }),
      requests: {},
    });
  });
});

describe("close", () => {
  it("closes layers from the top, an item and a thread going back to where each was opened from", () => {
    const opened = at({ view: "map", item: "a", thread: "t", cameFrom: { view: "threads", by: "thread" } });
    const one = go({ ...opened, overlays: { ...opened.overlays, command: true } }, { kind: "close", count: 2 });
    expect(one.place).toEqual({ ...opened, item: null });
    expect(go(one.place, { kind: "close", count: 1 }).place).toEqual(at({ view: "threads" }));
  });

  it("goes home last on a phone, and closes nothing but the command menu over a form holding typing", () => {
    expect(go(at({ view: "threads" }), { kind: "close", count: 1 }, { wide: false }).place).toEqual(START);
    const form = at({ overlays: { ...START.overlays, command: true, add: { parent: null, draft: null } } });
    expect(go(form, { kind: "close", count: 2 }, { dirty: true })).toEqual({
      place: form,
      requests: { unsaved: true },
    });
    expect(go(form, { kind: "close", count: 1 }, { dirty: true }).place.overlays.command).toBe(false);
  });
});

describe("shut", () => {
  it("closes one layer by its own control, a form holding typing or not", () => {
    const opened = at({ view: "map", item: "c", cameFrom: { view: "needs", by: "item" } });
    expect(go(opened, { kind: "shut", layer: "item" }, { dirty: true }).place).toEqual(at({ view: "needs" }));
  });
});

describe("the overlays", () => {
  it("open over the view: a move or link waiting, the command menu, the projects' sheet, and the add form", () => {
    const moving = go(START, { kind: "mode", mode: { kind: "move", item: "a" } }).place;
    const menu = go(moving, { kind: "command", open: true }).place;
    expect(menu.overlays).toMatchObject({ command: true, mode: { kind: "move", item: "a" } });
    // The add form drops a move waiting for its target.
    expect(go(moving, { kind: "add", parent: "a" }).place.overlays).toMatchObject({
      mode: { kind: "idle" },
      add: { parent: "a", draft: null },
    });
    const draft = { id: "n", title: "N", parent: null, thread: null };
    expect(go(START, { kind: "redraft", adding: { parent: null, draft } }).place.overlays.add).toEqual({
      parent: null,
      draft,
    });
    expect(go(START, { kind: "projects", open: true }).place.overlays.projects).toBe(true);
  });

  it("stack as layers: the projects' sheet and a view away from home only on a phone, an item only once drawn", () => {
    const place = at({ view: "map", item: "d", thread: "t", overlays: { ...START.overlays, projects: true } });
    expect(openLayers(openOf(place, world))).toEqual(["thread"]);
    expect(openLayers(openOf({ ...place, item: "a" }, { ...world, wide: false }))).toEqual([
      "projects",
      "item",
      "thread",
      "away",
    ]);
  });
});

describe("edit and reopen", () => {
  it("ask the item's panel for its editor, each ask a fresh one, an edit only with an item open", () => {
    const opened = at({ view: "map", item: "a" });
    const asked = go(opened, { kind: "edit" });
    expect(asked).toEqual({ place: opened, requests: { edit: { item: "a" } } });
    expect(go(opened, { kind: "edit" }).requests.edit).not.toBe(asked.requests.edit);
    expect(go(START, { kind: "edit" }).requests).toEqual({});
    const edited = { base: {}, draft: {} } as never;
    expect(go(opened, { kind: "reopen", id: "a", edited }).requests).toEqual({ reopenEditor: { id: "a", edited } });
  });
});

describe("wrote", () => {
  it("closes the add form, restores a written selection's frame, and leaves the thread", () => {
    const adding = at({ view: "map", thread: "t", overlays: { ...START.overlays, add: { parent: "c", draft: null } } });
    expect(go(adding, { kind: "wrote", select: "n", pin: "c" })).toEqual({
      place: at({ view: "map", thread: "t", item: "n" }),
      requests: { focus: { id: "n", pin: "c", preserveFrame: true } },
    });
    expect(go(adding, { kind: "wrote", select: "n", pin: null }).requests).toEqual({
      focus: { id: "n", pin: null, preserveFrame: true },
    });
    expect(go(adding, { kind: "wrote", select: null, pin: null })).toEqual({
      place: at({ view: "map", thread: "t" }),
      requests: {},
    });
  });
});

describe("added", () => {
  it("goes to the Map for an item added on another view while the user is still there", () => {
    expect(go(at({ view: "threads", item: "n" }), { kind: "added", id: "n", from: "threads" })).toEqual({
      place: at({ view: "map", item: "n", cameFrom: { view: "threads", by: "item" } }),
      requests: { focus: { id: "n", pin: null } },
    });
    const movedOn = at({ view: "overview", item: "n" });
    expect(go(movedOn, { kind: "added", id: "n", from: "threads" })).toEqual({ place: movedOn, requests: {} });
    expect(go(onMap, { kind: "added", id: "n", from: "map" })).toEqual({ place: onMap, requests: {} });
  });
});

describe("leaveNeeds", () => {
  it("keeps where Needs you was left, given back once as the user comes back to it", () => {
    const left = { top: 40, entry: "e", control: "button" };
    const away = go(go(at({ view: "needs" }), { kind: "leaveNeeds", left }).place, {
      kind: "select",
      id: "c",
      pin: null,
    });
    expect(away.place.needsLeft).toEqual(left);
    const back = go(away.place, { kind: "shut", layer: "item" });
    expect(back).toEqual({ place: at({ view: "needs" }), requests: { restoreNeeds: left } });
  });
});

describe("link", () => {
  it("opens what it names over nothing else: an item on the Map brought in, a thread there framed", () => {
    const busy = at({ view: "threads", item: "c", cameFrom: null, overlays: { ...START.overlays, command: true } });
    expect(go(busy, { kind: "link", wanted: { ...NOTHING, view: "map", item: "a", thread: "t" } })).toEqual({
      place: at({ view: "map", item: "a", thread: "t" }),
      requests: { focus: { id: "a", pin: null } },
    });
    expect(go(busy, { kind: "link", wanted: { ...NOTHING, view: "map", thread: "t" } }).requests).toEqual({
      frame: { thread: "t" },
    });
    // Over another view, left open there, as it was when the address was written.
    expect(go(busy, { kind: "link", wanted: { ...NOTHING, view: "needs", item: "a", needsAt: "e" } })).toEqual({
      place: at({ view: "needs", item: "a", needsAt: "e" }),
      requests: {},
    });
  });
});

const none = {
  command: false,
  mode: false,
  adding: false,
  item: false,
  projects: false,
  thread: false,
  under: false,
  away: false,
};

describe("topLayer", () => {
  it("puts the command menu over all, a waiting move over a form, a form over the projects, those over an item, and that over a thread", () => {
    const all = {
      command: false,
      mode: true,
      adding: true,
      item: true,
      projects: true,
      thread: true,
      under: false,
      away: true,
    };
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

describe("the address", () => {
  it("is bare at home with nothing open, and names what is open otherwise, read back as written", () => {
    expect(addressOf(NOTHING)).toBe("/");
    const open: Wanted[] = [
      { ...NOTHING, view: "threads", thread: "t-3" },
      { ...NOTHING, view: "map", item: "a b/c&d" },
      { ...NOTHING, view: "needs" },
      { ...NOTHING, view: "needs", needsAt: "unapproved:t-3:ready" },
      // An item left open over another view, as going there with it open leaves it.
      { ...NOTHING, view: "threads", item: "x" },
      { ...NOTHING, view: "overview", item: "x" },
    ];
    for (const wanted of open) expect(parse(addressOf(wanted).slice(1))).toEqual(wanted);
  });

  it("writes Needs you as a view, the entry it opens at beside it, and an item without the Map it implies", () => {
    expect(addressOf({ ...NOTHING, view: "needs" })).toBe("/?view=needs");
    expect(addressOf({ ...NOTHING, view: "needs", needsAt: "a:b" })).toBe("/?view=needs&needs=a%3Ab");
    // An entry is Needs you's alone: on another view, the address leaves it out.
    expect(addressOf({ ...NOTHING, view: "map", needsAt: "a:b" })).toBe("/?view=map");
    expect(parse("?item=x").view).toBe("map");
  });

  it("opens Needs you's view at an old address, as notifications and bookmarks still send it", () => {
    expect(parse("?needs")).toEqual({ ...NOTHING, view: "needs" });
    expect(parse("?needs=a:b")).toEqual({ ...NOTHING, view: "needs", needsAt: "a:b" });
    // Once opened over an item, the list is now the view, the item left open over it.
    expect(parse("?item=x&needs")).toEqual({ ...NOTHING, view: "needs", item: "x" });
    // A view named wins, and an entry on any other view is dropped.
    expect(parse("?view=threads&needs=a")).toEqual({ ...NOTHING, view: "threads" });
  });

  it("keeps what else the address says, such as a test run's switch, and changes only what is open", () => {
    expect(addressOf({ ...NOTHING, view: "map", item: "y" }, "?engine=inline&item=x&view=threads")).toBe(
      "/?engine=inline&item=y",
    );
    expect(addressOf({ ...NOTHING, view: "threads", item: "y" }, "?engine=inline")).toBe(
      "/?engine=inline&view=threads&item=y",
    );
    expect(addressOf(NOTHING, "?needs&engine=inline")).toBe("/?engine=inline");
  });

  it("opens at home what it doesn't know or finds empty", () => {
    expect(parse("?view=elsewhere&item=&thread=&utm=1")).toEqual(NOTHING);
    expect(parse("")).toEqual(NOTHING);
  });
});

describe("landing", () => {
  const plan = {
    item: (id: string): Found => ({ here: "shown" as const, old: "archived" as const })[id] ?? null,
    thread: (id: string) => id === "t-1",
  };

  it("opens what the plan holds as named, and an archived item with archived items shown", () => {
    const wanted = { ...NOTHING, view: "map" as const, item: "here", thread: "t-1" };
    expect(landing(wanted, plan)).toEqual({ wanted, archived: false, missing: [] });
    expect(landing({ ...wanted, item: "old" }, plan)).toMatchObject({ archived: true, missing: [] });
  });

  it("drops an item or thread the plan no longer holds, and says which, keeping the rest", () => {
    expect(landing({ ...NOTHING, view: "needs", item: "gone" }, plan)).toEqual({
      wanted: { ...NOTHING, view: "needs" },
      archived: false,
      missing: ["item"],
    });
    expect(landing({ ...NOTHING, view: "threads", thread: "t-9" }, plan)).toMatchObject({
      wanted: { ...NOTHING, view: "threads" },
      missing: ["thread"],
    });
    expect(landing({ ...NOTHING, view: "map", item: "gone", thread: "t-9" }, plan).missing).toEqual(["item", "thread"]);
  });
});

describe("Back and Forward", () => {
  type Move = Intent | "back" | "forward";
  /**
   * Walks `moves` from `start` as the history does: Back closes the top layer, leaving the entry it was on, with the
   * place that entry held, for Forward to open again, as cloned as the browser keeps it.
   */
  const walk = (start: Place, moves: Move[], over: Partial<World> = {}) => {
    let step: Step = { place: start, requests: {} };
    const ahead: unknown[] = [];
    for (const move of moves) {
      if (move === "back") {
        ahead.push(structuredClone({ layers: 1, place: stored(step.place) }));
        step = go(step.place, { kind: "close", count: 1 }, over);
      } else if (move === "forward") {
        const place = placeIn(ahead.pop());
        if (place === null) throw new Error("an entry with no place");
        step = go(step.place, { kind: "restore", place }, over);
      } else step = go(step.place, move, over);
    }
    return step;
  };
  const left = { top: 40, entry: "e", control: "button" };
  const fromOverview = { view: "overview" as const, by: "item" as const };
  const cases: [string, Place, Move[], Step, Partial<World>?][] = [
    [
      "an item's thread from its panel, then Back, opens the item again, brought in",
      onMap,
      [{ kind: "select", id: "a", pin: null }, { kind: "itemThread", id: "t" }, "back"],
      { place: at({ view: "map", item: "a" }), requests: { focus: { id: "a", pin: null } } },
    ],
    [
      "then Forward goes to the thread again, framed, the item under it",
      onMap,
      [{ kind: "select", id: "a", pin: null }, { kind: "itemThread", id: "t" }, "back", "forward"],
      {
        place: at({ view: "map", thread: "t", cameFrom: { view: "map", by: "thread", item: "a" } }),
        requests: { frame: { thread: "t" } },
      },
    ],
    [
      "an item's thread, the item hidden since, then Back, goes where the item came from",
      overview,
      [{ kind: "select", id: "e", pin: null }, { kind: "itemThread", id: "t" }, "back"],
      { place: overview, requests: {} },
    ],
    [
      "an item opened on the Overview, then Back, comes back to the Overview",
      overview,
      [{ kind: "select", id: "c", pin: null }, "back"],
      { place: overview, requests: {} },
    ],
    [
      "an item opened in Needs you, then Back, comes back to it scrolled and focused as it was left",
      at({ view: "needs" }),
      [{ kind: "leaveNeeds", left }, { kind: "select", id: "c", pin: null }, "back"],
      { place: at({ view: "needs" }), requests: { restoreNeeds: left } },
    ],
    [
      "Back over a form holding typing closes nothing, and says so",
      at({ view: "map", item: "c", overlays: { ...START.overlays, add: { parent: "c", draft: null } } }),
      ["back"],
      {
        place: at({ view: "map", item: "c", overlays: { ...START.overlays, add: { parent: "c", draft: null } } }),
        requests: { unsaved: true },
      },
      { dirty: true },
    ],
    [
      "a link to an item, then Back, leaves the Map shown",
      START,
      [{ kind: "link", wanted: { ...NOTHING, view: "map", item: "c" } }, "back"],
      { place: onMap, requests: {} },
    ],
    [
      "Forward after Back opens the item again where it came from, so closing it goes back there",
      overview,
      [{ kind: "select", id: "c", pin: null }, "back", "forward"],
      {
        place: at({ view: "map", item: "c", cameFrom: fromOverview }),
        requests: { focus: { id: "c", pin: null } },
      },
    ],
    [
      "Forward never brings back what passes, such as the add form open as the entry was left",
      overview,
      [{ kind: "select", id: "c", pin: null }, { kind: "add", parent: "c" }, "back", "forward"],
      {
        place: at({ view: "map", item: "c", cameFrom: fromOverview }),
        requests: { focus: { id: "c", pin: null } },
      },
    ],
    [
      "on a phone, Back closes the outline's sheet over a thread's panel, then the panel, staying on the Map",
      at({ view: "map", thread: "t", overlays: { ...START.overlays, projects: true } }),
      ["back", "back"],
      { place: onMap, requests: {} },
      { wide: false },
    ],
    [
      "on a phone, Back closes a thread's panel kept over the Threads view, then goes home",
      at({ view: "threads", thread: "t" }),
      ["back", "back"],
      { place: START, requests: {} },
      { wide: false },
    ],
  ];
  it.each(cases)("%s", (_, start, moves, want, over) => {
    expect(walk(start, moves, over)).toEqual(want);
  });

  it("opens from an entry written before entries held places what its address names, and none from another's", () => {
    expect(placeIn({ layers: 1 })).toBeNull();
    expect(placeIn(null)).toBeNull();
    expect(placeIn({ layers: 1, place: { view: "elsewhere", item: null, thread: null } })).toBeNull();
    expect(placeIn({ layers: 1, place: { view: "map", item: "c", thread: null } })).toEqual(
      at({ view: "map", item: "c" }),
    );
  });

  it("lands a place an entry held as a link does: what is gone since dropped and named, an archived item shown", () => {
    const has = {
      item: (id: string): Found => (id === "e" ? "archived" : id === "c" ? "shown" : null),
      thread: () => false,
    };
    const held = at({ view: "map", item: "gone", thread: "t", cameFrom: fromOverview });
    expect(landingPlace(held, has)).toEqual({
      place: { ...held, item: null, thread: null },
      archived: false,
      missing: ["item", "thread"],
    });
    expect(landingPlace({ ...held, item: "e", thread: null }, has)).toMatchObject({ archived: true, missing: [] });
  });
});
