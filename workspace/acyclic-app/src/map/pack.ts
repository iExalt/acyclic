// The top level's boxes packed tightly into a rectangle near the Map's shape. Each box, widest first once what it waits
// on is placed, takes the highest place, then the leftmost, where it fits under the boxes already placed and after
// every box it waits on: below one, or level with it or lower on its right. A narrow box so fills the space beside wide
// ones, where rows would leave it empty, and still nothing comes before what it waits on. Of a range of widths, the one whose rectangle wastes the
// least room, of those that read largest fitted to the Map, wins. Pure, so it is tested without ELK.

import type { BoxLayout, Size } from "./layout";

/** The space around the packed boxes and between them, as inside a box. */
const PAD = 12;
const GAP = 24;
/** How many widths are tried, from the widest box's to every box in one row. */
const TRIES = 24;
/** Past this many boxes, one order is tried rather than three. */
const MANY = 40;

interface Placed {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Packs `children`, given in dependency order, to a shape of `aspect` (width over height). `waits` maps a box to the
 * boxes it waits on.
 */
export function pack(
  children: readonly ({ id: string } & Size)[],
  waits: ReadonlyMap<string, readonly string[]>,
  aspect: number,
): BoxLayout {
  if (children.length === 0) return { width: 2 * PAD, height: 2 * PAD, at: {} };
  const widest = Math.max(...children.map((child) => child.width));
  const row = children.reduce((sum, child) => sum + child.width, 0) + GAP * (children.length - 1);
  const area = children.reduce((sum, child) => sum + (child.width + GAP) * (child.height + GAP), 0);
  const widths = new Set([widest, row, Math.round(Math.sqrt(area * aspect))]);
  for (let n = 1; n < TRIES; n++) widths.add(Math.round(widest + ((row - widest) * n) / TRIES));
  let best: { score: number; laid: BoxLayout } | null = null;
  // Each try is cubic in the boxes: a plan with many at the top tries one order, which is enough to fill its gaps.
  const orders = (children.length > MANY ? KEYS.slice(0, 1) : KEYS).map((key) => placing(children, waits, key));
  for (const width of [...widths].sort((a, b) => a - b))
    for (const order of orders) {
      const laid = packTo(order, waits, Math.max(widest, width));
      // Fitted to the Map, the plan reads as large as the side that fills it first allows: the screen-shaped rectangle
      // around it says how large, and of two that read as large, the one with less room wasted is tighter.
      const side = Math.max(laid.width, laid.height * aspect);
      const score = (side * side) / aspect + laid.width * laid.height;
      if (best === null || score < best.score) best = { score, laid };
    }
  return best?.laid ?? { width: 2 * PAD, height: 2 * PAD, at: {} };
}

/** Packs the children within `limit` wide, each at the highest, then leftmost, place it may take. */
function packTo(
  children: readonly ({ id: string } & Size)[],
  waits: ReadonlyMap<string, readonly string[]>,
  limit: number,
): BoxLayout {
  const placed = new Map<string, Placed>();
  const at: Record<string, { x: number; y: number }> = {};
  let [right, bottom] = [0, 0];
  for (const child of children) {
    const firsts = (waits.get(child.id) ?? []).flatMap((id) => placed.get(id) ?? []);
    let spot: { x: number; y: number } | null = null;
    for (const x of new Set([0, ...[...placed.values()].map((box) => box.x + box.width + GAP)])) {
      if (x > 0 && x + child.width > limit) continue;
      // Under whatever already stands across these columns.
      let y = 0;
      for (const box of placed.values())
        if (box.x < x + child.width + GAP && x < box.x + box.width + GAP) y = Math.max(y, box.y + box.height + GAP);
      // After what it waits on: below it, or level with it or lower on its right.
      for (const first of firsts)
        y = Math.max(y, x >= first.x + first.width + GAP ? first.y : first.y + first.height + GAP);
      if (spot === null || y < spot.y || (y === spot.y && x < spot.x)) spot = { x, y };
    }
    const { x, y } = spot ?? { x: 0, y: bottom };
    placed.set(child.id, { x, y, width: child.width, height: child.height });
    at[child.id] = { x: x + PAD, y: y + PAD };
    right = Math.max(right, x + child.width);
    bottom = Math.max(bottom, y + child.height);
  }
  return { width: right + 2 * PAD, height: bottom + 2 * PAD, at };
}

/** What a box is placed by, largest first: its width, its height, or its area. */
const KEYS: ((box: Size) => number)[] = [(box) => box.width, (box) => box.height, (box) => box.width * box.height];

/**
 * The children in the order they are placed: the largest by `key` first of those whose firsts are placed, ties in the
 * order given, so smaller ones fill what larger ones leave beside them.
 */
function placing(
  children: readonly ({ id: string } & Size)[],
  waits: ReadonlyMap<string, readonly string[]>,
  key: (box: Size) => number,
) {
  const left = new Map(children.map((child) => [child.id, child]));
  const out: ({ id: string } & Size)[] = [];
  while (left.size > 0) {
    const all = [...left.values()];
    const ready = all.filter((child) => (waits.get(child.id) ?? []).every((id) => !left.has(id)));
    // A cycle leaves none ready: it is broken at its largest.
    const next = (ready.length > 0 ? ready : all).reduce((a, b) => (key(b) > key(a) ? b : a));
    out.push(next);
    left.delete(next.id);
  }
  return out;
}

/**
 * The top level `was` laid out, for the same `children` at their sizes now: each where it was, but for what a box that
 * grew now covers, which moves right if it stood right of the box it meets, and down if not, and stays after what it
 * waits on. A box opening so moves only what it would cover, and the rest of the plan stays in view.
 */
export function hold(
  was: { children: readonly ({ id: string } & Size)[]; layout: BoxLayout },
  children: readonly ({ id: string } & Size)[],
  waits: ReadonlyMap<string, readonly string[]>,
): BoxLayout {
  const before = new Map(
    was.children.map((child) => {
      const at = was.layout.at[child.id] ?? { x: PAD, y: PAD };
      return [child.id, { x: at.x - PAD, y: at.y - PAD, width: child.width, height: child.height }];
    }),
  );
  const old = (id: string) => before.get(id) ?? { x: 0, y: 0, width: 0, height: 0 };
  // What a box waits on and stood after before, below it or level with it or lower on its right, it stays after.
  // Packing breaks a cycle of waits somewhere, and what it left before what it waits on stays there.
  const after = (id: string, first: string) => {
    const [me, then] = [old(id), old(first)];
    if (me.y >= then.y + then.height + GAP) return "below";
    return me.x >= then.x + then.width + GAP && me.y >= then.y ? "beside" : null;
  };
  // The boxes that grew first, where they were, so what they now cover makes way for them, each once what it stays
  // after is placed; then the rest, in reading order, so whatever a box meets has settled.
  const grew = (child: { id: string } & Size) =>
    child.width > old(child.id).width || child.height > old(child.id).height;
  const left = [...children].sort((a, b) => old(a.id).y - old(b.id).y || old(a.id).x - old(b.id).x);
  const waiting = (child: { id: string }) =>
    left.find((first) => (waits.get(child.id) ?? []).includes(first.id) && after(child.id, first.id) !== null);
  const order: ({ id: string } & Size)[] = [];
  while (left.length > 0) {
    // What a box stays after stood before it in reading order, so going down to what it waits on ends.
    let next = left.find(grew) ?? left[0];
    for (let first = next && waiting(next); first !== undefined; first = waiting(first)) next = first;
    if (next === undefined) break;
    order.push(next);
    left.splice(left.indexOf(next), 1);
  }
  const placed = new Map<string, Placed>();
  const at: Record<string, { x: number; y: number }> = {};
  let [right, bottom] = [0, 0];
  for (const child of order) {
    let { x, y } = old(child.id);
    // Each move takes it past one settled box for good, as x and y only grow: so this ends.
    for (let moved = true; moved; ) {
      moved = false;
      for (const id of waits.get(child.id) ?? []) {
        const first = placed.get(id);
        const relation = after(child.id, id);
        if (first === undefined || relation === null) continue;
        const least = relation === "below" ? first.y + first.height + GAP : first.y;
        if (y < least) [y, moved] = [least, true];
        // Beside it, clear of it on its right, unless now below it.
        if (relation === "beside" && x < first.x + first.width + GAP && y < first.y + first.height + GAP)
          [x, moved] = [first.x + first.width + GAP, true];
      }
      for (const [id, box] of placed) {
        const meets =
          box.x < x + child.width + GAP &&
          x < box.x + box.width + GAP &&
          box.y < y + child.height + GAP &&
          y < box.y + box.height + GAP;
        if (!meets) continue;
        const then = old(id);
        if (old(child.id).x >= then.x + then.width + GAP) x = box.x + box.width + GAP;
        else y = box.y + box.height + GAP;
        moved = true;
      }
    }
    placed.set(child.id, { x, y, width: child.width, height: child.height });
    at[child.id] = { x: x + PAD, y: y + PAD };
    right = Math.max(right, x + child.width);
    bottom = Math.max(bottom, y + child.height);
  }
  return { width: right + 2 * PAD, height: bottom + 2 * PAD, at };
}
