// A thread's badge: a dot in its line's colour.

import type { CSSProperties } from "react";
import { Icon, Inbox } from "./icons";
import type { Line } from "./lines";

/** The custom properties a line's badge, and the button around it, read their colours from. */
export function lineStyle(line: Line | undefined): CSSProperties | undefined {
  // A task alone has no colour: it is no thread's line.
  return line === undefined || line.task !== null
    ? undefined
    : ({ "--line": line.colour, "--line-ink": line.ink } as CSSProperties);
}

export function Badge({ line }: { line: Line | undefined }) {
  if (line === undefined) return null;
  // A task alone is no thread, so it has no colour: an open ring in the quiet ink, where a thread's dot would stand.
  if (line.task !== null) return <span className="badge dot task" aria-hidden="true" />;
  return <span className="badge dot" aria-hidden="true" />;
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
