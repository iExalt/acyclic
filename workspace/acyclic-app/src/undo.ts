// Undo: for a write that has an exact inverse, the write that takes it back, with what must still hold for that to be
// safe. A move, an item's fields but its thread, and a waits-for added or removed undo; nothing else does: an add (the app
// has no delete), a reopen, a conflict settled, a thread changed (an agent may have taken up its work). The guard is
// checked against a fresh plan and sent as conditions for the server's locked write. Pure, so tested without a browser.
import type { Command, Expected, Item, Stage } from "./api";

/** The plan a write was sent over, or the plan as it is when Undo is pressed. */
export interface Plan {
  items: ReadonlyMap<string, Item>;
}

export interface Undo {
  /** What was done, said over the Undo it offers. */
  text: string;
  /** What the undo did, said once it has. */
  undone: string;
  command: Command;
  /** The same field, link and started-work checks as `holds`, evaluated under the server's write lock. */
  expect: Expected[];
  /** Whether the plan still holds what the write left, so the undo takes back that write and nothing done since. */
  holds: (now: Plan) => boolean;
  /** An item the undo puts somewhere else, to select and bring into view where it lands, as the write did. */
  shows?: string;
}

/** An item's fields an edit can set and an undo set back, with the value each holds; an empty one is null. */
const FIELDS = ["title", "state", "notes", "size"] as const;
type Field = (typeof FIELDS)[number];
const held = (item: Item | undefined, field: Field): string | null => (item === undefined ? null : item[field]);

const record = (value: unknown): Record<string, unknown> | null =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null;

/** The undo of `command`, sent over `before`, or null when it has no exact inverse. */
export function undoOf(command: Command, before: Plan): Undo | null {
  const body = record(command);
  if (body === null) return null;
  const titleOf = (id: string) => before.items.get(id)?.title ?? id;

  const edit = record(record(body.item)?.edit);
  if (edit === null || typeof edit.item !== "string") return null;
  const id = edit.item;
  const was = before.items.get(id);
  if (was === undefined) return null;
  const keys = Object.keys(edit).filter((key) => key !== "item");

  const waits = keys.length === 1 ? waitEnd(edit, keys[0] ?? "") : null;
  if (waits !== null) {
    const added = keys[0] === "waits-for";
    const waiting = (now: Plan) => now.items.get(id)?.links.waits_for.includes(waits) === true;
    // Unwaited, an item may become ready and be started; waiting again wouldn't stop that, so only undo while the
    // work under it is no further along than it was.
    const unstarted = (now: Plan) => added || started(now.items.get(id)) <= started(was);
    // Said as the link reads, so which waits for which is plain.
    const [a, b] = [titleOf(id), titleOf(waits)];
    const [now, longer, again] = [
      `${a} now waits for ${b}`,
      `${a} no longer waits for ${b}`,
      `${a} waits for ${b} again`,
    ];
    return {
      text: added ? now : longer,
      undone: added ? longer : again,
      command: { item: { edit: { item: id, [added ? "no-waits-for" : "waits-for"]: [waits] } } },
      expect: [
        { kind: "waits-for", item: id, target: waits, present: added },
        ...(!added ? [{ kind: "started-at-most" as const, item: id, count: started(was) }] : []),
      ],
      holds: (now) => waiting(now) === added && unstarted(now),
    };
  }

  if (keys.length === 1 && (keys[0] === "parent" || keys[0] === "no-parent")) {
    const to = keys[0] === "parent" ? String(edit.parent) : null;
    const back = was.parent;
    return {
      text: to === null ? `Moved ${titleOf(id)} to the top level` : `Moved ${titleOf(id)} into ${titleOf(to)}`,
      undone: `Moved ${titleOf(id)} back`,
      command: { item: { edit: back === null ? { item: id, "no-parent": true } : { item: id, parent: back } } },
      expect: [{ kind: "item-field", item: id, field: "parent", value: to }],
      holds: (now) => (now.items.get(id)?.parent ?? null) === to,
      shows: id,
    };
  }

  // Each field set or cleared, and what it held before; a thread changed is left as it is, and any other key means an
  // edit this can't take back.
  const set = new Map<Field, string | null>();
  let thread = false;
  for (const key of keys) {
    if (key === "thread" || key === "no-thread") {
      thread = true;
      continue;
    }
    const field = FIELDS.find((one) => key === one || key === `no-${one}`);
    if (field === undefined) return null;
    set.set(field, key === field ? String(edit[key]) : null);
  }
  if (set.size === 0) return null;
  const back: Record<string, unknown> = { item: id };
  for (const field of set.keys()) {
    const old = held(was, field);
    // A title or state is never empty, so one that was can't be put back.
    if (old === null && (field === "title" || field === "state")) return null;
    if (old === null) back[`no-${field}`] = true;
    else back[field] = old;
  }
  // Named as it now reads: a title just changed is the new one.
  const named = set.get("title") ?? titleOf(id);
  const baselines = set.has("state")
    ? waitersOf(before, id).map((one) => ({ item: one, count: started(before.items.get(one)) }))
    : [];
  const stillSafeForWaiters = (now: Plan) => {
    const counts = new Map(baselines.map(({ item, count }) => [item, count]));
    const waiters = new Set([...counts.keys(), ...waitersOf(now, id)]);
    return [...waiters].every((one) => started(now.items.get(one)) <= (counts.get(one) ?? 0));
  };
  return {
    text: `Edited ${named}${thread ? ". Its thread can't be changed back from here" : ""}`,
    undone: `Set ${titleOf(id)} back${thread ? ", except its thread" : ""}`,
    command: { item: { edit: back } },
    expect: [
      ...[...set].map(([field, value]): Expected => ({ kind: "item-field", item: id, field, value })),
      ...(!set.has("state") ? [] : [{ kind: "waiters-started-at-most" as const, item: id, baselines }]),
    ],
    holds: (now) =>
      [...set].every(([field, value]) => held(now.items.get(id), field) === value) &&
      (!set.has("state") || stillSafeForWaiters(now)),
  };
}

/** The one item an edit's `waits-for` or `no-waits-for` key names, or null for any other key or list. */
function waitEnd(edit: Record<string, unknown>, key: string): string | null {
  const ends = edit[key];
  if ((key !== "waits-for" && key !== "no-waits-for") || !Array.isArray(ends) || ends.length !== 1) return null;
  return typeof ends[0] === "string" ? ends[0] : null;
}

/**
 * What waits on an item, or on a box above it, so a state set to done could make it ready: an agent may start it, and
 * setting the state back wouldn't stop that, so a state is only undone while none of it is further along.
 */
function waitersOf(plan: Plan, id: string): string[] {
  const up = above(plan, id);
  return [...plan.items.values()].filter((one) => one.links.waits_for.some((on) => up.has(on))).map((one) => one.id);
}

/** An item and the boxes above it. */
function above(plan: Plan, id: string): Set<string> {
  const up = new Set<string>();
  for (let at: string | null = id; at !== null && !up.has(at); at = plan.items.get(at)?.parent ?? null) up.add(at);
  return up;
}

const STARTED: ReadonlySet<Stage> = new Set(["in-progress", "in-review", "done"]);

/** How much of the work an item is, or holds, has been started: a leaf's own, or a parent's leaves under way or done. */
function started(item: Item | undefined): number {
  if (item === undefined) return 0;
  if (item.progress !== undefined) {
    const { counts } = item.progress;
    return counts.in_progress + counts.in_review + counts.done;
  }
  return item.stage !== undefined && STARTED.has(item.stage) ? 1 : 0;
}

/**
 * An offered Undo as it is kept over a reload, which would otherwise drop it before the next write or Dismiss: the
 * write it takes back, the plan that write was sent over, and what its notice said.
 */
export interface Kept {
  command: Command;
  items: Item[];
  text: string;
  undone: string;
}

export function keep(command: Command, before: Plan, text: string, undone: string): Kept {
  return { command, items: neededBy(command, before), text, undone };
}

/**
 * Of `before`, the items the undo of `command` reads, so a large plan isn't written whole after every write: what the
 * write names, and the boxes above an edited item and what waits on them.
 */
function neededBy(command: Command, before: Plan): Item[] {
  const body = record(command);
  const ids = new Set<string>();
  const edit = record(record(body?.item)?.edit);
  if (typeof edit?.item === "string") {
    const id = edit.item;
    if (typeof edit.parent === "string") ids.add(edit.parent);
    for (const key of ["waits-for", "no-waits-for"]) {
      const end = waitEnd(edit, key);
      if (end !== null) ids.add(end);
    }
    for (const one of [...above(before, id), ...waitersOf(before, id)]) ids.add(one);
  }
  return [...ids].flatMap((id) => before.items.get(id) ?? []);
}

/** The Undo a kept one offers again, saying what it said, or null when it no longer has one. */
export function unkeep(kept: Kept): Undo | null {
  const undo = undoOf(kept.command, { items: new Map(kept.items.map((item) => [item.id, item])) });
  return undo === null ? null : { ...undo, text: kept.text, undone: kept.undone };
}
