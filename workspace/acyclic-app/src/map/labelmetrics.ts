// The Map's labels and the room they need, shared by layout and drawing so titles never get laid out at one size and
// rendered at another. Item titles are clamped to a common tile size; box headers grow with their content.

import { countsText } from "../api";
import { cut, type Measure } from "./fit";
import { BOX_TEXT_LEFT, MARKS_LEFT, MARKS_RIGHT } from "./marks";
import type { Kind, VisibleNode } from "./model";

export const TILE_WIDTH = 200;
export const MIN_NODE_HEIGHT = 64;
export const ITEM_HEIGHT = 86;
export const BASE_HEADER = 52;
export const LINE_HEIGHT = 16.25;

/** A stub's chip: its height, the room it takes at the foot of a tile, and its inset from the tile's edges. */
export const STUB_CHIP = 20;
export const STUB_ROOM = 30;
export const STUB_INSET = 8;
/** Where a chip's text starts, after its arrow, and the room it leaves at the right. */
export const STUB_TEXT_LEFT = 26;
const STUB_TEXT_RIGHT = 8;

/** What a stub says of the waits that leave the initiative: one line, and everything it names for a hover. */
export interface StubLine {
  /** The first wait, cut to the chip beside its arrow, and how many more there are. */
  text: string;
  /** Every wait named in full, one a line. */
  full: string;
  /** The item a tap opens: the first named. */
  target: string;
}

export interface LabelFonts {
  regular: Measure;
  bold: Measure;
}

export interface LabelMetrics {
  width: number;
  height: number;
  /** What the foot of a leaf or folded box holds besides its label: the room of its stub, else 0. */
  reserve: number;
  stub: StubLine | null;
  header: number;
  textWidth: number;
  lines: string[];
}

export function textWidth(kind: Kind, width: number): number {
  return Math.max(40, width - (kind === "leaf" ? MARKS_LEFT + MARKS_RIGHT : BOX_TEXT_LEFT + MARKS_RIGHT));
}

export function metricsOf(
  node: Pick<VisibleNode, "title" | "kind" | "hidden" | "stage" | "progress"> & Partial<Pick<VisibleNode, "outside">>,
  fonts: LabelFonts,
  width = TILE_WIDTH,
  threadName: string | null = null,
): LabelMetrics {
  const available = textWidth(node.kind, width);
  const title = node.title.replace(/`([^`]+)`/g, "$1");
  const stub = node.kind === "box" ? null : stubOf(node.outside ?? [], width, fonts.regular);
  const reserve = stub === null ? 0 : STUB_ROOM;
  const titleLines = Math.floor((ITEM_HEIGHT - reserve - 20) / LINE_HEIGHT);
  const lines =
    node.kind === "leaf"
      ? clampTitle(title, available, titleLines, fonts.regular)
      : boxLines(node, title, available, fonts.bold);
  const membership = node.kind === "leaf" && threadName !== null;
  const footerWidth = width - MARKS_LEFT - 8;
  if (membership)
    lines.push("", ...clampTitle(`Thread: ${threadName.replace(/\s+/g, " ")}`, footerWidth, 1, fonts.regular));
  const header = node.kind === "leaf" ? 0 : Math.max(BASE_HEADER, Math.ceil(lines.length * LINE_HEIGHT + 16));
  return {
    width,
    height:
      node.kind === "leaf"
        ? ITEM_HEIGHT + (membership ? 2 * LINE_HEIGHT : 0)
        : Math.max(MIN_NODE_HEIGHT, Math.ceil(lines.length * LINE_HEIGHT + 20)) + reserve,
    reserve,
    stub,
    header,
    textWidth: membership ? footerWidth : available,
    lines,
  };
}

/** Wrap a title, then fit an ellipsis to its last visible line without splitting a grapheme. */
function clampTitle(title: string, width: number, limit: number, measure: Measure): string[] {
  const lines = wrap(title, width, measure);
  if (lines.length <= limit) return lines;
  const shown = lines.slice(0, limit);
  const tail = graphemes((shown.pop() ?? "").trimEnd());
  while (tail.length > 0 && measure(tail.join("") + "…") > width) tail.pop();
  shown.push(tail.join("").trimEnd() + "…");
  return shown;
}

/** The one line a tile or folded box carries for its waits outside the plan: the first, "+n" for the rest. */
export function stubOf(
  outside: readonly { id: string; title: string; initiative: string }[],
  width: number,
  measure: Measure,
): StubLine | null {
  const first = outside[0];
  if (first === undefined) return null;
  const more = outside.length > 1 ? ` +${outside.length - 1}` : "";
  const room = width - 2 * STUB_INSET - STUB_TEXT_LEFT - STUB_TEXT_RIGHT - measure(more);
  return {
    // The arrow is drawn apart, in the signal's colour, which a label's one colour can't give.
    text: cut(`${first.initiative}: ${first.title}`, room, measure) + more,
    full: outside.map((one) => `${one.initiative}: ${one.title}`).join("\n"),
    target: first.id,
  };
}

function boxLines(
  node: Pick<VisibleNode, "kind" | "hidden" | "progress" | "stage">,
  title: string,
  width: number,
  measure: Measure,
): string[] {
  const hidden = node.kind === "collapsed" ? ` (+${node.hidden})` : "";
  const named = wrap(title + hidden, width, measure);
  return node.progress === null ? named : [...named, ...wrap(countsText(node.progress), width, measure)];
}

function wrap(text: string, width: number, measure: Measure): string[] {
  const out: string[] = [];
  for (const paragraph of text.split("\n")) {
    let line = "";
    for (const word of paragraph.split(/(\s+)/).filter(Boolean)) {
      if (/^\s+$/.test(word)) {
        line += word;
        continue;
      }
      if (line !== "" && measure(line + word) <= width) {
        line += word;
        continue;
      }
      if (line !== "") {
        out.push(line);
        line = "";
      }
      for (const part of graphemes(word)) {
        if (line !== "" && measure(line + part) > width) {
          out.push(line);
          line = "";
        }
        line += part;
      }
    }
    out.push(line);
  }
  return out.length === 0 ? [""] : out;
}

function graphemes(value: string): string[] {
  const segmenter =
    typeof Intl.Segmenter === "undefined" ? null : new Intl.Segmenter(undefined, { granularity: "grapheme" });
  return segmenter === null ? fallbackGraphemes(value) : [...segmenter.segment(value)].map(({ segment }) => segment);
}

function fallbackGraphemes(value: string): string[] {
  const out: string[] = [];
  let regional = 0;
  for (const point of value) {
    const previous = out.length - 1;
    const joins =
      out.length > 0 &&
      (/\p{Mark}|\uFE0E|\uFE0F|\p{Emoji_Modifier}/u.test(point) ||
        point === "\u200D" ||
        out[previous]?.endsWith("\u200D") ||
        (/\p{Regional_Indicator}/u.test(point) && regional % 2 === 1));
    if (joins) out[previous] = `${out[previous] ?? ""}${point}`;
    else out.push(point);
    regional = /\p{Regional_Indicator}/u.test(point) ? regional + 1 : 0;
  }
  return out;
}
