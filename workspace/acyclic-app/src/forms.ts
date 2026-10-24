// The commands the app's forms send: an item's fields, a conflict settled, a reopen, a thread approved, its level and
// project-management flag, the global level, and a proposal kept or dropped. Pure, so each shape is tested against
// the commands the server's tests run.

import type { Command, Item } from "./api";

/** The states a command may set; `done` is set by `acyclic done`, with evidence. */
export const STATES = ["backlog", "parked", "in-progress"] as const;
export const SIZES = ["xs", "s", "m", "l", "xl"] as const;
export const LEVELS = ["everything", "scope-and-irreversible", "blockers"] as const;
export type Level = (typeof LEVELS)[number];

/** A level as the app says it: "scope-and-irreversible" reads "Scope and irreversible". */
export function levelLabel(level: string): string {
  const words = level.replaceAll("-", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The item's fields the editor changes; an empty string is a clear. */
export interface ItemDraft {
  title: string;
  state: string;
  thread: string;
  notes: string;
  gate: string;
  size: string;
}

export function draftOf(item: Item): ItemDraft {
  return {
    title: item.title,
    state: item.state,
    thread: item.thread ?? "",
    notes: item.notes ?? "",
    gate: item.gate ?? "",
    size: item.size ?? "",
  };
}

/**
 * The edit that takes `item` to `draft`, naming only what changed, or null when nothing did. A conflicted field left
 * as shown isn't written, so saving never settles a conflict the user didn't touch.
 */
export function itemEdit(item: Item, draft: ItemDraft): Command | null {
  const was = draftOf(item);
  const edit: Record<string, unknown> = {};
  const changed = (field: keyof ItemDraft) => draft[field] !== was[field];
  if (draft.title.trim() !== "" && changed("title")) edit.title = draft.title.trim();
  if (changed("state") && (STATES as readonly string[]).includes(draft.state)) edit.state = draft.state;
  for (const field of ["thread", "notes", "gate", "size"] as const) {
    if (!changed(field)) continue;
    if (draft[field] === "") edit[`no-${field}`] = true;
    else edit[field] = draft[field];
  }
  return Object.keys(edit).length === 0 ? null : { item: { edit: { item: item.id, ...edit } } };
}

/** An entity whose field can be in conflict, as `needs` reports it. */
export type Entity = "item" | "thread" | "settings";

/**
 * The command that settles a conflict by keeping one of its values (`null` for a clear), as the CLI's own hint for it
 * reads; none when the app can't write that field or value.
 */
export function resolveCommand(entity: Entity, id: string, field: string, value: unknown): Command | null {
  const text = typeof value === "string" ? value : null;
  if (entity === "settings") return field === "autonomy" && isLevel(text) ? globalLevelCommand(text) : null;
  if (entity === "item") {
    const edit = (fields: Record<string, unknown>) => ({ item: { edit: { item: id, ...fields } } });
    if (field === "title") return text === null ? null : edit({ title: text });
    if (field === "state")
      return text !== null && (STATES as readonly string[]).includes(text) ? edit({ state: text }) : null;
    if (["notes", "gate", "size", "parent", "thread"].includes(field)) {
      if (value === null) return edit({ [`no-${field}`]: true });
      return text === null ? null : edit({ [field]: text });
    }
    if (field === "reopened" && value === true) return reopenCommand(id);
    return null;
  }
  const edit = (fields: Record<string, unknown>) => ({ thread: { edit: { thread: id, ...fields } } });
  if (field === "title") return text === null ? null : edit({ title: text });
  if (field === "brief")
    return value === null ? edit({ "no-brief": true }) : text === null ? null : edit({ brief: text });
  if (field === "autonomy")
    return value === null ? edit({ "no-level": true }) : isLevel(text) ? edit({ level: text }) : null;
  if (field === "pm-flag") return edit(value === true ? { pm: true } : { "no-pm": true });
  if (field === "approved") return value === true ? approveCommand(id) : revokeCommand(id);
  // Approving again records the members as they are now.
  if (field === "approved-members") return approveCommand(id);
  return null;
}

/** Why a value of a conflict can't be kept from the app, for a value `resolveCommand` gives no command for. */
export function unsettledWhy(entity: Entity, field: string, value: unknown): string {
  if (entity === "item" && field === "state" && value === "done") return "Keep done with `acyclic done`, with evidence";
  if (value === null) return "This field can't be cleared";
  return "Settle this one with the CLI: `acyclic needs` says how";
}

function isLevel(value: string | null): value is Level {
  return value !== null && (LEVELS as readonly string[]).includes(value);
}

export function reopenCommand(item: string): Command {
  return { item: { reopen: { item } } };
}

export function approveCommand(thread: string): Command {
  return { thread: { approve: { thread } } };
}

export function revokeCommand(thread: string): Command {
  return { thread: { revoke: { thread } } };
}

/** A thread's own level, or none, so the global one applies. */
export function threadLevelCommand(thread: string, level: Level | null): Command {
  return { thread: { edit: level === null ? { thread, "no-level": true } : { thread, level } } };
}

export function pmCommand(thread: string, pm: boolean): Command {
  return { thread: { edit: pm ? { thread, pm: true } : { thread, "no-pm": true } } };
}

export function globalLevelCommand(level: Level): Command {
  return { level: { level } };
}

/** Keeps an item in a thread it joined after the thread's approval, as the user's own edit. */
export function keepMemberCommand(item: string, thread: string): Command {
  return { item: { edit: { item, thread } } };
}

/** Takes an item back out of a thread it joined after the thread's approval. */
export function dropMemberCommand(item: string): Command {
  return { item: { edit: { item, "no-thread": true } } };
}

export function keepCommand(proposal: string): Command {
  return { proposal: { keep: { proposal } } };
}

export function dropCommand(proposal: string): Command {
  return { proposal: { drop: { proposal } } };
}
