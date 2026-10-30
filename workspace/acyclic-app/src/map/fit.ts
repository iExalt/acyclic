// Labels cut to fit their nodes, line by line. The renderer can wrap a label or cut one line short, but not both, so
// the Map lays out each label's lines itself: words wrapped while lines are left, the last one cut short with an
// ellipsis. Pure, given how wide a run of text is drawn, so the rules are tested without a canvas.

/** How wide `text` is drawn, in the Map's units. */
export type Measure = (text: string) => number;

const ELLIPSIS = "…";

/** `text` cut short with an ellipsis to fit `width`, or whole when it fits already. */
export function cut(text: string, width: number, measure: Measure): string {
  if (measure(text) <= width) return text;
  // The longest start that fits beside the ellipsis, found by halving: a title is measured a handful of times.
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (measure(text.slice(0, mid).trimEnd() + ELLIPSIS) <= width) low = mid;
    else high = mid - 1;
  }
  return text.slice(0, low).trimEnd() + ELLIPSIS;
}

/**
 * `text` in at most `lines` lines of `width`: words wrapped onto the next line while one is left, and the last line
 * cut short. A word too long for a line of its own is cut there.
 */
export function wrap(text: string, width: number, lines: number, measure: Measure): string[] {
  const out: string[] = [];
  let rest = text.trim();
  while (rest !== "" && out.length < lines - 1) {
    if (measure(rest) <= width) break;
    const words = rest.split(/\s+/);
    let line = words[0] ?? "";
    let taken = 1;
    while (taken < words.length && measure(`${line} ${words[taken]}`) <= width) line = `${line} ${words[taken++]}`;
    if (measure(line) > width) break;
    out.push(line);
    rest = words.slice(taken).join(" ");
  }
  if (rest !== "") out.push(cut(rest, width, measure));
  return out;
}
