// What the Map draws for a laid-out view and the user's selection: every node at its place, the edges worth
// drawing, and which nodes and links the selected thread lights up. Pure, so the rules are tested without a canvas.

import { type Placed, withinOneBox } from "./layout";
import type { Kind, View } from "./model";

/** What the user has picked: an item, a thread, or neither. An item's thread lights up unless a thread is picked. */
export interface Selection {
  item: string | null;
  thread: string | null;
}

export interface SceneNode {
  id: string;
  label: string;
  kind: Kind;
  /** The node's centre, as the renderer places nodes. */
  x: number;
  y: number;
  width: number;
  height: number;
  /** How deep it is drawn, so a box lies under what it holds. */
  layer: number;
  lit: boolean;
  dim: boolean;
  selected: boolean;
}

export interface SceneEdge {
  id: string;
  source: string;
  target: string;
  /** An edge that crosses boxes, drawn only because its end is selected or it joins the lit thread. */
  crossing: boolean;
  /** Joins two of the lit thread's nodes, so the thread reads as a route in its line's colour. */
  lit: boolean;
  /** Outside the lit thread while one is lit. */
  dim: boolean;
}

export interface Scene {
  nodes: SceneNode[];
  edges: SceneEdge[];
  /** The thread lit up, if any. */
  thread: string | null;
}

/**
 * The scene for a view. `threadOf` names each item's thread, and `standIn` the visible node an item is drawn as, so
 * a selected item hidden by a collapse selects the box that hides it.
 */
export function scene(
  view: View,
  placed: Placed,
  selection: Selection,
  threadOf: (id: string) => string | null,
): Scene {
  const thread = selection.thread ?? (selection.item === null ? null : threadOf(selection.item));
  const lit = new Set(thread === null ? [] : (view.threads.get(thread) ?? []));
  const layer = new Map<string, number>();
  const nodes: SceneNode[] = [];
  for (const node of view.nodes) {
    const at = placed.nodes.get(node.id);
    if (at === undefined) continue;
    const depth = node.parent === null ? 0 : (layer.get(node.parent) ?? 0) + 1;
    layer.set(node.id, depth);
    nodes.push({
      id: node.id,
      label: node.kind === "collapsed" ? `${node.title} (+${node.hidden})` : node.title,
      kind: node.kind,
      x: at.x + at.width / 2,
      y: at.y + at.height / 2,
      width: at.width,
      height: at.height,
      layer: depth,
      lit: lit.has(node.id),
      // Dimming says "not in this thread"; a box isn't a member just because it holds members, so it isn't dimmed.
      dim: thread !== null && !lit.has(node.id) && node.kind !== "box",
      selected: node.id === selection.item,
    });
  }
  const within = withinOneBox(view);
  const edges: SceneEdge[] = [];
  for (const edge of view.edges) {
    const crossing = !within(edge);
    const touches = selection.item !== null && (edge.source === selection.item || edge.target === selection.item);
    const joins = lit.has(edge.source) && lit.has(edge.target);
    if (crossing && !touches && !joins) continue;
    edges.push({
      id: `${edge.source}->${edge.target}`,
      source: edge.source,
      target: edge.target,
      crossing,
      lit: joins,
      dim: thread !== null && !joins && !touches,
    });
  }
  return { nodes, edges, thread };
}
