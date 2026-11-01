// The sidebar's outline: the plan's projects, the top-level items the Map draws as zones, each opening into the threads
// at work in it, and the threads in none. Pure, so the rules are tested without a browser.

import { dependencyOrder, topOrder } from "./map/elk";
import type { Model } from "./map/model";

export interface ProjectRow {
  id: string;
  title: string;
  /** The threads with work under it, in the order given; `also` names the other projects each has work in. */
  threads: { id: string; also: string[] }[];
}

export interface Outline {
  projects: ProjectRow[];
  /** Threads with work in no project: none at all, or only in items at the top level. */
  others: string[];
}

/**
 * The outline of `model` as the Map shows it, archived subtrees only when `showArchived`, for `threads` in their
 * order, and its projects in the order the Map packs its top level in: each after those it waits on. A project is a
 * top-level item with something under it shown; a thread is in a project when any item shown in it, the project itself
 * included, is in the thread.
 */
export function outlineOf(model: Model, threads: readonly string[], showArchived: boolean): Outline {
  const shown = (id: string) => showArchived || !model.items.get(id)?.archived;
  const projects: { id: string; title: string; threads: Set<string> }[] = [];
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
    projects.push({ id: root, title: model.items.get(root)?.title ?? root, threads: touched });
  }
  const homes = (thread: string) =>
    projects.filter((project) => project.threads.has(thread)).map((project) => project.id);
  return {
    projects: projects.map((project) => ({
      id: project.id,
      title: project.title,
      threads: threads
        .filter((thread) => project.threads.has(thread))
        .map((thread) => ({ id: thread, also: homes(thread).filter((other) => other !== project.id) })),
    })),
    others: threads.filter((thread) => homes(thread).length === 0),
  };
}

/** The rows a selected item marks: its project, unless it is a tile at the top level, and its thread's row there. */
export function currentRows(
  model: Model,
  outline: Outline,
  selected: string | null,
): { project: string | null; thread: string | null } {
  if (selected === null || !model.items.has(selected)) return { project: null, thread: null };
  // Up to the top as the Map walks it: a root, or the item where a cycle of parents was broken.
  let root = selected;
  for (let parent = model.items.get(root)?.parent; (model.depth.get(root) ?? 0) > 0 && parent; ) {
    root = parent;
    parent = model.items.get(root)?.parent;
  }
  const project = outline.projects.some((one) => one.id === root) ? root : null;
  return { project, thread: model.items.get(selected)?.thread ?? null };
}

/**
 * A thread's items not archived, which can be selected, each after those in the thread it waits for, ties by id. An
 * approval covers the archived too, which the count of what it covers says.
 */
export function membersOf(model: Model, thread: string): string[] {
  const ids = [...model.items.values()]
    .filter((item) => item.thread === thread && !item.archived)
    .map((item) => item.id);
  const inThread = new Set(ids);
  const edges = ids.flatMap((id) =>
    (model.items.get(id)?.waitsFor ?? [])
      .filter((first) => inThread.has(first))
      .map((first) => ({ source: first, target: id })),
  );
  return dependencyOrder(ids, edges);
}

/**
 * The thread still picked once `id` is selected: one thing in focus, so selecting an item outside the picked thread
 * unpicks it, and one in it keeps it.
 */
export function pickedAfter(model: Model | null, picked: string | null, id: string): string | null {
  return picked !== null && model?.items.get(id)?.thread === picked ? picked : null;
}
