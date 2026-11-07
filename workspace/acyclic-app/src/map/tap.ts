// What a tap on the Map does, decided apart from the renderer so every case is tested without a canvas.

/** What a tap does: pick an edit's target, fold or unfold a box, or select an item. */
export type TapAction = "pick" | "toggle" | "select";
export interface Tapped {
  picking: boolean;
  chevron: boolean;
}
export function tapAction(tapped: Tapped): TapAction {
  if (tapped.picking) return "pick";
  return tapped.chevron ? "toggle" : "select";
}

/** What a tap on a stub does: open what it names, or, while an edit waits for a target, count as a tap on its tile. */
export type StubTap = { action: "open"; item: string } | { action: "through"; node: string };

/** A stub is no item to pick, so a tap on one while an edit waits for a target falls through to the tile under it. */
export function stubTap(picking: boolean, stub: { node: string; target: string }): StubTap {
  return picking ? { action: "through", node: stub.node } : { action: "open", item: stub.target };
}
