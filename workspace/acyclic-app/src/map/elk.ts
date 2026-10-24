// ELK as the Map's layout engine: one box at a time, as a flat layered graph, top to bottom.

import type { ElkNode } from "elkjs/lib/elk-api.js";
import type { LayBox } from "./layout";

const OPTIONS = {
  "elk.algorithm": "layered",
  "elk.direction": "DOWN",
  "elk.layered.spacing.nodeNodeBetweenLayers": "40",
  "elk.spacing.nodeNode": "24",
  // Room at the top for the box's title.
  "elk.padding": "[top=36,left=12,bottom=12,right=12]",
};

/** Anything that lays out an ELK graph: ELK's API in the browser, its bundled build in tests. */
export interface Engine {
  layout(graph: ElkNode): Promise<ElkNode>;
}

export function elkLayBox(engine: Engine): LayBox {
  return async (input) => {
    const graph: ElkNode = {
      id: input.id || "root",
      layoutOptions: OPTIONS,
      children: input.children.map(({ id, width, height }) => ({ id, width, height })),
      edges: input.edges.map((edge, i) => ({ id: `e${i}`, sources: [edge.source], targets: [edge.target] })),
    };
    const out = await engine.layout(graph);
    const at: Record<string, { x: number; y: number }> = {};
    for (const child of out.children ?? []) at[child.id] = { x: child.x ?? 0, y: child.y ?? 0 };
    return { width: out.width ?? 0, height: out.height ?? 0, at };
  };
}
