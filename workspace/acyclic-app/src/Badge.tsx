// A thread's badge: its number on its line's colour, or a dot in that colour when it has no number.

import type { CSSProperties } from "react";
import { Icon, Inbox } from "./icons";
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

/** A need's badge: its thread's, or for a need with no thread the inbox icon in its place, so every head lines up. */
export function NeedBadge({ line }: { line: Line | undefined }) {
  return line === undefined ? (
    <span className="badge plain" aria-hidden="true">
      <Icon of={Inbox} size={16} />
    </span>
  ) : (
    <Badge line={line} />
  );
}
