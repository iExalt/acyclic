// The Threads view's tiles: each thread with work left, in the wave the tracker puts it in, with the projects it works
// in, its leaves by stage, and the threads it waits on. Pure, so the rules are tested without a browser.

import { type Counts, NO_COUNTS, type Threads } from "./api";
import type { Outline } from "./outline";

export interface ThreadTile {
  id: string;
  /** The titles of the projects it has work in. */
  projects: string[];
  /** Its leaves by stage, as core counts them, whatever the Map shows. */
  counts: Counts;
  /** The threads in earlier waves it waits on. */
  after: string[];
  /** The threads in its own wave it waits on, which wait on it in turn: a cycle to break. */
  looped: string[];
}

export interface WavesView {
  waves: ThreadTile[][];
  anytime: ThreadTile[];
}

/** Project labels for a tile, excluding a project whose title merely repeats the thread's name. */
export function projectLabels(projects: string[], threadName: string): string[] {
  const name = threadName.trim().toLowerCase();
  return projects.filter((project) => project.trim().toLowerCase() !== name);
}

/**
 * `listed` narrowed to the threads in `keep`: a wave left with none is dropped, so the waves a view shows are numbered
 * without gaps, and a thread waits only for those kept. Waves are still the ones core computed across every thread,
 * so the order is shared, and only the numbers are the view's own.
 */
export function scopeThreads(listed: Threads, keep: ReadonlySet<string>): Threads {
  return {
    threads: listed.threads
      .filter((one) => keep.has(one.id))
      .map((one) => ({ ...one, waitsFor: one.waitsFor.filter((other) => keep.has(other)) })),
    waves: listed.waves.map((ids) => ids.filter((id) => keep.has(id))).filter((ids) => ids.length > 0),
    anytime: listed.anytime.filter((id) => keep.has(id)),
  };
}

/** The tiles for `listed`'s waves, with core's counts of each thread's leaves, placed in `outline`'s projects. */
export function wavesView(listed: Threads, outline: Outline): WavesView {
  const counts = new Map(listed.threads.map((thread) => [thread.id, thread.counts]));
  const wave = new Map<string, number>();
  listed.waves.forEach((ids, at) => {
    for (const id of ids) wave.set(id, at);
  });
  const waits = new Map(listed.threads.map((thread) => [thread.id, thread.waitsFor]));
  const tile = (id: string): ThreadTile => {
    const on = waits.get(id) ?? [];
    return {
      id,
      projects: outline.projects
        .filter((project) => project.threads.some((one) => one.id === id))
        .map((project) => project.title),
      counts: counts.get(id) ?? NO_COUNTS,
      after: on.filter((other) => (wave.get(other) ?? -1) < (wave.get(id) ?? -1)),
      looped: on.filter((other) => wave.get(other) === wave.get(id)),
    };
  };
  return { waves: listed.waves.map((ids) => ids.map(tile)), anytime: listed.anytime.map(tile) };
}
