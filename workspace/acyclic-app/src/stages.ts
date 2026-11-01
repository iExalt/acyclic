// Where a leaf stands, said the same way everywhere: each stage's icon, which the Map's tasks and the app's text share,
// and its word; and a parent's leaves counted by stage, as text a glance can read and nothing hides behind a tap.

import type { Counts, Stage } from "./api";

/** Each stage's icon, drawn in a 16-unit square. */
export const STAGE_ICONS: Record<Stage, string> = {
  done: '<path d="M3 8.5l3.2 3.2L13 5"/>',
  "in-review": '<path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z"/><circle cx="8" cy="8" r="2"/>',
  "in-progress": '<circle cx="8" cy="8" r="6"/><path d="M8 2a6 6 0 0 0 0 12z" fill="currentColor"/>',
  ready: '<circle cx="8" cy="8" r="6"/>',
  blocked: '<circle cx="8" cy="8" r="6"/><path d="M4.5 8h7"/>',
  parked: '<path d="M6 3.5v9M10 3.5v9"/>',
  // The dashed ring a task with no thread carries where its badge would be: one mark for one fact everywhere.
  unthreaded: '<circle cx="8" cy="8" r="6" stroke-dasharray="3 2.4"/>',
};

/** Each stage's word, as a sentence starts it. */
export const STAGE_WORDS: Record<Stage, string> = {
  done: "Done",
  "in-review": "In review",
  "in-progress": "In progress",
  ready: "Ready",
  blocked: "Blocked",
  parked: "Parked",
  // A status, and what settles it: an item in no thread is never ready.
  unthreaded: "Needs a thread",
};

/** `count` leaves at `stage`, as a parent's counts say it: "3 ready", "1 needs a thread", "3 need a thread". */
export function stageCountWords(stage: Stage, count: number): string {
  if (stage === "unthreaded") return count === 1 ? "needs a thread" : "need a thread";
  return STAGE_WORDS[stage].toLowerCase();
}

/** The stages whose work is finished or set aside, drawn quieter. */
export const QUIET: ReadonlySet<Stage> = new Set(["done", "blocked", "parked", "unthreaded"]);

/**
 * A parent's leaves by stage, in the order work moves through them, leaving out stages with none: done apart from
 * those in review, which `done` counts too.
 */
export function stageCounts(counts: Counts): { stage: Stage; count: number }[] {
  const all: [Stage, number][] = [
    ["done", counts.done - counts.in_review],
    ["in-review", counts.in_review],
    ["in-progress", counts.in_progress],
    ["ready", counts.ready],
    ["blocked", counts.blocked],
    ["parked", counts.parked],
    ["unthreaded", counts.unthreaded],
  ];
  return all.filter(([, count]) => count > 0).map(([stage, count]) => ({ stage, count }));
}

/**
 * Where a parent's work stands as a whole, as one stage's icon tells it: in progress while any of it is moving, in
 * review while any waits for review, done once all of it is, else ready while any can start, else blocked, else
 * parked.
 */
export function overallStage(counts: Counts): Stage {
  if (counts.in_progress > 0) return "in-progress";
  if (counts.in_review > 0) return "in-review";
  if (counts.leaves > 0 && counts.done === counts.leaves) return "done";
  if (counts.ready > 0) return "ready";
  if (counts.blocked > 0) return "blocked";
  if (counts.parked > 0) return "parked";
  return "unthreaded";
}

/** A state as the tracker stores it, such as `in-progress`, in words: "In progress". */
export function stateWords(state: string): string {
  const words = state.replaceAll("-", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}
