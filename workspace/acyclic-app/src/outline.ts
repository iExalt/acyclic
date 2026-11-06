// The sidebar's outline: the plan's projects, the top-level items the Map draws as zones, each opening into the threads
// at work in it, and the threads in none. Pure, so the rules are tested without a browser.

import { topOrder } from "./map/elk";
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
 * The outline of `model` as the Map shows it, for `threads` in their
 * order, and its projects in the order the Map packs its top level in: each after those it waits on. A project is a
 * top-level item with something under it shown; a thread is in a project when any item shown in it, the project itself
 * included, is in the thread.
 */
export function outlineOf(model: Model, threads: readonly string[]): Outline {
  const projects: { id: string; title: string; threads: Set<string> }[] = [];
  for (const root of topOrder(model)) {
    if ((model.children.get(root) ?? []).length === 0) continue;
    const touched = new Set<string>();
    const walk = (id: string) => {
      const thread = model.items.get(id)?.thread;
      if (thread !== null && thread !== undefined) touched.add(thread);
      for (const child of model.children.get(id) ?? []) walk(child);
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

/** A project's default thread only when its visible work belongs to exactly one thread. */
export function uniqueProjectThread(
  project: string | null,
  projects: readonly { id: string; threads: readonly { id: string }[] }[],
): string | null {
  if (project === null) return null;
  const threads = projects.find((one) => one.id === project)?.threads ?? [];
  return threads.length === 1 ? (threads[0]?.id ?? null) : null;
}
