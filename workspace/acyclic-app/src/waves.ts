// The Threads view's tiles: each thread with work left, in the wave the tracker puts it in, with the projects it works
// in, its leaves by stage, and the threads it waits on. Pure, so the rules are tested without a browser.

import { type Counts, NO_COUNTS, type Threads } from "./api";
import { NO_INITIATIVE } from "./initiatives";
import type { Model } from "./map/model";
import { natural } from "./natural";
import { type Outline, oneOf } from "./outline";

/** A thread in another initiative that a tile waits on, which the view doesn't show: named by where it is. */
export interface AwayThread {
  id: string;
  /** Its initiative's title, the first with a count of the rest as the lists' labels say them. */
  initiative: string;
}

/** A task of a tile's thread that is in another initiative, with that initiative's title. */
export interface ForeignTask {
  id: string;
  title: string;
  initiative: string;
}

/** What a tile has outside the initiative in context, which the view names rather than draws. */
export interface Outside {
  /** The threads in earlier waves it waits on. */
  after: AwayThread[];
  /** The threads in its own wave it waits on, which wait on it in turn. */
  looped: AwayThread[];
  /** Its unfinished tasks in other initiatives. */
  tasks: ForeignTask[];
}

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
  /** What it has in other initiatives. */
  outside: Outside;
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

const NOTHING: Outside = { after: [], looped: [], tasks: [] };

/**
 * What each thread in `keep` has outside it, from `listed`, which holds every thread with the waves core computed
 * across them: the threads it waits on that aren't kept, those in earlier waves apart from those in its own, named by
 * `where`; and the unfinished tasks of its thread in another initiative than `context`, named by `titles`. Without a
 * `context`, nothing is outside.
 */
export function outsideOf(
  listed: Threads,
  keep: ReadonlySet<string>,
  model: Model,
  context: string | null,
  where: (thread: string) => string,
  titles: ReadonlyMap<string, string>,
): Map<string, Outside> {
  const out = new Map<string, Outside>();
  if (context === null) return out;
  const wave = new Map<string, number>();
  listed.waves.forEach((ids, at) => {
    for (const id of ids) wave.set(id, at);
  });
  const tasks = new Map<string, ForeignTask[]>();
  for (const item of model.items.values()) {
    const leaf = (model.children.get(item.id) ?? []).length === 0;
    const thread = item.thread ?? (leaf ? oneOf(item.id) : null);
    if (thread === null || !leaf || item.initiative === context) continue;
    if (item.stage === "done" || item.stage === "canceled") continue;
    const found = tasks.get(thread) ?? [];
    found.push({
      id: item.id,
      title: item.title,
      initiative: item.initiative === null ? NO_INITIATIVE : (titles.get(item.initiative) ?? NO_INITIATIVE),
    });
    tasks.set(thread, found);
  }
  for (const one of listed.threads) {
    if (!keep.has(one.id)) continue;
    const mine = wave.get(one.id) ?? -1;
    // A thread in no wave has no work left to wait for.
    const gone = one.waitsFor.filter((other) => !keep.has(other) && wave.has(other));
    const away = (id: string): AwayThread => ({ id, initiative: where(id) });
    out.set(one.id, {
      after: gone.filter((other) => (wave.get(other) ?? -1) < mine).map(away),
      looped: gone.filter((other) => wave.get(other) === mine).map(away),
      tasks: (tasks.get(one.id) ?? []).sort(
        (a, b) =>
          a.initiative.localeCompare(b.initiative) || a.title.localeCompare(b.title) || a.id.localeCompare(b.id),
      ),
    });
  }
  return out;
}

/**
 * The tiles for `listed`'s waves, with core's counts of each thread's leaves, placed in `outline`'s projects, and what
 * each has `outside` the view.
 */
export function wavesView(
  listed: Threads,
  outline: Outline,
  outside: ReadonlyMap<string, Outside> = new Map(),
  empty: ReadonlySet<string> = new Set(),
): WavesView {
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
      outside: outside.get(id) ?? NOTHING,
    };
  };
  const placed = new Set([...listed.waves.flat(), ...listed.anytime]);
  const unplacedEmpty = listed.threads.filter((one) => empty.has(one.id) && !placed.has(one.id)).map((one) => one.id);
  return {
    waves: listed.waves.map((ids) => ids.map(tile)),
    anytime: [...listed.anytime, ...unplacedEmpty].sort(natural).map(tile),
  };
}
