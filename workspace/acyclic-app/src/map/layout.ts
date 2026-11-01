// Scoped layout: each box lays out only its own children, as a flat graph, with the edges between them lifted from
// their descendants, and each child box at the size its own layout gave it. A box's layout depends only on its
// signature (its children's ids, kinds and sizes, and its edges, all sorted), so layouts are cached by signature: an
// edit lays out again exactly the boxes whose signature changed, from the inside out, and returning to a view already
// laid out costs nothing. Pure apart from the `LayBox` it is given, which is ELK in the app and a fake in tests.

import { natural } from "../natural";
import type { Kind, View } from "./model";
import type { Point } from "./route";

/** The scope above the top-level items. */
export const ROOT = "";
export const LEAF = { width: 200, height: 64 };
export const COLLAPSED = { width: 200, height: 64 };

export interface Size {
  width: number;
  height: number;
}

/** What one box asks of the layout engine: its children's sizes and the edges among them, all sorted. */
export interface BoxInput {
  id: string;
  children: ({ id: string } & Size)[];
  edges: { source: string; target: string }[];
  /** The width over height the top level is packed to; only the top level has one. */
  aspect?: number;
}

/** A box's layout: its size, where each child sits within it, and the route of each link between its children. */
export interface BoxLayout extends Size {
  at: Record<string, { x: number; y: number }>;
  /** By `source->target`, from the source's border to the target's; none for a link the engine didn't route. */
  routes?: Record<string, Point[]>;
  /** Where a trunk of links leaving one node splits, by the `source->target` of a link that splits there. */
  junctions?: Record<string, Point[]>;
}

export type LayBox = (input: BoxInput) => Promise<BoxLayout>;

/** Each box's children and the edges among them, lifted to that box, less those `redundant`. */
export interface Scopes {
  kids: Map<string, string[]>;
  edges: Map<string, { source: string; target: string }[]>;
  kind: Map<string, Kind>;
  /** Links whose every item-level link a longer way inside the box implies, by `source->target`: drawn when asked for. */
  redundant: Set<string>;
}

export function scopes(view: View): Scopes {
  const kids = new Map<string, string[]>([[ROOT, []]]);
  const parent = new Map<string, string>();
  const kind = new Map<string, Kind>();
  for (const node of view.nodes) {
    const box = node.parent ?? ROOT;
    parent.set(node.id, box);
    kind.set(node.id, node.kind);
    if (!kids.has(box)) kids.set(box, []);
    kids.get(box)?.push(node.id);
    if (node.kind === "box" && !kids.has(node.id)) kids.set(node.id, []);
  }
  const chain = (id: string) => {
    const out = [id];
    for (let p = parent.get(id); p !== undefined; p = parent.get(p)) out.push(p);
    return out;
  };
  // Each box's edges, with whether every edge lifted to each is implied.
  const sets = new Map<string, Map<string, boolean>>();
  for (const edge of view.edges) {
    const { box, source, target } = lift(chain(edge.source), chain(edge.target));
    if (source === target) continue;
    const set = sets.get(box) ?? new Map<string, boolean>();
    const key = `${source}\u0000${target}`;
    set.set(key, (set.get(key) ?? true) && edge.implied === true);
    sets.set(box, set);
  }
  const edges = new Map<string, { source: string; target: string }[]>();
  const redundant = new Set<string>();
  for (const [box, set] of sets) {
    const kept: { source: string; target: string }[] = [];
    for (const key of [...set.keys()].sort()) {
      const [source = "", target = ""] = key.split("\u0000");
      if (set.get(key)) redundant.add(`${source}->${target}`);
      else kept.push({ source, target });
    }
    edges.set(box, kept);
  }
  for (const list of kids.values()) list.sort(natural);
  return { kids, edges, kind, redundant };
}

/** The lowest box holding both ends of an edge, and the children of that box that hold each end. */
function lift(from: string[], to: string[]) {
  const inTo = new Set(to);
  const box = from.find((id, i) => i > 0 && inTo.has(id) && to.indexOf(id) > 0) ?? ROOT;
  const source = from[from.indexOf(box) - 1] ?? from[from.length - 1] ?? "";
  const target = to[to.indexOf(box) - 1] ?? to[to.length - 1] ?? "";
  return { box, source, target };
}

/**
 * Tells whether an edge is drawn whatever is selected: when both its drawn ends are children of the same box, it
 * stays inside that box. Every other edge crosses boxes and is drawn only for the selected item, since without routing
 * around boxes it would cross whatever lies between. Built once per view, so classifying every edge is linear.
 */
export function withinOneBox(view: View): (edge: { source: string; target: string }) => boolean {
  const parent = new Map(view.nodes.map((node) => [node.id, node.parent]));
  return (edge) => parent.get(edge.source) === parent.get(edge.target);
}

/** A bounded cache of box layouts by signature, evicting the least recently used. */
export class LayoutCache {
  private readonly entries = new Map<string, BoxLayout>();
  /** The top level as last placed, which a top level that only shrank keeps where it was. */
  root: { input: BoxInput; layout: BoxLayout } | null = null;

  constructor(private readonly limit = 4096) {}

  get(signature: string): BoxLayout | undefined {
    const hit = this.entries.get(signature);
    if (hit !== undefined) {
      // Re-insertion moves it to the newest end.
      this.entries.delete(signature);
      this.entries.set(signature, hit);
    }
    return hit;
  }

  set(signature: string, layout: BoxLayout) {
    this.entries.delete(signature);
    this.entries.set(signature, layout);
    while (this.entries.size > this.limit) {
      const oldest = this.entries.keys().next().value;
      if (oldest === undefined) break;
      this.entries.delete(oldest);
    }
  }

  get size() {
    return this.entries.size;
  }
}

/** Where every visible node sits, in the view's own coordinates, and the whole drawing's size. */
export interface Placed extends Size {
  nodes: Map<string, { x: number; y: number } & Size>;
  /** Each routed link's way, by `source->target`, in the same coordinates. */
  routes: Map<string, Point[]>;
  /** Where trunks split, by the `source->target` of a link that splits there, in the same coordinates. */
  junctions: Map<string, Point[]>;
  /** The links a longer way inside their box implies, drawn only when asked for. */
  redundant: ReadonlySet<string>;
  /** The boxes laid out afresh for this view, rather than found in the cache. */
  laidOut: string[];
}

/** A child's size: a box's from its own layout, without where its children sit, which its parent doesn't depend on. */
function sizeOf(kind: Kind | undefined, boxes: Map<string, BoxLayout>, id: string): Size {
  if (kind === "collapsed") return COLLAPSED;
  const box = kind === "box" ? boxes.get(id) : undefined;
  return box === undefined ? LEAF : { width: box.width, height: box.height };
}

/**
 * Lays a view out box by box, from the inside out, through the cache. Boxes of the same height (the longest path to
 * a box with no boxes inside) don't depend on each other, so each height's boxes are asked for at once and the engine
 * never waits for the next request; a box is asked for only once every box inside it has its size.
 */
export async function place(view: View, cache: LayoutCache, layBox: LayBox, aspect: number): Promise<Placed> {
  const scope = scopes(view);
  const boxes = new Map<string, BoxLayout>();
  const laidOut: string[] = [];
  const levels: string[][] = [];
  const height = (id: string): number => {
    let h = 0;
    for (const child of scope.kids.get(id) ?? [])
      if (scope.kind.get(child) === "box") h = Math.max(h, height(child) + 1);
    const level = levels[h] ?? [];
    level.push(id);
    levels[h] = level;
    return h;
  };
  height(ROOT);
  for (const level of levels) {
    await Promise.all(
      level.map(async (id) => {
        const input: BoxInput = {
          id,
          children: (scope.kids.get(id) ?? []).map((child) => ({
            id: child,
            ...sizeOf(scope.kind.get(child), boxes, child),
          })),
          edges: scope.edges.get(id) ?? [],
          ...(id === ROOT ? { aspect } : {}),
        };
        // The box's own id isn't part of what its layout depends on, so equal content shares one entry.
        const signature = JSON.stringify([input.children, input.edges, input.aspect]);
        let layout = cache.get(signature);
        // A top level that only shrank, as a box collapsing makes it, keeps every box where it was, rather than
        // packing them again around the smaller one and moving the rest of the plan out of view. Kept out of the
        // cache, so a reload packs afresh; expanding again finds the layout from before in it.
        const kept = id === ROOT && layout === undefined ? held(cache.root, input) : undefined;
        if (kept !== undefined) layout = kept;
        if (layout === undefined) {
          layout = await layBox(input);
          cache.set(signature, layout);
          laidOut.push(id);
        }
        if (id === ROOT) cache.root = { input, layout };
        boxes.set(id, layout);
      }),
    );
  }
  const root = boxes.get(ROOT) ?? { width: 0, height: 0, at: {} };
  const nodes = new Map<string, { x: number; y: number } & Size>();
  const routes = new Map<string, Point[]>();
  const junctions = new Map<string, Point[]>();
  const absolute = (id: string, ox: number, oy: number) => {
    const box = boxes.get(id);
    if (box === undefined) return;
    const moved = (points: Point[]) => points.map((point) => ({ x: ox + point.x, y: oy + point.y }));
    for (const [key, way] of Object.entries(box.routes ?? {})) routes.set(key, moved(way));
    for (const [key, at] of Object.entries(box.junctions ?? {})) junctions.set(key, moved(at));
    for (const child of scope.kids.get(id) ?? []) {
      const at = box.at[child] ?? { x: 0, y: 0 };
      const x = ox + at.x;
      const y = oy + at.y;
      nodes.set(child, { x, y, ...sizeOf(scope.kind.get(child), boxes, child) });
      if (scope.kind.get(child) === "box") absolute(child, x, y);
    }
  };
  absolute(ROOT, 0, 0);
  return { width: root.width, height: root.height, nodes, routes, junctions, redundant: scope.redundant, laidOut };
}

/**
 * The top level's last layout, for a top level `now` that only shrank from it: the same boxes in the same order, the
 * same links and shape, and no box larger. Undefined for any other change, which packs it again.
 */
export function held(last: { input: BoxInput; layout: BoxLayout } | null, now: BoxInput): BoxLayout | undefined {
  if (last === null) return undefined;
  const was = last.input;
  if (was.aspect !== now.aspect || was.children.length !== now.children.length) return undefined;
  if (JSON.stringify(was.edges) !== JSON.stringify(now.edges)) return undefined;
  const shrank = now.children.every((child, n) => {
    const before = was.children[n];
    return before?.id === child.id && child.width <= before.width && child.height <= before.height;
  });
  return shrank ? last.layout : undefined;
}
