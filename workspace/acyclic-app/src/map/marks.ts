// What a task carries besides its title: its thread's stripe down the left and its badge beside it, as the sidebar
// leads a thread's row with its badge, and an icon for its stage at the right, which its tint repeats so no stage is
// told by colour alone. Drawn as one image per task, since the
// renderer gives a node a single label. Pure, so every stage's mark is tested without a canvas.

import type { Stage } from "../api";
import { STAGE_ICONS as ICONS, QUIET } from "../stages";

export { QUIET };

import type { Palette } from "../palette";
import { STRIP, TITLE } from "./chevron";

/** How wide the stripe is, and where the badge and the icon sit, in the Map's units. */
const STRIPE = 6;
const BADGE_AT = 14;
const BADGE = 20;
/** The widest a badge is drawn for a two-digit number; a clone's initial ("32m") may run a little past it. */
const BADGE_ROOM = 22;
const ICON = 16;
/** The room the marks take from a task's title: the stripe and badge at its left, the icon at its right. */
export const MARKS_LEFT = BADGE_AT + BADGE_ROOM + 8;
export const MARKS_RIGHT = ICON + 14;

/** A task's thread as its marks show it: the line's colour, its badge's text (a number, or none for a dot) and ink. */
export interface Thread {
  colour: string;
  number: string | null;
  ink: string;
}

const xml = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** The marks' image for a task `width` by `height`, as SVG markup. */
export function markSvg(
  stage: Stage | null,
  thread: Thread | null,
  width: number,
  height: number,
  palette: Palette,
): string {
  const middle = height / 2;
  const parts: string[] = [];
  if (thread !== null) parts.push(`<rect width="${STRIPE}" height="${height}" fill="${thread.colour}"/>`);
  const left = BADGE_AT;
  if (thread === null) {
    // No thread: an empty ring where its badge would be.
    parts.push(
      `<circle cx="${left + BADGE / 2}" cy="${middle}" r="${BADGE / 2 - 1}" fill="none" stroke="${palette.muted}" stroke-width="1.5" stroke-dasharray="3 2.4"/>`,
    );
  } else if (thread.number === null) {
    parts.push(`<circle cx="${left + BADGE / 2}" cy="${middle}" r="5" fill="${thread.colour}"/>`);
  } else {
    // A pill wide enough for a clone's initial, such as "32m".
    const wide = Math.max(BADGE, 8 + 7 * thread.number.length);
    parts.push(
      `<rect x="${left}" y="${middle - BADGE / 2}" width="${wide}" height="${BADGE}" rx="${BADGE / 2}" fill="${thread.colour}"/>`,
      `<text x="${left + wide / 2}" y="${middle}" dy="0.35em" text-anchor="middle" font-family="system-ui, sans-serif" font-size="11" font-weight="700" fill="${thread.ink}">${xml(thread.number)}</text>`,
    );
  }
  // With no thread, the ring at its left already says what its stage would repeat.
  if (stage !== null && !(stage === "unthreaded" && thread === null))
    parts.push(iconAt(stage, width - 12 - ICON, middle, palette));
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${parts.join("")}</svg>`;
}

/** A box's chevron, open (pointing down) or closed (right), drawn in a 12-unit square. */
const CHEVRONS = { box: "M2 4l4 4 4-4", collapsed: "M4 2l4 4-4 4" };
const CHEVRON = 12;

/** Where a box's title starts: after its chevron and the icon of where its work stands. */
export const BOX_TEXT_LEFT = TITLE - 4 + ICON + 6;

/**
 * A box's marks: its chevron at the left of its title strip, then the icon of where its work stands as a whole, so a
 * done phase is told from one in progress or blocked at a glance, leading its title as a status leads a project's name.
 * Drawn over the strip, or a collapsed box's whole height.
 */
export function boxMarkSvg(
  kind: "box" | "collapsed",
  stage: Stage | null,
  width: number,
  height: number,
  palette: Palette,
): string {
  const tall = kind === "box" ? STRIP : height;
  const middle = tall / 2;
  const left = (TITLE - CHEVRON) / 2;
  const stroke = `fill="none" stroke="${palette.muted}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"`;
  const parts = [`<path transform="translate(${left} ${middle - CHEVRON / 2})" d="${CHEVRONS[kind]}" ${stroke}/>`];
  if (stage !== null) parts.push(iconAt(stage, TITLE - 4, middle, palette));
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
): string {
  let mine = made.get(palette);
  if (mine === undefined) {
    mine = new Map();
    made.set(palette, mine);
  }
  return once(mine, JSON.stringify([stage, thread, width, height]), () =>
    markSvg(stage, thread, width, height, palette),
  );
}

/** A box's marks as the renderer takes them, made once for each palette, kind, stage and size. */
export function boxMarkOf(
  kind: "box" | "collapsed",
  stage: Stage | null,
  width: number,
  height: number,
  palette: Palette,
): string {
  let mine = made.get(palette);
  if (mine === undefined) {
    mine = new Map();
    made.set(palette, mine);
  }
  // A box's strip is as tall whatever it holds, so its height doesn't key it.
  const key = JSON.stringify([kind, stage, width, kind === "box" ? 0 : height]);
  return once(mine, key, () => boxMarkSvg(kind, stage, width, height, palette));
}

function once(mine: Map<string, string>, key: string, svg: () => string): string {
  let uri = mine.get(key);
  if (uri === undefined) {
    uri = `data:image/svg+xml;utf8,${encodeURIComponent(svg())}`;
    mine.set(key, uri);
  }
  return uri;
}
