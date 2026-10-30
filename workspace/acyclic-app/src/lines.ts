// Threads drawn as the lines of a transit map: each has a colour, and a badge with its number wherever the thread is
// named. Colours spiral round the hue wheel by number, so neighbouring threads look alike, and each lap of about 27
// threads is a notch lighter than the last: thread 7 and thread 34 share a hue but not a lightness. Lines far apart
// can still look alike, which is safe only because the Map lights one thread at a time and every list names the
// thread: never ask anyone to tell two threads apart by colour alone.

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

/**
 * How light lines are, in OKLCH: never darker than a border at 3:1 on the tile and a badge at 4.5:1 in dark ink allow,
 * nor so light that the colour washes out.
 */
const DARKEST = 0.7;
const LIGHTEST = 0.86;
/** How much lighter each lap round the wheel is than the last, until the lightest, when the laps turn darker again. */
const LAP_LIGHTER = 0.04;
/** How colourful every line is, in OKLCH, where the sRGB gamut allows. */
const CHROMA = 0.13;
/** The signal's hue in OKLCH, and how far every line keeps from it either side. */
const SIGNAL_HUE = 78;
const SIGNAL_GAP = 34;
/** The hue between one number's line and the next: the walk passes the wheel in about 27 threads. */
const STEP = 11;

/**
 * The line colour of position `at`: a spiral round the hue wheel that skips the band about the signal and grows lighter
 * a notch each lap, then darker again, so a line that comes round to an earlier one's hue differs from it in lightness.
 */
export function lineColour(at: number): string {
  const band = 360 - 2 * SIGNAL_GAP;
  const hue = (SIGNAL_HUE + SIGNAL_GAP + ((at * STEP) % band)) % 360;
  const span = LIGHTEST - DARKEST;
  const climbed = ((at * STEP) / band) * LAP_LIGHTER;
  const along = climbed % (2 * span);
  return oklch(DARKEST + (along <= span ? along : 2 * span - along), CHROMA, hue);
}

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
  const colour = lineColour(at);
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

/**
 * An OKLCH colour as `#RRGGBB`, with its chroma lowered until it fits sRGB, so the hue and the lightness hold: at the lines'
 * lightness, a few cyans fit slightly less chroma than the rest.
 */
export function oklch(lightness: number, chroma: number, hue: number): string {
  const angle = (hue * Math.PI) / 180;
  for (let c = chroma; ; c -= 0.002) {
    const rgb = oklabToLinear(lightness, Math.max(0, c) * Math.cos(angle), Math.max(0, c) * Math.sin(angle));
    if (c <= 0 || rgb.every((channel) => channel >= 0 && channel <= 1)) {
      return `#${rgb
        .map((channel) =>
          Math.round(255 * encode(Math.min(1, Math.max(0, channel))))
            .toString(16)
            .padStart(2, "0"),
        )
        .join("")}`;
    }
  }
}

/** Linear sRGB from OKLab, as Björn Ottosson defines it. */
function oklabToLinear(l: number, a: number, b: number): [number, number, number] {
  const [x, y, z] = [
    l + 0.3963377774 * a + 0.2158037573 * b,
    l - 0.1055613458 * a - 0.0638541728 * b,
    l - 0.0894841775 * a - 1.291485548 * b,
  ].map((value) => value ** 3) as [number, number, number];
  return [
    4.0767416621 * x - 3.3077115913 * y + 0.2309699292 * z,
    -1.2684380046 * x + 2.6097574011 * y - 0.3413193965 * z,
    -0.0041960863 * x - 0.7034186147 * y + 1.707614701 * z,
  ];
}

/** sRGB's transfer function, from a linear channel. */
function encode(channel: number): number {
  return channel <= 0.0031308 ? 12.92 * channel : 1.055 * channel ** (1 / 2.4) - 0.055;
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
