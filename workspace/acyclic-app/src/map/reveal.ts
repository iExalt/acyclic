// How far to pan so a node shows in the part of the Map nothing covers, such as above the sheet on a phone or left of
// the pane on a laptop. Pure, so the rule is tested without a canvas.

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
