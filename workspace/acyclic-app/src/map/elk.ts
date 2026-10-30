// ELK as the Map's layout engine, one box at a time. Inside a box, children are layered top to bottom by their links.
// The top level is packed into rows instead: there the children are the plan's largest boxes, layering them stacks
// them in a tall diagonal strip, and its links all run between items inside them, which the Map draws only for the
// selection anyway. Packing keeps only the order it is given, so the top level's boxes are given in dependency order.

import type { ElkNode } from "elkjs/lib/elk-api.js";
import { natural } from "../natural";
import { STRIP } from "./chevron";
import { type BoxInput, type LayBox, ROOT } from "./layout";

const LAYERED = {
  "elk.algorithm": "layered",
  "elk.direction": "DOWN",
  "elk.layered.spacing.nodeNodeBetweenLayers": "40",
  "elk.spacing.nodeNode": "24",
  // Room at the top for the box's title strip.
  "elk.padding": `[top=${STRIP},left=12,bottom=12,right=12]`,
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

export function elkLayBox(engine: Engine): LayBox {
  return async (input) => {
    const root = input.id === ROOT;
    const size = new Map(input.children.map((child) => [child.id, child]));
    const children = (root ? dependencyOrder(input) : input.children.map((child) => child.id)).map((id) => ({
      id,
      width: size.get(id)?.width ?? 0,
      height: size.get(id)?.height ?? 0,
    }));
    const graph: ElkNode = {
      id: root ? "root" : input.id,
      layoutOptions: root ? { ...PACKED, "elk.aspectRatio": String(input.aspect ?? PORTRAIT) } : LAYERED,
      children,
      edges: root
        ? []
        : input.edges.map((edge, i) => ({ id: `e${i}`, sources: [edge.source], targets: [edge.target] })),
    };
    const out = await engine.layout(graph);
    const at: Record<string, { x: number; y: number }> = {};
    for (const child of out.children ?? []) at[child.id] = { x: child.x ?? 0, y: child.y ?? 0 };
    return { width: out.width ?? 0, height: out.height ?? 0, at };
  };
}
