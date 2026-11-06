// Where a leaf stands, said the same way everywhere: each stage's icon, which the Map's items and the app's text share,
// and its word; and a parent's leaves counted by stage, as text a glance can read and nothing hides behind a tap.

import type { Counts, Stage } from "./api";

/** Each stage's icon, drawn in a 16-unit square. */
export const STAGE_ICONS: Record<Stage, string> = {
  done: '<path d="M3 8.5l3.2 3.2L13 5"/>',
  "in-progress": '<circle cx="8" cy="8" r="6"/><path d="M8 2a6 6 0 0 0 0 12z" fill="currentColor"/>',
  ready: '<circle cx="8" cy="8" r="6"/>',
  // A clock: it waits for something, rather than being barred.
  waiting: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8h2.8"/>',
  parked: '<path d="M6 3.5v9M10 3.5v9"/>',
  // The dashed ring an item with no thread carries where its badge would be: one mark for one fact everywhere.
  unthreaded: '<circle cx="8" cy="8" r="6" stroke-dasharray="3 2.4"/>',
  // A circle struck through: dropped on purpose, never done.
  canceled: '<circle cx="8" cy="8" r="6"/><path d="M3.8 12.2l8.4-8.4"/>',
  // A circle with an exclamation mark: ready, yet its holder's lease ran out and it wants a new one.
  stalled: '<circle cx="8" cy="8" r="6"/><path d="M8 4.8v3.4M8 10.6v.6"/>',
};

/** Each stage's word, as a sentence starts it. */
export const STAGE_WORDS: Record<Stage, string> = {
  done: "Done",
  "in-progress": "In progress",
  ready: "Ready",
  waiting: "Waiting",
  parked: "Parked",
  // A status, and what settles it: an item in no thread is never ready.
  unthreaded: "Needs a thread",
  canceled: "Canceled",
  stalled: "Stalled",
};

/** `count` leaves at `stage`, as a parent's counts say it: "3 ready", "1 needs a thread", "3 need a thread". */
export function stageCountWords(stage: Stage, count: number): string {
  if (stage === "unthreaded") return count === 1 ? "needs a thread" : "need a thread";
  return STAGE_WORDS[stage].toLowerCase();
}

/** The stages whose work is finished, set aside or dropped, drawn quieter. */
export const QUIET: ReadonlySet<Stage> = new Set(["done", "waiting", "parked", "unthreaded", "canceled"]);

/** A parent's leaves by stage, in the order work moves through them, leaving out stages with none. */
export function stageCounts(counts: Counts): { stage: Stage; count: number }[] {
  const all: [Stage, number][] = [
    ["done", counts.done],
    ["in-progress", counts.in_progress],
    ["stalled", counts.stalled],
    ["ready", counts.ready],
    ["waiting", counts.waiting],
    ["parked", counts.parked],
    ["unthreaded", counts.unthreaded],
    ["canceled", counts.canceled],
  ];
  return all.filter(([, count]) => count > 0).map(([stage, count]) => ({ stage, count }));
}

/** A state as the tracker stores it, such as `in-progress`, in words: "In progress". */
export function stateWords(state: string): string {
  const words = state.replaceAll("-", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}
