// Small plans for the Map's unit tests, written as an outline.

import type { Item } from "../api";

/** The ids `commands.examples.json` uses, which the server's tests give to the tracker they run it on. */
export const ID = {
  a: "01900000-0000-7000-8000-0000000000a1",
  b: "01900000-0000-7000-8000-0000000000b1",
  c: "01900000-0000-7000-8000-0000000000c1",
  d: "01900000-0000-7000-8000-0000000000d1",
  e: "01900000-0000-7000-8000-0000000000e1",
  /** The thread. */
  t: "01900000-0000-7000-8000-0000000000f1",
};

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
    links: { waits_for: waitsFor, waited_on_by: [] },
    conflicts: {},
    ...extra,
  } satisfies Item;
}
