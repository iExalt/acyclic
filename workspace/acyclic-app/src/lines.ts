// Threads drawn as the lines of a transit map: each has a colour, and a badge with its number wherever the thread is
// named. Some colours merge for colour-blind eyes, which is safe only because the Map lights one thread at a time and
// every list names the thread too: never ask anyone to tell two threads apart by colour alone.

import type { Thread } from "./api";

/** The app's colours, which app.css repeats as custom properties and the Map's stylesheet reads from here. */
export const TOKENS = {
  /** The Map's surface. */
  ground: "#141a1f",
  /** The bar and the thread list. */
  chrome: "#1b2228",
  /** A box on the Map. */
  box: "#1b2329",
  /** An item on the Map. */
  tile: "#26313a",
  /** A sheet or pane over the Map. */
  sheet: "#222b33",
  rule: "#34404a",
  ink: "#e6ebef",
  muted: "#95a3ae",
  /**
   * What needs the user, conflicts, and links that cross boxes: never a line's colour. It is never a cue by colour
   * alone either (crossing links are dashed, "Needs you" is text), since brown, its nearest line, looks alike to
   * colour-blind eyes: so never draw it as a solid border.
   */
  signal: "#f2b33d",
} as const;

/** Eight line colours, none near the signal amber, each readable as a border on an item, a box and the ground. */
export const LINE_COLOURS = [
  "#ec5d5e",
  "#46a758",
  "#12a594",
  "#0090ff",
  "#a97be0",
  "#e25aab",
  "#a18072",
  "#8da4ef",
] as const;

/** The badge's text colours: the ground's dark for light lines, white for dark ones. */
const INKS = [TOKENS.ground, "#ffffff"] as const;

export interface Line {
  /** The thread's own number, from its title, or null when it has none: a position in a list isn't a name. */
  number: string | null;
  /** The title without the number it starts with. */
  name: string;
  colour: string;
  /** The badge's text colour, whichever of the two reads better on `colour`. */
  ink: string;
}

/**
 * "Thread 7: Build it" and "7. Build it" both start with the line's number; "2.8 Draw the Map" starts with a step's,
 * which isn't one, and "Thread 7 Build it", without a separator, has none either.
 */
const NUMBERED = /^\s*(?:thread\s+)?(\d+)\s*[.:](?!\d)\s*/i;

/**
 * The line for a thread: coloured by its number, so colours hold still as threads come and go, else by its id. Its
 * number is shown as written, so "07" and "7" stay two badges. A title that is only a number, such as "Thread 7:",
 * keeps its colour but has no name to put beside the number, so it is named by its title, with a dot for a badge.
 */
export function lineOf(thread: Thread): Line {
  const numbered = NUMBERED.exec(thread.title);
  const rest = numbered === null ? "" : thread.title.slice(numbered[0].length);
  const number = rest === "" ? null : (numbered?.[1] ?? null);
  const at = numbered?.[1] === undefined ? hash(thread.id) : Number(numbered[1]);
  const colour = LINE_COLOURS[at % LINE_COLOURS.length] ?? LINE_COLOURS[0];
  return { number, name: number === null ? thread.title : rest, colour, ink: inkOn(colour) };
}

/** Each thread's line, by id. */
export function linesOf(threads: Thread[]): Map<string, Line> {
  return new Map(threads.map((thread) => [thread.id, lineOf(thread)]));
}

/** The ink with the higher contrast on `colour`. */
export function inkOn(colour: string): string {
  const [dark, light] = INKS;
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
