// Where the user is in the app, and how each thing they do moves them: the view, the item and the thread open, what
// those were opened from, and what is open over them. Each way of moving is one intent, which `navigate` turns into
// the next place and the requests it makes of what is drawn (an item to bring into view, a thread to frame, an editor
// to open). Then the layers the place stacks, which Escape and Back close one by one, the history entries that follow
// them, and the address that names what is open. Pure, so the rules are tested without a browser.

import type { Edited, Mode, NewItem } from "./edits";
import { HOME, type View } from "./Shell";

/** The add form, open under a parent or at the top level, with what the user wrote if a failed add reopened it. */
export interface Adding {
  parent: string | null;
  draft: NewItem | null;
}

/**
 * Where Needs you's view was left for an item or a thread an entry named, which shows on the Map: how far it was
 * scrolled, and the entry and control picked, so closing what it opened comes back to the list as it was, the focus on
 * that control.
 */
export interface NeedsLeft {
  top: number;
  entry: string;
  /** A selector for the control within the entry. */
  control: string;
}

/** What is open over the view and its panels. */
export interface Overlays {
  /** The command menu, over everything. */
  command: boolean;
  /** A move or a link waiting for its target. */
  mode: Mode;
  add: Adding | null;
  /** The projects' sheet, open over the Map on a phone; side by side, the outline is always in view. */
  projects: boolean;
}

export interface CameFrom {
  view: View;
  by: "item" | "thread";
  /** The item a thread was opened from, under the thread: closing the thread opens it again. */
  item?: string;
}

export interface Place {
  /** The Map, or a view over it: the Overview, the threads in waves, or Needs you. */
  view: View;
  /** The item selected. */
  item: string | null;
  /** The thread picked. */
  thread: string | null;
  /**
   * The view an item or a thread was opened from, which closing the one opened from it returns to: both are shown on
   * the Map, but the user came from there. An item chosen from a thread's panel leaves the thread's, for the thread's
   * close. Dropped once the user selects on the Map or goes to another view, which makes that where they are. A
   * thread opened from an item's panel keeps the item, which closing the thread opens again.
   */
  cameFrom: CameFrom | null;
  /** The entry Needs you's view opens at, when the Overview went there from one. */
  needsAt: string | null;
  /** Where Needs you's view was left for what an entry opened, until the user comes back to it. */
  needsLeft: NeedsLeft | null;
  overlays: Overlays;
}

const CLOSED: Overlays = { command: false, mode: { kind: "idle" }, add: null, projects: false };

export const START: Place = {
  view: HOME,
  item: null,
  thread: null,
  cameFrom: null,
  needsAt: null,
  needsLeft: null,
  overlays: CLOSED,
};

/** What the transitions read of the plan, the screen and the page as they are. */
export interface World {
  /** Whether the item is drawn. */
  has: (id: string) => boolean;
  /** Whether the item is hidden, in an archived subtree while archived items aren't shown. */
  archived: (id: string) => boolean;
  /** The thread an item is in, if any. */
  threadOf: (id: string) => string | null;
  /** Side by side rather than a phone's one column. */
  wide: boolean;
  /** A form holds what the user typed. */
  dirty: boolean;
}

/**
 * What a step asks of what is drawn. Each a fresh object: its consumer acts on one it hasn't seen, so the same ask made
 * twice is two asks. Never kept in the history.
 */
export interface Requests {
  /** An item for the Map to bring into view. A write may preserve its existing root frame and zoom. */
  focus?: { id: string; pin: string | null; preserveFrame?: true };
  /** A thread for the Map to frame, its items fitted in the part of the Map nothing covers. */
  frame?: { thread: string };
  /** For the item's panel to open its editor, as Edit does. */
  edit?: { item: string };
  /** For the item's panel to open its editor again with what the user wrote, after their edit failed. */
  reopenEditor?: { id: string; edited: Edited };
  /** For Needs you's view, shown again, to come back as it was left. */
  restoreNeeds?: NeedsLeft;
  /** Nothing closed: a form holds what the user typed, and says how to leave it instead. Read by the caller at once. */
  unsaved?: true;
}

export interface Step {
  place: Place;
  requests: Requests;
}

export type Intent =
  /** An item chosen anywhere but on the Map, which goes to the Map for it, brought into view. */
  | { kind: "select"; id: string; pin: string | null }
  /** A tap on the Map: an item, or the background (null), which makes the Map where the user is. */
  | { kind: "tap"; id: string | null }
  /** The item selected went away, deleted or hidden with archived items. */
  | { kind: "gone" }
  /** A thread's row or tile anywhere: to the Map, the thread picked with its panel, its items framed. */
  | { kind: "thread"; id: string }
  /** A thread's row in the outline: the thread picked on the Map with nothing over it is unpicked; else as `thread`. */
  | { kind: "outline"; id: string }
  /** The thread an item's panel names: to it as `thread` does, the item kept under it, for closing it to reopen. */
  | { kind: "itemThread"; id: string }
  /** To a view, the tabs, keys, the command menu and the Overview's links all: Needs you at the entry `at`, if any. */
  | { kind: "goTo"; view: View; at: string | null }
  /** Escape and Back: `count` layers from the top, unless a form holding typing would close. */
  | { kind: "close"; count: number }
  /** One layer closed by its own control, as its close button does. */
  | { kind: "shut"; layer: Layer }
  | { kind: "command"; open: boolean }
  | { kind: "projects"; open: boolean }
  | { kind: "mode"; mode: Mode }
  /** The add form, empty, under `parent`. */
  | { kind: "add"; parent: string | null }
  /** The add form again, with what the user wrote, after their add failed. */
  | { kind: "redraft"; adding: Adding }
  /** The editor of the item selected, asked for from the keyboard or the menu. */
  | { kind: "edit" }
  /** The editor again, with what the user wrote, after their edit of `id` failed. */
  | { kind: "reopen"; id: string; edited: Edited }
  /** A write drawn or saved: the add form closes, and `select`, if any, is selected and brought in with `pin` still. */
  | { kind: "wrote"; select: string | null; pin: string | null }
  /** An item added from `from`, other than the Map, while the user is still there: to the Map for it. */
  | { kind: "added"; id: string; from: View }
  /** Needs you's view left for what an entry opens. */
  | { kind: "leaveNeeds"; left: NeedsLeft }
  /** A link, landed on the plan as it is: opened as named, over nothing else. */
  | { kind: "link"; wanted: Wanted }
  /** Forward onto an entry: the place it holds, landed on the plan as it is, opened again over nothing else. */
  | { kind: "restore"; place: Place };

/** The place after `intent`, and what it asks of what is drawn. */
export function navigate(place: Place, intent: Intent, world: World): Step {
  const step = move(place, intent, world);
  return settle(place, step);
}

/**
 * What holds after any move: one opened from another view is shown on the Map, so going to another view, or closing
 * both the item and the thread, leaves the user where they are; and coming back to Needs you finds it as it was left.
 */
function settle(before: Place, { place, requests }: Step): Step {
  let next = place;
  if (next.cameFrom !== null && (next.view !== "map" || (next.item === null && next.thread === null)))
    next = { ...next, cameFrom: null };
  if (before.view !== "needs" && next.view === "needs" && next.needsLeft !== null)
    return { place: { ...next, needsLeft: null }, requests: { ...requests, restoreNeeds: next.needsLeft } };
  return { place: next, requests };
}

/**
 * The thread still picked once `id` is selected: one thing in focus, so selecting an item outside the picked thread
 * unpicks it, and one in it keeps it.
 */
export function pickedAfter(picked: string | null, id: string, world: Pick<World, "threadOf">): string | null {
  return picked !== null && world.threadOf(id) === picked ? picked : null;
}

const overlay = (place: Place, change: Partial<Overlays>): Place => ({
  ...place,
  overlays: { ...place.overlays, ...change },
});

function move(place: Place, intent: Intent, world: World): Step {
  const only = (next: Place): Step => ({ place: next, requests: {} });
  switch (intent.kind) {
    case "select": {
      const { id, pin } = intent;
      // An item chosen from a thread's panel keeps where the thread came from, for the thread's close.
      const cameFrom =
        place.view !== "map"
          ? { view: place.view, by: "item" as const }
          : place.thread !== null && place.cameFrom?.by === "thread"
            ? place.cameFrom
            : null;
      return {
        place: { ...place, item: id, thread: pickedAfter(place.thread, id, world), cameFrom, view: "map" },
        requests: { focus: { id, pin } },
      };
    }
    case "tap":
      return only({
        ...place,
        item: intent.id,
        thread: intent.id === null ? place.thread : pickedAfter(place.thread, intent.id, world),
        cameFrom: null,
      });
    case "gone":
      return only({ ...place, item: null });
    case "thread":
      return {
        place: {
          ...place,
          item: null,
          thread: intent.id,
          cameFrom: place.view === "map" ? null : { view: place.view, by: "thread" },
          view: "map",
        },
        requests: { frame: { thread: intent.id } },
      };
    case "outline":
      // With one of its items open over it, the row goes back to the thread, framed, rather than unpicking it.
      if (place.thread === intent.id && place.view === "map" && place.item === null)
        return only({ ...place, thread: null });
      return move(place, { kind: "thread", id: intent.id }, world);
    case "itemThread": {
      const from: CameFrom = { view: place.cameFrom?.view ?? place.view, by: "thread" };
      return {
        place: {
          ...place,
          item: null,
          thread: intent.id,
          view: "map",
          cameFrom: place.item === null ? from : { ...from, item: place.item },
        },
        requests: { frame: { thread: intent.id } },
      };
    }
    case "goTo": {
      // An item is shown on the Map, so going to another view leaves it. The Overview and Needs you are lists, over
      // which a thread's panel would only cover what the user went to see; the Threads view shows it picked.
      const to = intent.view;
      let next: Place = { ...place, view: to };
      if (to !== "map") next = { ...next, item: null };
      if (to === "overview" || to === "needs") next = { ...next, thread: null };
      // Needs you from its top, or at the entry asked for: only closing what an entry opened comes back where it was.
      if (to === "needs") next = { ...next, needsAt: intent.at, needsLeft: null };
      return only(next);
    }
    case "close": {
      const closing = openLayers(openOf(place, world)).slice(0, intent.count);
      // The command menu closes over a form without touching it.
      if (closing.some((layer) => layer !== "command") && world.dirty) return { place, requests: { unsaved: true } };
      return closing.reduce<Step>(
        (step, layer) => {
          const next = shut(step.place, layer, world);
          return { place: next.place, requests: { ...step.requests, ...next.requests } };
        },
        { place, requests: {} },
      );
    }
    case "shut":
      return shut(place, intent.layer, world);
    case "command":
      return only(overlay(place, { command: intent.open }));
    case "projects":
      return only(overlay(place, { projects: intent.open }));
    case "mode":
      return only(overlay(place, { mode: intent.mode }));
    case "add":
      return only(overlay(place, { mode: { kind: "idle" }, add: { parent: intent.parent, draft: null } }));
    case "redraft":
      return only(overlay(place, { add: intent.adding }));
    case "edit":
      return { place, requests: place.item === null ? {} : { edit: { item: place.item } } };
    case "reopen":
      return { place, requests: { reopenEditor: { id: intent.id, edited: intent.edited } } };
    case "wrote": {
      const closed = overlay(place, { add: null });
      if (intent.select === null) return only(closed);
      return {
        place: { ...closed, item: intent.select },
        requests: { focus: { id: intent.select, pin: intent.pin, preserveFrame: true } },
      };
    }
    case "added":
      if (intent.from === "map" || place.view !== intent.from) return only(place);
      // As one picked: on a Map zoomed out past reading, it comes in at a zoom its title reads at.
      return {
        place: { ...place, cameFrom: { view: intent.from, by: "item" }, view: "map" },
        requests: { focus: { id: intent.id, pin: null } },
      };
    case "leaveNeeds":
      return only({ ...place, needsLeft: intent.left });
    case "restore": {
      // As a link opens what it names, with where it came from as it was: the transient layers never come back.
      const to: Place = { ...intent.place, overlays: CLOSED };
      const onMap = to.view === "map";
      return {
        place: to,
        requests:
          onMap && to.item !== null
            ? { focus: { id: to.item, pin: null } }
            : onMap && to.thread !== null
              ? { frame: { thread: to.thread } }
              : {},
      };
    }
    case "link": {
      // A link names what it opens where it is, so the user came from nowhere else.
      const to = intent.wanted;
      const onMap = to.view === "map";
      return {
        place: { ...START, view: to.view, item: to.item, thread: to.thread, needsAt: to.needsAt },
        // On the Map, an item is brought into view, else a thread framed; over another view, left open there, as it
        // was when the address was written.
        requests:
          onMap && to.item !== null
            ? { focus: { id: to.item, pin: null } }
            : onMap && to.thread !== null
              ? { frame: { thread: to.thread } }
              : {},
      };
    }
  }
}

/**
 * Closes one layer: an item chosen from a thread's panel goes back to the thread, whose own close goes back further, to
 * the item it was opened from if that is still shown, brought into view.
 */
function shut(place: Place, layer: Layer, world: Pick<World, "has" | "archived">): Step {
  const only = (next: Place): Step => ({ place: next, requests: {} });
  const goBack = (next: Place): Place => ({
    ...next,
    view: place.cameFrom === null ? next.view : place.cameFrom.view,
    cameFrom: null,
  });
  if (layer === "command") return only(overlay(place, { command: false }));
  if (layer === "mode") return only(overlay(place, { mode: { kind: "idle" } }));
  if (layer === "add") return only(overlay(place, { add: null }));
  if (layer === "projects") return only(overlay(place, { projects: false }));
  if (layer === "away") return only({ ...place, view: HOME });
  if (layer === "item")
    return only(place.cameFrom?.by === "item" ? goBack({ ...place, item: null }) : { ...place, item: null });
  // The item under a thread opened from it, once that thread has closed and opened it again.
  if (layer === "under") return place.thread === null && place.item !== null ? shut(place, "item", world) : only(place);
  const under = itemUnder(place, world);
  if (under === null) return only(goBack({ ...place, thread: null }));
  const from = place.cameFrom?.view ?? "map";
  return {
    place: {
      ...place,
      thread: null,
      item: under,
      view: "map",
      cameFrom: from === "map" ? null : { view: from, by: "item" },
    },
    requests: { focus: { id: under, pin: null } },
  };
}

/** The item the thread open was opened from, if it is still shown: else closing the thread goes back past it. */
function itemUnder(place: Place, world: Pick<World, "has" | "archived">): string | null {
  const item = place.cameFrom?.by === "thread" ? place.cameFrom.item : undefined;
  return place.thread !== null && item !== undefined && world.has(item) && !world.archived(item) ? item : null;
}

// What's on top: the one sheet shown over the Map, which Escape and Back close first. One order for both, so a key
// never closes a panel hidden under the one in view; and one history entry per layer open, so Back on a phone closes
// them one by one before it leaves the app.

export type Layer = "command" | "mode" | "add" | "projects" | "item" | "thread" | "under" | "away";

export interface Open {
  /** The command menu: over everything, as it can be opened from anywhere. */
  command: boolean;
  /** A move or a link waiting for its target. */
  mode: boolean;
  adding: boolean;
  /** An item selected that is drawn. */
  item: boolean;
  /** The projects' sheet, on a phone. */
  projects: boolean;
  thread: boolean;
  /** The item a thread was opened from, under the thread's panel, which closing the panel opens again. */
  under: boolean;
  /**
   * On a phone, a view other than the home one: Back returns home before it leaves the app, as a phone's tabs do. The
   * bottom layer, under everything shown over the view.
   */
  away: boolean;
}

/** What of `place` is open as a layer: an item only once drawn; the projects' sheet and a view away only on a phone. */
export function openOf(place: Place, world: Pick<World, "has" | "archived" | "wide">): Open {
  return {
    command: place.overlays.command,
    mode: place.overlays.mode.kind !== "idle",
    adding: place.overlays.add !== null,
    item: place.item !== null && world.has(place.item),
    projects: place.overlays.projects && !world.wide,
    thread: place.thread !== null,
    under: itemUnder(place, world) !== null,
    away: !world.wide && place.view !== HOME,
  };
}

/** The layers open, the one shown first. */
export function openLayers(open: Open): Layer[] {
  // The projects' sheet is opened over an item, to see where it is, so it shows over it; an item chosen from a
  // thread's panel shows over the thread's.
  const order: Layer[] = ["command", "mode", "add", "projects", "item", "thread", "under", "away"];
  return order.filter((layer) => open[layer === "add" ? "adding" : layer]);
}

/** The layer shown, of those open, or null when the Map has none over it. */
export const topLayer = (open: Open): Layer | null => openLayers(open)[0] ?? null;

/** How many layers are open, each with its own history entry. */
export const depth = (open: Open): number => openLayers(open).length;

/**
 * The app's entries in the history: one per layer open, each holding how many layers it stands for, so Back lands on
 * one saying how many should stay open. `waiting` while a move back the app made itself hasn't landed yet: pushing
 * then would put the new entry where that move is about to leave.
 */
export interface Entries {
  pushed: number;
  waiting: boolean;
  /**
   * Forward landed on an entry whose layers open again from its address. Once they have, the entry is relabelled with
   * how many did, as one may stand for layers no address names, such as the command menu: going back from there would
   * only meet Forward again.
   */
  restoring?: boolean;
}

export type HistoryAction =
  | { kind: "push"; from: number; to: number }
  | { kind: "back"; by: number }
  /** Back, over `by` entries, closes as many layers from the top. */
  | { kind: "close"; by: number }
  /** Forward: open what the entry's address names. */
  | { kind: "restore" }
  /** The entry shown stands for `layers` layers. */
  | { kind: "relabel"; layers: number }
  | null;

/** Brings the entries to `open` layers: pushing more as they open, going back over the extra as the app closes them. */
export function follow(entries: Entries, open: number): { entries: Entries; action: HistoryAction } {
  if (entries.restoring) return restored(entries, open);
  if (entries.waiting || open === entries.pushed) return { entries, action: null };
  if (open > entries.pushed)
    return { entries: { pushed: open, waiting: false }, action: { kind: "push", from: entries.pushed, to: open } };
  return { entries: { pushed: open, waiting: true }, action: { kind: "back", by: entries.pushed - open } };
}

/**
 * Where a move through the history landed, on an entry standing for `at` layers, with `open` layers open: the app's
 * own move back done, which may let a push it held wait go ahead; Back, which closes the layer on top; or Forward,
 * which opens again what the entry's address names.
 */
export function arrived(entries: Entries, at: number, open: number): { entries: Entries; action: HistoryAction } {
  if (entries.waiting) return follow({ ...entries, waiting: false }, open);
  if (at > entries.pushed)
    return { entries: { pushed: at, waiting: false, restoring: true }, action: { kind: "restore" } };
  if (at === entries.pushed) return { entries, action: null };
  return { entries: { pushed: at, waiting: false }, action: { kind: "close", by: entries.pushed - at } };
}

/**
 * The entries once what Forward restored is open, or once its address waits behind a form: the entry shown says how
 * many layers are open, never moving through the history, and a layer more than it counted takes an entry of its own.
 */
export function restored(entries: Entries, open: number): { entries: Entries; action: HistoryAction } {
  const settled = { pushed: open, waiting: false };
  if (open > entries.pushed) return { entries: settled, action: { kind: "push", from: entries.pushed, to: open } };
  return { entries: settled, action: open < entries.pushed ? { kind: "relabel", layers: open } : null };
}

// The app's address: what is open, in the query, so the address bar always says it and a link opens it again. Only
// what a link should bring back is in it: the view (Needs you at an entry), an item and a thread. The add form, the
// outline's sheet and a move waiting for its target are passing states, left out.

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

/** What a history entry holds of the place it was written at: never what passes, the overlays, nor any request. */
export const stored = (place: Place): Place => ({ ...place, overlays: CLOSED });

/** Whether two places are the same, as an entry holds them. */
export const samePlace = (a: Place | null, b: Place | null): boolean => JSON.stringify(a) === JSON.stringify(b);

const idOrNull = (value: unknown): value is string | null => value === null || typeof value === "string";

/**
 * The place a history entry holds, or null for one that holds none, as entries written before places were do, or one
 * not this app's: those open what their address names, as a link does.
 */
export function placeIn(state: unknown): Place | null {
  const place = (state as { place?: Partial<Place> } | null)?.place;
  if (typeof place !== "object" || place === null) return null;
  const { view, item, thread, needsAt, cameFrom, needsLeft } = place;
  if (!VIEWS.some((one) => one === view) || !idOrNull(item) || !idOrNull(thread)) return null;
  return {
    ...START,
    view: view as View,
    item,
    thread,
    needsAt: idOrNull(needsAt) ? needsAt : null,
    cameFrom: typeof cameFrom === "object" && cameFrom !== null ? cameFrom : null,
    needsLeft: typeof needsLeft === "object" && needsLeft !== null ? needsLeft : null,
  };
}

/**
 * What of a place an entry held opens in the plan as it is, as `landing` says of a link: an item or a thread gone since
 * is dropped, and named, and an archived item opens with archived items shown.
 */
export function landingPlace(
  place: Place,
  has: Parameters<typeof landing>[1],
): { place: Place; archived: boolean; missing: ("item" | "thread")[] } {
  const found = landing({ view: place.view, item: place.item, thread: place.thread, needsAt: place.needsAt }, has);
  return {
    place: { ...place, item: found.wanted.item, thread: found.wanted.thread },
    archived: found.archived,
    missing: found.missing,
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
