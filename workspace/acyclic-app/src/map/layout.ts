// Scoped layout: each box lays out only its own children, as a flat graph, with the edges between them lifted from
// their descendants, and each child box at the size its own layout gave it. A box's layout depends only on its
// signature (its children's ids, kinds and sizes, and its edges, all sorted), so layouts are cached by signature: an
// edit lays out again exactly the boxes whose signature changed, from the inside out, and returning to a view already
// laid out costs nothing. Pure apart from the `LayBox` it is given, which is ELK in the app and a fake in tests.

import { natural } from "../natural";
import type { Kind, View } from "./model";
import { hold } from "./pack";
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
  /** Unlinked top-level leaves, kept left of the packed boxes regardless of thread or stage. */
  aside?: readonly string[];
  /** This box's independently measured title strip, also used above its graph and aside. */
  header?: number;
  /** Minimum outer width measured with the label so this box's own geometry participates in its cache key. */
  minWidth?: number;
}

/** A box's layout: its size, where each child sits within it, and the route of each link between its children. */
export interface BoxLayout extends Size {
  at: Record<string, { x: number; y: number }>;
  /** By `source->target`, from the source's border to the target's; none for a link the engine didn't route. */
  routes?: Record<string, Point[]>;
  /** Where a trunk of links leaving one node splits, by the `source->target` of a link that splits there. */
  junctions?: Record<string, Point[]>;
  /** The line between the children aside and the graph, when the box has both. */
  divider?: Divider;
}

/** A vertical line at `x` from `top` to `bottom`. */
export interface Divider {
  x: number;
  top: number;
  bottom: number;
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
  /** Each source snapshot's original packing, retained across folds and edits that change sizes. */
  private readonly roots = new Map<string, { input: BoxInput; layout: BoxLayout }>();
  /** Last accepted root view, including a held fold, for detecting a shrink in the next source snapshot. */
  private displayed: {
    topology: string;
    source: string;
    input: BoxInput;
    baseline: { input: BoxInput; layout: BoxLayout };
  } | null = null;

  constructor(
    private readonly limit = 4096,
    private readonly rootLimit = 64,
  ) {}

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

  root(input: BoxInput, source: string): { input: BoxInput; layout: BoxLayout } | undefined {
    const key = historyKey(input, source);
    const hit = this.roots.get(key);
    if (hit !== undefined) {
      this.roots.delete(key);
      this.roots.set(key, hit);
    }
    return hit;
  }

  displayedRoot(input: BoxInput) {
    return this.displayed?.topology === rootKey(input) ? this.displayed : undefined;
  }

  rememberRoot(input: BoxInput, source: string, baseline: { input: BoxInput; layout: BoxLayout }) {
    const key = historyKey(input, source);
    this.roots.delete(key);
    this.roots.set(key, baseline);
    while (this.roots.size > this.rootLimit) {
      const oldest = this.roots.keys().next().value;
      if (oldest === undefined) break;
      this.roots.delete(oldest);
    }
    this.displayed = { topology: rootKey(input), source, input, baseline };
  }

  get rootSize() {
    return this.roots.size;
  }
}

/** A root's source snapshot and topology/shape identify its original packing, independent of size and folds. */
function rootKey(input: BoxInput): string {
  return JSON.stringify([input.children.map(({ id }) => id), input.edges, input.aspect, input.aside]);
}

function historyKey(input: BoxInput, source: string): string {
  return JSON.stringify([source, rootKey(input)]);
}

/** A new source is shrink-only when no root child grew in either dimension and at least one got smaller. */
function shrinkOnly(was: BoxInput, now: BoxInput): boolean {
  if (was.children.length !== now.children.length) return false;
  let shrank = false;
  for (const [index, child] of now.children.entries()) {
    const before = was.children[index];
    if (before?.id !== child.id || child.width > before.width || child.height > before.height) return false;
    if (child.width < before.width || child.height < before.height) shrank = true;
  }
  return shrank;
}

/** Where every visible node sits, in the view's own coordinates, and the whole drawing's size. */
export interface Placed extends Size {
  nodes: Map<string, { x: number; y: number } & Size>;
  /** Each routed link's way, by `source->target`, in the same coordinates. */
  routes: Map<string, Point[]>;
  /** Where trunks split, by the `source->target` of a link that splits there, in the same coordinates. */
  junctions: Map<string, Point[]>;
  /** Each box's line between its children aside and its graph, by the box's id, in the same coordinates. */
  dividers: Map<string, Divider>;
  /** The links a longer way inside their box implies, drawn only when asked for. */
  redundant: ReadonlySet<string>;
  /** The boxes laid out afresh for this view, rather than found in the cache. */
  laidOut: string[];
}

/** A child's size: a box's from its own layout, without where its children sit, which its parent doesn't depend on. */
function sizeOf(
  kind: Kind | undefined,
  boxes: Map<string, BoxLayout>,
  id: string,
  sizes: ReadonlyMap<string, Size>,
): Size {
  if (kind === "collapsed" || kind === "leaf") return sizes.get(id) ?? (kind === "collapsed" ? COLLAPSED : LEAF);
  const box = kind === "box" ? boxes.get(id) : undefined;
  return box === undefined ? (sizes.get(id) ?? LEAF) : { width: box.width, height: box.height };
}

/**
 * Lays a view out box by box, from the inside out, through the cache. Boxes of the same height (the longest path to
 * a box with no boxes inside) don't depend on each other, so each height's boxes are asked for at once and the engine
 * never waits for the next request; a box is asked for only once every box inside it has its size.
 */
export async function place(
  view: View,
  cache: LayoutCache,
  layBox: LayBox,
  aspect: number,
  isCurrent: () => boolean = () => true,
  source = "",
  sizes: ReadonlyMap<string, Size> = new Map(),
  headers: ReadonlyMap<string, number> = new Map(),
): Promise<Placed> {
  const scope = scopes(view);
  const rootLinked = new Set((scope.edges.get(ROOT) ?? []).flatMap((edge) => [edge.source, edge.target]));
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
            ...sizeOf(scope.kind.get(child), boxes, child, sizes),
          })),
          edges: scope.edges.get(id) ?? [],
          ...(id === ROOT
            ? {
                aspect,
                aside: (scope.kids.get(ROOT) ?? []).filter(
                  (child) => scope.kind.get(child) === "leaf" && !rootLinked.has(child),
                ),
              }
            : {}),
          header: headers.get(id) ?? 52,
          minWidth: sizes.get(id)?.width ?? 200,
        };
        // The box's own id isn't part of what its layout depends on, so equal content shares one entry.
        const signature = JSON.stringify([
          input.children,
          input.edges,
          input.aspect,
          input.aside,
          input.header ?? 52,
          input.minWidth ?? 200,
        ]);
        // Folds hold their source snapshot's packing. A new source that only shrank packs afresh; Undo can then find
        // the earlier source baseline and recreate a held view, even though that view was never freshly packed.
        const exactRoot = id === ROOT ? cache.root(input, source) : undefined;
        const displayed = id === ROOT ? cache.displayedRoot(input) : undefined;
        const baseline = exactRoot ?? displayed?.baseline;
        const shrink =
          id === ROOT &&
          exactRoot === undefined &&
          displayed !== undefined &&
          displayed.source !== source &&
          shrinkOnly(displayed.input, input);
        let layout = id === ROOT && !shrink && baseline !== undefined ? held(baseline, input) : undefined;
        const heldRoot = layout !== undefined;
        if (layout === undefined) {
          layout = cache.get(signature);
          if (layout === undefined) {
            layout = await layBox(input);
            cache.set(signature, layout);
            laidOut.push(id);
          }
        }
        // ROOT is alone on the final level: after its last await, save the baseline and displayed-size comparator
        // together only if no newer request overtook it. A held result is never promoted as a packing baseline.
        const nextBaseline = heldRoot ? baseline : layout === undefined ? undefined : { input, layout };
        if (id === ROOT && nextBaseline !== undefined && isCurrent()) cache.rememberRoot(input, source, nextBaseline);
        boxes.set(id, layout);
      }),
    );
  }
  const root = boxes.get(ROOT) ?? { width: 0, height: 0, at: {} };
  const nodes = new Map<string, { x: number; y: number } & Size>();
  const routes = new Map<string, Point[]>();
  const junctions = new Map<string, Point[]>();
  const dividers = new Map<string, Divider>();
  const absolute = (id: string, ox: number, oy: number) => {
    const box = boxes.get(id);
    if (box === undefined) return;
    if (box.divider !== undefined)
      dividers.set(id, { x: ox + box.divider.x, top: oy + box.divider.top, bottom: oy + box.divider.bottom });
    const moved = (points: Point[]) => points.map((point) => ({ x: ox + point.x, y: oy + point.y }));
    for (const [key, way] of Object.entries(box.routes ?? {})) routes.set(key, moved(way));
    for (const [key, at] of Object.entries(box.junctions ?? {})) junctions.set(key, moved(at));
    for (const child of scope.kids.get(id) ?? []) {
      const at = box.at[child] ?? { x: 0, y: 0 };
      const x = ox + at.x;
      const y = oy + at.y;
      nodes.set(child, { x, y, ...sizeOf(scope.kind.get(child), boxes, child, sizes) });
      if (scope.kind.get(child) === "box") absolute(child, x, y);
    }
  };
  absolute(ROOT, 0, 0);
  return {
    width: root.width,
    height: root.height,
    nodes,
    routes,
    junctions,
    dividers,
    redundant: scope.redundant,
    laidOut,
  };
}

/**
 * The top level's last packing, held for a top level `now` of the same boxes in the same order, with the same links and
 * shape: each box where it was packed, but for what one larger than it was packed at now covers. Undefined for any
 * other change, which packs it again.
 */
export function held(last: { input: BoxInput; layout: BoxLayout } | null, now: BoxInput): BoxLayout | undefined {
  if (last === null) return undefined;
  const was = last.input;
  if (was.aspect !== now.aspect || was.children.length !== now.children.length) return undefined;
  if (JSON.stringify(was.aside) !== JSON.stringify(now.aside)) return undefined;
  if (JSON.stringify(was.edges) !== JSON.stringify(now.edges)) return undefined;
  if (!now.children.every((child, n) => was.children[n]?.id === child.id)) return undefined;
  const waits = new Map<string, string[]>();
  for (const { source, target } of now.edges) waits.set(target, [...(waits.get(target) ?? []), source]);
  const layout = hold({ children: was.children, layout: last.layout }, now.children, waits);
  if (layout === undefined || last.layout.divider === undefined) return layout;
  const aside = new Set(now.aside);
  let right = 12;
  let left = Number.POSITIVE_INFINITY;
  for (const child of now.children) {
    const at = layout.at[child.id];
    if (at === undefined) return undefined;
    if (aside.has(child.id)) right = Math.max(right, at.x + child.width);
    else left = Math.min(left, at.x);
  }
  // A wider aside must clear the entire forest, including boxes below its footprint.
  const dx = Math.max(0, right + 40 - left);
  for (const [id, at] of Object.entries(layout.at)) if (!aside.has(id)) at.x += dx;
  return { ...layout, width: layout.width + dx, divider: { x: right + 20, top: 12, bottom: layout.height - 12 } };
}
