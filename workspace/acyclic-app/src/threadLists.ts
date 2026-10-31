// What the Overview lists of the threads: those at work, and those to start next. Pure, so the rules are tested
// without a browser.

import type { Counts } from "./api";
import type { WavesView } from "./waves";

export interface ThreadLists {
  /** Threads with work in progress or in review, by id in natural order. */
  working: string[];
  /** The threads of `wave` not at work yet: the ones to start. */
  next: string[];
  /** The earliest wave with a thread not at work yet, from 0, or null when no wave has one. */
  wave: number | null;
}

export function threadLists(counts: ReadonlyMap<string, Counts>, waves: WavesView | null): ThreadLists {
  const working = [...counts.entries()]
    .filter(([, one]) => one.in_progress + one.in_review > 0)
    .map(([id]) => id)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  const idle = (card: { id: string }) => !working.includes(card.id);
  const at = waves?.waves.findIndex((cards) => cards.some(idle)) ?? -1;
  if (waves === null || at < 0) return { working, next: [], wave: null };
  return { working, next: (waves.waves[at] ?? []).filter(idle).map((card) => card.id), wave: at };
}
