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

/**
 * Whether resizing the Map from `was` to `now` (each a width and height) covers `node`, clear of the sheet or pane at
 * `cover` before: a strip taken from the Map's foot, or a window turned, can hide the selection, which is then shown
 * again; one already hidden, or panned away by the user, is left where it is. The panels sit at the Map's foot or right
 * edge, so before the resize the cover is taken to have sat as far from the top-left as the Map has shrunk: exact for a
 * pane and for a sheet of fixed height. A half sheet, which shrinks with the Map, is put a little low, so a selection
 * already partly under its edge counts as newly hidden and is shown, which errs the harmless way.
 */
export function newlyCovered(
  node: Box,
  was: { width: number; height: number },
  now: { width: number; height: number },
  cover: Box | null,
): boolean {
  const dx = was.width - now.width;
  const dy = was.height - now.height;
  const before = cover === null ? null : { x1: cover.x1 + dx, y1: cover.y1 + dy, x2: cover.x2 + dx, y2: cover.y2 + dy };
  const still = (pan: { x: number; y: number }) => pan.x === 0 && pan.y === 0;
  return (
    still(reveal(node, uncovered(was.width, was.height, before))) &&
    !still(reveal(node, uncovered(now.width, now.height, cover)))
  );
}

/** The furthest the first view zooms out: 13 px titles drawn at 11 px, about the least a phone's reader can read. */
export const READABLE = 0.85;

/**
 * The first view of a plan `plan` big on a screen `screen` big: the whole plan, centred, when it fits at `least` or
 * closer, else at `least` with `start` at the top-left, the plan's first item, so the screen opens on titles rather
 * than on the empty corner of a box. A pinch still zooms out to the whole.
 */
export function firstView(
  plan: { width: number; height: number },
  screen: { width: number; height: number },
  padding: number,
  maxZoom: number,
  start: { x: number; y: number } = { x: 0, y: 0 },
  least = READABLE,
): { zoom: number; pan: { x: number; y: number } } {
  const fit = Math.min(
    (screen.width - 2 * padding) / Math.max(1, plan.width),
    (screen.height - 2 * padding) / Math.max(1, plan.height),
  );
  if (fit < least) return { zoom: least, pan: { x: padding - start.x * least, y: padding - start.y * least } };
  const zoom = Math.min(maxZoom, fit);
  return { zoom, pan: { x: (screen.width - plan.width * zoom) / 2, y: (screen.height - plan.height * zoom) / 2 } };
}

type Placed = { x: number; y: number; width: number; height: number };

/**
 * Where the first view starts when the plan is too big to show whole, of `nodes` (top-left corners, `box` for those
 * holding others, `top` for those at the top level): the first top-level node's corner, where a box's title is, moved
 * right only as far as shows its first item whole on a screen `width` wide at `zoom`. Starting at the item itself
 * would cut off whatever of its box lies left of it.
 */
export function startOf(
  nodes: readonly (Placed & { box: boolean; top: boolean })[],
  width: number,
  padding: number,
  zoom: number,
): { x: number; y: number } | undefined {
  const top = firstOf(nodes.filter((node) => node.top));
  const box = nodes.find((node) => node.top && node.x === top?.x && node.y === top?.y);
  if (box === undefined) return undefined;
  const within = (node: Placed) =>
    node.x >= box.x &&
    node.y >= box.y &&
    node.x + node.width <= box.x + box.width &&
    node.y + node.height <= box.y + box.height;
  const corner = firstOf(nodes.filter((node) => !node.box && within(node)));
  const item = nodes.find((node) => !node.box && node.x === corner?.x && node.y === corner?.y);
  if (item === undefined) return { x: box.x, y: box.y };
  const seen = (width - 2 * padding) / zoom;
  return { x: Math.min(item.x, Math.max(box.x, item.x + item.width - seen)), y: box.y };
}

/** How far apart two tops can be and still read as one row. */
const ROW = 8;

/** The top-left of the first of `nodes` in reading order: in the highest row, the leftmost. */
export function firstOf(nodes: Iterable<{ x: number; y: number }>): { x: number; y: number } | undefined {
  let first: { x: number; y: number } | undefined;
  for (const node of nodes)
    if (first === undefined || node.y < first.y - ROW || (Math.abs(node.y - first.y) <= ROW && node.x < first.x))
      first = { x: node.x, y: node.y };
  return first;
}

/** Of `nodes`, as drawn, the one to bring into `open`: none if any shows there already, else the nearest its centre. */
export function nearest(nodes: Box[], open: Box): Box | null {
  const shows = (b: Box) => b.x2 > open.x1 && b.x1 < open.x2 && b.y2 > open.y1 && b.y1 < open.y2;
  if (nodes.length === 0 || nodes.some(shows)) return null;
  const cx = (open.x1 + open.x2) / 2;
  const cy = (open.y1 + open.y2) / 2;
  const away = (b: Box) => Math.hypot((b.x1 + b.x2) / 2 - cx, (b.y1 + b.y2) / 2 - cy);
  return nodes.reduce((best, b) => (away(b) < away(best) ? b : best));
}

/**
 * `boxes` with, for each, the corner of the innermost of `holders` around it, down to the foot of its title strip,
 * `strip` tall, so a frame of a thread's items starts at the titles of the boxes they are in rather than cutting them at
 * the Map's edge.
 */
export function headed(boxes: readonly Box[], holders: readonly Box[], strip: number): Box[] {
  const area = (b: Box) => (b.x2 - b.x1) * (b.y2 - b.y1);
  const around = (inner: Box, outer: Box) =>
    outer !== inner && outer.x1 <= inner.x1 && outer.y1 <= inner.y1 && outer.x2 >= inner.x2 && outer.y2 >= inner.y2;
  const titles = boxes.flatMap((box) => {
    const holder = holders
      .filter((one) => around(box, one))
      .reduce<Box | null>((best, one) => (best === null || area(one) < area(best) ? one : best), null);
    return holder === null ? [] : [{ x1: holder.x1, y1: holder.y1, x2: holder.x1, y2: holder.y1 + strip }];
  });
  return [...boxes, ...titles];
}

/** Whether `frame` would show all of `boxes` together, at a zoom titles can be read at. */
export function framesAll(boxes: readonly Box[], open: Box, padding = 24): boolean {
  return boxes.length > 0 && fitOf(boxes, open, padding) >= READABLE;
}

/** The closest zoom that shows all of `boxes` in `open`, `padding` from its edges. */
function fitOf(boxes: readonly Box[], open: Box, padding: number): number {
  const all = spanOf(boxes);
  return Math.min(
    (open.x2 - open.x1 - 2 * padding) / Math.max(1, all.x2 - all.x1),
    (open.y2 - open.y1 - 2 * padding) / Math.max(1, all.y2 - all.y1),
  );
}

function spanOf(boxes: readonly Box[]): Box {
  return {
    x1: Math.min(...boxes.map((b) => b.x1)),
    y1: Math.min(...boxes.map((b) => b.y1)),
    x2: Math.max(...boxes.map((b) => b.x2)),
    y2: Math.max(...boxes.map((b) => b.y2)),
  };
}

/**
 * The view that frames `boxes`, in the Map's own units, in `open`, the part of the screen nothing covers: all of them
 * at the closest zoom up to `most` that fits them, when that is readable, centred; else at a readable zoom from the
 * first of them in reading order, at the top-left of `open`. Either way moved, if need be, so no edge of the `plan`
 * stands inside `open` with blank canvas beyond it.
 */
export function frame(
  boxes: readonly Box[],
  plan: Box,
  open: Box,
  most: number,
  padding = 24,
): { zoom: number; pan: { x: number; y: number } } | null {
  if (boxes.length === 0) return null;
  const all = spanOf(boxes);
  const [width, height] = [open.x2 - open.x1, open.y2 - open.y1];
  const fit = fitOf(boxes, open, padding);
  // Held between the plan's far edge at the open part's far side and its near edge at the near side, so no blank canvas
  // shows beyond the plan; a plan smaller than the open part stays where it was put.
  const held = (zoom: number, pan: { x: number; y: number }) => {
    const along = (at: number, near: number, far: number, from: number, to: number) => {
      const least = to - padding - far * zoom;
      const upper = from + padding - near * zoom;
      return least > upper ? at : Math.min(upper, Math.max(least, at));
    };
    return {
      zoom,
      pan: {
        x: along(pan.x, plan.x1, plan.x2, open.x1, open.x2),
        y: along(pan.y, plan.y1, plan.y2, open.y1, open.y2),
      },
    };
  };
  if (fit >= READABLE) {
    const zoom = Math.min(most, fit);
    return held(zoom, {
      x: open.x1 + (width - (all.x2 - all.x1) * zoom) / 2 - all.x1 * zoom,
      y: open.y1 + (height - (all.y2 - all.y1) * zoom) / 2 - all.y1 * zoom,
    });
  }
  const first = firstOf(boxes.map((b) => ({ x: b.x1, y: b.y1 }))) ?? { x: all.x1, y: all.y1 };
  return held(READABLE, { x: open.x1 + padding - first.x * READABLE, y: open.y1 + padding - first.y * READABLE });
}
