// What a tap on the Map does, decided apart from the renderer so every case is tested without a canvas.

/** What a tap does: nothing, pick an edit's target, fold or unfold a box, pick a thread, or select an item. */
export type TapAction = "ignore" | "pick" | "toggle" | "thread" | "select";

export interface Tapped {
  /** Whether an edit waits for a target. */
  picking: boolean;
  /** Whether a done box was tapped, which is the Map's own and no item. */
  doneBox: boolean;
  /** The node's kind: a box, a folded box, or a tile. */
  kind: string;
  /** A finished thread's box's thread; null for anything else. */
  thread: string | null;
  /** Whether the tap fell on the node's chevron. */
  chevron: boolean;
}

/**
 * What a tap on a node does. An edit waiting for a target takes any item, but never a done box. A chevron folds or
 * unfolds its box. Anywhere else, a finished thread's box picks its thread; the Done box, which holds no one thing to
 * pick, unfolds when folded, and, open, does nothing, so a tap between its tiles doesn't fold them all away; an item is
 * selected.
 */
export function tapAction(tapped: Tapped): TapAction {
  if (tapped.picking) return tapped.doneBox ? "ignore" : "pick";
  if (tapped.chevron) return "toggle";
  if (!tapped.doneBox) return "select";
  if (tapped.thread !== null) return "thread";
  return tapped.kind === "collapsed" ? "toggle" : "ignore";
}

/** What a tap on a stub does: open what it names, or, while an edit waits for a target, count as a tap on its tile. */
export type StubTap = { action: "open"; item: string } | { action: "through"; node: string };

/** A stub is no item to pick, so a tap on one while an edit waits for a target falls through to the tile under it. */
export function stubTap(picking: boolean, stub: { node: string; target: string }): StubTap {
  return picking ? { action: "through", node: stub.node } : { action: "open", item: stub.target };
}
