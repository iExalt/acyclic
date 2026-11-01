// How long a notice lasts. Every notice shows in one place, the slot at the stage's foot, which takes its height from
// the view rather than covering it; how long it stays there follows what it is about: news goes once the user moves
// on, a refusal once the surface that raised it closes, a form's warning with the form, and what is offered (Undo, a
// refresh that failed) whatever the user does next. Pure, so the rules are tested without a browser.

import type { Layer } from "./layers";
import type { View } from "./Shell";

/** Where the user is: the panel on top, the item and the thread open, and the view under them. */
export interface Place {
  layer: Layer | null;
  item: string | null;
  thread: string | null;
  view: View;
}

/**
 * What a notice is, which says how long it lasts:
 * - "news": what just happened, such as a save, an undo or a link opened; until the user moves.
 * - "refusal": what the surface it was raised on couldn't do; until that surface closes.
 * - "offer": Undo, and what an undo came to; until the next write or Dismiss.
 * - "stale": a read of the plan that failed, the plan shown out of date; until a read lands, a write, or Dismiss.
 * - "form": said of a form holding typing; until the form closes.
 */
export type Lasts = "news" | "refusal" | "offer" | "stale" | "form";

export interface Raised {
  lasts: Lasts;
  /** Where it was first shown, or null until it has been. */
  at: Place | null;
}

export type NoticeEvent =
  /** The user is now at `to`; a notice not yet shown is shown here. */
  | { kind: "moved"; to: Place }
  /** A write starts. */
  | { kind: "write" }
  /** A read of the plan landed. */
  | { kind: "read" }
  /** A form closed, saved or not. */
  | { kind: "form-closed" }
  | { kind: "dismiss" };

/** The notice after `event`, or null when it goes. */
export function noticeAfter<T extends Raised>(notice: T | null, event: NoticeEvent): T | null {
  if (notice === null || event.kind === "dismiss") return null;
  if (event.kind === "write") return notice.lasts === "form" ? notice : null;
  if (event.kind === "form-closed") return notice.lasts === "form" ? null : notice;
  if (event.kind === "read") return notice.lasts === "stale" ? null : notice;
  if (notice.at === null) return { ...notice, at: event.to };
  const at = notice.at;
  const to = event.to;
  if (notice.lasts === "news") return same(at, to) ? notice : null;
  if (notice.lasts === "refusal") return sameSurface(at, to) ? notice : null;
  return notice;
}

const same = (a: Place, b: Place) =>
  a.layer === b.layer && a.item === b.item && a.thread === b.thread && a.view === b.view;

/**
 * The same surface: the same panel showing the same item or thread, or, with no panel, the same view. Any other layer,
 * such as a move waiting for its target or the add form, is the same surface for as long as it stays open, so a
 * refused target is said for as long as the move waits.
 */
function sameSurface(a: Place, b: Place): boolean {
  if (a.layer !== b.layer) return false;
  if (a.layer === "item") return a.item === b.item;
  if (a.layer === "thread") return a.thread === b.thread;
  if (a.layer === null || a.layer === "away") return a.view === b.view;
  return true;
}
