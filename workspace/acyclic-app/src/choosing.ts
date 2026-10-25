// A setting picked from a list, such as an autonomy level, where each pick is a write. A pick from the open list
// writes at once; an arrow key on the closed list, which steps through the options in some browsers, only moves a
// draft, written on Enter and dropped on Escape or on leaving the list. Pure, so every rule is tested without a browser.

/** The draft shown in place of the value written, if any, and whether the next change comes from a key. */
export interface ChoiceState {
  draft: string | null;
  keyed: boolean;
}

export type ChoiceEvent =
  | { kind: "key" }
  | { kind: "pointer" }
  | { kind: "change"; value: string }
  | { kind: "enter" }
  /** Escape, or the list losing focus. */
  | { kind: "cancel" };

export const UNCHOSEN: ChoiceState = { draft: null, keyed: false };

/** The state after `event`, with what to write, if anything, given the value written now, `applied`. */
export function choose(
  state: ChoiceState,
  applied: string,
  event: ChoiceEvent,
): { state: ChoiceState; write: string | null } {
  switch (event.kind) {
    case "key":
      return { state: { ...state, keyed: true }, write: null };
    case "pointer":
      return { state: { ...state, keyed: false }, write: null };
    case "change":
      if (state.keyed)
        return { state: { draft: event.value === applied ? null : event.value, keyed: false }, write: null };
      return { state: UNCHOSEN, write: event.value === applied ? null : event.value };
    case "enter":
      return { state: UNCHOSEN, write: state.draft };
    case "cancel":
      return { state: UNCHOSEN, write: null };
  }
}
