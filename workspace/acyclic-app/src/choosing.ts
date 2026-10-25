// A setting picked from a list, such as an autonomy level, where each pick is a write. A pick from the open list
// writes at once; an arrow key on the closed list, which steps through the options in some browsers, only moves a
// draft, written on Enter and dropped on Escape or on leaving the list. The two are told apart by when the change
// comes: an arrow key's within the key's own task, a pick's later, since the page never sees the keys in the open
// list. Pure, so every rule is tested without a browser.

/** The draft shown in place of the value written, if any. */
export interface ChoiceState {
  draft: string | null;
}

export type ChoiceEvent =
  /** `keyed` when it comes within a key's own task. */
  | { kind: "change"; value: string; keyed: boolean }
  | { kind: "enter" }
  /** Escape, or the list losing focus. */
  | { kind: "cancel" };

export const UNCHOSEN: ChoiceState = { draft: null };

/** The state after `event`, with what to write, if anything, given the value written now, `applied`. */
export function choose(
  state: ChoiceState,
  applied: string,
  event: ChoiceEvent,
): { state: ChoiceState; write: string | null } {
  switch (event.kind) {
    case "change":
      if (event.keyed) return { state: { draft: event.value === applied ? null : event.value }, write: null };
      return { state: UNCHOSEN, write: event.value === applied ? null : event.value };
    case "enter":
      return { state: UNCHOSEN, write: state.draft };
    case "cancel":
      return { state: UNCHOSEN, write: null };
  }
}
