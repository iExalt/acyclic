// Where the Map looks: first, at a zoom its titles can be read at, and then, how far to pan so a node shows in the
// part of the Map nothing covers, such as above the sheet on a phone or left of the pane on a laptop. Pure, so the
// rules are tested without a canvas.

export interface Box {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** The pan, in screen pixels, that brings `node` inside `open` with `margin` to spare: none if it is already in. */
export function reveal(node: Box, open: Box, margin = 8): { x: number; y: number } {
  const along = (start: number, end: number, from: number, to: number) => {
    // Too big to fit: show its start, where its title is.
    if (end - start > to - from - 2 * margin) return from + margin - start;
    if (start < from + margin) return from + margin - start;
    if (end > to - margin) return to - margin - end;
    return 0;
  };
  return { x: along(node.x1, node.x2, open.x1, open.x2), y: along(node.y1, node.y2, open.y1, open.y2) };
}

/** The part of a map `width` by `height` that a sheet or pane at `cover` (relative to the map) leaves open. */
export function uncovered(width: number, height: number, cover: Box | null): Box {
  const all = { x1: 0, y1: 0, x2: width, y2: height };
  if (cover === null || cover.x2 <= 0 || cover.y2 <= 0 || cover.x1 >= width || cover.y1 >= height) return all;
  // A sheet across the whole width covers the bottom; anything narrower is a pane on the right.
  if (cover.x2 - cover.x1 >= width * 0.9) return { ...all, y2: Math.max(0, cover.y1) };
  return { ...all, x2: Math.max(0, cover.x1) };
}

/** The furthest the first view zooms out: 13 px titles drawn at 11 px, about the least a phone's reader can read. */
export const READABLE = 0.85;

/**
 * The first view of a plan `plan` big on a screen `screen` big: the whole plan, centred, when it fits at a readable
 * zoom, else at that zoom with `start` at the top-left, the plan's first item, so the screen opens on titles rather
 * than on the empty corner of a box. A pinch still zooms out to the whole.
 */
export function firstView(
  plan: { width: number; height: number },
  screen: { width: number; height: number },
  padding: number,
  maxZoom: number,
  start: { x: number; y: number } = { x: 0, y: 0 },
): { zoom: number; pan: { x: number; y: number } } {
  const fit = Math.min(
    (screen.width - 2 * padding) / Math.max(1, plan.width),
    (screen.height - 2 * padding) / Math.max(1, plan.height),
  );
  if (fit < READABLE)
    return { zoom: READABLE, pan: { x: padding - start.x * READABLE, y: padding - start.y * READABLE } };
  const zoom = Math.min(maxZoom, fit);
  return { zoom, pan: { x: (screen.width - plan.width * zoom) / 2, y: (screen.height - plan.height * zoom) / 2 } };
}

/** The top-left of the first of `nodes` in reading order: the highest, and of those, the leftmost. */
export function firstOf(nodes: Iterable<{ x: number; y: number }>): { x: number; y: number } | undefined {
  let first: { x: number; y: number } | undefined;
  for (const node of nodes)
    if (first === undefined || node.y < first.y || (node.y === first.y && node.x < first.x))
      first = { x: node.x, y: node.y };
  return first;
}

/** Of `nodes` on screen, the one to bring into `open`: none if any shows there already, else the nearest its centre. */
export function nearest(nodes: Box[], open: Box): Box | null {
  const shows = (b: Box) => b.x2 > open.x1 && b.x1 < open.x2 && b.y2 > open.y1 && b.y1 < open.y2;
  if (nodes.length === 0 || nodes.some(shows)) return null;
  const cx = (open.x1 + open.x2) / 2;
  const cy = (open.y1 + open.y2) / 2;
  const away = (b: Box) => Math.hypot((b.x1 + b.x2) / 2 - cx, (b.y1 + b.y2) / 2 - cy);
  return nodes.reduce((best, b) => (away(b) < away(best) ? b : best));
}
