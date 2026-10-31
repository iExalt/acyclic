// What the Map draws for a laid-out view and the user's selection: every node at its place, the edges worth
// drawing, and which nodes and links the selected thread lights up. Pure, so the rules are tested without a canvas.

import { countsText, type Stage } from "../api";
import { overallStage } from "../stages";
import { cut, type Measure, wrap } from "./fit";
import { type Placed, withinOneBox } from "./layout";
import { BOX_TEXT_LEFT, MARKS_LEFT, MARKS_RIGHT } from "./marks";
import type { Kind, View, VisibleNode } from "./model";
import { type Segments, segmentsOf } from "./route";

/** What the user has picked: an item, a thread, or neither. An item's thread lights up unless a thread is picked. */
export interface Selection {
  item: string | null;
  thread: string | null;
  /** The item under the pointer, whose links show, as the selection's do, while the rest recede. */
  hovered?: string | null;
}

export interface SceneNode {
  id: string;
  /** Its label, a line at a time, each cut to fit `textWidth`. */
  label: string;
  textWidth: number;
  kind: Kind;
  /** A task's stage and thread, which its marks show; null for a box. */
  stage: Stage | null;
  thread: string | null;
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
  /** In an archived subtree, which only shows when asked for. */
  archived: boolean;
}

/** How wide labels are drawn: a task's in the face's regular weight, a box's in bold. */
export interface Fonts {
  regular: Measure;
  bold: Measure;
}

/** Everything fits: for scenes whose labels nobody reads, such as tests of anything else. */
export const UNMEASURED: Fonts = { regular: () => 0, bold: () => 0 };

/** How wide a node's label may be drawn: a task's between its marks, a box's after its chevron. */
export function textWidthOf(kind: Kind, width: number): number {
  return Math.max(40, width - (kind === "leaf" ? MARKS_LEFT + MARKS_RIGHT : BOX_TEXT_LEFT + 10));
}

export interface SceneEdge {
  id: string;
  source: string;
  target: string;
  /** An edge that crosses boxes, drawn only because its end is selected or it joins the lit thread. */
  crossing: boolean;
  /**
   * A link inside one box that a longer way through items in the box already implies, drawn, straight, only for the
   * same reasons as one that crosses boxes.
   */
  implied: boolean;
  /** Its way around the box's other nodes, as the layout routed it; null to draw it straight. */
  route: Segments | null;
  /** Joins two of the lit thread's nodes, so the thread reads as a route in its line's colour. */
  lit: boolean;
  /** Outside the lit thread while one is lit, or not the hovered item's while one is hovered. */
  dim: boolean;
  /** One of the hovered item's links. */
  hovered: boolean;
}

/** A dot where a trunk of links leaving one node splits, lit or dimmed with a link that splits there. */
export interface SceneJunction {
  id: string;
  x: number;
  y: number;
  lit: boolean;
  dim: boolean;
}

export interface Scene {
  nodes: SceneNode[];
  edges: SceneEdge[];
  junctions: SceneJunction[];
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
  fonts: Fonts = UNMEASURED,
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
    const textWidth = textWidthOf(node.kind, at.width);
    nodes.push({
      id: node.id,
      label: labelOf(node, textWidth, fonts),
      textWidth,
      kind: node.kind,
      // A box's, where its work stands as a whole.
      stage: node.kind === "leaf" ? node.stage : node.progress === null ? null : overallStage(node.progress),
      thread: node.kind === "leaf" ? node.thread : null,
      x: at.x + at.width / 2,
      y: at.y + at.height / 2,
      width: at.width,
      height: at.height,
      layer: depth,
      lit: lit.has(node.id),
      // Dimming says "not in this thread"; a box isn't a member just because it holds members, so it isn't dimmed. Nor is
      // the selection, outside the thread or not.
      dim: thread !== null && !lit.has(node.id) && node.kind !== "box" && node.id !== selection.item,
      selected: node.id === selection.item,
      archived: node.archived,
    });
  }
  const within = withinOneBox(view);
  const hovered = selection.hovered ?? null;
  const edges: SceneEdge[] = [];
  const centre = new Map(nodes.map((node) => [node.id, { x: node.x, y: node.y }]));
  for (const edge of view.edges) {
    const id = `${edge.source}->${edge.target}`;
    const crossing = !within(edge);
    const implied = !crossing && placed.redundant.has(id);
    const touches = selection.item !== null && (edge.source === selection.item || edge.target === selection.item);
    const joins = lit.has(edge.source) && lit.has(edge.target);
    // Not one a drawn way already implies: drawn straight, it would cut across the items between.
    const shows = hovered !== null && !implied && (edge.source === hovered || edge.target === hovered);
    if ((crossing || implied) && !touches && !joins && !shows) continue;
    const way = crossing || implied ? undefined : placed.routes.get(id);
    const source = centre.get(edge.source);
    const target = centre.get(edge.target);
    edges.push({
      id,
      source: edge.source,
      target: edge.target,
      crossing,
      implied,
      route: way === undefined || source === undefined || target === undefined ? null : segmentsOf(way, source, target),
      lit: joins,
      dim: (thread !== null && !joins && !touches) || (hovered !== null && !shows),
      hovered: shows,
    });
  }
  // A dot drawn once where several links split, lit if any of them is.
  const junctions = new Map<string, SceneJunction>();
  for (const edge of edges)
    for (const at of edge.route === null ? [] : (placed.junctions.get(edge.id) ?? [])) {
      const id = `junction ${Math.round(at.x)},${Math.round(at.y)}`;
      const was = junctions.get(id);
      junctions.set(id, {
        id,
        x: at.x,
        y: at.y,
        lit: edge.lit || (was?.lit ?? false),
        dim: edge.dim && (was?.dim ?? true),
      });
    }
  return { nodes, edges, junctions: [...junctions.values()], thread };
}

/**
 * A node's label, in lines cut to fit `width`. A task's title takes two lines. A box's takes one, ending, when it is
 * collapsed, with how many items it hides, and a parent's leaves by stage take the line under it.
 */
export function labelOf(
  node: Pick<VisibleNode, "title" | "kind" | "hidden" | "progress">,
  width: number,
  fonts: Fonts,
): string {
  if (node.kind === "leaf") return wrap(node.title, width, 2, fonts.regular).join("\n");
  const hidden = node.kind === "collapsed" ? ` (+${node.hidden})` : "";
  const title = cut(node.title, width - fonts.bold(hidden), fonts.bold) + hidden;
  return node.progress === null ? title : `${title}\n${cut(countsText(node.progress), width, fonts.bold)}`;
}
