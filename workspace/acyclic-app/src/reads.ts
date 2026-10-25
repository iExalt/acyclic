// The plan's reads, in the order shown. Reads overlap: a refresh on coming back to the app, a write's own read. Each
// is numbered as it starts, and one overtaken by a later read already shown is never shown. A write the server
// answered whose own read then failed stays drawn, as unread, until a read started after the answer is shown: an
// older read still out can't hold it. Pure, so every ordering is tested without a browser.

import type { Item } from "./api";
import { type Pending, withPending } from "./edits";

/** A write the server holds that no read shown holds yet, with the number of reads started when it answered. */
export interface Unread {
  pending: Pending;
  after: number;
}

export interface Reads {
  started: number;
  shown: number;
  /** The latest shown read's items, as the server had them. */
  items: Item[];
  unread: Unread[];
}

export const NO_READS: Reads = { started: 0, shown: 0, items: [], unread: [] };

/** The reads with one more started, which is numbered `started`. */
export const start = (reads: Reads): Reads => ({ ...reads, started: reads.started + 1 });

/** The reads with read `n`'s items shown, or null when a read started later is shown already. */
export function show(reads: Reads, n: number, items: Item[]): Reads | null {
  if (n < reads.shown) return null;
  return { ...reads, shown: n, items, unread: reads.unread.filter((one) => one.after >= n) };
}

/** The reads with `pending`, answered as saved before any read started from now, drawn until a read holds it. */
export const answered = (reads: Reads, pending: Pending): Reads => ({
  ...reads,
  unread: [...reads.unread, { pending, after: reads.started }],
});

/** The plan to draw: the server's items, then writes it holds unread, then the write still out. */
export const drawn = (items: Item[], unread: Unread[], pending: Pending | null): Item[] =>
  withPending(
    unread.reduce((plan, one) => withPending(plan, one.pending), items),
    pending,
  );
