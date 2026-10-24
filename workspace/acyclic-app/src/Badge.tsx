// A thread's badge: its number on its line's colour, or a dot in that colour when it has no number.

import type { CSSProperties } from "react";
import type { Line } from "./lines";

/** The custom properties a line's badge, and the button around it, read their colours from. */
export function lineStyle(line: Line | undefined): CSSProperties | undefined {
  return line === undefined ? undefined : ({ "--line": line.colour, "--line-ink": line.ink } as CSSProperties);
}

export function Badge({ line }: { line: Line | undefined }) {
  if (line === undefined) return null;
  return line.number === null ? (
    <span className="badge dot" aria-hidden="true" />
  ) : (
    <span className="badge">{line.number}</span>
  );
}
