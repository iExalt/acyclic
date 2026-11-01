// What the Overview lists of the threads: those at work, and those to start next. Pure, so the rules are tested
// without a browser.

import type { Counts } from "./api";
import type { WavesView } from "./waves";

export interface ThreadLists {
  /** Threads with work in progress or in review, by id in natural order. */
  working: string[];
  /**
   * Threads to start: not at work, approved, with a task ready, the Any time ones first (what can start now reads
   * first, as on the Map), then by wave. A thread waiting for approval is in Needs you instead, and one with every task
   * blocked can't be started.
   */
  next: string[];
}

/** Threads with work in progress or in review, by id in natural order. */
export function workingThreads(counts: ReadonlyMap<string, Counts>): string[] {
  return [...counts.entries()]
    .filter(([, one]) => one.in_progress + one.in_review > 0)
    .map(([id]) => id)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
}

/** The lists, with `awaiting` the threads Needs you asks the user to approve. */
export function threadLists(
  counts: ReadonlyMap<string, Counts>,
  waves: WavesView | null,
  awaiting: ReadonlySet<string>,
): ThreadLists {
  const working = workingThreads(counts);
  if (waves === null) return { working, next: [] };
  const startable = (id: string) => !working.includes(id) && !awaiting.has(id) && (counts.get(id)?.ready ?? 0) > 0;
  const next = [...waves.anytime, ...waves.waves.flat()].map((card) => card.id).filter(startable);
  return { working, next };
}
