// Finished work folded away on the Map. In each box, a finished thread's items there become one box named after the
// thread, and the finished work no link ties to anything unfinished in the box goes into one box, Done, at the end of
// its columns, so what is left to do reads first. Each is a done box, which only the Map's drawing holds: folded, it
// is drawn as a tile, and unfolded, as a frame around what it holds, so folding, links, lighting and bringing an item
// into view work as they do for any box. Pure, so the rule is tested without a browser.

import type { Counts } from "../api";
import { natural } from "../natural";
import type { MapItem, Model } from "./model";

/** The ids of done boxes, which sort after every item's, as the Done box stands last. */
const DONE = "~done";
const THREAD = "~thread";

/** Whether an id names a box's Done box. */
export function isRest(id: string): boolean {
  return id.startsWith(DONE);
}

/** What a done box holds: a finished thread's items, or the rest of a box's finished work. */
export type DoneBox = { kind: "thread"; thread: string } | { kind: "rest"; threads: number };

/** Whether an id names a done box rather than an item. */
export function isDoneBox(id: string): boolean {
  return id.startsWith("~");
}

/** The model with each box's finished work folded into done boxes, and what each holds. */
export interface Folded {
  model: Model;
  doneBoxes: ReadonlyMap<string, DoneBox>;
}

/**
 * Folds the finished work in each box of `model`. A thread is finished when every leaf it has is
 * done or canceled, and a canceled leaf stays where it is, out of the Done box. Each finished thread with two or more of a box's leaves becomes a box in that box. Those boxes and the box's
 * other done leaves that are finished (a finished thread's last one there, or one in no thread) go into the box's
 * Done box, unless a link inside the box joins them, directly or through each other, to anything else; it
 * forms only when it would hold two or more and leave something beside it. `titleOf` names a thread.
 */
export function foldDone(model: Model, titleOf: (thread: string) => string | null): Folded {
  const live = (item: MapItem) => !model.children.has(item.id);
  // A thread the plan doesn't name, as one deleted elsewhere, counts as none.
  const threadOf = (item: MapItem) => (item.thread !== null && titleOf(item.thread) !== null ? item.thread : null);
  const finished = new Map<string, boolean>();
  for (const item of model.items.values()) {
    const thread = threadOf(item);
    // Canceled work is finished too, but stays out of the Done box: only done leaves fold.
    if (thread !== null && live(item))
      finished.set(thread, (finished.get(thread) ?? true) && (item.stage === "done" || item.stage === "canceled"));
  }
  const doneLeaf = (item: MapItem) => {
    const thread = threadOf(item);
    return live(item) && item.stage === "done" && (thread === null || finished.get(thread) === true);
  };

  const items = new Map(model.items);
  const children = new Map(model.children);
  const depth = new Map(model.depth);
  const doneBoxes = new Map<string, DoneBox>();
  const add = (id: string, title: string, parent: string, thread: string | null, members: string[]) => {
    const leaves = members.reduce((sum, member) => sum + leavesOf(items, children, member), 0);
    const counts: Counts = { ...NONE, done: leaves, leaves };
    items.set(id, {
      id,
      title,
      state: "done",
      parent,
      thread,
      stage: "done",
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
    if (isDoneBox(box)) continue;
    const done = kids.flatMap((id) => {
      const item = model.items.get(id);
      return item !== undefined && doneLeaf(item) ? [item] : [];
    });
    if (done.length === 0) continue;
    // Each finished thread's leaves here, in the order the box lists them.
    const byThread = new Map<string, string[]>();
    for (const item of done) {
      const thread = threadOf(item);
      if (thread !== null) byThread.set(thread, [...(byThread.get(thread) ?? []), item.id]);
    }
    // What each child stands as among the box's children once its thread's leaves are a box.
    const as = new Map<string, string>(kids.map((id) => [id, id]));
    const made: string[] = [];
    for (const [thread, members] of byThread) {
      if (members.length < 2) continue;
      const id = `${THREAD} ${box} ${thread}`;
      for (const member of members) as.set(member, id);
      made.push(id);
      doneBoxes.set(id, { kind: "thread", thread });
      add(id, titleOf(thread) ?? thread, box, thread, members);
    }
    const finishedNodes = new Set([...made, ...done.map((item) => item.id).filter((id) => as.get(id) === id)]);
    // The box's children as they now stand, and the links between them, each lifted from what is inside a child.
    const standing = [...new Set(kids.map((id) => as.get(id) ?? id))].sort(natural);
    const owner = new Map<string, string>();
    for (const kid of kids) for (const id of subtree(model, kid)) owner.set(id, as.get(kid) ?? kid);
    const touching = new Map<string, Set<string>>(standing.map((id) => [id, new Set()]));
    for (const [id, at] of owner)
      for (const first of model.items.get(id)?.waitsFor ?? []) {
        const from = owner.get(first);
        if (from === undefined || from === at) continue;
        touching.get(at)?.add(from);
        touching.get(from)?.add(at);
      }
    // Finished work stays out of the Done box when a way of links among finished work reaches anything else.
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
    // Something must stand beside it.
    const beside = standing.filter((id) => !loose.includes(id));
    if (loose.length < 2 || beside.length === 0) {
      if (made.length > 0) children.set(box, standing);
      continue;
    }
    const rest = `${DONE} ${box}`;
    const threads = new Set(
      loose.flatMap((id) => {
        const made = doneBoxes.get(id);
        const item = model.items.get(id);
        const thread = made?.kind === "thread" ? made.thread : item === undefined ? null : threadOf(item);
        return thread === null || thread === undefined ? [] : [thread];
      }),
    );
    doneBoxes.set(rest, { kind: "rest", threads: threads.size });
    add(rest, "Done", box, null, loose);
    // Last, as the end of its box's columns, which `beside` keeps.
    children.set(box, [...standing.filter((id) => !loose.includes(id)), rest]);
  }
  return { model: { ...model, items, children, depth }, doneBoxes };
}

const NONE: Counts = {
  done: 0,
  leaves: 0,
  in_progress: 0,
  ready: 0,
  stalled: 0,
  waiting: 0,
  parked: 0,
  canceled: 0,
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
  if (kids === undefined) return 1;
  return kids.reduce((sum, kid) => sum + leavesOf(items, children, kid), 0);
}

/** Moves an item and everything under it `by` levels deeper. */
function deepen(depth: Map<string, number>, children: Map<string, string[]>, id: string, by: number) {
  depth.set(id, (depth.get(id) ?? 0) + by);
  for (const kid of children.get(id) ?? []) deepen(depth, children, kid, by);
}

/** The words under a done box's name: how many items it holds, and, for the Done box, in how many threads. */
export function noteOf(doneBox: DoneBox, leaves: number): string {
  const items = `${leaves} done`;
  if (doneBox.kind === "thread") return items;
  return doneBox.threads === 0
    ? items
    : `${items} in ${doneBox.threads} ${doneBox.threads === 1 ? "thread" : "threads"}`;
}
