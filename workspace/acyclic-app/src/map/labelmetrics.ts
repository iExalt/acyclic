// The Map's labels and the room they need, shared by layout and drawing so titles never get laid out at one size and
// rendered at another. Text wraps without losing characters, including a word or grapheme wider than the line.

import { countsText } from "../api";
import { STAGE_WORDS } from "../stages";
import type { Measure } from "./fit";
import { BOX_TEXT_LEFT, MARKS_LEFT, MARKS_RIGHT } from "./marks";
import type { Kind, VisibleNode } from "./model";

export const TILE_WIDTH = 200;
export const MIN_NODE_HEIGHT = 64;
export const BASE_HEADER = 52;
export const LINE_HEIGHT = 16.25;

export interface LabelFonts {
  regular: Measure;
  bold: Measure;
}

export interface LabelMetrics {
  width: number;
  height: number;
  header: number;
  textWidth: number;
  lines: string[];
}

export function textWidth(kind: Kind, width: number): number {
  return Math.max(40, width - (kind === "leaf" ? MARKS_LEFT + MARKS_RIGHT : BOX_TEXT_LEFT + 10));
}

export function metricsOf(
  node: Pick<VisibleNode, "title" | "kind" | "hidden" | "stage" | "progress" | "note" | "doneBox">,
  fonts: LabelFonts,
  width = TILE_WIDTH,
): LabelMetrics {
  const available = textWidth(node.kind, width);
  const title = node.title.replace(/`([^`]+)`/g, "$1");
  const lines =
    node.kind === "leaf"
      ? [...wrap(title, available, fonts.regular), ...(node.stage === null ? [] : [STAGE_WORDS[node.stage]])]
      : boxLines(node, title, available, fonts.bold);
  const header = node.kind === "leaf" ? 0 : Math.max(BASE_HEADER, Math.ceil(lines.length * LINE_HEIGHT + 16));
  return {
    width,
    height: Math.max(MIN_NODE_HEIGHT, Math.ceil(lines.length * LINE_HEIGHT + 20)),
    header,
    textWidth: available,
    lines,
  };
}

function boxLines(
  node: Pick<VisibleNode, "kind" | "hidden" | "progress" | "note" | "doneBox" | "stage">,
  title: string,
  width: number,
  measure: Measure,
): string[] {
  const hidden = node.kind === "collapsed" && node.note === undefined ? ` (+${node.hidden})` : "";
  const named =
    node.kind === "collapsed" && node.doneBox?.kind === "thread"
      ? wrap(title, width, measure)
      : wrap(title + hidden, width, measure);
  const status = node.stage === null ? [] : [STAGE_WORDS[node.stage]];
  if (node.note !== undefined) return [...named, ...status, ...wrap(node.note, width, measure)];
  return node.progress === null
    ? [...named, ...status]
    : [...named, ...status, ...wrap(countsText(node.progress), width, measure)];
}

function wrap(text: string, width: number, measure: Measure): string[] {
  const segmenter =
    typeof Intl.Segmenter === "undefined" ? null : new Intl.Segmenter(undefined, { granularity: "grapheme" });
  const graphemes = (value: string) =>
    segmenter === null ? fallbackGraphemes(value) : [...segmenter.segment(value)].map(({ segment }) => segment);
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
