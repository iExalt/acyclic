// The plan as the Map draws it: the outcome tree as nested boxes, and waits-for links as edges from the item that
// must come first to the item that waits. Pure functions, tested without a browser.

import type { Item } from "../api";

/** An item, with what the Map reads of it. */
export interface MapItem {
  id: string;
  title: string;
  state: string;
  parent: string | null;
  thread: string | null;
  archived: boolean;
  /** The items this one waits for. */
  waitsFor: string[];
}

export interface Model {
  items: Map<string, MapItem>;
  /** Each item's children, sorted by id, so every view of the same plan is built in the same order. */
  children: Map<string, string[]>;
  roots: string[];
  depth: Map<string, number>;
}

export function buildModel(items: Item[]): Model {
  const byId = new Map<string, MapItem>(
    items.map((item) => [
      item.id,
      {
        id: item.id,
        title: item.title,
        state: item.state,
        parent: item.parent,
        thread: item.thread,
        archived: item.archived,
        waitsFor: [...item.links.waits_for],
      },
    ]),
  );
  const listed = new Map<string, string[]>();
  const roots: string[] = [];
  for (const item of byId.values()) {
    // A parent the plan doesn't hold, such as one deleted elsewhere, makes its child a root.
    if (item.parent !== null && byId.has(item.parent)) {
      const siblings = listed.get(item.parent) ?? [];
      siblings.push(item.id);
      listed.set(item.parent, siblings);
    } else {
      roots.push(item.id);
    }
  }
  for (const siblings of listed.values()) siblings.sort();
  roots.sort();
  // The tree as walked from the roots, each item once. Two replicas can each move one item under the other, and
  // their merge leaves parents in a cycle that no root reaches; the cycle's first item in id order is drawn as a root
  // too, so nothing vanishes and what hangs off the cycle stays under its parent, and the walk drops the link that
  // closes the cycle.
  const children = new Map<string, string[]>();
  const depth = new Map<string, number>();
  const walk = (id: string, d: number) => {
    depth.set(id, d);
    const kids = (listed.get(id) ?? []).filter((child) => !depth.has(child));
    if (kids.length > 0) children.set(id, kids);
    for (const child of kids) if (!depth.has(child)) walk(child, d + 1);
  };
  for (const root of roots) walk(root, 0);
  for (const id of [...byId.keys()].sort()) {
    if (depth.has(id)) continue;
    // Climb from an unreached item until an item repeats: everything from there on is the cycle above it.
    const climbed: string[] = [];
    let at: string | null = id;
    while (at !== null && !climbed.includes(at)) {
      climbed.push(at);
      at = byId.get(at)?.parent ?? null;
    }
    const cycle = at === null ? [id] : climbed.slice(climbed.indexOf(at));
    const root = [...cycle].sort()[0] ?? id;
    roots.push(root);
    walk(root, 0);
  }
  return { items: byId, children, roots, depth };
}

/** What the user has chosen to see: which boxes are collapsed, and whether archived subtrees show. */
export interface ViewState {
  collapsed: ReadonlySet<string>;
  showArchived: boolean;
}

/** The view the Map opens on: the roots open, every box below them collapsed. */
export function defaultView(model: Model): ViewState {
  const collapsed = new Set<string>();
  for (const [id, d] of model.depth) if (d >= 1 && model.children.has(id)) collapsed.add(id);
  return { collapsed, showArchived: false };
}

/**
 * The boxes collapsed once the user's own toggles are applied to the default view. A toggle outlives a refresh, while
 * a box the user hasn't touched, such as one new since the last refresh, keeps the default.
 */
export function collapsedWith(model: Model, toggles: ReadonlyMap<string, boolean>): ReadonlySet<string> {
  const collapsed = new Set(defaultView(model).collapsed);
  for (const [id, closed] of toggles) {
    if (closed) collapsed.add(id);
    else collapsed.delete(id);
  }
  return collapsed;
}

export type Kind = "box" | "collapsed" | "leaf";

export interface VisibleNode {
  id: string;
  title: string;
  state: string;
  /** The visible box it is drawn in, or null at the top. */
  parent: string | null;
  kind: Kind;
  /** How many items a collapsed box hides. */
  hidden: number;
}

/** An edge from the item that must come first to the one that waits for it, between visible nodes. */
export interface VisibleEdge {
  source: string;
  target: string;
}

export interface View {
  /** Parents before their children. */
  nodes: VisibleNode[];
  /** Sorted, without duplicates. */
  edges: VisibleEdge[];
  /** Each thread's visible stand-ins: the members themselves, or the collapsed boxes that hide them. */
  threads: Map<string, string[]>;
}

/**
 * The nodes and edges a view shows. Anything inside a collapsed box is drawn as that box, so edges and thread members
 * lift to their nearest visible ancestor. An archived item and everything under it vanish unless archived subtrees
 * show. Edges that end up inside one node, or between a box and something it contains, are dropped: the box already
 * says the one holds the other.
 */
export function visible(model: Model, state: ViewState): View {
  // The visible item standing in for each item, absent when it is hidden in an archived subtree.
  const stand = new Map<string, string>();
  const nodes: VisibleNode[] = [];
  const shown = (id: string) => state.showArchived || !model.items.get(id)?.archived;
  const count = (id: string): number =>
    (model.children.get(id) ?? []).reduce((sum, child) => (shown(child) ? sum + 1 + count(child) : sum), 0);
  const walk = (id: string, parent: string | null, standIn: string | null) => {
    if (!shown(id)) return;
    if (standIn !== null) {
      stand.set(id, standIn);
      for (const child of model.children.get(id) ?? []) walk(child, parent, standIn);
      return;
    }
    const item = model.items.get(id);
    if (item === undefined) return;
    stand.set(id, id);
    const kids = (model.children.get(id) ?? []).filter(shown);
    const collapsed = kids.length > 0 && state.collapsed.has(id);
    nodes.push({
      id,
      title: item.title,
      state: item.state,
      parent,
      kind: kids.length === 0 ? "leaf" : collapsed ? "collapsed" : "box",
      hidden: collapsed ? count(id) : 0,
    });
    for (const child of kids) walk(child, id, collapsed ? id : null);
  };
  for (const root of model.roots) walk(root, null, null);

  const parentOf = new Map(nodes.map((node) => [node.id, node.parent]));
  const contains = (outer: string, inner: string) => {
    for (let p = parentOf.get(inner) ?? null; p !== null; p = parentOf.get(p) ?? null) if (p === outer) return true;
    return false;
  };
  const keys = new Set<string>();
  for (const item of model.items.values()) {
    const target = stand.get(item.id);
    for (const first of item.waitsFor) {
      const source = stand.get(first);
      if (source === undefined || target === undefined || source === target) continue;
      if (contains(source, target) || contains(target, source)) continue;
      keys.add(`${source}\u0000${target}`);
    }
  }
  const edges = [...keys].sort().map((key) => {
    const [source = "", target = ""] = key.split("\u0000");
    return { source, target };
  });

  const threads = new Map<string, string[]>();
  for (const item of model.items.values()) {
    const at = stand.get(item.id);
    if (item.thread === null || at === undefined) continue;
    const members = threads.get(item.thread) ?? [];
    if (!members.includes(at)) members.push(at);
    threads.set(item.thread, members);
  }
  for (const members of threads.values()) members.sort();
  return { nodes, edges, threads };
}
