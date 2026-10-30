// Threads drawn as the lines of a transit map: each has a colour, and a badge with its number wherever the thread is
// named. Colours spiral round the hue wheel by number, so neighbouring threads look alike, and each lap of 27
// threads is lighter than the last: thread 7 and thread 34 share a hue but not a lightness. After 108 threads the
// spiral starts again, and colours that differ only in lightness are hard to tell apart for some eyes, which is safe
// only because the Map lights one thread at a time and every list names the thread: never ask anyone to tell two
// threads apart by colour alone.

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
   * What needs the user, conflicts, and links that cross boxes. Lines may share its hue, so it is never a cue by colour
   * alone: crossing links are dashed, conflicts have a dashed border, and the needs-you badge reads "!", which no
   * thread's badge does. So never draw it as a solid border or a badge with a number.
   */
  signal: "#f2b33d",
} as const;

/**
 * How light lines are, in OKLCH: never darker than a border at 3:1 on the tile and a badge at 4.5:1 in dark ink allow,
 * nor so light that the colour washes out.
 */
const DARKEST = 0.7;
const LIGHTEST = 0.86;
/** Threads to a lap of the hue wheel. */
const LAP = 27;
/** Laps from the darkest to the lightest, after which the spiral starts again from the darkest. */
const LAPS = 4;
/** How colourful every line is, in OKLCH, where the sRGB gamut allows. */
const CHROMA = 0.13;
/** Where the walk starts: thread 0 a yellow green. */
const START = 110;

/**
 * The line colour of position `at`: a spiral round the hue wheel that grows lighter as it goes, so a line that comes
 * round to an earlier one's hue differs from it in lightness. After four laps, 108 threads, it starts again from the
 * darkest: those are the only lines that share a colour.
 */
export function lineColour(at: number): string {
  const along = at % (LAP * LAPS);
  const hue = (START + (along * 360) / LAP) % 360;
  return oklch(DARKEST + ((LIGHTEST - DARKEST) * along) / (LAP * LAPS), CHROMA, hue);
}

/** The badge's text colours: the ground's dark for light lines, white for dark ones. */
const INKS = [TOKENS.ground, "#ffffff"] as const;

export interface Line {
  /**
   * The thread's number, from its id, or null when its id has none. Two clones can give threads the same number, so
   * while they clash each badge also carries its clone's initial, such as "32m".
   */
  number: string | null;
  /** The title, without a number it starts with that is the thread's own. */
  name: string;
  colour: string;
  /** The badge's text colour, whichever of the two reads better on `colour`. */
  ink: string;
}

/**
 * The tracker's thread ids: `thread-<n>`, or `thread-<clone>-<n>` for one a clone assigned, where the clone's id may
 * hold dashes itself.
 */
const ID = /^thread-(?:([a-z0-9-]+)-)?(\d+)$/;

/**
 * A number a title starts with, from before threads took theirs from their ids: "7. Build it" or "Thread 7: Build
 * it". "2.8 Draw the Map" starts with a step's, which isn't one.
 */
const NUMBERED = /^\s*(?:thread\s+)?(\d+)\s*[.:](?!\d)\s*/i;

/** A thread's number and clone, from its id. */
function parts(id: string): { number: number; clone: string | null } | null {
  const found = ID.exec(id);
  return found === null ? null : { number: Number(found[2]), clone: found[1] ?? null };
}

/**
 * The line for a thread: numbered and coloured by the number its id ends in, so colours hold still as threads come
 * and go, else coloured by its id and badged with a dot. A title that starts with that same number has it taken off
 * its name, so the number shows once.
 */
export function lineOf(thread: Thread, clone = ""): Line {
  const own = parts(thread.id);
  const numbered = NUMBERED.exec(thread.title);
  const rest = numbered === null ? "" : thread.title.slice(numbered[0].length);
  const name = own !== null && rest !== "" && Number(numbered?.[1]) === own.number ? rest : thread.title;
  const colour = lineColour(own === null ? hash(thread.id) : own.number);
  return { number: own === null ? null : `${own.number}${clone}`, name, colour, ink: inkOn(colour) };
}

/**
 * Each thread's line, by id. Threads whose numbers clash are told apart by their clones' initials, or by their whole
 * clone ids when two initials match; an older id, with no clone, keeps its bare number.
 */
export function linesOf(threads: Thread[]): Map<string, Line> {
  const owned = threads.map((thread) => ({ thread, own: parts(thread.id) }));
  const sharing = (number: number) => owned.filter(({ own }) => own?.number === number);
  return new Map(
    owned.map(({ thread, own }) => {
      const others = own === null ? [] : sharing(own.number);
      if (own?.clone == null || others.length < 2) return [thread.id, lineOf(thread)];
      const initial = own.clone.slice(0, 1);
      const alike = others.filter((one) => one.own?.clone?.slice(0, 1) === initial).length;
      return [thread.id, lineOf(thread, alike > 1 ? own.clone : initial)];
    }),
  );
}

/** Threads in the order they are listed: by number, then by title, with those without a number last. */
export function byNumber(a: Thread, b: Thread): number {
  const [x, y] = [parts(a.id)?.number ?? null, parts(b.id)?.number ?? null];
  if (x !== y) return x === null ? 1 : y === null ? -1 : x - y;
  return a.title.localeCompare(b.title, undefined, { numeric: true });
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
