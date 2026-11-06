// What the Overview lists of the threads: those at work, and those to start next. Pure, so the rules are tested
// without a browser.

import type { Counts } from "./api";
import { natural } from "./natural";

export interface ThreadLists {
  /** Threads with work in progress or stalled, by id in natural order. */
  working: string[];
  /** Threads to start: not at work, with an item ready, by id in natural order, as the CLI lists ready work. */
  next: string[];
}

/** Threads with work in progress, by id in natural order, and the stalled ones: their holder's lease ran out. */
export function workingThreads(counts: ReadonlyMap<string, Counts>): string[] {
  return [...counts.entries()]
    .filter(([, one]) => one.in_progress > 0 || one.stalled > 0)
    .map(([id]) => id)
    .sort(natural);
}

/** The lists. */
export function threadLists(counts: ReadonlyMap<string, Counts>): ThreadLists {
  const working = workingThreads(counts);
  const next = [...counts.entries()]
    .filter(([id, one]) => !working.includes(id) && one.ready > 0)
    .map(([id]) => id)
    .sort(natural);
  return { working, next };
}
