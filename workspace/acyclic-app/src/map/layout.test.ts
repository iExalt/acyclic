import ELK from "elkjs/lib/elk.bundled.js";
import type { ElkNode } from "elkjs/lib/elk-api.js";
import { describe, expect, it } from "vitest";
import { STRIP } from "./chevron";
import { beside, dependencyOrder, elkLayBox, PORTRAIT, shapeAfter, shapeOf } from "./elk";
import { item } from "./fixture";
import { metricsOf } from "./labelmetrics";
import { type LayBox, LayoutCache, LEAF, place, ROOT, scopes, withinOneBox } from "./layout";
import { buildModel, sourceKey, type ViewState, visible } from "./model";

/**
 * A layout engine whose box sizes depend on content: children in rows of two, as far apart as packing puts them, each row as tall as its tallest child,
 * so a box grows with every second child and with any child that grows. Edges don't change a size.
 */
function grid() {
  const calls: string[] = [];
  const layBox: LayBox = async (input) => {
    calls.push(input.id);
    const at: Record<string, { x: number; y: number }> = {};
    let width = 0;
    let y = 36;
    for (let row = 0; row < input.children.length; row += 2) {
      const pair = input.children.slice(row, row + 2);
      pair.forEach((child, i) => {
        at[child.id] = { x: i * (child.width + 24), y };
        width = Math.max(width, child.width);
      });
      y += Math.max(...pair.map((child) => child.height)) + 32;
    }
    return { width: width * 2 + 48, height: y + 12, at };
  };
  return { calls, layBox };
}

// top: left (box: l1, l2, l3), right (box: r1); far
const base = [item("top"), item("left", "top"), item("l1", "left"), item("l2", "left"), item("l3", "left")];
const plan = [...base, item("right", "top"), item("r1", "right"), item("far")];
const open: ViewState = { collapsed: new Set() };

/** The boxes laid out again after an edit, with the cache warm from the plan before it. */
async function relaid(before: ReturnType<typeof item>[], after: ReturnType<typeof item>[], state = open, next = open) {
  const engine = grid();
  const cache = new LayoutCache();
  await place(visible(buildModel(before), state), cache, engine.layBox, PORTRAIT);
  const placed = await place(visible(buildModel(after), next), cache, engine.layBox, PORTRAIT);
  return [...placed.laidOut].sort();
}

describe("place", () => {
  it("invalidates a box's cached layout when its measured header changes", async () => {
    const cache = new LayoutCache();
    const view = visible(buildModel([item("box"), item("tile", "box")]), open);
    const layBox: LayBox = async (input) => ({
      width: 224,
      height: (input.header ?? STRIP) + 76,
      at: Object.fromEntries(input.children.map((child) => [child.id, { x: 12, y: input.header ?? STRIP }])),
    });
    const sizes = new Map([["tile", { width: 200, height: 64 }]]);
    await place(view, cache, layBox, PORTRAIT, () => true, "same-source", sizes, new Map([["box", STRIP]]));
    const grown = await place(view, cache, layBox, PORTRAIT, () => true, "same-source", sizes, new Map([["box", 84]]));
    expect(grown.laidOut).toContain("box");
  });

  it("lays out again exactly the boxes whose content changed", async () => {
    // A fourth child fills the second row of two: right's size holds, so only right is laid out again.
    const three = [...plan, item("r2", "right"), item("r3", "right")];
    expect(await relaid(three, [...three, item("r4", "right")])).toEqual(["right"]);
    // A third child opens a row in right, but right stays no taller than left, its neighbour in top's row: top's
    // size holds, so the top level isn't laid out again.
    const grown = [...plan, item("r2", "right"), item("r3", "right")];
    expect(await relaid(plan, grown)).toEqual(["right", "top"]);
    // A fifth opens a third row: right outgrows left, so top grows, and the top level holds every box where it was.
    expect(await relaid(grown, [...grown, item("r4", "right"), item("r5", "right")])).toEqual(["right", "top"]);
    // A move changes both boxes; left loses its second row, so top shrinks, and the top level keeps where it was.
    const moved = plan.map((one) => (one.id === "l3" ? item("l3", "right") : one));
    expect(await relaid(plan, moved)).toEqual(["left", "right", "top"]);
    // A link between siblings changes only the box that holds both.
    const linked = plan.map((one) => (one.id === "l2" ? item("l2", "left", ["l1"]) : one));
    expect(await relaid(plan, linked)).toEqual(["left"]);
    // A link between items in different boxes lifts to the box holding both.
    const across = plan.map((one) => (one.id === "r1" ? item("r1", "right", ["l1"]) : one));
    expect(await relaid(plan, across)).toEqual(["top"]);
    // Collapsing a box changes its parent and everything above, but the top level, which only shrank.
    expect(await relaid(plan, plan, open, { collapsed: new Set(["left"]) })).toEqual(["top"]);
  });

  it("keeps the top level where it was when a box in it shrank or grew, and packs it again for anything else", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const model = buildModel(plan);
    const first = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    // Collapsing the top-level box leaves its sibling where it was, and the box at its own corner.
    const folded = await place(visible(model, { collapsed: new Set(["top"]) }), cache, engine.layBox, PORTRAIT);
    expect(folded.laidOut).toEqual([]);
    for (const id of ["top", "far"])
      expect([folded.nodes.get(id)?.x, folded.nodes.get(id)?.y]).toEqual([
        first.nodes.get(id)?.x,
        first.nodes.get(id)?.y,
      ]);
    expect(folded.nodes.get("top")?.height).toBeLessThan(first.nodes.get("top")?.height ?? 0);
    // Folded again elsewhere, it still holds; expanding again finds the layout from before.
    expect((await place(visible(model, open), cache, engine.layBox, PORTRAIT)).nodes).toEqual(first.nodes);
    // Expanded from collapsed in a fresh cache, top grows to the right, where nothing stands: far, on its left, stays.
    const shut = { collapsed: new Set(["top"]) };
    const fromShut = new LayoutCache();
    const was = await place(visible(model, shut), fromShut, engine.layBox, PORTRAIT);
    const grew = await place(visible(model, open), fromShut, engine.layBox, PORTRAIT);
    expect(grew.laidOut).not.toContain(ROOT);
    const corner = (placed: typeof was, id: string) => [placed.nodes.get(id)?.x, placed.nodes.get(id)?.y];
    for (const id of ["top", "far"]) expect(corner(grew, id)).toEqual(corner(was, id));
    // A new top-level box packs the top level again; a fresh cache, as a reload has, does too.
    const added = await place(visible(buildModel([...plan, item("near")]), open), cache, engine.layBox, PORTRAIT);
    expect(added.laidOut).toContain(ROOT);
    const fresh = await place(
      visible(model, { collapsed: new Set(["top"]) }),
      new LayoutCache(),
      engine.layBox,
      PORTRAIT,
    );
    expect(fresh.laidOut).toContain(ROOT);

    // A measured two-root plan must repack a novel shrink, then restore the grown hold exactly on Undo.
    const metricsCache = new LayoutCache();
    const fonts = { regular: (text: string) => text.length * 8, bold: (text: string) => text.length * 8 };
    let rootCalls = 0;
    const measured: LayBox = async (input) => {
      const header = input.header ?? STRIP;
      if (input.id !== ROOT) {
        const child = input.children[0];
        if (child === undefined) return { width: input.minWidth ?? 200, height: header + 24, at: {} };
        return {
          width: Math.max(input.minWidth ?? 200, child.width + 24),
          height: header + child.height + 24,
          at: { [child.id]: { x: 12, y: header + 12 } },
        };
      }
      rootCalls += 1;
      const children = dependencyOrder(
        input.children.map(({ id }) => id),
        input.edges,
      ).flatMap((id) => {
        const child = input.children.find((one) => one.id === id);
        return child === undefined ? [] : [child];
      });
      const at: Record<string, { x: number; y: number }> = {};
      let y = 12;
      for (const child of children) {
        at[child.id] = { x: 12, y };
        y += child.height + 24;
      }
      return {
        width: Math.max(0, ...children.map((child) => child.width)) + 24,
        height: children.length === 0 ? 24 : y - 12,
        at,
      };
    };
    const makeModel = (title: string) =>
      buildModel([
        item("a", null, [], { title }),
        item("a1", "a"),
        item("b", null, ["a"], { title: "B" }),
        item("b1", "b"),
      ]);
    const prepare = (snapshot: ReturnType<typeof makeModel>, state = open) => {
      const view = visible(snapshot, state);
      const labels = new Map(view.nodes.map((node) => [node.id, metricsOf(node, fonts)] as const));
      return {
        view,
        source: sourceKey(snapshot),
        sizes: new Map([...labels].map(([id, label]) => [id, { width: label.width, height: label.height }] as const)),
        headers: new Map([...labels].map(([id, label]) => [id, label.header || label.height] as const)),
      };
    };
    const medium = prepare(makeModel("W".repeat(36)));
    const long = prepare(makeModel("W".repeat(90)));
    const novelShort = prepare(makeModel("New short"));
    expect(long.source).not.toBe(medium.source);
    expect(novelShort.source).not.toBe(medium.source);
    expect(novelShort.source).not.toBe(long.source);
    expect(medium.headers.get("a")).toBe(65);
    expect(long.headers.get("a")).toBe(114);
    expect(novelShort.headers.get("a")).toBe(52);
    const run = (input: ReturnType<typeof prepare>) =>
      place(input.view, metricsCache, measured, PORTRAIT, () => true, input.source, input.sizes, input.headers);
    const beforeTitle = await run(medium);
    const afterGrow = await run(long);
    expect(afterGrow.laidOut).not.toContain(ROOT);
    expect(afterGrow.nodes.get("a")).toMatchObject({
      x: beforeTitle.nodes.get("a")?.x,
      y: beforeTitle.nodes.get("a")?.y,
    });
    expect(afterGrow.nodes.get("b")?.y).toBeGreaterThan(beforeTitle.nodes.get("b")?.y ?? 0);
    const headerClearsTile = (placed: Awaited<ReturnType<typeof place>>, input: ReturnType<typeof prepare>) =>
      (placed.nodes.get("a1")?.y ?? 0) - (placed.nodes.get("a")?.y ?? 0) >= (input.headers.get("a") ?? 0);
    expect(headerClearsTile(beforeTitle, medium)).toBe(true);
    expect(headerClearsTile(afterGrow, long)).toBe(true);

    const callsBeforeNovelShort = rootCalls;
    const shrunk = await run(novelShort);
    expect(rootCalls).toBe(callsBeforeNovelShort + 1);
    expect(shrunk.nodes.get("b")?.y).toBeLessThan(beforeTitle.nodes.get("b")?.y ?? 0);
    expect(headerClearsTile(shrunk, novelShort)).toBe(true);
    const callsBeforeFresh = rootCalls;
    const freshShort = await place(
      novelShort.view,
      new LayoutCache(),
      measured,
      PORTRAIT,
      () => true,
      novelShort.source,
      novelShort.sizes,
      novelShort.headers,
    );
    expect(rootCalls).toBe(callsBeforeFresh + 1);
    expect(shrunk.nodes).toEqual(freshShort.nodes);

    const afterUndo = await run(long);
    expect(afterUndo.laidOut).not.toContain(ROOT);
    expect(headerClearsTile(afterUndo, long)).toBe(true);
    expect(afterUndo.nodes).toEqual(afterGrow.nodes);

    const foldedLong = prepare(makeModel("W".repeat(90)), { collapsed: new Set(["a"]) });
    expect(foldedLong.source).toBe(long.source);
    await run(foldedLong);
    expect((await run(long)).nodes).toEqual(afterUndo.nodes);
  });

  it("restores a topology's held packing across an add and undo, and later folds", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const model = buildModel(plan);
    const folded = await place(visible(model, { collapsed: new Set(["top"]) }), cache, engine.layBox, PORTRAIT);
    await place(visible(buildModel([...plan, item("near")]), open), cache, engine.layBox, PORTRAIT);

    // Undoing the top-level add restores the original topology, but the top box is now expanded from its folded
    // baseline. A signature-only cache has never seen this expanded root input.
    const undone = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    const corner = (placed: typeof folded, id: string) => [placed.nodes.get(id)?.x, placed.nodes.get(id)?.y];
    for (const id of ["top", "far"]) expect(corner(undone, id)).toEqual(corner(folded, id));

    // The held expanded result must not become the baseline: a subsequent fold and reopen retain the original packing.
    const foldedAgain = await place(visible(model, { collapsed: new Set(["top"]) }), cache, engine.layBox, PORTRAIT);
    const reopened = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    for (const id of ["top", "far"]) {
      expect(corner(foldedAgain, id)).toEqual(corner(folded, id));
      expect(corner(reopened, id)).toEqual(corner(folded, id));
    }
  });

  it("repacks an actual shrink and restores the held view and packing on Undo", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const foldedView = visible(buildModel(plan), { collapsed: new Set(["top"]) });
    const openView = visible(buildModel(plan), open);
    const reduced = plan.filter((one) => one.id !== "l3");
    const shrinkView = visible(buildModel(reduced), open);
    const folded = await place(foldedView, cache, engine.layBox, PORTRAIT, undefined, "source-a");
    const expanded = await place(openView, cache, engine.layBox, PORTRAIT, undefined, "source-a");
    expect(expanded.laidOut).not.toContain(ROOT);
    const shrunk = await place(shrinkView, cache, engine.layBox, PORTRAIT, undefined, "source-b");
    expect(shrunk.laidOut).toContain(ROOT);

    // Undo returns to the old source key, whose original folded packing reproduces the held expanded view.
    const undone = await place(openView, cache, engine.layBox, PORTRAIT, undefined, "source-a");
    expect(undone.nodes).toEqual(expanded.nodes);
    const refolded = await place(foldedView, cache, engine.layBox, PORTRAIT, undefined, "source-a");
    expect(refolded.nodes).toEqual(folded.nodes);
  });

  it("inherits the original hold baseline for a new source that only grows", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const before = buildModel(plan);
    const grown = buildModel([...plan, item("l4", "left"), item("l5", "left")]);
    const first = await place(visible(before, open), cache, engine.layBox, PORTRAIT, undefined, "source-a");
    const next = await place(visible(grown, open), cache, engine.layBox, PORTRAIT, undefined, "source-b");
    expect(next.laidOut).not.toContain(ROOT);
    const folded = await place(
      visible(grown, { collapsed: new Set(["top"]) }),
      cache,
      engine.layBox,
      PORTRAIT,
      undefined,
      "source-b",
    );
    expect([folded.nodes.get("top")?.x, folded.nodes.get("top")?.y]).toEqual([
      first.nodes.get("top")?.x,
      first.nodes.get("top")?.y,
    ]);
  });

  it("keeps a stale same-topology request from changing the displayed shrink comparison", async () => {
    let releaseStale: ((layout: Awaited<ReturnType<LayBox>>) => void) | undefined;
    let waitingForStale = false;
    const sizes = new Map([
      [1, 500],
      [2, 400],
      [3, 450],
      [4, 430],
    ]);
    const layBox: LayBox = async (input) => {
      if (input.id === "box") {
        const height = sizes.get(input.children.length) ?? 100;
        return {
          width: 200,
          height,
          at: Object.fromEntries(input.children.map((child, index) => [child.id, { x: 12, y: 12 + index * 100 }])),
        };
      }
      const box = input.children[0];
      if (box?.height === 400) {
        waitingForStale = true;
        return new Promise((resolve) => {
          releaseStale = resolve;
        });
      }
      const y = (box?.height ?? 0) + 36;
      return { width: 300, height: y + 64, at: { box: { x: 12, y: 12 }, other: { x: 12, y } } };
    };
    const cache = new LayoutCache();
    let latest = 0;
    const items = (count: number) => [
      item("box"),
      ...Array.from({ length: count }, (_, index) => item(`child-${index}`, "box")),
      item("other"),
    ];
    const placeLatest = (count: number, source: string) => {
      const request = ++latest;
      return place(visible(buildModel(items(count)), open), cache, layBox, PORTRAIT, () => request === latest, source);
    };

    await placeLatest(1, "source-a");
    const stale = placeLatest(2, "source-b");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(waitingForStale).toBe(true);

    // The accepted C input is 450. When stale B (400) answers later, it must not replace that displayed comparator.
    expect((await placeLatest(3, "source-c")).laidOut).toContain(ROOT);
    releaseStale?.({
      width: 300,
      height: 500,
      at: { box: { x: 12, y: 12 }, other: { x: 12, y: 436 } },
    });
    await stale;

    // D at 430 shrinks from accepted C at 450, so root packing runs. A stale comparator of 400 would hold instead.
    expect((await placeLatest(4, "source-d")).laidOut).toContain(ROOT);
  });

  it("restores the matching aspect's packing after a turn", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const model = buildModel(plan);
    const folded = await place(visible(model, { collapsed: new Set(["top"]) }), cache, engine.layBox, PORTRAIT);
    await place(visible(model, open), cache, engine.layBox, 1.4);
    const turnedBack = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    const corner = (placed: typeof folded, id: string) => [placed.nodes.get(id)?.x, placed.nodes.get(id)?.y];
    for (const id of ["top", "far"]) expect(corner(turnedBack, id)).toEqual(corner(folded, id));
  });

  it("keeps a box's position after the other box opens and closes", async () => {
    // Two top-level boxes, both collapsed at first, each opened and closed in turn.
    const two = buildModel([item("x"), item("x1", "x"), item("x2", "x"), item("x3", "x"), item("y"), item("y1", "y")]);
    const at = (collapsed: string[]) => visible(two, { collapsed: new Set(collapsed) });
    const engine = grid();
    const direct = new LayoutCache();
    await place(at(["x", "y"]), direct, engine.layBox, PORTRAIT);
    const yOpen = await place(at(["x"]), direct, engine.layBox, PORTRAIT);
    const walked = new LayoutCache();
    await place(at(["x", "y"]), walked, engine.layBox, PORTRAIT);
    await place(at(["y"]), walked, engine.layBox, PORTRAIT);
    await place(at([]), walked, engine.layBox, PORTRAIT);
    // Reached by way of x opening and closing, y open stands as it does opened alone, and both closed as at first.
    expect((await place(at(["x"]), walked, engine.layBox, PORTRAIT)).nodes).toEqual(yOpen.nodes);
  });

  it("pulls the boxes under or beside a box that closes into the room it gave up, and returns them when it reopens", async () => {
    // A tall box opened where the top level was packed, with others stacked and set beside it.
    const kids = (parent: string, n: number) => Array.from({ length: n }, (_, i) => item(`${parent}${i}`, parent));
    const items = [
      item("big"),
      ...kids("big", 8),
      item("under"),
      ...kids("under", 2),
      item("side"),
      ...kids("side", 2),
      item("last"),
      ...kids("last", 2),
    ];
    const model = buildModel(items);
    const engine = grid();
    const cache = new LayoutCache();
    const packed = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    const shut = await place(visible(model, { collapsed: new Set(["big"]) }), cache, engine.layBox, PORTRAIT);
    const corner = (placed: typeof packed, id: string) => ({ x: placed.nodes.get(id)?.x, y: placed.nodes.get(id)?.y });
    const [was, now] = [packed.nodes.get("big"), shut.nodes.get("big")];
    const gave = (was?.height ?? 0) - (now?.height ?? 0);
    expect(gave).toBeGreaterThan(0);
    expect(corner(shut, "big")).toEqual(corner(packed, "big"));
    let pulled = 0;
    for (const id of ["under", "side", "last"]) {
      const [from, to] = [corner(packed, id), corner(shut, id)];
      // Only ever up or left, by no more than it gave up, and never into another box.
      expect(to.y).toBeLessThanOrEqual(from.y ?? 0);
      expect(to.x).toBeLessThanOrEqual(from.x ?? 0);
      expect((from.y ?? 0) - (to.y ?? 0)).toBeLessThanOrEqual(gave);
      if (to.x !== from.x || to.y !== from.y) pulled++;
    }
    expect(pulled).toBeGreaterThan(0);
    const ids = ["big", "under", "side", "last"];
    for (const a of ids)
      for (const b of ids.filter((id) => id > a)) {
        const [p, q] = [shut.nodes.get(a), shut.nodes.get(b)];
        if (p === undefined || q === undefined) throw new Error("missing node");
        expect(p.x < q.x + q.width && q.x < p.x + p.width && p.y < q.y + q.height && q.y < p.y + p.height).toBe(false);
      }
    // Open again, every box is where it was packed.
    const reopened = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    expect(reopened.nodes).toEqual(packed.nodes);
    // And closed again, they take the same places.
    expect(
      (await place(visible(model, { collapsed: new Set(["big"]) }), cache, engine.layBox, PORTRAIT)).nodes,
    ).toEqual(shut.nodes);
  });
  it("finds every layout in the cache on returning to a view", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const model = buildModel(plan);
    const first = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    await place(visible(model, { collapsed: new Set(["left"]) }), cache, engine.layBox, PORTRAIT);
    const back = await place(visible(model, open), cache, engine.layBox, PORTRAIT);
    expect(back.laidOut).toEqual([]);
    expect(back.nodes).toEqual(first.nodes);
  });

  it("lays out only the top level again for another shape, which only it takes", async () => {
    const engine = grid();
    const cache = new LayoutCache();
    const view = visible(buildModel(plan), open);
    await place(view, cache, engine.layBox, PORTRAIT);
    expect((await place(view, cache, engine.layBox, 1.4)).laidOut).toEqual([ROOT]);
    expect((await place(view, cache, engine.layBox, PORTRAIT)).laidOut).toEqual([]);
  });

  it("places children at their box's position plus their own, with real ELK, the same every time", async () => {
    const elk = elkLayBox(new ELK());
    const linked = [...plan, item("r2", "right", ["l1"]), item("far2", null, ["far"])];
    const view = visible(buildModel(linked), open);
    const one = await place(view, new LayoutCache(), elk, PORTRAIT);
    const two = await place(view, new LayoutCache(), elk, PORTRAIT);
    expect(two.nodes).toEqual(one.nodes);
    const top = one.nodes.get("top");
    const left = one.nodes.get("left");
    const l1 = one.nodes.get("l1");
    expect(top && left && l1).toBeTruthy();
    // Every child lies within its box.
    for (const [child, box] of [
      [l1, left],
      [left, top],
    ] as const) {
      expect(child?.x).toBeGreaterThanOrEqual(box?.x ?? Infinity);
      expect((child?.x ?? 0) + (child?.width ?? 0)).toBeLessThanOrEqual((box?.x ?? 0) + (box?.width ?? 0));
      expect(child?.y).toBeGreaterThanOrEqual(box?.y ?? Infinity);
      expect((child?.y ?? 0) + (child?.height ?? 0)).toBeLessThanOrEqual((box?.y ?? 0) + (box?.height ?? 0));
    }
    // The edge from l1 to r2 runs between boxes, so ELK lays it out between left and right in top.
    expect(scopes(view).edges.get("top")).toEqual([{ source: "left", target: "right" }]);
    expect(one.width).toBeGreaterThan(0);
  });

  it("bundles a node's links on the way out, with a dot where they split, and never on the way in", async () => {
    // In p: a and b, then c waiting for both, and d for a alone.
    const items = [item("p"), item("a", "p"), item("b", "p"), item("c", "p", ["a", "b"]), item("d", "p", ["a"])];
    const placed = await place(visible(buildModel(items), open), new LayoutCache(), elkLayBox(new ELK()), PORTRAIT);
    const route = (key: string) => placed.routes.get(key) ?? [];
    // a's two links leave from one point, and split where a dot is drawn.
    expect(route("a->c")[0]).toEqual(route("a->d")[0]);
    expect([...placed.junctions.values()].flat().length).toBeGreaterThan(0);
    // a's and b's links into c arrive apart, each with its own arrowhead.
    expect(route("a->c").at(-1)).not.toEqual(route("b->c").at(-1));
  });

  it("routes the links inside a box, with real ELK, from the source's border to the target's where they are drawn", async () => {
    // In left, inside top: l1 to l2 to l3, and l1 to l3, which the way through l2 implies.
    const chained = [...plan.filter((one) => !["l2", "l3"].includes(one.id)), item("l2", "left", ["l1"])];
    const view = visible(buildModel([...chained, item("l3", "left", ["l1", "l2"])]), open);
    const placed = await place(view, new LayoutCache(), elkLayBox(new ELK()), PORTRAIT);
    expect([...placed.redundant]).toEqual(["l1->l3"]);
    expect([...placed.routes.keys()].sort()).toEqual(["l1->l2", "l2->l3"]);
    for (const [key, from, to] of [
      ["l1->l2", "l1", "l2"],
      ["l2->l3", "l2", "l3"],
    ] as const) {
      const way = placed.routes.get(key) ?? [];
      const source = placed.nodes.get(from);
      const target = placed.nodes.get(to);
      // Down from the source's foot to the target's head, within each one's width: so offset by top and left both.
      expect(way[0]?.y).toBeCloseTo((source?.y ?? 0) + (source?.height ?? 0));
      expect(way.at(-1)?.y).toBeCloseTo(target?.y ?? 0);
      expect(way[0]?.x).toBeGreaterThanOrEqual(source?.x ?? Infinity);
      expect(way[0]?.x).toBeLessThanOrEqual((source?.x ?? 0) + (source?.width ?? 0));
      expect(way.at(-1)?.x).toBeGreaterThanOrEqual(target?.x ?? Infinity);
      expect(way.at(-1)?.x).toBeLessThanOrEqual((target?.x ?? 0) + (target?.width ?? 0));
    }
  });

  it("asks for a box only once every box inside it has answered, and for a level's boxes at once", async () => {
    const pending: { id: string; done: () => void }[] = [];
    const asked: string[][] = [];
    const layBox: LayBox = (input) =>
      new Promise((resolve) => {
        pending.push({ id: input.id, done: () => resolve({ width: 100, height: 100, at: {} }) });
      });
    const placing = place(visible(buildModel(plan), open), new LayoutCache(), layBox, PORTRAIT);
    // Let each level's requests go out, record them, then answer them all.
    for (let level = 0; level < 3; level++) {
      await new Promise((resolve) => setTimeout(resolve, 0));
      asked.push(pending.map((one) => one.id).sort());
      for (const one of pending.splice(0)) one.done();
    }
    await placing;
    expect(asked).toEqual([["left", "right"], ["top"], [ROOT]]);
  });
});

describe("elkLayBox", () => {
  it("orders the top level by its links, and packs it without ELK", async () => {
    const order = dependencyOrder(["a", "b", "c"], [{ source: "b", target: "a" }]);
    expect(order).toEqual(["b", "a", "c"]);
    // Without links, ids come in natural order, as the CLI lists them.
    expect(dependencyOrder(["i10", "i2", "project-10", "project-2"], [])).toEqual([
      "i2",
      "i10",
      "project-2",
      "project-10",
    ]);
    const asked: string[] = [];
    const watched = {
      layout: async (graph: ElkNode) => {
        asked.push(graph.id);
        return graph;
      },
    };
    const packed = await elkLayBox(watched)({ id: ROOT, children: [{ id: "a", width: 90, height: 40 }], edges: [] });
    expect([asked, packed.at.a]).toEqual([[], { x: 12, y: 12 }]);
  });

  it("shapes the top level like the Map, to a fifth, within portrait and three wide", () => {
    expect(shapeOf(1180, 850)).toBe(1.4);
    expect(shapeOf(1000, 1000)).toBe(1);
    expect(shapeOf(300, 900)).toBe(PORTRAIT);
    expect(shapeOf(5000, 400)).toBe(3);
  });

  it("keeps its shape while a notice takes a strip of the Map's height, and takes a new one when turned", () => {
    // 1192 wide: 900 tall is 1.32, a step from 1.4's edge; the slot's 52 px makes it 1.41.
    const laid = shapeAfter(null, 1192, 900);
    expect(laid).toBe(1.4);
    expect(shapeAfter(laid, 1192, 848)).toBe(laid);
    expect(shapeAfter(1.2, 1192, 900)).toBe(1.2);
    expect(shapeAfter(laid, 900, 1192)).toBe(0.8);
  });

  it("stands a box's Done box last among its children aside, whatever order it is given in", async () => {
    const none = { layout: async (graph: ElkNode) => graph };
    const children = ["~done p", "a", "b"].map((id) => ({ id, width: LEAF.width, height: LEAF.height }));
    const laid = await elkLayBox(none)({ id: "p", children, edges: [] });
    const order = Object.entries(laid.at)
      .sort(([, one], [, two]) => one.x - two.x || one.y - two.y)
      .map(([id]) => id);
    expect(order).toEqual(["a", "b", "~done p"]);
  });

  it("breaks a dependency cycle at its least id", () => {
    const edges = [
      { source: "a", target: "b" },
      { source: "b", target: "a" },
    ];
    expect(dependencyOrder(["a", "b"], edges)).toEqual(["a", "b"]);
  });
});

describe("beside", () => {
  const tiles = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `t${i}`, ...LEAF }));
  const columns = (layout: { at: Record<string, { x: number; y: number }> }) =>
    new Set(Object.values(layout.at).map((at) => at.x)).size;

  it("leaves a box with every child linked as its graph lays it out", () => {
    const graph = { width: 224, height: 200, at: { a: { x: 12, y: STRIP } } };
    expect(beside(graph, [])).toBe(graph);
  });

  it("stands the unlinked on the graph's left, in columns as tall as it, a wider gap before the graph", () => {
    // A graph of three layers holds three tiles a column; five more take two columns.
    const graph = { width: 224, height: STRIP + 3 * LEAF.height + 2 * 40 + 12, at: { a: { x: 12, y: STRIP } } };
    const routes = { "a->b": [{ x: 112, y: 100 }] };
    const junctions = { "a->b": [{ x: 50, y: 90 }] };
    const out = beside({ ...graph, routes, junctions }, tiles(5));
    expect(out.at.t0).toEqual({ x: 12, y: STRIP });
    expect(out.at.t3).toEqual({ x: 12 + 200 + 24, y: STRIP });
    // The graph moves right past the two columns and the gap, its links and their splits with it.
    const dx = 12 + 2 * 200 + 24 + 40 - 12;
    expect(out.at.a).toEqual({ x: 12 + dx, y: STRIP });
    expect(out.routes).toEqual({ "a->b": [{ x: 112 + dx, y: 100 }] });
    expect(out.junctions).toEqual({ "a->b": [{ x: 50 + dx, y: 90 }] });
    expect(out.width).toBe(dx + 224);
    expect(out.height).toBe(graph.height);
    // A dashed line halfway across the wider gap, from under the title to the foot's padding.
    expect(out.divider).toEqual({ x: 12 + 2 * 200 + 24 + 40 / 2, top: STRIP, bottom: graph.height - 12 });
    expect(beside(null, tiles(5)).divider).toBeUndefined();
  });

  it("grids a box with nothing linked about as many tiles tall as wide, and grows past a short graph", () => {
    const alone = beside(null, tiles(9));
    expect(columns(alone)).toBe(3);
    expect(alone.height).toBe(STRIP + 3 * LEAF.height + 2 * 24 + 12);
    expect(columns(beside(null, tiles(10)))).toBe(3);
    // A graph of one layer is shorter than four tiles' grid, so the column takes the grid's height and the box grows.
    const flat = beside({ width: 224, height: STRIP + LEAF.height + 12, at: {} }, tiles(4));
    expect(columns(flat)).toBe(2);
    expect(flat.height).toBe(STRIP + 2 * LEAF.height + 24 + 12);
  });

  it("shares the unlinked evenly among the columns they take, leaving none nearly empty", () => {
    // Four fit beside this graph, so nine take three columns: three each, not four, four and one.
    const graph = { width: 224, height: STRIP + 4 * LEAF.height + 3 * 24 + 12, at: {} };
    const out = beside(graph, tiles(9));
    const xs = Object.values(out.at).map((at) => at.x);
    expect([...new Set(xs)].map((x) => xs.filter((one) => one === x).length)).toEqual([3, 3, 3]);
  });

  it("makes each column as wide as its widest child", () => {
    const out = beside(null, [
      { id: "box", width: 500, height: 300 },
      { id: "a", ...LEAF },
      { id: "b", ...LEAF },
    ]);
    expect(out.at.a).toEqual({ x: 12 + 500 + 24, y: STRIP });
    expect(out.width).toBe(12 + 500 + 24 + 200 + 12);
  });
});

describe("withinOneBox", () => {
  it("draws an edge always only when both drawn ends are in the same box", () => {
    const linked = [...plan, item("r2", "right", ["r1"]), item("far2", null, ["l1"])];
    const view = visible(buildModel(linked), { collapsed: new Set(["left"]) });
    const within = withinOneBox(view);
    const edges = view.edges.map((edge) => [`${edge.source}->${edge.target}`, within(edge)]);
    // r1 to r2 is inside right; left (standing in for l1) to far2 crosses from top to the top level.
    expect(edges).toEqual([
      ["left->far2", false],
      ["r1->r2", true],
    ]);
  });
});

describe("LayoutCache", () => {
  it("evicts the least recently used entry beyond its limit", () => {
    const cache = new LayoutCache(2);
    const layout = { width: 1, height: 1, at: {} };
    cache.set("a", layout);
    cache.set("b", layout);
    cache.get("a");
    cache.set("c", layout);
    expect([cache.get("a"), cache.get("b"), cache.get("c")].map(Boolean)).toEqual([true, false, true]);
    expect(cache.size).toBe(2);
  });

  it("bounds root packing history independently and evicts its least recently used topology", () => {
    const cache = new LayoutCache(2, 2);
    const input = (id: string) => ({ id: ROOT, children: [{ id, width: 1, height: 1 }], edges: [], aspect: 1 });
    const layout = { width: 1, height: 1, at: {} };
    const a = input("a");
    const b = input("b");
    const c = input("c");
    const baseline = (one: typeof a) => ({ input: one, layout });
    cache.rememberRoot(a, "source", baseline(a));
    cache.rememberRoot(b, "source", baseline(b));
    cache.root(a, "source");
    cache.rememberRoot(c, "source", baseline(c));
    expect([cache.root(a, "source"), cache.root(b, "source"), cache.root(c, "source")].map(Boolean)).toEqual([
      true,
      false,
      true,
    ]);
    expect(cache.rootSize).toBe(2);
  });
});
