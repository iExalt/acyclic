// What's on top: the one sheet shown over the Map, which Escape and Back close first. One order for both, so a key
// never closes a panel hidden under the one in view; and one history entry per layer open, so Back on a phone closes
// them one by one before it leaves the app. Pure, so the rules are tested without a browser.

export type Layer = "command" | "mode" | "add" | "projects" | "item" | "needs" | "thread" | "away";

export interface Open {
  /** The command menu: over everything, as it can be opened from anywhere. */
  command: boolean;
  /** A move or a link waiting for its target. */
  mode: boolean;
  adding: boolean;
  /** An item selected that is drawn. */
  item: boolean;
  needs: boolean;
  /** The projects' sheet, on a phone. */
  projects: boolean;
  thread: boolean;
  /**
   * On a phone, a view other than the home one: Back returns home before it leaves the app, as a phone's tabs do. The
   * bottom layer, under everything shown over the view.
   */
  away: boolean;
  /**
   * Not a layer but an order: a thread opened from the command menu while Needs you shows goes over it, where the user
   * is looking, rather than under it as a thread Needs you was opened over.
   */
  threadOverNeeds: boolean;
  /** Likewise, Needs you opened over an item shows over it, and closing it shows the item again. */
  needsOverItem: boolean;
}

/** The layers open, the one shown first. */
export function openLayers(open: Open): Layer[] {
  // The projects' sheet is opened over an item or the needs list, to see where they are, so it shows over both.
  const order: Layer[] = ["command", "mode", "add", "projects", "item", "needs", "thread", "away"];
  const swap = (a: Layer, b: Layer) => {
    const [i, j] = [order.indexOf(a), order.indexOf(b)];
    [order[i], order[j]] = [b, a];
  };
  if (open.needsOverItem) swap("item", "needs");
  if (open.threadOverNeeds) swap("needs", "thread");
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
