// What's on top: the one sheet shown over the Map, which Escape and Back close first. One order for both, so a key
// never closes a panel hidden under the one in view; and one history entry per layer open, so Back on a phone closes
// them one by one before it leaves the app. Pure, so the rules are tested without a browser.

export type Layer = "mode" | "add" | "phases" | "item" | "needs" | "thread" | "away";

export interface Open {
  /** A move or a link waiting for its target. */
  mode: boolean;
  adding: boolean;
  /** An item selected that is drawn. */
  item: boolean;
  needs: boolean;
  /** The phases' sheet, on a phone. */
  phases: boolean;
  thread: boolean;
  /**
   * On a phone, a view other than the home one: Back returns home before it leaves the app, as a phone's tabs do. The
   * bottom layer, under everything shown over the view.
   */
  away: boolean;
}

/** The layers open, the one shown first. */
export function openLayers(open: Open): Layer[] {
  // The phases' sheet is opened over an item or the needs list, to see where they are, so it shows over both.
  const order: Layer[] = ["mode", "add", "phases", "item", "needs", "thread", "away"];
  return order.filter((layer) => open[layer === "add" ? "adding" : layer]);
}

/** The layer shown, of those open, or null when the Map has none over it. */
export const topLayer = (open: Open): Layer | null => openLayers(open)[0] ?? null;

/** How many layers are open, each with its own history entry. */
export const depth = (open: Open): number => Object.values(open).filter(Boolean).length;

/**
 * The app's entries in the history: one per layer open, each holding how many layers it stands for, so Back lands on
 * one saying how many should stay open. `waiting` while a move back the app made itself hasn't landed yet: pushing
 * then would put the new entry where that move is about to leave.
 */
export interface Entries {
  pushed: number;
  waiting: boolean;
}

export type HistoryAction =
  | { kind: "push"; from: number; to: number }
  | { kind: "back"; by: number }
  /** Back, over `by` entries, closes as many layers from the top. */
  | { kind: "close"; by: number }
  | null;

/** Brings the entries to `open` layers: pushing more as they open, going back over the extra as the app closes them. */
export function follow(entries: Entries, open: number): { entries: Entries; action: HistoryAction } {
  if (entries.waiting || open === entries.pushed) return { entries, action: null };
  if (open > entries.pushed)
    return { entries: { pushed: open, waiting: false }, action: { kind: "push", from: entries.pushed, to: open } };
  return { entries: { pushed: open, waiting: true }, action: { kind: "back", by: entries.pushed - open } };
}

/**
 * Where a move through the history landed, on an entry standing for `landed` layers, with `open` layers open: the
 * app's own move back done, which may let a push it held wait go ahead; or Back, which closes the layer on top.
 */
export function arrived(entries: Entries, at: number, open: number): { entries: Entries; action: HistoryAction } {
  if (entries.waiting) return follow({ ...entries, waiting: false }, open);
  if (at >= entries.pushed) return { entries, action: null };
  return { entries: { pushed: at, waiting: false }, action: { kind: "close", by: entries.pushed - at } };
}
