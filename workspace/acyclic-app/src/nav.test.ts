import { describe, expect, it } from "vitest";
import {
  addressOf,
  depth,
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
  placesByDepth,
  START,
  type Step,
  samePlace,
  stored,
  threadIntent,
  topLayer,
  type Wanted,
  type World,
} from "./nav";

// Items a, b in thread t; c in none; d not drawn.
const world: World = {
  has: (id) => id !== "d",
  threadOf: (id) => (id === "a" || id === "b" ? "t" : null),
  initiativeOf: () => null,
  initiativesOf: () => [],
  contextOf: (chosen) => chosen,
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

describe("follow", () => {
  const phone = { ...world, wide: false };
  const layers = (place: Place) => depth(openOf(place, phone));

  it("preserves a home item origin through Search selection and Inspector follow", () => {
    const homeItem = go(overview, { kind: "select", id: "a", pin: null }).place;
    const followed = go(homeItem, { kind: "follow", id: "b", pin: null });
    const searched = go(homeItem, { kind: "select", id: "b", pin: null });
    expect(followed.place.cameFrom).toEqual({ view: "overview", by: "item" });
    expect(searched.place.cameFrom).toEqual({ view: "overview", by: "item" });
    expect([layers(followed.place), layers(searched.place)]).toEqual([1, 1]);
    expect(followed.requests).toEqual({ focus: { id: "b", pin: null } });
    expect(go(homeItem, { kind: "follow", id: "b", pin: null }, { dirty: true })).toEqual({
      place: homeItem,
      requests: { unsaved: true },
    });
  });

  it("keeps a picked thread's under-item origin until following outside the thread", () => {
    const thread = go(overview, { kind: "thread", id: "t" }).place;
    const selected = go(thread, { kind: "select", id: "a", pin: null }).place;
    const sameThread = go(selected, { kind: "follow", id: "b", pin: null }).place;
    expect(sameThread.cameFrom).toEqual({ view: "overview", by: "thread" });
    expect(sameThread.thread).toBe("t");

    const under = go(go(overview, { kind: "select", id: "a", pin: null }).place, {
      kind: "itemThread",
      id: "t",
    }).place;
    expect(go(under, { kind: "follow", id: "b", pin: null }).place.cameFrom).toEqual({
      view: "overview",
      by: "thread",
      item: "a",
    });
    const outside = go(under, { kind: "follow", id: "c", pin: null }).place;
    expect(outside.thread).toBeNull();
    expect(outside.cameFrom).toEqual({ view: "overview", by: "item" });
    expect(outside.cameFrom).not.toHaveProperty("item");
    expect(go(outside, { kind: "close", count: 1 }).place).toEqual(overview);
  });

  it("keeps explicit Map navigation as its own layer while following items", () => {
    const map = go(overview, { kind: "goTo", view: "map", at: null }).place;
    const first = go(map, { kind: "follow", id: "a", pin: null }).place;
    const second = go(first, { kind: "follow", id: "b", pin: null }).place;
    expect(first.cameFrom).toBeNull();
    expect(second.cameFrom).toBeNull();
    expect([layers(first), layers(second)]).toEqual([2, 2]);
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
    // Gone since, it is no layer.
    expect(openLayers(openOf({ ...step.place, cameFrom: { view: "map", by: "thread", item: "d" } }, world))).toEqual([
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

  it("clears an item origin when the user explicitly returns to the Map", () => {
    const opened = go(overview, { kind: "select", id: "a", pin: null }).place;
    expect(opened.cameFrom).toEqual({ view: "overview", by: "item" });
    const map = go(opened, { kind: "goTo", view: "map", at: null }).place;
    expect(map.cameFrom).toBeNull();
    expect(go(map, { kind: "close", count: 1 }).place).toEqual(at({ view: "map" }));
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

describe("history places by depth", () => {
  const phone = { ...world, wide: false };
  const left = { top: 40, entry: "need:one", control: "#need\\:one" };
  const wanted = (place: Place): Wanted => ({
    view: place.view,
    item: place.item,
    thread: place.thread,
    needsAt: place.needsAt,
    initiative: place.initiative,
  });

  it("keeps Needs you restoration data in each projected entry", () => {
    const needs = at({ view: "needs", needsAt: "need:one", needsLeft: left });
    const opened = navigate(needs, { kind: "select", id: "a", pin: null }, phone).place;
    const places = placesByDepth(opened, phone);
    expect(places).toHaveLength(3);
    expect(places.map((place) => depth(openOf(place, phone)))).toEqual([0, 1, 2]);
    expect(places[0]).toMatchObject({ view: "overview", item: null, needsLeft: left });
    expect(places[1]).toMatchObject({ view: "needs", item: null, needsAt: "need:one", needsLeft: left });
    expect(places[2]).toEqual(stored(opened));
    expect(places.map((place) => place.needsLeft)).toEqual([left, left, left]);
    expect(places.map((place) => addressOf(wanted(place)))).toEqual(["/", "/?view=needs&needs=need%3Aone", "/?item=a"]);
  });

  it("projects each open depth to its own stored place and address", () => {
    const open = at({
      view: "map",
      item: "a",
      thread: "t",
      cameFrom: { view: "overview", by: "thread", item: "a" },
      needsLeft: left,
    });
    const places = placesByDepth(open, phone);
    expect(places).toHaveLength(4);
    expect(places.map((place) => depth(openOf(place, phone)))).toEqual([0, 1, 2, 3]);
    expect(places.every((place) => !place.overlays.command && place.needsLeft === left)).toBe(true);
    expect(new Set(places.map((place) => addressOf(wanted(place)))).size).toBe(places.length);
    expect(places.map((place) => addressOf(wanted(place)))).toEqual([
      "/",
      "/?item=a",
      "/?view=map&thread=t",
      "/?item=a&thread=t",
    ]);

    const withCommand = placesByDepth({ ...open, overlays: { ...START.overlays, command: true } }, phone);
    expect(withCommand.map((place) => depth(openOf(place, phone)))).toEqual([0, 1, 2, 3, 3]);
    expect(samePlace(withCommand[3] ?? null, withCommand[4] ?? null)).toBe(true);
    expect(addressOf(wanted(withCommand[3] ?? START))).toBe(addressOf(wanted(withCommand[4] ?? START)));
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

describe("history places", () => {
  const needsLeft = { top: 42.5, entry: "need:one", control: "#need\\:one" };
  const cameFrom = { view: "map" as const, by: "thread" as const, item: "a" };
  const persistent = at({
    view: "needs",
    item: "a",
    thread: "t",
    needsAt: "need:one",
    cameFrom,
    needsLeft,
  });

  it("compares persistent place fields independent of key order and ignores transient overlays", () => {
    const reordered: Place = {
      overlays: { ...START.overlays, command: true, projects: true },
      needsLeft: { control: needsLeft.control, entry: needsLeft.entry, top: needsLeft.top },
      cameFrom: { item: "a", by: "thread", view: "map" },
      needsAt: "need:one",
      initiative: null,
      thread: "t",
      item: "a",
      view: "needs",
    };
    expect(samePlace(persistent, reordered)).toBe(true);

    const changed: Place[] = [
      { ...persistent, view: "threads" },
      { ...persistent, item: "b" },
      { ...persistent, thread: "other" },
      { ...persistent, needsAt: "other" },
      { ...persistent, initiative: "init" },
      { ...persistent, cameFrom: { ...cameFrom, view: "overview" } },
      { ...persistent, cameFrom: { ...cameFrom, by: "item" } },
      { ...persistent, cameFrom: { ...cameFrom, item: "b" } },
      { ...persistent, needsLeft: { ...needsLeft, top: 43 } },
      { ...persistent, needsLeft: { ...needsLeft, entry: "need:two" } },
      { ...persistent, needsLeft: { ...needsLeft, control: "button.close" } },
    ];
    for (const place of changed) expect(samePlace(persistent, place)).toBe(false);
  });

  it("rejects malformed restored places and nested history fields", () => {
    const base = stored(at({ view: "map" }));
    const parsePlace = (place: unknown) => placeIn({ layers: 1, place });
    for (const state of [null, 1, "place", [], { layers: 1 }, { layers: 1, place: [] }])
      expect(placeIn(state)).toBeNull();
    for (const place of [
      { ...base, view: "elsewhere" },
      { ...base, item: [] },
      { ...base, needsAt: 17 },
      { ...base, cameFrom: [] },
      { ...base, cameFrom: { view: "elsewhere", by: "item" } },
      { ...base, cameFrom: { view: "map", by: "unknown" } },
      { ...base, cameFrom: { view: "map", by: "item", item: "a" } },
      { ...base, cameFrom: { view: "map", by: "thread", item: 1 } },
      { ...base, cameFrom: { view: "map", by: "thread", item: "" } },
      { ...base, needsLeft: [] },
      { ...base, needsLeft: { top: Number.NaN, entry: "e", control: "#e" } },
      { ...base, needsLeft: { top: Number.POSITIVE_INFINITY, entry: "e", control: "#e" } },
      { ...base, needsLeft: { top: -1, entry: "e", control: "#e" } },
      { ...base, needsLeft: { top: 1, entry: 2, control: "#e" } },
      { ...base, needsLeft: { top: 1, entry: "e", control: 2 } },
      { ...base, needsLeft: { top: 1, entry: "e", control: "" } },
    ])
      expect(parsePlace(place)).toBeNull();
  });

  it("preserves a valid escaped Needs you selector from a restored entry", () => {
    const needs = { ...needsLeft, control: "#need\\:one" };
    const place = placeIn({ layers: 1, place: { ...stored(at({ view: "needs" })), needsLeft: needs } });
    expect(place?.needsLeft).toEqual(needs);
  });
});

describe("phone history depths", () => {
  const phone = { ...world, wide: false };
  const layers = (place: Place) => depth(openOf(place, phone));

  it("uses one home-return entry for Overview selections, but keeps explicit Map and direct-link origins", () => {
    const overview = at({ view: "overview" });
    const itemOne = navigate(overview, { kind: "select", id: "a", pin: null }, phone).place;
    const itemTwo = navigate(overview, { kind: "select", id: "b", pin: null }, phone).place;
    expect([layers(itemOne), layers(itemTwo)]).toEqual([1, 1]);

    const thread = navigate(overview, { kind: "thread", id: "t" }, phone).place;
    const threadItemOne = navigate(thread, { kind: "select", id: "a", pin: null }, phone).place;
    const threadItemTwo = navigate(threadItemOne, { kind: "select", id: "b", pin: null }, phone).place;
    expect([layers(thread), layers(threadItemOne), layers(threadItemTwo)]).toEqual([1, 2, 2]);

    const map = navigate(overview, { kind: "goTo", view: "map", at: null }, phone).place;
    const mapItemOne = navigate(map, { kind: "select", id: "a", pin: null }, phone).place;
    const mapItemTwo = navigate(mapItemOne, { kind: "select", id: "b", pin: null }, phone).place;
    expect([layers(map), layers(mapItemOne), layers(mapItemTwo)]).toEqual([1, 2, 2]);

    const direct = navigate(START, { kind: "link", wanted: { ...NOTHING, view: "map", item: "a" } }, phone).place;
    expect(direct.cameFrom).toBeNull();
    expect(openOf(direct, phone).away).toBe(true);
    expect(layers(direct)).toBe(2);
  });
});

describe("landing", () => {
  const plan = {
    item: (id: string) => id === "here",
    thread: (id: string) => id === "t-1",
  };

  it("opens what the plan holds as named", () => {
    const wanted = { ...NOTHING, view: "map" as const, item: "here", thread: "t-1" };
    expect(landing(wanted, plan)).toEqual({ wanted, missing: [] });
  });

  it("drops an item or thread the plan no longer holds, and says which, keeping the rest", () => {
    expect(landing({ ...NOTHING, view: "needs", item: "gone" }, plan)).toEqual({
      wanted: { ...NOTHING, view: "needs" },
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

  it("lands a place an entry held as a link does: what is gone since dropped and named", () => {
    const has = {
      item: (id: string) => id === "c",
      thread: () => false,
    };
    const held = at({ view: "map", item: "gone", thread: "t", cameFrom: fromOverview });
    expect(landingPlace(held, has)).toEqual({
      place: { ...held, item: null, thread: null },
      missing: ["item", "thread"],
    });
    expect(landingPlace({ ...held, item: "c", thread: null }, has)).toMatchObject({ missing: [] });
  });
});

describe("the initiative in context", () => {
  it("is chosen by the switcher and kept through every other move that stays inside it", () => {
    const chosen = go(onMap, { kind: "initiative", id: "i2" });
    expect(chosen).toEqual({ place: at({ view: "map", initiative: "i2" }), requests: {} });
    for (const intent of [
      { kind: "goTo", view: "needs", at: null },
      { kind: "select", id: "c", pin: null },
      { kind: "thread", id: "t" },
      { kind: "add", parent: null },
    ] satisfies Intent[])
      expect(go(chosen.place, intent).place.initiative).toBe("i2");
    expect(go(chosen.place, { kind: "initiative", id: null }).place.initiative).toBeNull();
  });

  // Items a and b are under i1, item e and thread u under i2, thread t under both; the first initiative is i1.
  const homes: Partial<World> = {
    initiativeOf: (id) => ({ a: "i1", b: "i1", c: "i1", e: "i2" })[id] ?? null,
    initiativesOf: (thread) => (thread === "t" ? ["i1", "i2"] : thread === "u" ? ["i2"] : []),
    contextOf: (chosen) => chosen ?? "i1",
  };

  it("follows an item opened from another initiative, from every way of opening one", () => {
    const here = at({ view: "map", initiative: "i1" });
    const lands = (intent: Intent, from: Place = here) => go(from, intent, homes).place.initiative;
    expect(lands({ kind: "select", id: "e", pin: null })).toBe("i2");
    expect(lands({ kind: "follow", id: "e", pin: null })).toBe("i2");
    expect(lands({ kind: "link", wanted: parse("?item=e") })).toBe("i2");
    expect(lands({ kind: "restore", place: at({ view: "map", item: "e" }) })).toBe("i2");
    expect(lands({ kind: "wrote", select: "e", pin: null })).toBe("i2");
    // Nothing chosen yet: the first initiative is in context, so an item of another one still switches.
    expect(lands({ kind: "select", id: "e", pin: null }, overview)).toBe("i2");
  });

  it("opens what a stub names in its own initiative: its item from the Map, its thread from the Threads view", () => {
    const map = at({ view: "map", initiative: "i1", item: "a" });
    const opened = go(map, { kind: "follow", id: "e", pin: "a" }, homes);
    expect(opened.place).toMatchObject({ view: "map", initiative: "i2", item: "e" });
    const threads = at({ view: "threads", initiative: "i1" });
    const there = go(threads, { kind: "thread", id: "u" }, homes);
    expect(there.place).toMatchObject({ view: "map", initiative: "i2", thread: "u" });
    expect(there.requests.frame).toEqual({ thread: "u" });
    // With edits unsaved, the stub asks first, as every way of opening an item does.
    expect(go(map, { kind: "follow", id: "e", pin: "a" }, { ...homes, dirty: true }).requests.unsaved).toBe(true);
  });

  it("stays put moving between items of one initiative, or to one with none known", () => {
    const here = at({ view: "map", initiative: "i1", item: "a" });
    expect(go(here, { kind: "select", id: "b", pin: null }, homes).place.initiative).toBe("i1");
    expect(go(here, { kind: "tap", id: "c" }, homes).place.initiative).toBe("i1");
    // The first initiative is in context when none was chosen, and the item is in it: nothing to record.
    expect(go(overview, { kind: "select", id: "a", pin: null }, homes).place.initiative).toBeNull();
    expect(go(here, { kind: "select", id: "d", pin: null }, homes).place.initiative).toBe("i1");
  });

  it("follows a thread with no task in the initiative in context, and not one with a task there", () => {
    const here = at({ view: "map", initiative: "i1" });
    expect(go(here, { kind: "thread", id: "u" }, homes).place.initiative).toBe("i2");
    expect(go(here, { kind: "thread", id: "t" }, homes).place.initiative).toBe("i1");
  });

  it("closes what is open that isn't in the initiative chosen, and keeps what is", () => {
    const open = at({ view: "map", initiative: "i1", item: "a", thread: "t" });
    expect(go(open, { kind: "initiative", id: "i2" }, homes).place).toMatchObject({
      initiative: "i2",
      item: null,
      thread: "t",
    });
    expect(go(open, { kind: "initiative", id: "i1" }, homes).place).toMatchObject({ item: "a", thread: "t" });
    expect(
      go(at({ initiative: "i1", item: "e", thread: "t" }), { kind: "initiative", id: "i1" }, homes).place,
    ).toMatchObject({
      item: null,
      thread: "t",
    });
  });

  it("leaves a form holding typing alone: choosing another initiative that would close the item says so instead", () => {
    const open = at({ view: "map", initiative: "i1", item: "a" });
    expect(go(open, { kind: "initiative", id: "i2" }, { ...homes, dirty: true })).toEqual({
      place: open,
      requests: { unsaved: true },
    });
    // Nothing closes, so the form stays and the choice goes through.
    expect(go(open, { kind: "initiative", id: "i1" }, { ...homes, dirty: true }).place.initiative).toBe("i1");
    expect(
      go(at({ initiative: "i1" }), { kind: "initiative", id: "i2" }, { ...homes, dirty: true }).place.initiative,
    ).toBe("i2");
  });

  it("survives a reload: it is in the address, read back, and an address without it leaves the one in context", () => {
    expect(addressOf({ ...NOTHING, initiative: "i2" })).toBe("/?initiative=i2");
    expect(addressOf({ ...NOTHING, view: "needs", initiative: "i2" }, "?engine=inline")).toBe(
      "/?engine=inline&view=needs&initiative=i2",
    );
    expect(parse("?initiative=i2")).toEqual({ ...NOTHING, initiative: "i2" });
    const reloaded = go(START, { kind: "link", wanted: parse("?view=threads&initiative=i2") });
    expect(reloaded.place).toMatchObject({ view: "threads", initiative: "i2" });
    // A link that names none, a notification's, leaves it be; so does an entry written before one was chosen.
    const here = at({ initiative: "i2" });
    expect(go(here, { kind: "link", wanted: parse("?item=a") }).place.initiative).toBe("i2");
    expect(go(here, { kind: "restore", place: START }).place.initiative).toBe("i2");
    expect(go(here, { kind: "restore", place: at({ initiative: "i1" }) }).place.initiative).toBe("i1");
  });

  it("is held in a history entry, and one holding none reads as none", () => {
    expect(placeIn({ layers: 0, place: stored(at({ initiative: "i2" })) })?.initiative).toBe("i2");
    expect(placeIn({ layers: 1, place: { view: "map", item: null, thread: null } })?.initiative).toBeNull();
    expect(placeIn({ layers: 1, place: { view: "map", item: null, thread: null, initiative: 3 } })).toBeNull();
  });
});

describe("threadIntent", () => {
  const tasks = new Map([["a.one", "a"]]);
  it("opens the task a thread of one stands for, from any row that names the thread", () => {
    expect(threadIntent(tasks, "a.one")).toEqual({ kind: "select", id: "a", pin: null });
    expect(threadIntent(tasks, "a.one", "outline")).toEqual({ kind: "select", id: "a", pin: null });
  });

  it("picks any other thread as the row asks", () => {
    expect(threadIntent(tasks, "t")).toEqual({ kind: "thread", id: "t" });
    expect(threadIntent(tasks, "t", "outline")).toEqual({ kind: "outline", id: "t" });
  });
});

describe("saved deletion over a stale plan", () => {
  it("clears a deleted task, its derived thread and the underlying item reference", () => {
    const before = at({
      view: "map",
      item: "a",
      thread: "a.one",
      cameFrom: { view: "threads", by: "thread", item: "a" },
    });
    const result = go(before, { kind: "deleted", record: "item", id: "a" }).place;
    expect(result).toMatchObject({ item: null, thread: null, cameFrom: null });
    expect(addressOf(result)).not.toContain("item=");
    expect(addressOf(result)).not.toContain("thread=");
  });
  it("drops a deleted thread while preserving an unrelated item", () => {
    const result = go(at({ view: "map", item: "b", thread: "t" }), {
      kind: "deleted",
      record: "thread",
      id: "t",
    }).place;
    expect(result).toMatchObject({ item: "b", thread: null });
  });
  it("drops the initiative address even when the stale context still names it", () => {
    const result = go(at({ initiative: "init" }), { kind: "deleted", record: "initiative", id: "init" }).place;
    expect(result.initiative).toBeNull();
    expect(addressOf(result)).not.toContain("initiative=");
  });
});

it("keeps the current initiative when an inactive initiative is deleted", () => {
  const result = go(at({ initiative: "other" }), { kind: "deleted", record: "initiative", id: "empty" }).place;
  expect(result.initiative).toBe("other");
});
