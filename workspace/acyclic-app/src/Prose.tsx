// Text as a person or agent wrote it, with `backticked` runs set as code, as Markdown sets them: in titles, notes and
// briefs, ids and commands are written that way.

import { Fragment, type ReactNode } from "react";

/** A run of text, set as code when it was backticked. */
export interface Span {
  text: string;
  code: boolean;
}

/**
 * Splits text into plain runs and code spans: a code span runs from a backtick to the next, holding something. A
 * backtick with no partner, or two with nothing between them, stays as written.
 */
export function spans(text: string): Span[] {
  const out: Span[] = [];
  const plain = (run: string) => {
    if (run === "") return;
    const last = out.at(-1);
    if (last !== undefined && !last.code) last.text += run;
    else out.push({ text: run, code: false });
  };
  let at = 0;
  for (const match of text.matchAll(/`([^`]+)`/g)) {
    plain(text.slice(at, match.index));
    out.push({ text: match[1] ?? "", code: true });
    at = match.index + match[0].length;
  }
  plain(text.slice(at));
  return out;
}

/** A run of notes, set as it was marked: code as above, and **strong** or *emphasised* as Markdown sets them. */
export interface Marked extends Span {
  strong: boolean;
  em: boolean;
}

/** An emphasis: a single `*` or `_` hugging a run on both sides, and never inside a word, as in snake_case. */
const EMPHASIS = /(?<![\p{L}\p{N}*_])([*_])(?=\S)(.+?)(?<=\S)\1(?![\p{L}\p{N}*_])/gu;

/**
 * Splits notes into runs, a small, safe part of Markdown: code spans, then strong runs between pairs of `**`, which may
 * hold code, then emphasis within a plain run. A marker with no partner stays as written; nothing else is read.
 */
export function marked(text: string): Marked[] {
  const out: Marked[] = [];
  const push = (run: Marked) => {
    if (run.text === "") return;
    const last = out.at(-1);
    if (last !== undefined && !last.code && !run.code && last.strong === run.strong && last.em === run.em)
      last.text += run.text;
    else out.push(run);
  };
  const runs = spans(text);
  // The `**` in plain runs, paired in order; an odd one out at the end stays as text.
  const markers = runs.flatMap((run, at) =>
    run.code ? [] : [...run.text.matchAll(/\*\*/g)].map((match) => ({ at, index: match.index })),
  );
  const paired = markers.slice(0, markers.length - (markers.length % 2));
  const emphasised = (run: string, strong: boolean) => {
    let from = 0;
    for (const match of run.matchAll(EMPHASIS)) {
      push({ text: run.slice(from, match.index), code: false, strong, em: false });
      push({ text: match[2] ?? "", code: false, strong, em: true });
      from = match.index + match[0].length;
    }
    push({ text: run.slice(from), code: false, strong, em: false });
  };
  let strong = false;
  runs.forEach((run, at) => {
    if (run.code) {
      push({ ...run, strong, em: false });
      return;
    }
    let from = 0;
    for (const marker of paired.filter((one) => one.at === at)) {
      emphasised(run.text.slice(from, marker.index), strong);
      strong = !strong;
      from = marker.index + 2;
    }
    emphasised(run.text.slice(from), strong);
  });
  return out;
}

/** Text as written, its backticked runs as code; with `notes`, its strong and emphasised runs too. */
export function Prose({ text, notes = false }: { text: string; notes?: boolean }) {
  if (!notes)
    return spans(text).map((span, at) =>
      // A run's place in the text is its key: the runs change only with the text.
      span.code ? <code key={at}>{span.text}</code> : span.text,
    );
  return marked(text).map((run, at) => {
    let shown: ReactNode = run.code ? <code>{run.text}</code> : run.text;
    if (run.em) shown = <em>{shown}</em>;
    if (run.strong) shown = <strong>{shown}</strong>;
    return <Fragment key={at}>{shown}</Fragment>;
  });
}
