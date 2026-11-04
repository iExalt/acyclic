import { describe, expect, it } from "vitest";
import {
  boundary,
  type Cursor,
  close,
  type HistoryPlan,
  plan,
  pushDepths,
  readHistory,
  replaceDepth,
  visit,
} from "./history";
import { openLayers, openOf, type Place, START, stored } from "./nav";

const places: Place[] = [
  stored(START),
  stored({ ...START, view: "map" }),
  stored({ ...START, view: "map", thread: "thread-a" }),
  stored({ ...START, view: "map", thread: "thread-a", item: "item-a" }),
  stored({ ...START, view: "map", thread: "thread-a", item: "item-b" }),
  stored({ ...START, view: "map", thread: "thread-a", item: "item-c" }),
  stored({ ...START, view: "map", thread: "thread-b" }),
];

function placeAt(depth: number): Place {
  const place = places[depth];
  if (place === undefined) throw new Error(`missing fixture place at depth ${depth}`);
  return place;
}

function apply(entries: { cursor: Cursor; place: Place }[], at: number, historyPlan: HistoryPlan) {
  for (const effect of historyPlan.effects) {
    if (effect.kind === "go") at += effect.offset;
    else if (effect.kind === "replace") entries[at] = { cursor: effect.cursor, place: effect.place };
    else {
      entries.splice(at + 1);
      entries.push({ cursor: effect.cursor, place: effect.place });
      at++;
    }
  }
  return at;
}

describe("history cursor", () => {
  it("records each opened depth and branches from the current position after Back", () => {
    const root = boundary(0);
    expect(root).toEqual({ position: 0, layers: 0, parents: [] });
    if (root === null) return;
    const [map, thread, item] = pushDepths(root, 3) ?? [];
    expect([map, thread, item]).toEqual([
      { position: 1, layers: 1, parents: [0] },
      { position: 2, layers: 2, parents: [0, 1] },
      { position: 3, layers: 3, parents: [0, 1, 2] },
    ]);
    if (item === undefined) return;
    const b = visit(item);
    const c = b === null ? null : visit(b);
    expect(b).toEqual({ position: 4, layers: 3, parents: [0, 1, 2] });
    expect(c).toEqual({ position: 5, layers: 3, parents: [0, 1, 2] });
    expect(c === null ? null : close(c, 2, 6)).toEqual({ kind: "go", offset: -3 });
    expect(b === null ? null : visit(b)).toEqual({ position: 5, layers: 3, parents: [0, 1, 2] });
    expect(close(item, 3, 6)).toEqual({ kind: "replace", cursor: item });
    expect(close(item, 4, 6)).toBeNull();
  });

  it("round-trips validated metadata and canonicalizes the stored place", () => {
    const raw = { position: 4, layers: 3, parents: [0, 1, 2], place: stored(START) };
    expect(readHistory(raw)).toEqual({ cursor: { position: 4, layers: 3, parents: [0, 1, 2] }, place: stored(START) });
    raw.parents[0] = 99;
    expect(readHistory({ position: 4, layers: 3, parents: [0, 1, 2] })).toEqual({
      cursor: { position: 4, layers: 3, parents: [0, 1, 2] },
      place: null,
    });
    expect(readHistory({ position: 4, layers: 3, parents: [0, 1, 2], place: { view: "invalid" } })?.place).toBeNull();
  });

  it("keeps legacy ancestry unknown and closes below it by replacement", () => {
    const legacy = boundary(3);
    expect(legacy).toEqual({ position: 0, layers: 3, parents: [] });
    if (legacy === null) return;
    const [search] = pushDepths(legacy, 4) ?? [];
    expect(search).toEqual({ position: 1, layers: 4, parents: [0] });
    expect(search === undefined ? null : close(search, 3, 2)).toEqual({ kind: "go", offset: -1 });
    expect(close(legacy, 2, 0)).toEqual({
      kind: "replace",
      cursor: { position: 0, layers: 2, parents: [] },
    });
    expect(replaceDepth({ position: 8, layers: 4, parents: [5] }, 2)).toEqual({
      position: 8,
      layers: 2,
      parents: [],
    });
  });

  it("uses only reachable parents and never waits on an unreachable history entry", () => {
    expect(close({ position: 4, layers: 2, parents: [0] }, 1, 4)).toEqual({
      kind: "replace",
      cursor: { position: 4, layers: 1, parents: [] },
    });
    expect(close({ position: 5, layers: 3, parents: [0, 1, 2] }, 2, 3)).toEqual({
      kind: "replace",
      cursor: { position: 5, layers: 2, parents: [0, 1] },
    });
  });

  it("bounds a Back offset by retained history length, not absolute cursor positions", () => {
    expect(close({ position: 100, layers: 2, parents: [97, 99] }, 1, 10)).toEqual({
      kind: "go",
      offset: -1,
    });
  });

  it("rejects malformed metadata and derives its maximum from the layer model", () => {
    const max = openLayers({
      command: true,
      mode: true,
      adding: true,
      projects: true,
      item: true,
      thread: true,
      under: true,
      away: true,
    }).length;
    expect(boundary(max)).toEqual({ position: 0, layers: max, parents: [] });
    expect(boundary(max + 1)).toBeNull();
    for (const raw of [
      null,
      [],
      { position: 0.5, layers: 0, parents: [] },
      { position: Number.MAX_SAFE_INTEGER + 1, layers: 0, parents: [] },
      { position: 0, layers: max + 1, parents: [] },
      { position: 1, layers: 0, parents: [0] },
      { position: 4, layers: 3, parents: [2, 1] },
      { position: 4, layers: 3, parents: [-1] },
      { position: 4, layers: 3, parents: [4] },
      { position: 4, layers: 3, parents: [0.5] },
      { position: 1, layers: 1 },
    ])
      expect(readHistory(raw)).toBeNull();
    expect(readHistory({ position: 1, layers: 1, parents: new Array(1) })).toBeNull();
  });

  it("returns null rather than overflowing cursor arithmetic", () => {
    const last = { position: Number.MAX_SAFE_INTEGER, layers: 0, parents: [] };
    expect(visit(last)).toBeNull();
    expect(pushDepths(last, 1)).toBeNull();
    expect(pushDepths({ position: 0, layers: 3, parents: [] }, 2)).toBeNull();
  });

  it("writes the Home entry before growing to an explicit Map destination", () => {
    const root = boundary(0);
    if (root === null) return;
    const next = plan(root, places.slice(0, 2), "visit", 1);
    expect(next).toEqual({
      cursor: { position: 1, layers: 1, parents: [0] },
      effects: [
        { kind: "replace", cursor: root, place: places[0] },
        { kind: "push", cursor: { position: 1, layers: 1, parents: [0] }, place: places[1] },
      ],
    });
  });

  it("writes the place for each depth while ordinary navigation grows or stays put", () => {
    const root = boundary(0);
    if (root === null) return;
    const entries = [{ cursor: root, place: placeAt(0) }];
    const first = plan(root, places.slice(0, 2), "ordinary", 1);
    if (first === null) return;
    let at = apply(entries, 0, first);
    expect(at).toBe(1);
    expect(entries.map((entry) => entry.place)).toEqual(places.slice(0, 2));

    const grown = plan(entries[at]?.cursor ?? root, places.slice(0, 3), "ordinary", 2);
    if (grown === null) return;
    expect(grown.effects).toEqual([
      { kind: "replace", cursor: entries[at]?.cursor, place: placeAt(1) },
      { kind: "push", cursor: { position: 2, layers: 2, parents: [0, 1] }, place: placeAt(2) },
    ]);
    at = apply(entries, at, grown);
    expect(entries.map((entry) => entry.place)).toEqual(places.slice(0, 3));

    const same = plan(entries[at]?.cursor ?? root, places.slice(0, 3), "ordinary", 3);
    expect(same?.effects).toEqual([{ kind: "replace", cursor: grown.cursor, place: placeAt(2) }]);

    const needsBefore = stored({
      ...START,
      view: "needs",
      needsLeft: { top: 12, entry: "entry-a", control: "next" },
    });
    const needsAfter = stored({
      ...START,
      view: "needs",
      needsLeft: { top: 48, entry: "entry-a", control: "next" },
    });
    const needs = plan(root, [placeAt(0), needsBefore], "ordinary", 1);
    if (needs === null) return;
    const nextDepth = plan(needs.cursor, [placeAt(0), needsAfter, placeAt(2)], "ordinary", 2);
    expect(nextDepth?.effects[0]).toEqual({ kind: "replace", cursor: needs.cursor, place: needsAfter });
  });

  it("keeps a known ordinary shrink as Back, then branches from B without replacing it", () => {
    const root = boundary(0);
    if (root === null) return;
    const [map, thread, item] = pushDepths(root, 3) ?? [];
    if (item === undefined) return;
    const b = visit(item);
    const c = b === null ? null : visit(b);
    if (map === undefined || thread === undefined || b === null || c === null) return;
    const entries = [
      { cursor: root, place: placeAt(0) },
      { cursor: map, place: placeAt(1) },
      { cursor: thread, place: placeAt(2) },
      { cursor: item, place: placeAt(3) },
      { cursor: b, place: placeAt(4) },
      { cursor: c, place: placeAt(5) },
    ];
    const shrink = plan(c, places.slice(0, 3), "ordinary", 6);
    if (shrink === null) return;
    let at = apply(entries, 5, shrink);
    expect(at).toBe(2);
    expect(entries[5]?.place).toEqual(placeAt(5));

    at = 4; // Forward to B, then follow a different thread at a shallower depth.
    const branch = plan(b, [placeAt(0), placeAt(1), placeAt(6)], "visit", entries.length);
    expect(branch).toEqual({
      cursor: { position: 5, layers: 2, parents: [0, 1] },
      effects: [{ kind: "push", cursor: { position: 5, layers: 2, parents: [0, 1] }, place: placeAt(6) }],
    });
    if (branch === null) return;
    at = apply(entries, at, branch);
    expect(at).toBe(5);
    expect(entries[4]?.place).toEqual(placeAt(4));
    expect(entries[5]?.place).toEqual(placeAt(6));
  });

  it("grows C to Search at six, then replaces that entry with D while preserving C at five", () => {
    const c = { position: 5, layers: 3, parents: [0, 1, 2] };
    const d = stored({ ...START, view: "map", thread: "thread-a", item: "item-d" });
    const phone = { has: () => true, archived: () => false, wide: false };
    expect(openLayers(openOf(d, phone))).toHaveLength(3);
    const entries = Array.from({ length: 6 }, (_, position) => ({
      cursor: { ...c, position },
      place: placeAt(Math.min(position, 5)),
    }));
    const openedSearch = plan(
      c,
      [placeAt(0), placeAt(1), placeAt(2), placeAt(5), placeAt(5)],
      "ordinary",
      entries.length,
    );
    expect(openedSearch?.cursor).toEqual({ position: 6, layers: 4, parents: [0, 1, 2, 5] });
    if (openedSearch === null || openedSearch === undefined) return;
    let at = apply(entries, 5, openedSearch);
    expect(entries[5]?.place).toEqual(placeAt(5));
    const replaceSearch = plan(
      openedSearch.cursor,
      [placeAt(0), placeAt(1), placeAt(2), d],
      "replaceTransient",
      entries.length,
    );
    expect(replaceSearch?.effects).toEqual([
      { kind: "replace", cursor: { position: 6, layers: 3, parents: [0, 1, 2] }, place: d },
    ]);
    if (replaceSearch === null || replaceSearch === undefined) return;
    at = apply(entries, at, replaceSearch);
    expect(at).toBe(6);
    expect(entries[5]?.place).toEqual(placeAt(5));
    expect(entries[6]?.place).toEqual(d);
  });

  it("replaces the Home portal entry after Search without inserting a Map entry", () => {
    const root = boundary(0);
    if (root === null) return;
    const entries = [{ cursor: root, place: placeAt(0) }];
    const search = placeAt(0);
    const openedSearch = plan(root, [search, search], "ordinary", entries.length);
    if (openedSearch === null) return;
    expect(openedSearch.cursor).toEqual({ position: 1, layers: 1, parents: [0] });
    const at = apply(entries, 0, openedSearch);
    expect(at).toBe(1);
    expect(entries).toHaveLength(2);
    const portal = stored({
      ...START,
      view: "map",
      item: "item-a",
      cameFrom: { view: "overview", by: "item" },
    });
    const phone = { has: () => true, archived: () => false, wide: false };
    expect(openLayers(openOf(portal, phone))).toHaveLength(1);
    const openedItem = plan(openedSearch.cursor, [placeAt(0), portal], "replaceTransient", entries.length);
    expect(openedItem?.effects).toEqual([{ kind: "replace", cursor: openedSearch.cursor, place: portal }]);
    if (openedItem === null || openedItem === undefined) return;
    expect(apply(entries, at, openedItem)).toBe(1);
    expect(entries).toHaveLength(2);
    expect(entries[0]?.place).toEqual(placeAt(0));
    expect(entries[1]?.place).toEqual(portal);
  });

  it("replaces transient depth locally and falls back when an ordinary shrink has unknown parents", () => {
    const c = { position: 5, layers: 3, parents: [0, 1, 2] };
    expect(plan(c, places.slice(0, 2), "replaceTransient", 6)).toEqual({
      cursor: { position: 5, layers: 1, parents: [0] },
      effects: [{ kind: "replace", cursor: { position: 5, layers: 1, parents: [0] }, place: places[1] }],
    });
    expect(plan({ position: 0, layers: 3, parents: [] }, places.slice(0, 3), "ordinary", 1)).toEqual({
      cursor: { position: 0, layers: 2, parents: [] },
      effects: [{ kind: "replace", cursor: { position: 0, layers: 2, parents: [] }, place: places[2] }],
    });
    expect(plan(c, places.slice(0, 3), "ordinary", 6)?.effects).toEqual([{ kind: "go", offset: -3 }]);
  });

  it("rejects planner inputs without canonical projections or valid history lengths", () => {
    const root = boundary(0);
    if (root === null) return;
    expect(plan(root, [], "ordinary", 1)).toBeNull();
    expect(plan(root, [{ view: "elsewhere" } as unknown as Place], "ordinary", 1)).toBeNull();
    expect(plan(root, places.slice(0, 2), "ordinary", -1)).toBeNull();
    expect(plan(root, places.slice(0, 2), "other" as "ordinary", 1)).toBeNull();
  });
});
