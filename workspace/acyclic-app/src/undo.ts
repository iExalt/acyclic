// Undo: for a write that has an exact inverse, the write that takes it back, with what must still hold for that to be
// safe. A move, an item's fields but its thread, a link added or removed, the global autonomy level, and a thread's
// first approval while no agent has started its work undo; nothing else does: an add (the app has no delete), a
// reopen, an approval again or a revoking (agents act on them), a proposal kept or dropped, a conflict settled, a
// thread changed (back in an approved thread, an item is a late member waiting for approval again). The guard is
// checked against a plan read just before the undo is sent; the server has no compare-and-set, so a write landing
// between that read and the undo isn't caught. Pure, so tested without a browser.
import type { Command, Item, Stage } from "./api";
import type { LinkKind } from "./edits";

/** The plan a write was sent over, or the plan as it is when Undo is pressed. */
export interface Plan {
  items: ReadonlyMap<string, Item>;
  level: string;
}

export interface Undo {
  /** What was done, said over the Undo it offers. */
  text: string;
  /** What the undo did, said once it has. */
  undone: string;
  command: Command;
  /** Whether the plan still holds what the write left, so the undo takes back that write and nothing done since. */
  holds: (now: Plan) => boolean;
  /** An item the undo puts somewhere else, to select and bring into view where it lands, as the write did. */
  shows?: string;
}

/** An item's fields an edit can set and an undo set back, with the value each holds; an empty one is null. */
const FIELDS = ["title", "state", "notes", "gate", "size"] as const;
type Field = (typeof FIELDS)[number];
const held = (item: Item | undefined, field: Field): string | null => (item === undefined ? null : item[field]);

const LINKS: Record<LinkKind, "waits_for" | "relates_to"> = { "waits-for": "waits_for", "relates-to": "relates_to" };

const record = (value: unknown): Record<string, unknown> | null =>
  typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null;

/** The undo of `command`, sent over `before`, or null when it has no exact inverse. */
export function undoOf(
  command: Command,
  before: Plan,
  levelLabel = (level: string) => level,
  threadName = (thread: string) => thread,
): Undo | null {
  const body = record(command);
  if (body === null) return null;
  const titleOf = (id: string) => before.items.get(id)?.title ?? id;

  const level = record(body.level);
  if (level !== null && typeof level.level === "string") {
    const set = level.level;
    return {
      text: "Changed the autonomy",
      // Not "undone": what agents did meanwhile stands.
      undone: `Autonomy set back to ${levelLabel(before.level)}`,
      command: { level: { level: before.level } },
      holds: (now) => now.level === set,
    };
  }

  // Sent only for a first approval, which a revoke takes back exactly while no agent has started on what it approved.
  const approve = record(record(body.thread)?.approve);
  if (approve !== null && typeof approve.thread === "string") {
    const thread = approve.thread;
    const members = [...before.items.values()].filter((one) => one.thread === thread);
    return {
      text: `Approved ${threadName(thread)}`,
      undone: `Took back the approval of ${threadName(thread)}`,
      command: { thread: { revoke: { thread } } },
      holds: (now) => members.every((one) => started(now.items.get(one.id)) <= started(one)),
    };
  }

  const link = record(body.link);
  const linking = record(link?.add) ?? record(link?.remove);
  if (link !== null && linking !== null) {
    const { from, kind, to } = linking as { from: string; kind: LinkKind; to: string };
    const added = record(link.add) !== null;
    const linked = (now: Plan) => now.items.get(from)?.links[LINKS[kind]].includes(to) === true;
    // Unwaited, an item may become ready and be started; waiting again wouldn't stop that, so only undo while the
    // work under it is no further along than it was.
    const unstarted = (now: Plan) =>
      added || kind !== "waits-for" || started(now.items.get(from)) <= started(before.items.get(from));
    // Said as the link reads, so which waits for which is plain.
    const [a, b, verb] = [titleOf(from), titleOf(to), kind === "waits-for" ? "waits for" : "relates to"];
    const [now, longer, again] = [`${a} now ${verb} ${b}`, `${a} no longer ${verb} ${b}`, `${a} ${verb} ${b} again`];
    return {
      text: added ? now : longer,
      undone: added ? longer : again,
      command: { link: { [added ? "remove" : "add"]: { from, kind, to } } },
      holds: (now) => linked(now) === added && unstarted(now),
    };
  }

  const edit = record(record(body.item)?.edit);
  if (edit === null || typeof edit.item !== "string") return null;
  const id = edit.item;
  const was = before.items.get(id);
  if (was === undefined) return null;
  const keys = Object.keys(edit).filter((key) => key !== "item");

  if (keys.length === 1 && (keys[0] === "parent" || keys[0] === "no-parent")) {
    const to = keys[0] === "parent" ? String(edit.parent) : null;
    const back = was.parent;
    return {
      text: to === null ? `Moved ${titleOf(id)} to the top level` : `Moved ${titleOf(id)} into ${titleOf(to)}`,
      undone: `Moved ${titleOf(id)} back`,
      command: { item: { edit: back === null ? { item: id, "no-parent": true } : { item: id, parent: back } } },
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
  return {
    text: `Edited ${named}${thread ? ". Its thread can't be changed back from here" : ""}`,
    undone: `Set ${titleOf(id)} back${thread ? ", except its thread" : ""}`,
    command: { item: { edit: back } },
    holds: (now) =>
      [...set].every(([field, value]) => held(now.items.get(id), field) === value) &&
      (!set.has("state") ||
        waitersOf(before, id).every((one) => started(now.items.get(one)) <= started(before.items.get(one)))),
  };
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
  level: string;
  text: string;
  undone: string;
}

export function keep(command: Command, before: Plan, text: string, undone: string): Kept {
  return { command, items: neededBy(command, before), level: before.level, text, undone };
}

/**
 * Of `before`, the items the undo of `command` reads, so a large plan isn't written whole after every write: what the
 * write names, the boxes above an edited item and what waits on them, or an approved thread's members.
 */
function neededBy(command: Command, before: Plan): Item[] {
  const body = record(command);
  const ids = new Set<string>();
  const thread = record(record(body?.thread)?.approve)?.thread;
  if (typeof thread === "string") for (const one of before.items.values()) if (one.thread === thread) ids.add(one.id);
  const link = record(body?.link);
  const linking = record(link?.add) ?? record(link?.remove);
  for (const end of [linking?.from, linking?.to]) if (typeof end === "string") ids.add(end);
  const edit = record(record(body?.item)?.edit);
  if (typeof edit?.item === "string") {
    const id = edit.item;
    if (typeof edit.parent === "string") ids.add(edit.parent);
    for (const one of [...above(before, id), ...waitersOf(before, id)]) ids.add(one);
  }
  return [...ids].flatMap((id) => before.items.get(id) ?? []);
}

/** The Undo a kept one offers again, saying what it said, or null when it no longer has one. */
export function unkeep(kept: Kept): Undo | null {
  const undo = undoOf(kept.command, { items: new Map(kept.items.map((item) => [item.id, item])), level: kept.level });
  return undo === null ? null : { ...undo, text: kept.text, undone: kept.undone };
}
