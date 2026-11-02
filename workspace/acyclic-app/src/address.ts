// The app's address: what is open, in the query, so the address bar always says it and a link opens it again. Only
// what a link should bring back is in it: the view (Needs you at an entry), an item and a thread. The add form, the
// outline's sheet and a move waiting for its target are passing states, left out. Pure, so tested without a browser.

import { HOME, type View } from "./Shell";

export interface Wanted {
  view: View;
  item: string | null;
  thread: string | null;
  /** The entry Needs you's view opens at, if any. */
  needsAt: string | null;
}

export const NOTHING: Wanted = { view: HOME, item: null, thread: null, needsAt: null };

const VIEWS: readonly View[] = ["overview", "map", "threads", "needs"];
const OWNED = ["view", "item", "thread", "needs"];

/**
 * The address for what is open, from `/` on: bare at home with nothing open. Only the parameters the app's state owns
 * change; others in `search`, such as a switch a test run set, stay. An item is on the Map, so says so, unless the
 * user went to another view with it open.
 */
export function addressOf(wanted: Wanted, search = ""): string {
  const query = new URLSearchParams(search);
  for (const key of OWNED) query.delete(key);
  // An item implies the Map; one left open over another view, which the user went to with it open, names that view.
  if (wanted.item === null ? wanted.view !== HOME : wanted.view !== "map") query.set("view", wanted.view);
  if (wanted.item !== null) query.set("item", wanted.item);
  if (wanted.thread !== null) query.set("thread", wanted.thread);
  if (wanted.view === "needs" && wanted.needsAt !== null) query.set("needs", wanted.needsAt);
  const text = query.toString();
  return text === "" ? "/" : `/?${text}`;
}

/**
 * What a query asks to open: what it doesn't name, or names wrongly, stays as at home. `needs`, bare or naming an
 * entry, is Needs you's view, as notifications and addresses from when it opened over the others still say.
 */
export function parse(search: string): Wanted {
  const query = new URLSearchParams(search);
  const named = query.get("view");
  const item = query.get("item") || null;
  const view = VIEWS.find((one) => one === named) ?? (query.has("needs") ? "needs" : item !== null ? "map" : HOME);
  return {
    view,
    item,
    thread: query.get("thread") || null,
    needsAt: view === "needs" ? query.get("needs") || null : null,
  };
}

/** What the plan holds of something an address names: the item drawn, hidden in an archived subtree, or gone. */
export type Found = "shown" | "archived" | null;

/**
 * What of `wanted` opens in the plan as it is: an item or a thread it no longer holds is dropped, and named, so a link
 * gone stale says so rather than opening an empty Map; an archived item opens with archived items shown.
 */
export function landing(
  wanted: Wanted,
  has: { item: (id: string) => Found; thread: (id: string) => boolean },
): { wanted: Wanted; archived: boolean; missing: ("item" | "thread")[] } {
  const item = wanted.item === null ? null : has.item(wanted.item);
  const threadGone = wanted.thread !== null && !has.thread(wanted.thread);
  return {
    wanted: {
      ...wanted,
      item: item === null ? null : wanted.item,
      thread: threadGone ? null : wanted.thread,
    },
    archived: item === "archived",
    missing: [
      ...(wanted.item !== null && item === null ? ["item" as const] : []),
      ...(threadGone ? ["thread" as const] : []),
    ],
  };
}
