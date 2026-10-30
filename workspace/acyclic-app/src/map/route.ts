// A link as the layout routed it, in the terms the renderer draws a bent line in: its ends as offsets from its nodes'
// centres, and each bend as a weight along the line between those ends and a distance off it. Pure, so the arithmetic
// is tested without a canvas.

export interface Point {
  x: number;
  y: number;
}

/** A routed link's drawing, each list as the renderer reads it: numbers apart by spaces. */
export interface Segments {
  sourceEndpoint: string;
  targetEndpoint: string;
  weights: string;
  distances: string;
}

const px = (value: number) => `${round(value)}px`;
const round = (value: number) => Math.round(value * 1000) / 1000;

/**
 * The drawing for a route, its first point on the source's border and its last on the target's, between nodes centred
 * at `source` and `target`. A bend at P sits at weight w along S→T and distance d off it, to the left of the way S→T
 * runs on screen: P = S + w(T − S) + d·n, with n the unit normal (−dy, dx). A route with no bend gets one at its
 * middle, since the renderer draws a bent line only with a bend to draw.
 */
export function segmentsOf(route: readonly Point[], source: Point, target: Point): Segments | null {
  const start = route[0];
  const end = route[route.length - 1];
  if (start === undefined || end === undefined || route.length < 2) return null;
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const length = Math.hypot(dx, dy);
  if (length === 0) return null;
  const bends = route.length > 2 ? route.slice(1, -1) : [{ x: start.x + dx / 2, y: start.y + dy / 2 }];
  const weights = bends.map((bend) => ((bend.x - start.x) * dx + (bend.y - start.y) * dy) / length ** 2);
  const distances = bends.map((bend) => ((bend.x - start.x) * -dy + (bend.y - start.y) * dx) / length);
  return {
    sourceEndpoint: `${px(start.x - source.x)} ${px(start.y - source.y)}`,
    targetEndpoint: `${px(end.x - target.x)} ${px(end.y - target.y)}`,
    weights: weights.map(round).join(" "),
    distances: distances.map(round).join(" "),
  };
}
