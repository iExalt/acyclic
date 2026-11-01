// Edits made on the Map: the commands they send, the ids new items get, which targets a move or a link may take,
// and what a tap does in each mode. Pure, so every rule is tested without a browser.

import type { Command, Item } from "./api";
import { changes, draftOf, type ItemDraft } from "./forms";
import type { Model } from "./map/model";

export type LinkKind = "waits-for" | "relates-to";

/** The longest id the tracker takes. */
const MAX_ID = 64;

/** A new item. A thread or parent left out is none. */
export interface NewItem {
  item: string;
  title: string;
  parent: string | null;
  thread: string | null;
}

export function addCommand({ item, title, parent, thread }: NewItem): Command {
  return {
    item: { add: { item, title, ...(parent === null ? {} : { parent }), ...(thread === null ? {} : { thread }) } },
  };
}

/** Moves an item under a new parent, or to the top level. */
export function moveCommand(item: string, parent: string | null): Command {
  return { item: { edit: parent === null ? { item, "no-parent": true } : { item, parent } } };
}

/** `from` waits for or relates to `to`: with "waits-for", `to` comes first. */
export function linkCommand(from: string, kind: LinkKind, to: string): Command {
  return { link: { add: { from, kind, to } } };
}

export function unlinkCommand(from: string, kind: LinkKind, to: string): Command {
  return { link: { remove: { from, kind, to } } };
}

/**
 * An id for a new item, from its title: lowercase ASCII letters and digits joined by '-', or `untitled` for a title
 * with none, with -2, -3… when the id is taken. The client can't see every id taken, such as a deleted item's or one in a
 * replica not yet synced, so the user can still change it.
 */
export function newId(title: string, taken: ReadonlySet<string>): string {
  const words = title
    .normalize("NFKD")
    // Accents come apart from their letters, and go.
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .split(" ")
    .filter((word) => word !== "");
  // "item-" starts the ids the tracker assigns, which it refuses from anyone else.
  const base = words.length === 0 ? "untitled" : words.join("-").replace(/^item(-|$)/, "items$1");
  const trimmed = (stem: string, suffix: string) =>
    `${stem.slice(0, MAX_ID - suffix.length).replace(/-+$/, "")}${suffix}`;
  let id = trimmed(base, "");
  for (let n = 2; taken.has(id); n++) id = trimmed(base, `-${n}`);
  return id;
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

/** Why `item` can't go under `parent` (null for the top level), or null if it can. */
export function moveRefusal(model: Model, item: string, parent: string | null): string | null {
  if (parent === null) return model.items.get(item)?.parent === null ? `${item} is already at the top level` : null;
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
export type Mode = { kind: "idle" } | { kind: "move"; item: string } | { kind: "link"; item: string; link: LinkKind };

/** A target: an item on the Map, the "Top level" button (null), or the Map's background. */
export type Pick = { kind: "item"; id: string } | { kind: "top" } | { kind: "background" } | { kind: "cancel" };

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
    if (target.kind === "background") return refuse("Choose the box it goes in, or Top level");
    const parent = target.kind === "top" ? null : target.id;
    const refusal = moveRefusal(model, mode.item, parent);
    if (refusal !== null) return refuse(refusal);
    return { mode: idle, command: moveCommand(mode.item, parent), refusal: null };
  }
  if (target.kind !== "item") return refuse("Choose the item to link to");
  const refusal = linkRefusal(model, mode.item, target.id);
  if (refusal !== null) return refuse(refusal);
  return { mode: idle, command: linkCommand(mode.item, mode.link, target.id), refusal: null };
}

/** How an edit went: the write, then, if it was saved, reading the plan again. */
export type Sent =
  | { write: "failed"; code: string; message: string }
  | { write: "saved"; proposals: number; reload: "done" | { failed: string } };

/** What the app tells the user after an edit, and what it offers: the same write again, or a refresh. */
export interface Told {
  text: string | null;
  offer: "retry" | "refresh" | null;
  /** The new item's id was refused: the user should change it. */
  fixId: boolean;
}

export function told(sent: Sent): Told {
  if (sent.write === "failed") {
    const { retry, fixId } = recovery(sent.code);
    // Decided or gone elsewhere: what the app shows is out of date.
    const stale = sent.code === "decided" || sent.code === "not-found";
    return { text: sent.message, offer: retry ? "retry" : stale ? "refresh" : null, fixId };
  }
  // Saved is saved: a failed refresh is offered again, never the write.
  if (sent.reload !== "done")
    return { text: `Saved, but the refresh failed: ${sent.reload.failed}`, offer: "refresh", fixId: false };
  if (sent.proposals > 0) return { text: "Sent as a proposal for you to keep", offer: null, fixId: false };
  return { text: null, offer: null, fixId: false };
}

/** What the app offers after a refusal or failure, by the envelope's code. */
export interface Recovery {
  /** The server was busy: the same command may simply be sent again. */
  retry: boolean;
  /** The new item's id was refused: the user should change it. */
  fixId: boolean;
}

export function recovery(code: string): Recovery {
  return { retry: code === "busy", fixId: ["already-exists", "deleted", "invalid-name"].includes(code) };
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
    const { item: id, title, parent, thread } = pending.item;
    if (items.some((one) => one.id === id)) return items;
    const links = { waits_for: [], waited_on_by: [], relates_to: [], related_from: [] };
    const added: Item = {
      id,
      title,
      parent,
      thread,
      state: "backlog",
      notes: null,
      gate: null,
      size: null,
      archived: false,
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
          gate: or(changed.gate, one.gate),
          size: or(changed.size, one.size),
        },
  );
}

/** Whether `items`, read from the server, hold `pending`: the item added, or every field the user changed. */
export function landed(pending: Pending, items: Item[]): boolean {
  if (pending.kind === "add") return items.some((one) => one.id === pending.item.item);
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
