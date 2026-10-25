// What's on top: the one sheet shown over the Map, which Escape (and Back) closes first. One order for both, so a
// key never closes a panel hidden under the one in view. Pure, so the order is tested without a browser.

export type Layer = "mode" | "add" | "item" | "needs" | "thread";

export interface Open {
  /** A move or a link waiting for its target. */
  mode: boolean;
  adding: boolean;
  /** An item selected that is drawn. */
  item: boolean;
  needs: boolean;
  thread: boolean;
}

/** The layer shown, of those open, or null when the Map has none over it. */
export function topLayer(open: Open): Layer | null {
  const order: Layer[] = ["mode", "add", "item", "needs", "thread"];
  return order.find((layer) => open[layer === "add" ? "adding" : layer]) ?? null;
}
