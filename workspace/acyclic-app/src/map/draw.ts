// What the renderer holds for a scene, and the least change that takes it from one scene to the next: elements
// added, removed, or with their data, position or classes changed. Redrawing only that keeps a write's redraw small on
// a large plan. Pure but for `apply`, which hands the change to Cytoscape. Nodes are never compound (a box is a plain
// node drawn under what it holds), so moving one never drags others, and an edge's id names its ends, which never
// change in place.

import type { BoundingBox12, BoundingBoxWH, Core, ElementDefinition } from "cytoscape";
import type { Line } from "../lines";
import type { Palette } from "../palette";
import { boxMarkOf, boxStageOf, markOf, QUIET } from "./marks";
import { track } from "./motion";
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
    ...scene.nodes.map(({ lit, selected, ...node }) => ({
      group: "nodes" as const,
      data: {
        ...node,
        ...(lit ? { line } : {}),
        mark:
          node.kind === "leaf"
            ? markOf(node.stage, threadOf(node.thread, lines), node.width, node.height, palette, node.reserve)
            : boxMarkOf(node.kind, null, 54, node.height, palette, node.header, node.reserve),
        ...(node.kind === "leaf" ? {} : { stageMark: boxStageOf(node.stage, palette) }),
      },
      position: { x: node.x, y: node.y },
      classes: [
        node.kind,
        lit ? "lit" : "",
        selected ? "selected" : "",
        node.stage === null ? "" : `stage-${node.stage}`,
        node.kind !== "leaf" && node.stage !== null && QUIET.has(node.stage) ? "quiet" : "",
        node.kind === "box" && !node.label.includes("\n") ? "one-line" : "",
      ].join(" "),
    })),
    // On its tile, which it is drawn over, as a dashed chip.
    ...scene.stubs.map((stub) => ({
      group: "nodes" as const,
      data: {
        id: stub.id,
        label: stub.text,
        // The chip's own width is the room its text may take, as no label is wrapped again.
        textWidth: stub.width,
        width: stub.width,
        height: stub.height,
        layer: stub.layer,
        node: stub.node,
        owner: stub.node,
        target: stub.target,
        full: stub.full,
      },
      position: { x: stub.x, y: stub.y },
      classes: "stub",
    })),
    // Over the links, under what they join.
    ...scene.junctions.map((junction) => ({
      group: "nodes" as const,
      // No label, but the width every node's label is wrapped to, so the style's mapper has a number to read.
      data: { id: junction.id, owners: junction.owners, ...(junction.lit ? { line } : {}), textWidth: 0 },
      position: { x: junction.x, y: junction.y },
      // Not "lit": that names the thread's items, which a dot isn't.
      classes: ["junction", junction.lit ? "junction-lit" : ""].join(" "),
    })),
    // Over its box, under everything in it.
    ...scene.dividers.map((divider) => ({
      group: "nodes" as const,
      data: {
        id: divider.id,
        owner: divider.owner,
        layer: divider.layer,
        height: divider.bottom - divider.top,
        textWidth: 0,
      },
      position: { x: divider.x, y: (divider.top + divider.bottom) / 2 },
      classes: "divider",
    })),
    ...scene.edges.map((edge) => ({
      group: "edges" as const,
      data: {
        id: edge.id,
        source: edge.source,
        target: edge.target,
        ...(edge.lit ? { line } : {}),
        // A link across boxes is drawn over every box, as it is drawn only for the selected item or the lit thread, to
        // be seen; and under the tiles, so it never runs through a title.
        z: edge.crossing ? TILES - 1 : 2 * Math.min(layers.get(edge.source) ?? 0, layers.get(edge.target) ?? 0) - 1,
        ...edge.route,
      },
      classes: [
        edge.crossing ? "crossing" : "",
        edge.route === null ? "" : "routed",
        edge.lit ? "lit" : "",
        edge.hovered ? "hovered" : "",
      ].join(" "),
    })),
  ];
}

/**
 * A tile's thread as its marks show it, or null for one in no thread, in a thread of one (a task alone, which has no
 * thread colour, as one in no thread has none), or in one the plan doesn't list.
 */
function threadOf(thread: string | null, lines: ReadonlyMap<string, Line>) {
  const line = thread === null ? undefined : lines.get(thread);
  return line === undefined || line.task !== null ? null : { colour: line.colour, ink: line.ink };
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
    for (const [key, value] of Object.entries(element.data)) {
      const previous = was.data[key];
      // A scene rebuilds its junction owner lists even when their links have not changed.
      if (
        previous === value ||
        (key === "owners" &&
          Array.isArray(previous) &&
          Array.isArray(value) &&
          previous.length === value.length &&
          previous.every((owner, index) => owner === value[index]))
      )
        continue;
      data[key] = value;
    }
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

/** How long a tile takes to glide to its new place. */
export const GLIDE_MS = 200;
/** A frame grows first; its contents appear at the end, with no fade. */
export const FRAME_MS = 160;
/** A link's fade back in: it waits this long, then takes `FADE_MS`, so it is whole as the tiles settle. */
const FADE_DELAY_MS = 50;
const FADE_MS = 150;
/** The most tiles that glide in one write; a write that would move more puts them all in place at once. */
export const MAX_GLIDES = 300;

interface Rect {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** How a write's moves are shown: `extent` is what the viewport covers, in the Map's own coordinates. */
export interface Glide {
  reduced: boolean;
  extent: Rect | null;
}

/** Whether the user asks for less motion. `match` is where the browser's `matchMedia` goes. */
export function prefersReducedMotion(
  match: ((query: string) => { matches: boolean }) | undefined = globalThis.matchMedia?.bind(globalThis),
): boolean {
  return match?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

interface Moving {
  target: { x: number; y: number };
  release: () => void;
  size?: { width: number; height: number };
  growing?: boolean;
}

/** What is in motion on a renderer: tiles gliding, by id, to their targets, and links fading back in. */
interface Motion {
  moving: Map<string, Moving>;
  fading: Map<string, () => void>;
  hidden: Set<string>;
  scene: ReadonlyMap<string, Drawn>;
}

const motions = new WeakMap<Core, Motion>();
const motionOf = (core: Core): Motion => {
  let motion = motions.get(core);
  if (motion === undefined) {
    motion = { moving: new Map(), fading: new Map(), hidden: new Set(), scene: new Map() };
    motions.set(core, motion);
  }
  return motion;
};

/** Whether a tile of this size, moving from `from` to `to`, is in view at either end. No extent: everything is. */
export function touchesView(
  extent: Rect | null,
  from: { x: number; y: number },
  to: { x: number; y: number },
  width: number,
  height: number,
): boolean {
  if (extent === null) return true;
  return [from, to].some(
    (at) =>
      at.x + width / 2 >= extent.x1 &&
      at.x - width / 2 <= extent.x2 &&
      at.y + height / 2 >= extent.y1 &&
      at.y - height / 2 <= extent.y2,
  );
}

function stopMoving(core: Core, id: string) {
  const motion = motionOf(core);
  const running = motion.moving.get(id);
  if (running === undefined) return;
  motion.moving.delete(id);
  core.getElementById(id).stop(true, false);
  if (running.size !== undefined) core.getElementById(id).removeStyle("width height");
  running.release();
}

function stopFading(core: Core, id: string) {
  const motion = motionOf(core);
  const release = motion.fading.get(id);
  if (release === undefined) return;
  motion.fading.delete(id);
  const element = core.getElementById(id);
  element.stop(true, false);
  element.removeStyle("opacity");
  release();
}

/** Shows newly added contents only after every growing frame above them has landed. */
function revealReady(core: Core) {
  const motion = motionOf(core);
  const growing = (id: unknown): boolean => {
    const seen = new Set<string>();
    while (typeof id === "string" && !seen.has(id)) {
      seen.add(id);
      if (motion.moving.get(id)?.growing) return true;
      id = motion.scene.get(id)?.data.owner;
    }
    return false;
  };
  for (const id of [...motion.hidden]) {
    const data = motion.scene.get(id)?.data;
    const waiting =
      data !== undefined &&
      (growing(data.owner) ||
        growing(data.source) ||
        growing(data.target) ||
        (Array.isArray(data.owners) && data.owners.some(growing)));
    if (!waiting) {
      core.getElementById(id).removeStyle("visibility");
      motion.hidden.delete(id);
    }
  }
}

/** One native tween keeps the centre and dimensions on exactly the same easing clock. */
function glideTo(core: Core, id: string, target: { x: number; y: number }, size?: { width: number; height: number }) {
  const motion = motionOf(core);
  const element = core.getElementById(id);
  const release = track();
  const running: Moving = {
    target,
    release,
    ...(size === undefined
      ? {}
      : {
          size,
          growing: size.width > element.width() || size.height > element.height(),
        }),
  };
  motion.moving.set(id, running);
  element.animate(
    { position: { ...target }, ...(size === undefined ? {} : { style: size }) },
    {
      duration: size === undefined ? GLIDE_MS : FRAME_MS,
      // Cytoscape accepts parameters here; its types only list the bare function name.
      easing: size === undefined ? "ease-out" : ("cubic-bezier(0.2,0,0,1)" as "cubic-bezier"),
      complete: () => {
        if (motion.moving.get(id) === running) {
          motion.moving.delete(id);
          if (size !== undefined) element.removeStyle("width height");
          revealReady(core);
        }
        release();
      },
    },
  );
}

/** Hides a link, divider or junction that has just taken its new data, and brings it back as the tiles settle. */
function fadeIn(core: Core, id: string) {
  stopFading(core, id);
  const element = core.getElementById(id);
  const shown = Number(element.style("opacity"));
  element.style("opacity", 0);
  const release = track();
  const motion = motionOf(core);
  motion.fading.set(id, release);
  element.delay(FADE_DELAY_MS).animate(
    { style: { opacity: Number.isFinite(shown) ? shown : 1 } },
    {
      duration: FADE_MS,
      complete: () => {
        if (motion.fading.get(id) !== release) return;
        motion.fading.delete(id);
        element.removeStyle("opacity");
        release();
      },
    },
  );
}

/** Stops everything in motion on a renderer that is about to be destroyed. */
export function settle(core: Core) {
  const motion = motionOf(core);
  for (const id of [...motion.moving.keys()]) stopMoving(core, id);
  for (const id of [...motion.fading.keys()]) stopFading(core, id);
  for (const id of motion.hidden) core.getElementById(id).removeStyle("visibility");
  motion.hidden.clear();
}

/** Puts every gliding tile at its target at once, for what must measure the plan as it will rest. */
export function land(core: Core) {
  const motion = motionOf(core);
  for (const [id, running] of [...motion.moving]) {
    stopMoving(core, id);
    core.getElementById(id).position(running.target);
  }
  revealReady(core);
}

/** Where a node is on screen once it has glided to its target: what the camera is held or shown against. */
export function settledBox(core: Core, id: string): BoundingBox12 & BoundingBoxWH {
  const node = core.getElementById(id);
  const box = node.renderedBoundingBox({ includeLabels: false });
  const moving = motionOf(core).moving.get(id);
  const target = moving?.target;
  if (target === undefined) return box;
  const now = node.position();
  const dx = (target.x - now.x) * core.zoom();
  const dy = (target.y - now.y) * core.zoom();
  const dw = moving?.size === undefined ? 0 : (moving.size.width - node.width()) * core.zoom();
  const dh = moving?.size === undefined ? 0 : (moving.size.height - node.height()) * core.zoom();
  return {
    ...box,
    x1: box.x1 + dx - dw / 2,
    x2: box.x2 + dx + dw / 2,
    y1: box.y1 + dy - dh / 2,
    y2: box.y2 + dy + dh / 2,
    w: box.w + dw,
    h: box.h + dh,
  };
}

const isPlain = (classes: string) => /\b(junction|divider|stub)\b/.test(classes);

/**
 * Takes the renderer from the elements `drawn` to `next`, and returns what it now holds, with every position its
 * target. With a `glide`, tiles whose move touches the view glide there, from where they are drawn now if they were
 * already on their way. A resizing frame tweens its size with its centre. A link, junction or divider whose tile glides is hidden as it takes its
 * new shape, and fades back in. Without one, everything moves at once.
 */
export function apply(core: Core, drawn: ReadonlyMap<string, Drawn>, next: Drawn[], glide?: Glide): Map<string, Drawn> {
  const changes = diff(drawn, next);
  const motion = motionOf(core);
  const to = new Map(next.map((element) => [element.data.id, element]));
  const glides = new Map<string, { x: number; y: number }>();
  const candidates = new Map<string, { x: number; y: number }>();
  motion.scene = to;
  const sizes = new Map<string, { width: number; height: number }>();
  const fold = changes.update.some(
    (update) =>
      update.classes !== null &&
      /\b(box|collapsed)\b/.test(update.classes) &&
      /\bbox\b/.test(drawn.get(update.id)?.classes ?? "") !== /\bbox\b/.test(update.classes),
  );
  const hidden = new Set<string>();
  const edges: Update[] = [];
  core.batch(() => {
    for (const id of changes.remove) {
      stopMoving(core, id);
      stopFading(core, id);
      motion.hidden.delete(id);
      core.getElementById(id).remove();
    }
    // Cytoscape keeps its input objects: copy them so motion and data writes leave earlier scene snapshots intact.
    core.add(
      changes.add.map((one) => ({
        ...one,
        data: { ...one.data },
        ...(one.position === undefined ? {} : { position: { ...one.position } }),
      })),
    );
    const change = (update: Update) => {
      const element = core.getElementById(update.id);
      if (update.data !== null) element.data(update.data);
      if (update.unset.length > 0) element.removeData(update.unset.join(" "));
      if (update.classes !== null) element.classes(update.classes);
      return element;
    };
    for (const update of changes.update) {
      if (to.get(update.id)?.group === "edges") {
        edges.push(update);
        continue;
      }
      const element = core.getElementById(update.id);
      const data = to.get(update.id)?.data;
      const was = drawn.get(update.id);
      const frame = /\b(box|collapsed)\b/.test(to.get(update.id)?.classes ?? "");
      const resized =
        frame &&
        typeof data?.width === "number" &&
        typeof data.height === "number" &&
        (data.width !== was?.data.width || data.height !== was?.data.height);
      const size =
        resized || (update.position !== null && motion.moving.get(update.id)?.size !== undefined)
          ? { width: Number(data?.width), height: Number(data?.height) }
          : undefined;
      const from =
        size !== undefined && glide !== undefined && !glide.reduced
          ? { width: element.width(), height: element.height() }
          : null;
      const target = update.position ?? to.get(update.id)?.position;
      if (update.position !== null || resized) stopMoving(core, update.id);
      change(update);
      if ((update.position === null && !resized) || target === undefined) continue;
      if (isPlain(to.get(update.id)?.classes ?? "")) {
        element.position(target);
        hidden.add(update.id);
        continue;
      }
      const width = typeof data?.width === "number" ? data.width : element.width();
      const height = typeof data?.height === "number" ? data.height : element.height();
      if (
        glide !== undefined &&
        !glide.reduced &&
        touchesView(glide.extent, element.position(), target, width, height)
      ) {
        candidates.set(update.id, { ...target });
        if (size !== undefined && from !== null) {
          sizes.set(update.id, size);
          element.style(from);
        }
      } else element.position(target);
    }
    // A mass move costs more than it shows, as every tile is an image: past the cap, all of it snaps.
    if (candidates.size > MAX_GLIDES)
      for (const [id, at] of candidates) {
        core.getElementById(id).position(at);
        if (sizes.has(id)) core.getElementById(id).removeStyle("width height");
      }
    else for (const [id, at] of candidates) glides.set(id, at);
    const gliding = (id: unknown) => glides.has(String(id));
    for (const update of edges) {
      const element = change(update);
      const ends = to.get(update.id)?.data;
      if (gliding(ends?.source) || gliding(ends?.target)) hidden.add(update.id);
      else if (element.isEdge()) stopFading(core, update.id);
    }
    // What this write adds is drawn only once its tiles have settled, rather than at its final route between tiles
    // still on their way.
    for (const added of changes.add) {
      const ends = added.data;
      if (added.group === "edges" ? gliding(ends.source) || gliding(ends.target) : isPlain(added.classes))
        hidden.add(added.data.id);
    }
  });
  // Started once the batch has settled, so a tile starts from where it is drawn.
  for (const [id, target] of glides) glideTo(core, id, target, sizes.get(id));
  if ([...motion.moving.values()].some((moving) => moving.growing))
    for (const added of changes.add) motion.hidden.add(added.data.id);
  revealReady(core);
  for (const id of motion.hidden) {
    stopFading(core, id);
    core.getElementById(id).style("visibility", "hidden");
  }
  if (glides.size > 0 && !fold)
    for (const id of hidden) {
      if (!motion.hidden.has(id)) fadeIn(core, id);
    }
  else for (const id of hidden) stopFading(core, id);
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
  return to;
}
