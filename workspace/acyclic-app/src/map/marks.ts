// A tile's thread stripe at the left and stage icon at the right. Boxes keep their fold chevron at the left.
// Drawn as one image per tile, since the renderer gives a node a single label. Pure, so marks need no canvas to test.

import type { Stage } from "../api";
import { STAGE_ICONS as ICONS, QUIET } from "../stages";

export { QUIET };

import type { Palette } from "../palette";
import { STRIP, TITLE } from "./chevron";

/** The stripe's width and the stage icon's size, in the Map's units. */
const STRIPE = 6;
const ICON = 16;
/** Room for the stripe at the left and the stage icon at the right. */
export const MARKS_LEFT = STRIPE + 8;
export const MARKS_RIGHT = ICON + 14;

/** A tile's thread as its marks show it: the line's colour and ink. */
export interface Thread {
  colour: string;
  ink: string;
}

/**
 * The marks' image for a tile `width` by `height`, as SVG markup. The `reserve` at its foot, a stub's room, is left
 * clear: its stripe runs the whole height, and the icon sits mid-way in the rest.
 */
export function markSvg(
  stage: Stage | null,
  thread: Thread | null,
  width: number,
  height: number,
  palette: Palette,
  reserve = 0,
): string {
  const middle = (height - reserve) / 2;
  const parts: string[] = [];
  // A task in no thread has a thread's stripe, left grey: the muted ink in place of a line's colour.
  const colour = thread === null ? palette.muted : thread.colour;
  parts.push(`<rect width="${STRIPE}" height="${height}" fill="${colour}"/>`);
  if (stage !== null) parts.push(iconAt(stage, width - 12 - ICON, middle, palette));
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${parts.join("")}</svg>`;
}

/** A box's chevron, open (pointing down) or closed (right), drawn in a 12-unit square. */
const CHEVRONS = { box: "M2 4l4 4 4-4", collapsed: "M4 2l4 4-4 4" };
const CHEVRON = 12;

/** Keep the title beyond the chevron's 44-unit touch target, with its existing gap. */
export const BOX_TEXT_LEFT = 54;

/**
 * A box's chevron at the left and stage icon at the right, over its title strip or collapsed height.
 */
export function boxMarkSvg(
  kind: "box" | "collapsed",
  stage: Stage | null,
  width: number,
  height: number,
  palette: Palette,
  thread: Thread | null = null,
  header = STRIP,
  reserve = 0,
): string {
  const tall = kind === "box" ? header : height;
  // Keep the open-box controls inside the fixed hit area at the top of a taller header.
  const middle = kind === "box" ? STRIP / 2 : (tall - reserve) / 2;
  const left = (TITLE - CHEVRON) / 2;
  const stroke = `fill="none" stroke="${palette.muted}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"`;
  const parts = [`<path transform="translate(${left} ${middle - CHEVRON / 2})" d="${CHEVRONS[kind]}" ${stroke}/>`];
  if (stage !== null) parts.push(iconAt(stage, width - 12 - ICON, middle, palette));
  // A finished thread's box: its thread's stripe down the left, as its items carry it.
  if (thread !== null) parts.push(`<rect width="${STRIPE}" height="${tall}" fill="${thread.colour}"/>`);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${tall}" viewBox="0 0 ${width} ${tall}">${parts.join("")}</svg>`;
}

/** A stage's icon from `left`, centred on `middle`: ink for work moving, muted for the rest. */
function iconAt(stage: Stage, left: number, middle: number, palette: Palette): string {
  const colour = QUIET.has(stage) ? palette.muted : palette.ink;
  return `<g transform="translate(${left} ${middle - ICON / 2})" fill="none" stroke="${colour}" color="${colour}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${ICONS[stage]}</g>`;
}

const made = new WeakMap<Palette, Map<string, string>>();

/** The marks' image as the renderer takes it, made once for each palette, stage, thread and size. */
export function markOf(
  stage: Stage | null,
  thread: Thread | null,
  width: number,
  height: number,
  palette: Palette,
  reserve = 0,
): string {
  let mine = made.get(palette);
  if (mine === undefined) {
    mine = new Map();
    made.set(palette, mine);
  }
  return once(mine, JSON.stringify([stage, thread, width, height, reserve]), () =>
    markSvg(stage, thread, width, height, palette, reserve),
  );
}

/** A box's marks as the renderer takes them, made once for each palette, kind, stage and size. */
export function boxMarkOf(
  kind: "box" | "collapsed",
  stage: Stage | null,
  width: number,
  height: number,
  palette: Palette,
  thread: Thread | null = null,
  header = STRIP,
  reserve = 0,
): string {
  let mine = made.get(palette);
  if (mine === undefined) {
    mine = new Map();
    made.set(palette, mine);
  }
  // A box's strip is as tall whatever it holds, so its height doesn't key it.
  const key = JSON.stringify([
    kind,
    stage,
    width,
    kind === "box" ? 0 : height,
    thread,
    kind === "box" ? header : 0,
    kind === "box" ? 0 : reserve,
  ]);
  return once(mine, key, () => boxMarkSvg(kind, stage, width, height, palette, thread, header, reserve));
}

function once(mine: Map<string, string>, key: string, svg: () => string): string {
  let uri = mine.get(key);
  if (uri === undefined) {
    uri = `data:image/svg+xml;utf8,${encodeURIComponent(svg())}`;
    mine.set(key, uri);
  }
  return uri;
}
