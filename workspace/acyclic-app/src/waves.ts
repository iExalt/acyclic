// The Threads view's cards: each thread with work left, in the wave the tracker puts it in, with the phases it works
// in, its leaves by stage, and the threads it waits on. Pure, so the rules are tested without a browser.

import type { Counts, Item, Threads } from "./api";
import type { Outline } from "./outline";

export interface Card {
  id: string;
  /** The titles of the phases it has work in. */
  phases: string[];
  /** Its leaves by stage; parents and archived leaves have none. */
  counts: Counts;
  /** The threads in earlier waves it waits on. */
  after: string[];
  /** The threads in its own wave it waits on, which wait on it in turn: a cycle to break. */
  looped: string[];
}

export interface WavesView {
  waves: Card[][];
  anytime: Card[];
}

const NONE: Counts = {
  done: 0,
  leaves: 0,
  in_review: 0,
  in_progress: 0,
  ready: 0,
  blocked: 0,
  parked: 0,
  unthreaded: 0,
};

/** The cards for `listed`'s waves, counted from `items` and placed in `outline`'s phases. */
export function wavesView(listed: Threads, items: readonly Item[], outline: Outline): WavesView {
  const counts = new Map<string, Counts>();
  for (const item of items) {
    if (item.thread === null || item.stage === undefined) continue;
    const was = counts.get(item.thread) ?? { ...NONE };
    was.leaves += 1;
    if (item.stage === "done" || item.stage === "in-review") was.done += 1;
    if (item.stage === "in-review") was.in_review += 1;
    else if (item.stage === "in-progress") was.in_progress += 1;
    else if (item.stage !== "done") was[item.stage] += 1;
    counts.set(item.thread, was);
  }
  const wave = new Map<string, number>();
  listed.waves.forEach((ids, at) => {
    for (const id of ids) wave.set(id, at);
  });
  const waits = new Map(listed.threads.map((thread) => [thread.id, thread.waitsFor]));
  const card = (id: string): Card => {
    const on = waits.get(id) ?? [];
    return {
      id,
      phases: outline.phases.filter((phase) => phase.threads.some((one) => one.id === id)).map((phase) => phase.title),
      counts: counts.get(id) ?? NONE,
      after: on.filter((other) => (wave.get(other) ?? -1) < (wave.get(id) ?? -1)),
      looped: on.filter((other) => wave.get(other) === wave.get(id)),
    };
  };
  return { waves: listed.waves.map((ids) => ids.map(card)), anytime: listed.anytime.map(card) };
}
