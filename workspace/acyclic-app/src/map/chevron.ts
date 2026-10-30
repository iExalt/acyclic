// A box's chevron: where it is drawn, and whether a tap reached it. The chevron opens and closes a box, while a tap
// anywhere else on the box selects it. Pure, so the rules are tested without a canvas.

import type { Box } from "./reveal";

/** How wide the square at the left of a box's title strip is, where the chevron sits. */
export const TITLE = 36;
/** How tall a box's title strip is: its title, and a parent's leaves by stage under it. */
export const STRIP = 52;
/** The least a fingertip can reliably hit, in screen pixels. */
const FINGER = 44;

/**
 * Whether a tap at `point` on a node drawn at `node` (both in the Map's own units, with the Map at `zoom`) reached its
 * chevron: an open box's at the left of its title strip, a collapsed box's at its left from top to bottom. However far
 * the Map is zoomed out, the chevron takes a fingertip's width and height, and never more than half the node, so its
 * title can still be tapped.
 */
export function onChevron(kind: string, node: Box, point: { x: number; y: number }, zoom: number): boolean {
  if (kind !== "box" && kind !== "collapsed") return false;
  const reach = Math.max(TITLE, FINGER / zoom);
  const wide = Math.min(reach, (node.x2 - node.x1) / 2);
  // Below an open box's title strip are what it holds, drawn over it, so a fingertip's reach there takes nothing.
  const tall = kind === "box" ? Math.min(reach, (node.y2 - node.y1) / 2) : node.y2 - node.y1;
  return point.x >= node.x1 && point.x - node.x1 <= wide && point.y >= node.y1 && point.y - node.y1 <= tall;
}
