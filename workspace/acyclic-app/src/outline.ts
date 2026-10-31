// The sidebar's outline: the plan's phases, the top-level items the Map draws as zones, each opening into the threads
// at work in it, and the threads in none. Pure, so the rules are tested without a browser.

import { dependencyOrder, topOrder } from "./map/elk";
import type { Model } from "./map/model";

export interface PhaseRow {
  id: string;
  title: string;
  /** The threads with work under it, in the order given; `also` names the other phases each has work in. */
  threads: { id: string; also: string[] }[];
}

export interface Outline {
  phases: PhaseRow[];
  /** Threads with work in no phase: none at all, or only in tasks at the top level. */
  others: string[];
}

/**
 * The outline of `model` as the Map shows it, archived subtrees only when `showArchived`, for `threads` in their
 * order, and its phases in the order the Map packs its top level in: each after those it waits on. A phase is a
 * top-level item with something under it shown; a thread is in a phase when any item shown in it, the phase itself
 * included, is in the thread.
 */
export function outlineOf(model: Model, threads: readonly string[], showArchived: boolean): Outline {
  const shown = (id: string) => showArchived || !model.items.get(id)?.archived;
  const phases: { id: string; title: string; threads: Set<string> }[] = [];
  for (const root of topOrder(model, showArchived)) {
    const kids = (model.children.get(root) ?? []).filter(shown);
    if (!shown(root) || kids.length === 0) continue;
    const touched = new Set<string>();
    const walk = (id: string) => {
      const thread = model.items.get(id)?.thread;
      if (thread !== null && thread !== undefined) touched.add(thread);
      for (const child of model.children.get(id) ?? []) if (shown(child)) walk(child);
    };
    walk(root);
    phases.push({ id: root, title: model.items.get(root)?.title ?? root, threads: touched });
  }
  const homes = (thread: string) => phases.filter((phase) => phase.threads.has(thread)).map((phase) => phase.id);
  return {
    phases: phases.map((phase) => ({
      id: phase.id,
      title: phase.title,
      threads: threads
        .filter((thread) => phase.threads.has(thread))
        .map((thread) => ({ id: thread, also: homes(thread).filter((other) => other !== phase.id) })),
    })),
    others: threads.filter((thread) => homes(thread).length === 0),
  };
}

/** The rows a selected item marks: its phase, unless it is a task at the top level, and its thread's row there. */
export function currentRows(
  model: Model,
  outline: Outline,
  selected: string | null,
): { phase: string | null; thread: string | null } {
  if (selected === null || !model.items.has(selected)) return { phase: null, thread: null };
  // Up to the top as the Map walks it: a root, or the item where a cycle of parents was broken.
  let root = selected;
  for (let parent = model.items.get(root)?.parent; (model.depth.get(root) ?? 0) > 0 && parent; ) {
    root = parent;
    parent = model.items.get(root)?.parent;
  }
  const phase = outline.phases.some((one) => one.id === root) ? root : null;
  return { phase, thread: model.items.get(selected)?.thread ?? null };
}

/**
 * A thread's items, every one the plan puts in it, archived too, as an approval covers them: each after those in the
 * thread it waits for, ties by id.
 */
export function membersOf(model: Model, thread: string): string[] {
  const ids = [...model.items.values()].filter((item) => item.thread === thread).map((item) => item.id);
  const inThread = new Set(ids);
  const edges = ids.flatMap((id) =>
    (model.items.get(id)?.waitsFor ?? [])
      .filter((first) => inThread.has(first))
      .map((first) => ({ source: first, target: id })),
  );
  return dependencyOrder(ids, edges);
}
