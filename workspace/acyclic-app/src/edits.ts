// Edits made on the Map: the commands they send, which targets a move or a link may take,
// and what a tap does in each mode. Pure, so every rule is tested without a browser.

import type { Command, Item } from "./api";
import { changes, draftOf, type ItemDraft } from "./forms";
import type { Model } from "./map/model";
import { uuidv7 } from "./uuid";

/**
 * A new item, under the id the app made for it. A null parent sends none, which the server fills in when the tracker has one initiative; a null thread asks the tracker to inherit.
 */
export interface NewItem {
  id: string;
  title: string;
  parent: string | null;
  thread: string | null;
}

export function addCommand({ id, title, parent, thread }: NewItem): Command {
  return {
    item: { add: { id, title, ...(parent === null ? {} : { parent }), ...(thread === null ? {} : { thread }) } },
  };
}

/**
 * The draft a failed add reopens with. It keeps its id, since an add that went unanswered may have been saved and
 * sending it again is then the same add; but an id refused as `deleted` is retired for good, so it gets a new one.
 */
export function redraft(item: NewItem, code: string): NewItem {
  return code === "deleted" ? { ...item, id: uuidv7() } : item;
}

/** Moves an item under a new parent: every item has one, so there is no moving it out. */
export function moveCommand(item: string, parent: string): Command {
  return { item: { edit: { item, parent } } };
}

/** `from` waits for `to`: an edit of `from`. */
export function linkCommand(from: string, to: string): Command {
  return { item: { edit: { item: from, "waits-for": [to] } } };
}

/** `from` no longer waits for `to`. */
export function unlinkCommand(from: string, to: string): Command {
  return { item: { edit: { item: from, "no-waits-for": [to] } } };
}

/** The item and its ancestors, nearest first, stopping where parents form a cycle. */
export function chain(model: Model, id: string): string[] {
  const out: string[] = [];
  let at: string | null = id;
  while (at !== null && !out.includes(at)) {
    out.push(at);
    at = model.items.get(at)?.parent ?? null;
  }
  return out;
}

/** Why `item` can't go under `parent`, or null if it can. */
export function moveRefusal(model: Model, item: string, parent: string): string | null {
  if (parent === item) return `${item} can't go under itself`;
  if (chain(model, parent).includes(item)) return `${item} can't go under its own descendant ${parent}`;
  if (model.items.get(item)?.parent === parent) return `${item} is already under ${parent}`;
  return null;
}

/**
 * Why `from` can't link to `to`, or null if it can. The Map draws every item inside its ancestors, so a link between
 * an item and what holds it would never show; a cycle of links is the server's to refuse.
 */
export function linkRefusal(model: Model, from: string, to: string): string | null {
  if (from === to) return `${from} can't link to itself`;
  if (chain(model, from).includes(to)) return `${to} holds ${from}, so the Map couldn't draw the link`;
  if (chain(model, to).includes(from)) return `${from} holds ${to}, so the Map couldn't draw the link`;
  return null;
}

/** The toggles that show `id`: every box above it expanded, and everything else as it was. */
export function expandTo(
  model: Model,
  id: string,
  toggles: ReadonlyMap<string, boolean>,
): ReadonlyMap<string, boolean> {
  const above = chain(model, id).slice(1);
  if (above.every((box) => toggles.get(box) === false)) return toggles;
  const next = new Map(toggles);
  for (const box of above) next.set(box, false);
  return next;
}

/** What a tap on the Map picks while a mode waits for a target. */
export type Mode = { kind: "idle" } | { kind: "move"; item: string } | { kind: "link"; item: string };

/** A target: an item on the Map, or the Map's background. */
export type Pick = { kind: "item"; id: string } | { kind: "background" } | { kind: "cancel" };

export interface Step {
  mode: Mode;
  command: Command | null;
  /** Why the pick was refused; the mode stays, so the user can pick again. */
  refusal: string | null;
}

export function pick(model: Model, mode: Mode, target: Pick): Step {
  const idle: Mode = { kind: "idle" };
  if (target.kind === "cancel" || mode.kind === "idle") return { mode: idle, command: null, refusal: null };
  const refuse = (refusal: string): Step => ({ mode, command: null, refusal });
  if (mode.kind === "move") {
    if (target.kind === "background") return refuse("Choose the box it goes in");
    if (target.kind !== "item") return refuse("Choose the box it goes in");
    const refusal = moveRefusal(model, mode.item, target.id);
    if (refusal !== null) return refuse(refusal);
    return { mode: idle, command: moveCommand(mode.item, target.id), refusal: null };
  }
  if (target.kind !== "item") return refuse("Choose the item to link to");
  const refusal = linkRefusal(model, mode.item, target.id);
  if (refusal !== null) return refuse(refusal);
  return { mode: idle, command: linkCommand(mode.item, target.id), refusal: null };
}

/** How an edit went: the write, then, if it was saved, reading the plan again. */
export type Sent =
  | { write: "failed"; code: string; message: string }
  | { write: "saved"; reload: "done" | { failed: string } };

/** What the app tells the user after an edit, and what it offers: the same write again, or a refresh. */
export interface Told {
  text: string | null;
  offer: "retry" | "refresh" | null;
}

export function told(sent: Sent): Told {
  if (sent.write === "failed") {
    // A busy server may simply be sent the same command again; gone elsewhere, what the app shows is out of date.
    const stale = sent.code === "not-found";
    return { text: sent.message, offer: sent.code === "busy" ? "retry" : stale ? "refresh" : null };
  }
  // Saved is saved: a failed refresh is offered again, never the write.
  if (sent.reload !== "done") return { text: `Saved, but the refresh failed: ${sent.reload.failed}`, offer: "refresh" };
  return { text: null, offer: null };
}

/**
 * A write drawn before the server has it, so the Map answers a tap at once: an item added, or an item's fields
 * edited. The app holds at most one, since it sends one write at a time.
 */
export type Pending = { kind: "add"; item: NewItem } | { kind: "edit"; id: string; edited: Edited };

/** An item's fields as its editor opened on them, `base`, and as the user left them, `draft`. */
export interface Edited {
  base: ItemDraft;
  draft: ItemDraft;
}

/** The items with `pending` drawn over them. */
export function withPending(items: Item[], pending: Pending | null): Item[] {
  if (pending === null) return items;
  if (pending.kind === "add") {
    const { id, title, parent, thread } = pending.item;
    if (items.some((one) => one.id === id)) return items;
    const links = { waits_for: [], waited_on_by: [] };
    const added: Item = {
      id,
      title,
      parent,
      thread,
      state: "open",
      notes: null,
      // Under an initiative it is in that one; under an item, in that item's; with no parent, the server picks.
      initiative: parent === null ? null : (items.find((one) => one.id === parent)?.initiative ?? parent),
      links,
      conflicts: {},
    };
    return [...items, added];
  }
  const { id, edited } = pending;
  const changed = changes(edited.base, edited.draft);
  const or = (value: string | undefined, was: string | null) =>
    value === undefined ? was : value === "" ? null : value;
  return items.map((one) =>
    one.id !== id
      ? one
      : {
          ...one,
          title: changed.title ?? one.title,
          // A done item's state isn't the editor's to change.
          state: one.state === "done" ? one.state : (changed.state ?? one.state),
          thread: or(changed.thread, one.thread),
          notes: or(changed.notes, one.notes),
        },
  );
}

/** Whether `items`, read from the server, hold `pending`: the item added, or every field the user changed. */
export function landed(pending: Pending, items: Item[]): boolean {
  if (pending.kind === "add") return items.some((one) => one.id === pending.item.id);
  const item = items.find((one) => one.id === pending.id);
  if (item === undefined) return false;
  const now = draftOf(item);
  return Object.entries(changes(pending.edited.base, pending.edited.draft)).every(
    ([field, value]) => now[field as keyof ItemDraft] === value,
  );
}

/** A failure that leaves unknown whether the server applied the write: no answer, or one that wasn't the server's. */
export function unanswered(code: string): boolean {
  return code === "network" || code === "http";
}
