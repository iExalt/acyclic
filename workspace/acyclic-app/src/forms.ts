// The commands the app's forms send: an item's fields, a conflict settled and a reopen. Pure, so each shape is tested
// against the commands the server's tests run.

import type { Command, Item } from "./api";

/** The states a command may set; `done` is set by `acyclic item done`, with evidence. */
export const STATES = ["open", "parked", "canceled"] as const;
/** Explain an open state separately from the status computed for a leaf. */
export function stateHint(draft: Pick<ItemDraft, "state" | "thread">, isLeaf: boolean): string | null {
  if (draft.state === "done") return "Done work lets work waiting on it start. Describe what was completed";
  if (draft.state === "canceled")
    return "Canceled work is dropped: it counts as neither done nor open, and work waiting on it needs you. Open undoes it";
  if (!isLeaf || draft.state !== "open") return null;
  return "Open is work still to do. Status also reflects its thread, waits and active claims";
}

/** The item's fields the editor changes; an empty string is a clear. */
export interface ItemDraft {
  title: string;
  state: string;
  thread: string;
  notes: string;
  /** Unsaved completion evidence, kept when a failed save restores the editor. */
  evidence?: string;
}

export function draftOf(item: Item): ItemDraft {
  return {
    title: item.title,
    state: item.state,
    thread: item.thread ?? "",
    notes: item.notes ?? "",
  };
}

/**
 * What the user changed from `base`, the fields as the editor opened on them, to `draft`: a title only when it isn't
 * blank, trimmed, and a state only one a command may set. Fields left alone aren't here, so a save never writes back
 * what changed elsewhere while the editor was open, and never settles a conflict the user didn't touch.
 */
export function changes(base: ItemDraft, draft: ItemDraft): Partial<ItemDraft> {
  const out: Partial<ItemDraft> = {};
  const title = draft.title.trim();
  if (title !== "" && title !== base.title) out.title = title;
  if (draft.state !== base.state && (STATES as readonly string[]).includes(draft.state)) out.state = draft.state;
  for (const field of ["thread", "notes"] as const) if (draft[field] !== base[field]) out[field] = draft[field];
  return out;
}

/** The edit of item `id` that makes the user's changes from `base` to `draft`, or null when they made none. */
export function itemEdit(id: string, base: ItemDraft, draft: ItemDraft): Command | null {
  const changed = changes(base, draft);
  if (base.state !== "done" && draft.state === "done") {
    const evidence = draft.evidence?.trim() ?? "";
    if (Object.keys(changed).length !== 0 || evidence === "") return null;
    return { item: { done: { item: id, evidence } } };
  }
  const edit: Record<string, unknown> = {};
  for (const [field, value] of Object.entries(changed)) {
    if (value === "") edit[`no-${field}`] = true;
    else edit[field] = value;
  }
  return Object.keys(edit).length === 0 ? null : { item: { edit: { item: id, ...edit } } };
}

export function reopenCommand(item: string): Command {
  return { item: { reopen: { item } } };
}
