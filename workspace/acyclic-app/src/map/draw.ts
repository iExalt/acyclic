// What the renderer holds for a scene, and the least change that takes it from one scene to the next: elements
// added, removed, or with their data, position or classes changed. Redrawing only that keeps a write's redraw small on
// a large plan. Pure but for `apply`, which hands the change to Cytoscape. Nodes are never compound (a box is a plain
// node drawn under what it holds), so moving one never drags others, and an edge's id names its ends, which never
// change in place.

import type { Core, ElementDefinition } from "cytoscape";
import type { Line } from "../lines";
import type { Palette } from "../palette";
import { boxMarkOf, markOf, QUIET } from "./marks";
import type { Scene } from "./scene";
import { TILES } from "./style";

/** An element as the renderer is given it; `data.id` is its id. */
export type Drawn = ElementDefinition & { data: { id: string } & Record<string, unknown>; classes: string };

/**
 * The elements for a scene, with the lit thread's colour, if any, and each tile's marks from its thread's line, drawn
 * in `palette`.
 */
export function elementsOf(
  scene: Scene,
  colour: string | undefined,
  lines: ReadonlyMap<string, Line>,
  palette: Palette,
): Drawn[] {
  const layers = new Map(scene.nodes.map((node) => [node.id, node.layer]));
  const line = colour ?? palette.ink;
  return [
    ...scene.nodes.map((node) => ({
      group: "nodes" as const,
      data: {
        ...node,
        line,
        mark:
          node.kind === "leaf"
            ? markOf(node.stage, threadOf(node.thread, lines), node.width, node.height, palette)
            : boxMarkOf(node.kind, node.stage, node.width, node.height, palette),
      },
      position: { x: node.x, y: node.y },
      classes: [
        node.kind,
        node.lit ? "lit" : "",
        node.dim ? "dim" : "",
        node.selected ? "selected" : "",
        node.archived ? "archived" : "",
        node.stage === null ? "" : `stage-${node.stage}`,
        node.kind !== "leaf" && node.stage !== null && QUIET.has(node.stage) ? "quiet" : "",
        node.kind === "box" && !node.label.includes("\n") ? "one-line" : "",
      ].join(" "),
    })),
    // Over the links, under what they join.
    ...scene.junctions.map((junction) => ({
      group: "nodes" as const,
      // No label, but the width every node's label is wrapped to, so the style's mapper has a number to read.
      data: { id: junction.id, line, textWidth: 0 },
      position: { x: junction.x, y: junction.y },
      // Not "lit": that names the thread's items, which a dot isn't.
      classes: ["junction", junction.lit ? "junction-lit" : "", junction.dim ? "dim" : ""].join(" "),
    })),
    ...scene.edges.map((edge) => ({
      group: "edges" as const,
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        line,
        // A link across boxes is drawn over every box, as it is drawn only for the selected item or the lit thread, to
        // be seen; and under the tiles, so it never runs through a title.
        z: edge.crossing ? TILES - 1 : 2 * Math.min(layers.get(edge.source) ?? 0, layers.get(edge.target) ?? 0) - 1,
        ...edge.route,
      },
      classes: [
        edge.crossing ? "crossing" : "",
        edge.route === null ? "" : "routed",
        edge.lit ? "lit" : "",
        edge.dim ? "dim" : "",
        edge.hovered ? "hovered" : "",
      ].join(" "),
    })),
  ];
}

/** A tile's thread as its marks show it, or null for one in no thread, or in one the plan doesn't list. */
function threadOf(thread: string | null, lines: ReadonlyMap<string, Line>) {
  const line = thread === null ? undefined : lines.get(thread);
  return line === undefined ? null : { colour: line.colour, number: line.number, ink: line.ink };
}

export interface Update {
  id: string;
  /** The data that changed, whole values; `unset` names keys the element no longer has. */
  data: Record<string, unknown> | null;
  unset: string[];
  position: { x: number; y: number } | null;
  classes: string | null;
}

export interface Changes {
  /** Nodes before edges, so each edge's ends exist when it is added. */
  add: Drawn[];
  remove: string[];
  update: Update[];
}

/** The change from the elements drawn, by id, to `next`. */
export function diff(drawn: ReadonlyMap<string, Drawn>, next: Drawn[]): Changes {
  const add: Drawn[] = [];
  const update: Update[] = [];
  const kept = new Set<string>();
  for (const element of next) {
    const id = element.data.id;
    const was = drawn.get(id);
    if (was === undefined) {
      add.push(element);
      continue;
    }
    kept.add(id);
    const data: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(element.data)) if (was.data[key] !== value) data[key] = value;
    const unset = Object.keys(was.data).filter((key) => !(key in element.data));
    const moved =
      element.position !== undefined &&
      (was.position?.x !== element.position.x || was.position?.y !== element.position.y);
    const changed = {
      id,
      data: Object.keys(data).length === 0 ? null : data,
      unset,
      position: moved && element.position !== undefined ? element.position : null,
      classes: was.classes === element.classes ? null : element.classes,
    };
    if (changed.data !== null || unset.length > 0 || changed.position !== null || changed.classes !== null)
      update.push(changed);
  }
  const remove = [...drawn.keys()].filter((id) => !kept.has(id));
  add.sort((a, b) => (a.group === b.group ? 0 : a.group === "nodes" ? -1 : 1));
  return { add, remove, update };
}

/** Takes the renderer from the elements `drawn` to `next`, and returns what it now holds. */
export function apply(core: Core, drawn: ReadonlyMap<string, Drawn>, next: Drawn[]): Map<string, Drawn> {
  const changes = diff(drawn, next);
  core.batch(() => {
    for (const id of changes.remove) core.getElementById(id).remove();
    core.add(changes.add);
    for (const change of changes.update) {
      const element = core.getElementById(change.id);
      if (change.data !== null) element.data(change.data);
      if (change.unset.length > 0) element.removeData(change.unset.join(" "));
      if (change.position !== null) element.position(change.position);
      if (change.classes !== null) element.classes(change.classes);
    }
  });
  // Cytoscape measures a new label only as it next draws, but keeps a label's bounds, and the texture it draws the
  // label from, as first worked out: asked where a node is before then, it kept the old height, and a title that gained
  // a line was drawn squeezed or cut into it until a reload. Measured now, it is drawn whole.
  const relabelled = changes.update.filter((change) => change.data?.label !== undefined);
  if (relabelled.length > 0) {
    const measured = relabelled.reduce((all, change) => all.union(core.getElementById(change.id)), core.collection());
    // A method the types leave out, called only if it is there, so a Cytoscape without it draws as before; false
    // measures even what Cytoscape thinks it has measured.
    (measured as unknown as { recalculateRenderedStyle?(useCache: boolean): void }).recalculateRenderedStyle?.(false);
  }
  return new Map(next.map((element) => [element.data.id, element]));
}
