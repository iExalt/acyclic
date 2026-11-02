// Threads drawn as the lines of a transit map: each has a colour, and a badge, a dot in that colour, wherever the thread
// is named. Colours spiral round the hue wheel by a hash of the thread's id, and each lap of 27 positions is a quarter
// of the band lighter than the last: positions 7 and 34 share a hue and differ only a little in lightness, nearly alike. After 108 positions the spiral starts again. That is safe only because the Map lights one
// thread at a time and every list names the thread: never ask anyone to tell two threads apart by colour alone.

import type { Thread } from "./api";
import { okhsl } from "./okhsl";
import type { Palette } from "./palette";

/** What a line is drawn from: a thread's id and title. */
type Named = Pick<Thread, "id" | "title">;

/** Threads to a lap of the hue wheel. */
const LAP = 27;
/** Laps from the darkest to the lightest, after which the spiral starts again from the darkest. */
const LAPS = 4;
/** How colourful every line is: in Okhsl, a share of the most each hue holds at its lightness, so all are alike. */
const SATURATION = 0.9;
/** Where the walk starts: position 0 a yellow green. */
const START = 110;

/**
 * The line colour of position `at`: a spiral round the hue wheel that grows lighter as it goes, so a line that comes
 * round to an earlier one's hue differs from it in lightness. After four laps, 108 threads, it starts again from the
 * darkest: those are the only lines that share a colour. The scheme's band sets how light: never darker than a border
 * at 3:1 on its surfaces and a badge at 4.5:1 in either ink allow, nor so light that the colour washes out. A thread
 * keeps its hue between schemes, not its lightness.
 */
export function lineColour(at: number, palette: Palette): string {
  const [darkest, lightest] = palette.band;
  const along = at % (LAP * LAPS);
  const hue = (START + (along * 360) / LAP) % 360;
  return okhsl(hue, SATURATION, darkest + ((lightest - darkest) * along) / (LAP * LAPS));
}

export interface Line {
  /** The thread's title. */
  name: string;
  colour: string;
  /** The badge's text colour, whichever of the two reads better on `colour`. */
  ink: string;
}

/** The line for a thread, coloured by its id, so colours hold still as threads come and go. */
export function lineOf(thread: Named, palette: Palette): Line {
  const colour = lineColour(hash(thread.id), palette);
  return { name: thread.title, colour, ink: inkOn(colour, palette) };
}

/** Each thread's line, by id. */
export function linesOf(threads: Named[], palette: Palette): Map<string, Line> {
  return new Map(threads.map((thread) => [thread.id, lineOf(thread, palette)]));
}

/** Threads in the order they are listed: by title, then by id. */
export function byTitle(a: Named, b: Named): number {
  return a.title.localeCompare(b.title, undefined, { numeric: true }) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** The badge ink with the higher contrast on `colour`. */
export function inkOn(colour: string, palette: Palette): string {
  const [dark, light] = palette.badgeInks;
  return contrast(colour, dark) >= contrast(colour, light) ? dark : light;
}

/** A small, stable hash of a string (FNV-1a). */
function hash(text: string): number {
  let value = 0x811c9dc5;
  for (const char of text) value = Math.imul(value ^ (char.codePointAt(0) ?? 0), 0x01000193) >>> 0;
  return value;
}

/** WCAG's relative luminance of a `#RRGGBB` colour. */
export function luminance(hex: string): number {
  const [r = 0, g = 0, b = 0] = [1, 3, 5].map((at) => {
    const channel = Number.parseInt(hex.slice(at, at + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG's contrast ratio between two `#RRGGBB` colours. */
export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
