// Finished work folded away on the Map. In each box, a finished thread's items there become one group named after the
// thread, and the finished work no link ties to anything unfinished in the box goes into one group of its own at the
// end of its columns, so what is left to do reads first. Each group is a box only the Map's drawing holds: folded, it
// is drawn as a tile, and unfolded, as a frame around what it holds, so folding, links, lighting and bringing an item
// into view work as they do for any box. Pure, so the rule is tested without a browser.

import type { Counts } from "../api";
import type { MapItem, Model } from "./model";

/** The ids of the groups, which sort after every item's, so the group of done work stands last. */
const DONE = "~done";
const THREAD = "~thread";

/** Whether an id names a box's group of done work. */
export function isPile(id: string): boolean {
  return id.startsWith(DONE);
}

/** What a group is: a finished thread's items, or a box's finished work. */
export type Group = { kind: "thread"; thread: string } | { kind: "done"; threads: number };

/** Whether an id names a group rather than an item. */
export function isGroup(id: string): boolean {
  return id.startsWith("~");
}

/** The model with each box's finished work folded into groups, and what each group is. */
export interface Folded {
  model: Model;
  groups: ReadonlyMap<string, Group>;
}

/**
 * Folds the finished work in each box of `model`. A thread is finished when every leaf it has, archived ones aside, is
 * done. Each finished thread with two or more of a box's leaves becomes a group in that box. Those groups and the box's
 * other done leaves that are finished (a finished thread's last one there, or one in no thread) go into the box's
 * group of done work, unless a link inside the box joins them, directly or through each other, to anything else; it
 * forms only when it would hold two or more and leave something beside it. `titleOf` names a thread.
 */
export function foldDone(model: Model, titleOf: (thread: string) => string | null): Folded {
  const live = (item: MapItem) => !item.archived && !model.children.has(item.id);
  const finished = new Map<string, boolean>();
  for (const item of model.items.values())
    if (item.thread !== null && live(item))
      finished.set(item.thread, (finished.get(item.thread) ?? true) && item.stage === "done");
  const doneLeaf = (item: MapItem) =>
    live(item) && item.stage === "done" && (item.thread === null || finished.get(item.thread) === true);

  const items = new Map(model.items);
  const children = new Map(model.children);
  const depth = new Map(model.depth);
  const groups = new Map<string, Group>();
  const add = (id: string, title: string, parent: string, thread: string | null, members: string[]) => {
    const leaves = members.reduce((sum, member) => sum + leavesOf(items, children, member), 0);
    const counts: Counts = { ...NONE, done: leaves, leaves };
    items.set(id, {
      id,
      title,
      state: "done",
      parent,
      thread,
      archived: false,
      stage: null,
      progress: counts,
      waitsFor: [],
    });
    children.set(id, members);
    depth.set(id, (depth.get(parent) ?? 0) + 1);
    for (const member of members) {
      const item = items.get(member);
      if (item !== undefined) items.set(member, { ...item, parent: id });
      deepen(depth, children, member, 1);
    }
  };

  for (const [box, kids] of model.children) {
    if (isGroup(box)) continue;
    const done = kids.flatMap((id) => {
      const item = model.items.get(id);
      return item !== undefined && doneLeaf(item) ? [item] : [];
    });
    if (done.length === 0) continue;
    // Each finished thread's leaves here, in the order the box lists them.
    const byThread = new Map<string, string[]>();
    for (const item of done)
      if (item.thread !== null) byThread.set(item.thread, [...(byThread.get(item.thread) ?? []), item.id]);
    // What each child stands as among the box's children once its thread's leaves are a group.
    const as = new Map<string, string>(kids.map((id) => [id, id]));
    const made: string[] = [];
    for (const [thread, members] of byThread) {
      if (members.length < 2) continue;
      const id = `${THREAD} ${box} ${thread}`;
      for (const member of members) as.set(member, id);
      made.push(id);
      groups.set(id, { kind: "thread", thread });
      add(id, titleOf(thread) ?? "A finished thread", box, thread, members);
    }
    const finishedNodes = new Set([...made, ...done.map((item) => item.id).filter((id) => as.get(id) === id)]);
    // The box's children as they now stand, and the links between them, each lifted from what is inside a child.
    const standing = [...new Set(kids.map((id) => as.get(id) ?? id))].sort();
    const owner = new Map<string, string>();
    for (const kid of kids) for (const id of subtree(model, kid)) owner.set(id, as.get(kid) ?? kid);
    const touching = new Map<string, Set<string>>(standing.map((id) => [id, new Set()]));
    for (const [id, at] of owner)
      for (const first of model.items.get(id)?.waitsFor ?? []) {
        const from = owner.get(first);
        const firstItem = model.items.get(first);
        if (from === undefined || from === at || firstItem?.archived || model.items.get(id)?.archived) continue;
        touching.get(at)?.add(from);
        touching.get(from)?.add(at);
      }
    // Finished work stays out of the group of done work when a way of links among finished work reaches anything else.
    const tied = new Set<string>();
    for (const start of finishedNodes) {
      if (tied.has(start)) continue;
      const component = new Set([start]);
      const stack = [start];
      let anchored = false;
      while (stack.length > 0) {
        const at = stack.pop() ?? "";
        for (const next of touching.get(at) ?? []) {
          if (!finishedNodes.has(next)) anchored = true;
          else if (!component.has(next)) {
            component.add(next);
            stack.push(next);
          }
        }
      }
      if (anchored) for (const id of component) tied.add(id);
    }
    const loose = standing.filter((id) => finishedNodes.has(id) && !tied.has(id));
    if (loose.length < 2 || loose.length === standing.length) {
      if (made.length > 0) children.set(box, standing);
      continue;
    }
    const pile = `${DONE} ${box}`;
    const threads = new Set(
      loose.flatMap((id) => {
        const group = groups.get(id);
        const thread = group?.kind === "thread" ? group.thread : model.items.get(id)?.thread;
        return thread === null || thread === undefined ? [] : [thread];
      }),
    );
    groups.set(pile, { kind: "done", threads: threads.size });
    add(pile, "Done", box, null, loose);
    // Last, as the end of its box's columns, which `beside` keeps.
    children.set(box, [...standing.filter((id) => !loose.includes(id)), pile]);
  }
  return { model: { ...model, items, children, depth }, groups };
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

/** An item and everything under it. */
function subtree(model: Model, id: string): string[] {
  const out = [id];
  for (let at = 0; at < out.length; at++) out.push(...(model.children.get(out[at] ?? "") ?? []));
  return out;
}

/** The leaves an item stands for: itself, or those under it. */
function leavesOf(items: Map<string, MapItem>, children: Map<string, string[]>, id: string): number {
  const kids = children.get(id);
  if (kids === undefined) return items.get(id)?.archived ? 0 : 1;
  return kids.reduce((sum, kid) => sum + leavesOf(items, children, kid), 0);
}

/** Moves an item and everything under it `by` levels deeper. */
function deepen(depth: Map<string, number>, children: Map<string, string[]>, id: string, by: number) {
  depth.set(id, (depth.get(id) ?? 0) + by);
  for (const kid of children.get(id) ?? []) deepen(depth, children, kid, by);
}

/** The words under a group's name: how many items it holds, and, for the group of done work, in how many threads. */
export function noteOf(group: Group, leaves: number): string {
  const items = `${leaves} done`;
  if (group.kind === "thread") return items;
  return group.threads === 0 ? items : `${items} in ${group.threads} ${group.threads === 1 ? "thread" : "threads"}`;
}
