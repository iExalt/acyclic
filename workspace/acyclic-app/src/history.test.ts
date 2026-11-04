import { describe, expect, it } from "vitest";
import { boundary, close, pushDepths, readHistory, replaceDepth, visit } from "./history";
import { openLayers, START, stored } from "./nav";

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
});
