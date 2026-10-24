// Small plans for the Map's unit tests, written as an outline.

import type { Item } from "../api";

/**
 * An item for tests: `id`, its parent, and what it waits for. Every other field takes a plain default, so a test
 * names only what it is about.
 */
export function item(id: string, parent: string | null = null, waitsFor: string[] = [], extra: Partial<Item> = {}) {
  return {
    id,
    title: id.toUpperCase(),
    parent,
    thread: null,
    state: "backlog",
    notes: null,
    archived: false,
    links: { waits_for: waitsFor, waited_on_by: [], relates_to: [], related_from: [] },
    conflicts: {},
    ...extra,
  } satisfies Item;
}
