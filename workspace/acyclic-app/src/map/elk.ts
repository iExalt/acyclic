// ELK as the Map's layout engine, one box at a time. Inside a box, children are layered top to bottom by their links,
// and those with no link in the box stand aside in columns, since they can start any time and would only widen the
// graph's first layer.
// The top level is packed into rows instead: there the children are the plan's largest boxes, layering them stacks
// them in a tall diagonal strip, and its links all run between items inside them, which the Map draws only for the
// selection anyway. Packing keeps only the order it is given, so the top level's boxes are given in dependency order.

import type { ElkNode } from "elkjs/lib/elk-api.js";
import { natural } from "../natural";
import { STRIP } from "./chevron";
import { type BoxInput, type BoxLayout, type LayBox, LEAF, ROOT, type Size } from "./layout";
import type { Point } from "./route";

/** A box's padding, the space between its children, and the wider gap between its graph and the tasks aside. */
const PAD = 12;
const GAP = 24;
const ASIDE = 40;

const LAYERED = {
  "elk.algorithm": "layered",
  "elk.direction": "DOWN",
  "elk.layered.spacing.nodeNodeBetweenLayers": "40",
  "elk.spacing.nodeNode": String(GAP),
  // Each link arrives at its own point, with its own arrowhead; the links leaving a node leave as one trunk, from one
  // port, which splits on the way. Shared on both ends, an outgoing trunk and an incoming one could run as one line, and
  // a link would seem to join two items it doesn't.
  "elk.layered.mergeEdges": "false",
  // Room at the top for the box's title strip.
  "elk.padding": `[top=${STRIP},left=${PAD},bottom=${PAD},right=${PAD}]`,
};

const PACKED = {
  "elk.algorithm": "rectpacking",
  "elk.spacing.nodeNode": "24",
  "elk.padding": "[top=12,left=12,bottom=12,right=12]",
  // Every box keeps the size its own layout gave it, or what it holds would no longer fit it.
  "elk.expandNodes": "false",
  "elk.rectpacking.orderBySize": "false",
};

/** The top level's shape, width over height, on a phone: portrait. A wider screen gives its own. */
export const PORTRAIT = 0.6;

/** A width over height, rounded to a fifth, within what packing boxes into rows can sensibly make. */
export function shapeOf(width: number, height: number): number {
  return Math.min(3, Math.max(PORTRAIT, Math.round((5 * width) / Math.max(1, height)) / 5));
}

/** Anything that lays out an ELK graph: ELK's API in the browser, its bundled build in tests. */
export interface Engine {
  layout(graph: ElkNode): Promise<ElkNode>;
}

/**
 * The children in dependency order: each after everything it waits for, ties by id in natural order, so the boxes of a
 * plan without links come in the order their ids were given (phase-2 before phase-10), and a cycle broken at its least
 * id.
 */
export function dependencyOrder(input: BoxInput): string[] {
  const ids = input.children.map((child) => child.id).sort(natural);
  const waiting = new Map(ids.map((id) => [id, new Set<string>()]));
  for (const edge of input.edges) waiting.get(edge.target)?.add(edge.source);
  const order: string[] = [];
  const left = new Set(ids);
  while (left.size > 0) {
    const ready = [...left].find((id) => [...(waiting.get(id) ?? [])].every((first) => !left.has(first)));
    const next = ready ?? [...left][0] ?? "";
    order.push(next);
    left.delete(next);
  }
  return order;
}

/**
 * A box's layout with its unlinked children aside: the graph as ELK laid it out, or none when nothing in the box is
 * linked, then the unlinked in order in columns to its right. A column holds as many as fit the graph's height, or a
 * near-square grid's when that is taller, so a box of only unlinked tasks is about as tall as it is wide in tasks.
 */
export function beside(graph: BoxLayout | null, unlinked: readonly ({ id: string } & Size)[]): BoxLayout {
  const empty = { width: 2 * PAD, height: STRIP + PAD, at: {} };
  if (unlinked.length === 0) return graph ?? empty;
  const inner = graph === null ? 0 : graph.height - STRIP - PAD;
  const cap = Math.max(inner, Math.ceil(Math.sqrt(unlinked.length)) * (LEAF.height + GAP) - GAP);
  // Filled to the cap, the last column could hold one; so the columns that takes share the children evenly.
  const share = Math.ceil(unlinked.length / stack(unlinked, cap, Infinity, 0, 0).columns);
  const placed = stack(unlinked, cap, share, graph === null ? PAD : graph.width - PAD + ASIDE, STRIP);
  return {
    ...graph,
    width: placed.right + PAD,
    height: Math.max(graph?.height ?? 0, placed.bottom + PAD),
    at: { ...graph?.at, ...placed.at },
  };
}

/** Children in columns from (`left`, `top`), each column no taller than `cap` unless one child is, and at most `share`. */
function stack(children: readonly ({ id: string } & Size)[], cap: number, share: number, left: number, top: number) {
  const at: Record<string, { x: number; y: number }> = {};
  let [x, y, column, count, columns, bottom] = [left, top, 0, 0, 1, top];
  for (const child of children) {
    if (count > 0 && (count === share || y + child.height > top + cap)) {
      [x, y, column, count] = [x + column + GAP, top, 0, 0];
      columns += 1;
    }
    at[child.id] = { x, y };
    column = Math.max(column, child.width);
    bottom = Math.max(bottom, y + child.height);
    y += child.height + GAP;
    count += 1;
  }
  return { at, columns, right: x + column, bottom };
}

export function elkLayBox(engine: Engine): LayBox {
  return async (input) => {
    const size = new Map(input.children.map(({ id, width, height }) => [id, { id, width, height }]));
    if (input.id === ROOT)
      return laid(
        await engine.layout({
          id: "root",
          layoutOptions: { ...PACKED, "elk.aspectRatio": String(input.aspect ?? PORTRAIT) },
          children: dependencyOrder(input).flatMap((id) => size.get(id) ?? []),
          edges: [],
        }),
      );
    const linked = new Set(input.edges.flatMap((edge) => [edge.source, edge.target]));
    const graph =
      linked.size === 0
        ? null
        : laid(
            await engine.layout({
              id: input.id,
              layoutOptions: LAYERED,
              children: [...size.values()]
                .filter((child) => linked.has(child.id))
                .map((child) => ({
                  ...child,
                  layoutOptions: { "elk.portConstraints": "FIXED_SIDE" },
                  ports: [{ id: out(child.id), width: 0, height: 0, layoutOptions: { "elk.port.side": "SOUTH" } }],
                })),
              edges: input.edges.map((edge, i) => ({
                id: `e${i}`,
                sources: [out(edge.source)],
                targets: [edge.target],
              })),
            }),
            input.edges,
          );
    return beside(
      graph,
      input.children.filter((child) => !linked.has(child.id)),
    );
  };
}

/** A node's one way out, which every link leaving it shares. */
const out = (id: string) => `${id}\u0000out`;

function laid(out: ElkNode, edges: readonly { source: string; target: string }[] = []): BoxLayout {
  const at: Record<string, { x: number; y: number }> = {};
  for (const child of out.children ?? []) at[child.id] = { x: child.x ?? 0, y: child.y ?? 0 };
  const routes: Record<string, Point[]> = {};
  const junctions: Record<string, Point[]> = {};
  for (const edge of out.edges ?? []) {
    const section = edge.sections?.[0];
    const link = edges[Number(edge.id.slice(1))];
    if (section === undefined || link === undefined || edge.sections?.length !== 1) continue;
    const key = `${link.source}->${link.target}`;
    routes[key] = [section.startPoint, ...(section.bendPoints ?? []), section.endPoint];
    // Where a trunk splits, on one of the links that split there.
    const at = (edge as { junctionPoints?: Point[] }).junctionPoints;
    if (at !== undefined && at.length > 0) junctions[key] = at.map(({ x, y }) => ({ x, y }));
  }
  if (edges.length === 0) return { width: out.width ?? 0, height: out.height ?? 0, at };
  return { width: out.width ?? 0, height: out.height ?? 0, at, routes, junctions };
}
