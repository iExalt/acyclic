// The Threads view's arrows, routed so none crosses a card: each runs along the gaps between the columns, crosses a
// wave it skips through a gap between two of its cards or below them, and turns at right angles. Each thread an
// arrow comes from keeps a lane of its own in every gap, and each meets a card at a point of its own, so two arrows
// from different threads never run along one line. Pure, so the rules are tested without a browser.

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** A wave's column as drawn: its heading and its cards, top to bottom. */
export interface Column {
  heading: Box;
  cards: { id: string; box: Box }[];
}

/** A thread `to` waiting on `from`: in a later wave, or in the same one when they wait on each other. */
export interface Wait {
  from: string;
  to: string;
  looped: boolean;
}

export interface Route extends Wait {
  /** Corners of the arrow, from `from`'s edge to `to`'s; each segment level or upright. */
  points: Point[];
}

export type Point = [number, number];

/** A lane's clearance from the column it leaves, and from the one it enters, where a head and a corner go. */
const LEAVE = 12;
const ENTER = 12;
/** The widest spacing between lanes in a gap, and between rows below a column's cards. */
const SPACING = 6;
/** The room between a column's last card and the first row below it. */
const BELOW = 12;
/** How near two level runs from different threads may come before they read as one. */
const APART = 3;
/** How near a card's corner a point on its edge may come. */
const INSET = 4;

/**
 * The routes of `waits` between `columns`, the waves in order. `beyond` is where the gap after the last wave ends,
 * as wide as `gapWidths` says. A wait between cards not drawn, or against the waves' order, has none.
 */
export function routes(columns: readonly Column[], waits: readonly Wait[], beyond: number): Route[] {
  const where = new Map<string, { wave: number; box: Box }>();
  columns.forEach((column, wave) => {
    for (const card of column.cards) where.set(card.id, { wave, box: card.box });
  });
  const spot = (id: string) => where.get(id) as { wave: number; box: Box };
  const known = waits.filter((wait) => {
    const from = where.get(wait.from);
    const to = where.get(wait.to);
    if (from === undefined || to === undefined || wait.from === wait.to) return false;
    return wait.looped ? from.wave === to.wave : from.wave < to.wave;
  });
  const extent = columns.map((column) => ({
    left: Math.min(column.heading.left, ...column.cards.map((card) => card.box.left)),
    right: Math.max(column.heading.right, ...column.cards.map((card) => card.box.right)),
    bottom: Math.max(column.heading.bottom, ...column.cards.map((card) => card.box.bottom)),
  }));
  // Sources in a stable order, top of the first wave first, so lanes and channels are handed out alike each time.
  const sources = [...new Set(known.map((wait) => wait.from))].sort((a, b) => {
    const one = spot(a);
    const other = spot(b);
    return one.wave - other.wave || one.box.top - other.box.top || (a < b ? -1 : 1);
  });

  // Lanes: in each gap, one x for every source whose arrows run down it, in the sources' order.
  const lanes = new Map<string, number>();
  const passing = gapUsers(new Map([...where].map(([id, { wave }]) => [id, wave])), known);
  extent.forEach((here, gap) => {
    const using = sources.filter((source) => passing[gap]?.has(source));
    if (using.length === 0) return;
    const left = here.right + LEAVE;
    const right = (extent[gap + 1]?.left ?? beyond) - ENTER;
    const spacing = Math.min(SPACING, Math.max(0, right - left) / (using.length + 1));
    const start = (left + right) / 2 - (spacing * (using.length - 1)) / 2;
    using.forEach((source, at) => {
      lanes.set(`${gap} ${source}`, start + spacing * at);
    });
  });
  const lane = (gap: number, source: string) => lanes.get(`${gap} ${source}`) as number;

  // The level runs in each gap: those from its left side out to a lane, and those from a lane on to its right side.
  // Two from different sources at one height overlap when they leave the same side, or when a run from the left
  // reaches past where one to the right starts; every height below is picked so they don't.
  const runs = new Map<number, { y: number; x: number; source: string; fromLeft: boolean }[]>();
  const clear = (gap: number, y: number, x: number, source: string, fromLeft: boolean) =>
    (runs.get(gap) ?? []).every(
      (run) =>
        run.source === source ||
        Math.abs(run.y - y) >= APART ||
        (run.fromLeft !== fromLeft && (fromLeft ? x <= run.x : run.x <= x)),
    );
  const run = (gap: number, y: number, x: number, source: string, fromLeft: boolean) => {
    if (gap < 0 || gap >= columns.length) return;
    runs.set(gap, [...(runs.get(gap) ?? []), { y, x, source, fromLeft }]);
  };

  // Ports: on a card's right edge, one point for what leaves it and one for each source in its wave that enters it;
  // on its left, one for each source from an earlier wave. Points on an edge are spread evenly down it, and those on
  // the left moved, if they must, off a run from the left side of the same gap.
  const ports = new Map<string, number>();
  // Sources in the order they stand top to bottom, so arrows into one edge don't cross each other just before it.
  const into = (id: string, looped: boolean) =>
    sources
      .filter((source) => known.some((wait) => wait.to === id && wait.from === source && wait.looped === looped))
      .sort((a, b) => spot(a).box.top - spot(b).box.top);
  const slot = (box: Box, at: number, count: number) => box.top + ((box.bottom - box.top) * (at + 1)) / (count + 1);
  for (const [id, { wave, box }] of where) {
    // The way out, among the ways in from its own wave, in the order of the cards they come from.
    const right = [...(sources.includes(id) ? [id] : []), ...into(id, true)].sort(
      (a, b) => spot(a).box.top - spot(b).box.top,
    );
    right.forEach((source, at) => {
      const y = slot(box, at, right.length);
      ports.set(source === id ? `out ${id}` : `loop ${id} ${source}`, y);
      run(wave, y, lane(wave, source), source, true);
    });
  }
  for (const [id, { wave, box }] of where) {
    const entering = into(id, false);
    entering.forEach((source, at) => {
      const x = lane(wave - 1, source);
      const fits = (y: number) =>
        y >= box.top + INSET &&
        y <= box.bottom - INSET &&
        clear(wave - 1, y, x, source, false) &&
        [...ports.entries()].every(([key, other]) => !key.startsWith(`in ${id} `) || Math.abs(other - y) >= APART);
      const y = nudged(slot(box, at, entering.length), box.bottom - box.top, fits);
      ports.set(`in ${id} ${source}`, y);
      run(wave - 1, y, x, source, false);
    });
  }

  // Channels across a skipped wave: a gap between two of its cards, or a row below it, clear of other sources' runs
  // in the gaps either side.
  const channel = (wave: number, source: string, now: number, then: number) => {
    const before = lane(wave - 1, source);
    const after = lane(wave, source);
    const fits = (y: number) => clear(wave - 1, y, before, source, false) && clear(wave, y, after, source, true);
    const cards = (columns[wave] as Column).cards;
    const candidates: number[] = [];
    for (const [at, card] of cards.entries()) {
      const below = cards[at + 1];
      if (below === undefined) break;
      const top = card.box.bottom;
      const bottom = below.box.top;
      const middle = (top + bottom) / 2;
      const room = (bottom - top) / 2 - INSET;
      for (const y of [middle, middle - room, middle + room]) if (room >= 0 && fits(y)) candidates.push(y);
    }
    let row = (extent[wave] as { bottom: number }).bottom + BELOW;
    while (!fits(row)) row += SPACING;
    candidates.push(row);
    const cost = (y: number) => Math.abs(y - now) + Math.abs(y - then);
    // The nearer, and of two as near the higher, so the routes hold still between measures.
    const y = candidates.reduce((best, y) => (cost(y) < cost(best) || (cost(y) === cost(best) && y < best) ? y : best));
    run(wave - 1, y, before, source, false);
    run(wave, y, after, source, true);
    return y;
  };

  return known.map((wait) => {
    const from = spot(wait.from);
    const to = spot(wait.to);
    const out = ports.get(`out ${wait.from}`) as number;
    if (wait.looped) {
      const x = lane(from.wave, wait.from);
      const y = ports.get(`loop ${wait.to} ${wait.from}`) as number;
      return {
        ...wait,
        points: tidy([
          [from.box.right, out],
          [x, out],
          [x, y],
          [to.box.right, y],
        ]),
      };
    }
    const end = ports.get(`in ${wait.to} ${wait.from}`) as number;
    const points: Point[] = [[from.box.right, out]];
    let y = out;
    for (let wave = from.wave + 1; wave < to.wave; wave++) {
      const x = lane(wave - 1, wait.from);
      const across = channel(wave, wait.from, y, end);
      points.push([x, y], [x, across]);
      y = across;
    }
    const x = lane(to.wave - 1, wait.from);
    points.push([x, y], [x, end], [to.box.left, end]);
    return { ...wait, points: tidy(points) };
  });
}

/** The box every route lies in, for the view to make room for routes that run below the columns or past them. */
export function boundsOf(found: readonly Route[]): Box | null {
  const points = found.flatMap((route) => route.points);
  if (points.length === 0) return null;
  const xs = points.map(([x]) => x);
  const ys = points.map(([, y]) => y);
  return { left: Math.min(...xs), top: Math.min(...ys), right: Math.max(...xs), bottom: Math.max(...ys) };
}

/**
 * How wide each gap after a wave must be, at least `least`, for the lanes of every thread whose arrows run down it to
 * stand their full spacing apart: the view widens a crowded gap rather than let its lanes merge.
 */
export function gapWidths(waves: readonly (readonly string[])[], waits: readonly Wait[], least: number): number[] {
  const wave = new Map(waves.flatMap((ids, at) => ids.map((id) => [id, at] as const)));
  const passing = gapUsers(wave, waits);
  return waves.map((_, gap) => Math.max(least, LEAVE + ENTER + SPACING * ((passing[gap]?.size ?? 0) + 1)));
}

/** The sources whose arrows run down each gap: a wait's every gap from its source's wave to its own, or for a loop
 * the one right of their wave. */
function gapUsers(wave: ReadonlyMap<string, number>, waits: readonly Wait[]): Set<string>[] {
  const users: Set<string>[] = [];
  for (const wait of waits) {
    const from = wave.get(wait.from);
    const to = wave.get(wait.to);
    if (from === undefined || to === undefined) continue;
    for (let gap = from; gap < (wait.looped ? from + 1 : to); gap++) {
      users[gap] = (users[gap] ?? new Set()).add(wait.from);
    }
  }
  return users;
}

/** `y`, or the nearest height a pixel at a time either side of it, up to `reach`, that `fits`; `y` when none does. */
function nudged(y: number, reach: number, fits: (y: number) => boolean): number {
  for (let step = 0; step <= reach; step += 1) {
    if (fits(y + step)) return y + step;
    if (fits(y - step)) return y - step;
  }
  return y;
}

/** Drops repeated points and those in the middle of a straight run. */
function tidy(points: Point[]): Point[] {
  const kept: Point[] = [];
  for (const point of points) {
    const last = kept[kept.length - 1];
    if (last !== undefined && last[0] === point[0] && last[1] === point[1]) continue;
    const before = kept[kept.length - 2];
    if (
      before !== undefined &&
      last !== undefined &&
      ((before[0] === last[0] && last[0] === point[0]) || (before[1] === last[1] && last[1] === point[1]))
    ) {
      kept[kept.length - 1] = point;
      continue;
    }
    kept.push(point);
  }
  return kept;
}

/** An SVG path through `points`, its corners rounded by up to `radius`, never more than half a segment. */
export function pathOf(points: readonly Point[], radius = 4): string {
  const [first, ...rest] = points;
  if (first === undefined) return "";
  let path = `M ${first[0]} ${first[1]}`;
  rest.forEach((corner, at) => {
    const next = rest[at + 1];
    if (next === undefined) {
      path += ` L ${corner[0]} ${corner[1]}`;
      return;
    }
    const before = (at === 0 ? first : rest[at - 1]) as Point;
    const r = Math.min(radius, length(before, corner) / 2, length(corner, next) / 2);
    const into = toward(corner, before, r);
    const onto = toward(corner, next, r);
    path += ` L ${into[0]} ${into[1]} Q ${corner[0]} ${corner[1]} ${onto[0]} ${onto[1]}`;
  });
  return path;
}

/** A head at the end of `points`, a small triangle pointing the way the last segment goes. */
export function headOf(points: readonly Point[]): string {
  const end = points[points.length - 1];
  const before = points[points.length - 2];
  if (end === undefined || before === undefined) return "";
  const back = toward(end, before, 7);
  // Half the head's width, across the way it points.
  const [dx, dy] = [back[1] === end[1] ? 0 : 4, back[0] === end[0] ? 0 : 4];
  return `M ${end[0]} ${end[1]} L ${back[0] + dx} ${back[1] + dy} L ${back[0] - dx} ${back[1] - dy} z`;
}

const length = (a: Point, b: Point) => Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]);

/** The point `by` along from `from` toward `to`, on a level or upright segment. */
const toward = (from: Point, to: Point, by: number): Point => [
  from[0] + Math.sign(to[0] - from[0]) * by,
  from[1] + Math.sign(to[1] - from[1]) * by,
];
