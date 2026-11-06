// A phone's sheets come in two heights, the first they open at and full, and a drag on the handle or the header moves
// between them or closes the sheet. Where a drag lands is decided here, from how far it went and how fast it was going
// at the end, so the rules are tested without a browser.

export type Detent = "first" | "full";

/** A panel's first height: its title, its status and its main action, so the Map keeps most of the screen. */
export const PEEK = 144;

/**
 * A sheet's first height in a stage `room` tall, as the stylesheet sets it, at most all of it: a panel's is `PEEK`, or
 * what its main action `needs` to show if more, as a long title wraps; the outline's, a list to pick from, half the room
 * and at least 320 px, so a small phone still shows a few of its rows.
 */
export function firstHeight(room: number, half: boolean, needs = 0): number {
  return Math.min(room, half ? Math.max(320, room / 2) : Math.max(PEEK, needs));
}

/** The most a row's end may be beyond the first height for the sheet to rise to it: a row of the panel, not a block. */
export const ROW = 48;

/**
 * The first height that ends on a row: the first row end at or below `needs` and `PEEK`, the least the sheet shows,
 * when it is within `ROW` of them, so the sheet shows its next row whole rather than cutting through its text.
 * `ends` are the rows' ends, in pixels from the sheet's top, in any order.
 */
export function endOnRow(needs: number, ends: readonly number[]): number {
  const least = Math.max(PEEK, needs);
  const end = Math.min(...ends.filter((one) => one >= least - 0.5));
  return end - least <= ROW ? Math.max(needs, Math.ceil(end)) : needs;
}

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
 * Where a drag from `from` that moved `dy` pixels (down positive) lands, ending at `speed`, with `first` the first
 * height and `travel` the pixels between it and full: a flick goes its way; otherwise up past a quarter of the travel
 * opens it full, down from full past a quarter comes back to the first height, and down from it past a third of that
 * height closes it. Anything less snaps back.
 */
export function detentAfter(from: Detent, dy: number, speed: number, travel: number, first: number): Detent | "close" {
  const down = speed > FLICK || (speed >= -FLICK && dy > 0 && dy > (from === "full" ? travel / 4 : first / 3));
  const up = speed < -FLICK || (speed <= FLICK && dy < 0 && -dy > travel / 4);
  if (from === "first") return up ? "full" : down ? "close" : "first";
  return down ? (dy > travel + first / 3 ? "close" : "first") : "full";
}

/** Whether a press on `target` may start a drag: not on a control, which a tap must still reach. */
export function draggableFrom(target: Element | null): boolean {
  return target !== null && target.closest("button, a, input, select, textarea, label, summary, [role=tab]") === null;
}

/** How far a press must move before it is a drag, so a tap on the title stays a tap. */
export const SLOP = 8;
