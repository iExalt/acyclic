// Text as a person or agent wrote it, with `backticked` runs set as code, as Markdown sets them: in titles, notes and
// briefs, ids and commands are written that way.

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

export function Prose({ text }: { text: string }) {
  return spans(text).map((span, at) =>
    // A run's place in the text is its key: the runs change only with the text.
    span.code ? <code key={at}>{span.text}</code> : span.text,
  );
}
