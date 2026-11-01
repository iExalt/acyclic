// A phone's sheets come in two heights, half and full, and a drag on the handle or the header moves between them or
// closes the sheet. Where a drag lands is decided here, from how far it went and how fast it was going at the end,
// so the rules are tested without a browser.

export type Detent = "half" | "full";

/** A pointer's position down the screen at a time, in milliseconds. */
export interface Sample {
  t: number;
  y: number;
}

/** How far back from its last sample a drag's speed is read: its end, not its start, says where it was going. */
export const RECENT_MS = 100;
/** A flick: faster than this, in pixels a millisecond, goes the way it went whatever the distance. */
export const FLICK = 0.5;

/** The speed at the end of a drag, in pixels a millisecond, down positive, from its samples in time order. */
export function velocity(samples: readonly Sample[]): number {
  const last = samples.at(-1);
  if (last === undefined) return 0;
  const first = samples.find((one) => last.t - one.t <= RECENT_MS) ?? last;
  return last.t === first.t ? 0 : (last.y - first.y) / (last.t - first.t);
}

/**
 * Where a drag from `from` that moved `dy` pixels (down positive) lands, ending at `speed`, with `travel` the pixels
 * between the two heights: a flick goes its way; otherwise up past a quarter of the travel opens it full, down from
 * full past a quarter comes back to half, and down from half past a third of the half sheet's height closes it.
 * Anything less snaps back.
 */
export function detentAfter(
  from: Detent,
  dy: number,
  speed: number,
  travel: number,
  halfHeight: number,
): Detent | "close" {
  const down = speed > FLICK || (speed >= -FLICK && dy > 0 && dy > (from === "full" ? travel / 4 : halfHeight / 3));
  const up = speed < -FLICK || (speed <= FLICK && dy < 0 && -dy > travel / 4);
  if (from === "half") return up ? "full" : down ? "close" : "half";
  return down ? (dy > travel + halfHeight / 3 ? "close" : "half") : "full";
}

/** Whether a press on `target` may start a drag: not on a control, which a tap must still reach. */
export function draggableFrom(target: Element | null): boolean {
  return target !== null && target.closest("button, a, input, select, textarea, label, summary, [role=tab]") === null;
}

/** How far a press must move before it is a drag, so a tap on the title stays a tap. */
export const SLOP = 8;
