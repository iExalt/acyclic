// What a task carries besides its title: its thread's stripe down the left and badge at the right, and an icon for
// its stage, which its tint repeats so no stage is told by colour alone. Drawn as one image per task, since the
// renderer gives a node a single label. Pure, so every stage's mark is tested without a canvas.

import type { Stage } from "../api";
import type { Palette } from "../palette";

/** How wide the stripe is, and where the icon and the badge sit, in the Map's units. */
const STRIPE = 6;
const ICON_AT = 14;
const ICON = 16;
const BADGE = 20;
/** The room the marks take from a task's title: the stripe and icon at its left, the badge at its right. */
export const MARKS_LEFT = ICON_AT + ICON + 6;
export const MARKS_RIGHT = BADGE + 14;

/** Each stage's icon, drawn in a 16-unit square. */
const ICONS: Record<Stage, string> = {
  done: '<path d="M3 8.5l3.2 3.2L13 5"/>',
  "in-review": '<path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/>',
  "in-progress": '<circle cx="8" cy="8" r="6"/><path d="M8 2a6 6 0 0 0 0 12z" fill="currentColor"/>',
  ready: '<circle cx="8" cy="8" r="6"/>',
  blocked: '<circle cx="8" cy="8" r="6"/><path d="M4.5 8h7"/>',
  parked: '<path d="M6 3.5v9M10 3.5v9"/>',
  unthreaded: '<path d="M2 8h4M10 8h4"/>',
};

/** The stages whose work is finished or set aside, drawn quieter: muted icon and title. */
export const QUIET: ReadonlySet<Stage> = new Set(["done", "blocked", "parked", "unthreaded"]);

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
  if (stage !== null) {
    const colour = QUIET.has(stage) ? palette.muted : palette.ink;
    parts.push(
      `<g transform="translate(${ICON_AT} ${middle - ICON / 2})" fill="none" stroke="${colour}" color="${colour}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">${ICONS[stage]}</g>`,
    );
  }
  const right = width - 10;
  if (thread === null) {
    // No thread: an empty ring where its badge would be.
    parts.push(
      `<circle cx="${right - BADGE / 2}" cy="${middle}" r="${BADGE / 2 - 1}" fill="none" stroke="${palette.muted}" stroke-width="1.5" stroke-dasharray="3 2.4"/>`,
    );
  } else if (thread.number === null) {
    parts.push(`<circle cx="${right - BADGE / 2}" cy="${middle}" r="5" fill="${thread.colour}"/>`);
  } else {
    // A pill wide enough for a clone's initial, such as "32m".
    const wide = Math.max(BADGE, 8 + 7 * thread.number.length);
    parts.push(
      `<rect x="${right - wide}" y="${middle - BADGE / 2}" width="${wide}" height="${BADGE}" rx="${BADGE / 2}" fill="${thread.colour}"/>`,
      `<text x="${right - wide / 2}" y="${middle}" dy="0.35em" text-anchor="middle" font-family="system-ui, sans-serif" font-size="11" font-weight="700" fill="${thread.ink}">${xml(thread.number)}</text>`,
    );
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${parts.join("")}</svg>`;
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
  const key = JSON.stringify([stage, thread, width, height]);
  let uri = mine.get(key);
  if (uri === undefined) {
    uri = `data:image/svg+xml;utf8,${encodeURIComponent(markSvg(stage, thread, width, height, palette))}`;
    mine.set(key, uri);
  }
  return uri;
}
