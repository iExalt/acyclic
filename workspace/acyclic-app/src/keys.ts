// Single-key shortcuts, in Linear's habits: G then a letter goes to a view, C adds, / searches, ? lists the keys, and
// with an item's panel shown, E, M, W, R and A act on it. No key approves, keeps, drops or deletes anything. Keys are
// ignored while the user is typing, composing, holding a modifier or a form holds what they typed, and all of them
// can be turned off from the menu (WCAG 2.1.4). Pure, so the rules are tested without a browser.

import type { LinkKind } from "./edits";
import type { Layer } from "./layers";
import type { View } from "./Shell";

export type KeyAction =
  | { kind: "go"; view: View }
  | { kind: "add" }
  | { kind: "search" }
  | { kind: "help" }
  | { kind: "edit" }
  | { kind: "move" }
  | { kind: "link"; link: LinkKind }
  | { kind: "addInside" }
  | { kind: "row"; by: 1 | -1 };

/** When G was pressed, waiting for the letter that says where to go; null when nothing waits. */
export type Waiting = number | null;

/** How long G waits for its letter. */
export const SEQUENCE_MS = 1000;

export type KeyPress = Pick<
  KeyboardEvent,
  "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode" | "repeat" | "timeStamp"
>;

export interface KeyContext {
  /** The menu's switch. */
  enabled: boolean;
  /** The layer shown under the command menu, if any; null with none. */
  panel: Layer | null;
  /** Whether the command menu is open: its field has the keys. */
  menu: boolean;
  /** A form holds what the user typed. */
  dirty: boolean;
  /** The key went to a field, or anything else that takes typing. */
  typing: boolean;
  /** The rows J and K step through in the list in view: with none, they are left to the page. */
  rows: number;
}

/** The letter a key is, from the layout's character, or from where it sits, for layouts without Latin letters. */
function letterOf(press: KeyPress): string | null {
  if (/^[a-z]$/i.test(press.key)) return press.key.toLowerCase();
  const at = /^Key([A-Z])$/.exec(press.code);
  return at === null ? null : (at[1]?.toLowerCase() ?? null);
}

const GO: Record<string, KeyAction> = {
  o: { kind: "go", view: "overview" },
  m: { kind: "go", view: "map" },
  t: { kind: "go", view: "threads" },
  n: { kind: "go", view: "needs" },
};

const ON_ITEM: Record<string, KeyAction> = {
  e: { kind: "edit" },
  m: { kind: "move" },
  w: { kind: "link", link: "waits-for" },
  r: { kind: "link", link: "relates-to" },
  a: { kind: "addInside" },
};

/** What a key press does, and what it leaves waiting. */
export function step(
  pending: Waiting,
  press: KeyPress,
  context: KeyContext,
): { pending: Waiting; action: KeyAction | null } {
  const none = { pending: null, action: null };
  if (!context.enabled || context.menu || context.typing || context.dirty) return none;
  if (press.isComposing || press.keyCode === 229 || press.repeat) return none;
  if (press.ctrlKey || press.metaKey || press.altKey) return none;
  // ? sits on a shifted key on most layouts: the character is what counts.
  if (press.key === "?") return { pending: null, action: { kind: "help" } };
  if (press.shiftKey) return none;
  if (press.key === "/") return { pending: null, action: { kind: "search" } };
  const letter = letterOf(press);
  if (letter === null) return none;
  if (pending !== null && press.timeStamp - pending <= SEQUENCE_MS)
    return { pending: null, action: GO[letter] ?? null };
  if (letter === "g") return { pending: press.timeStamp, action: null };
  if (letter === "c") return { pending: null, action: { kind: "add" } };
  if (letter === "j" || letter === "k")
    return context.rows === 0 ? none : { pending: null, action: { kind: "row", by: letter === "j" ? 1 : -1 } };
  if (context.panel === "item") return { pending: null, action: ON_ITEM[letter] ?? null };
  return none;
}

/** The row to move to in a list of `count`, from `at` (-1 for none), by one: the first or last from none, else stopping at the ends. */
export function nextRow(count: number, at: number, by: 1 | -1): number {
  if (count === 0) return -1;
  if (at < 0) return by > 0 ? 0 : count - 1;
  return Math.min(count - 1, Math.max(0, at + by));
}

/** Whether a key went to something that takes typing. */
export function typingIn(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const type = target instanceof HTMLInputElement ? target.type : null;
  return target.isContentEditable || takesTyping(target.tagName, type) || target.closest(".command") !== null;
}

/** The inputs a key press only presses, such as a checkbox: a single key there is still a shortcut. */
const PRESSED = new Set(["checkbox", "radio", "button", "submit", "reset", "image", "range", "color", "file"]);

/** Whether an element, by its tag and an input's type, takes typing: a text field or a list to pick from. */
export function takesTyping(tag: string, type: string | null): boolean {
  const name = tag.toLowerCase();
  if (name === "textarea" || name === "select") return true;
  return name === "input" && !PRESSED.has((type ?? "text").toLowerCase());
}

/** The shortcut for each command menu entry that has one, as it shows; G sequences read "G then M". */
export const SHORTCUTS: Record<string, string> = {
  "view:overview": "G then O",
  "view:map": "G then M",
  "view:threads": "G then T",
  "view:needs": "G then N",
  add: "C",
  "on-item:edit": "E",
  "on-item:move": "M",
  "on-item:waits-for": "W",
  "on-item:relates-to": "R",
  "on-item:add-inside": "A",
};

/** What each item key does, said as the reference says it, for the keys that act only with an item's panel shown. */
const ON_ITEM_WORDS: Record<string, string> = {
  "on-item:edit": "edit",
  "on-item:move": "move",
  "on-item:waits-for": "waits for",
  "on-item:relates-to": "relates to",
  "on-item:add-inside": "add inside",
};

/** The rest, as the menu's entries name them, should one not be runnable. */
const OTHER_WORDS: Record<string, string> = {
  "view:overview": "Go to Overview",
  "view:map": "Go to Map",
  "view:threads": "Go to Threads",
  "view:needs": "Go to Needs you",
  add: "New item",
};

/**
 * The keys ? can't list as entries to run: those of `SHORTCUTS` not runnable now (`runnable` holds the entries' keys
 * the menu lists), and the keys no entry stands for, with the platform's own chords.
 */
export function keyReference(runnable: ReadonlySet<string>, mac: boolean): { what: string; keys: string }[] {
  const resting = Object.keys(SHORTCUTS).filter((key) => !runnable.has(key));
  const onItem = resting.filter((key) => key in ON_ITEM_WORDS);
  const other = resting.filter((key) => !(key in ON_ITEM_WORDS));
  return [
    ...(onItem.length === 0
      ? []
      : [
          {
            what: "With an item open",
            keys: onItem.map((key) => `${SHORTCUTS[key]} ${ON_ITEM_WORDS[key]}`).join(", "),
          },
        ]),
    ...other.map((key) => ({ what: OTHER_WORDS[key] ?? key, keys: SHORTCUTS[key] ?? "" })),
    { what: "Next or previous row", keys: "J, K" },
    { what: "Search", keys: mac ? "/ or ⌘K" : "/ or Ctrl K" },
    { what: "Undo, while Undo shows", keys: mac ? "⌘Z" : "Ctrl Z" },
  ];
}
