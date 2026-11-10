// The server's API: one endpoint that runs a CLI command, given as JSON, and answers with the CLI's envelope.

import { stageCounts, stageCountWords } from "./stages";

export type Envelope<T> =
  | { ok: true; data: T }
  | { ok: false; error: { code: string; message: string; details: unknown } };

/** A scalar field as the tracker reports it: unset, one value, or concurrent writes that conflict. */
export type Raw<T> = T | null | { conflict: (T | null)[] };

/** A condition the server checks with an app write under the tracker lock. */
export type Expected =
  | { kind: "item-field"; item: string; field: "title" | "parent" | "state" | "notes"; value: Raw<string> }
  | { kind: "thread-field"; thread: string; field: "title" | "notes"; value: Raw<string> }
  | { kind: "waits-for"; item: string; target: string; present: boolean }
  | { kind: "started-at-most"; item: string; count: number }
  | { kind: "waiters-started-at-most"; item: string; baselines: { item: string; count: number }[] };

/** An item as `item show` returns it, with the fields the app reads. */
interface RawItem {
  id: string;
  title: Raw<string>;
  parent: Raw<string>;
  thread: Raw<string>;
  state: Raw<string>;
  notes: Raw<string>;
  /** Completion evidence is checked when a write loses its response. */
  evidence?: Raw<{ repository?: string | null; commit?: string | null; text?: string | null }>;
  /** The initiative above it at any depth; null when it is in none. */
  initiative: string | null;
  links: { waits_for: string[]; waited_on_by: string[] };
  /**
   * Where it stands, as core tells it: a leaf's own stage, or a parent's from its leaves. A parent with no leaves to
   * count has none.
   */
  stage?: Stage;
  /** A parent's leaves counted by stage; a leaf has none. */
  progress?: Progress;
}

/**
 * Where a leaf stands: core's seven stages, which tell every leaf apart. A parent takes one from its leaves, those
 * canceled left out: done once all are; else in progress once work under it has started and some of what is left isn't
 * parked; else the first of stalled, ready, waiting and parked that its leaves have. One with nothing but
 * canceled leaves, or canceled by hand, reads canceled.
 */

export type Stage = "done" | "in-progress" | "ready" | "waiting" | "parked" | "canceled" | "stalled";

/**
 * A parent's leaves: each under its stage, so the stages but `canceled` add up to `leaves`. A canceled leaf isn't one
 * of the leaves.
 */
export interface Counts {
  done: number;
  leaves: number;
  in_progress: number;
  ready: number;
  stalled: number;
  waiting: number;
  parked: number;
  canceled: number;
}

/** No leaves at all. */
export const NO_COUNTS: Counts = {
  done: 0,
  leaves: 0,
  in_progress: 0,
  ready: 0,
  stalled: 0,
  waiting: 0,
  parked: 0,
  canceled: 0,
};

export interface Progress {
  counts: Counts;
}

/** A parent's leaves by stage in words, in the order work moves through them, leaving out stages with none. */
export function countsText(counts: Counts): string {
  return stageCounts(counts)
    .map(({ stage, count }) => `${count} ${stageCountWords(stage)}`)
    .join(", ");
}

/** The fields whose conflicts the app settles for display. */
export type Settled = "title" | "parent" | "thread" | "state" | "notes";

/**
 * An item with every field ready to draw: a conflicted field reads as its first value in the tracker's order, and
 * `conflicts` keeps each conflicted field's values, a clear among them as `null`, until the user resolves it.
 */
export interface Item extends Omit<RawItem, Settled> {
  title: string;
  parent: string | null;
  thread: string | null;
  state: string;
  notes: string | null;
  conflicts: Partial<Record<Settled, (string | null)[]>>;
}

function isConflict<T>(raw: Raw<T>): raw is { conflict: (T | null)[] } {
  return typeof raw === "object" && raw !== null && "conflict" in raw;
}

/** A title to show: a conflicted one reads as its first value, as an item's does, and one unset or unreadable as `id`. */
export function titleOr(raw: unknown, id: string): string {
  if (typeof raw === "string") return raw;
  const values = isConflict(raw as Raw<unknown>) ? (raw as { conflict: unknown[] }).conflict : [];
  return values.find((one): one is string => typeof one === "string") ?? id;
}

/** Settles an item's conflicted fields, once, at the API's edge. */
export function settle(raw: RawItem): Item {
  const conflicts: Item["conflicts"] = {};
  const pick = (field: Settled): string | null => {
    const value = raw[field];
    if (!isConflict(value)) return value;
    conflicts[field] = value.conflict;
    return value.conflict.find((one) => one !== null) ?? null;
  };
  const title = pick("title") ?? raw.id;
  // An unset state reads as open, as the CLI reads it.
  const state = pick("state") ?? "open";
  return {
    ...raw,
    title,
    parent: pick("parent"),
    thread: pick("thread"),
    state,
    notes: pick("notes"),
    conflicts,
  };
}

/** A refusal or failure, carrying the envelope's stable code. */
export class CommandError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly details: unknown = null,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

/** A failure as the app tells it: a refusal or a lost request by its code, and anything else as `network`. */
export function failure(err: unknown): { code: string; message: string } {
  if (err instanceof CommandError) {
    return {
      code: err.code,
      message: err.code === "in-use" ? inUseMessage(err.details) : (REFUSALS[err.code] ?? sentence(err.message)),
    };
  }
  return { code: "network", message: err instanceof Error ? err.message : String(err) };
}

/**
 * The refusals a write from the app can meet, in its words, as the server's name records by id. Any other is the
 * server's own message, as a sentence.
 */
const REFUSALS: Record<string, string> = {
  "parent-cycle": "An item can't go inside itself, or inside anything under it",
  "waits-for-cycle": "That link would close a loop: the items would wait on each other",
  "self-link": "An item can't link to itself",
  deleted: "That item was deleted after it was added: send it again to add it anew",
  "not-found": "It isn't in the plan",
  "changed-since": "Changed since, so it can't be undone",
  locked: "The tracker is busy with another write: try again in a moment",
};

const IN_USE_MESSAGES = {
  "has-members": "Move or delete this thread's items before deleting the thread",
  claimed: "Release this thread's active claim before deleting the thread",
  "holds-items": "Move or delete this initiative's items before deleting the initiative",
  "task-thread": "Delete the task instead of its thread",
};

/** Refusals name the action to take, never the server's record IDs or claim holder. */
function inUseMessage(details: unknown): string {
  const fallback = "It is still in use and can't be deleted yet";
  if (
    typeof details !== "object" ||
    details === null ||
    !("reasons" in details) ||
    !Array.isArray(details.reasons) ||
    details.reasons.length === 0
  )
    return fallback;
  const messages: string[] = [];
  for (const reason of details.reasons) {
    if (
      typeof reason !== "object" ||
      reason === null ||
      typeof reason.why !== "string" ||
      !Object.hasOwn(IN_USE_MESSAGES, reason.why)
    )
      return fallback;
    messages.push(IN_USE_MESSAGES[reason.why as keyof typeof IN_USE_MESSAGES]);
  }
  return [...new Set(messages)].join(". ");
}

/** A message as a sentence: its first letter capitalised, as the server's start in lower case. */
function sentence(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1);
}

export type Fetch = (input: string, init: RequestInit) => Promise<Response>;

/**
 * A CLI command as the API takes it: a bare name for one without arguments, such as `"needs"`, else `{name: args}`,
 * nested under its group, such as `{item: "list"}`.
 */
export type Command = string | Record<string, unknown>;

/** Runs one command as the user. */
export async function command<T>(
  body: Command,
  fetcher: Fetch = fetch,
  expect?: readonly Expected[],
  observed?: readonly string[],
): Promise<T> {
  // A request that never reached the server says so in each browser's own words ("Failed to fetch", "Load
  // failed"…), so it's told in the app's.
  const response = await fetcher("/api/command", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(
      expect === undefined && observed === undefined
        ? body
        : {
            command: body,
            ...(expect === undefined ? {} : { expect }),
            ...(observed === undefined ? {} : { observed }),
          },
    ),
  }).catch(() => {
    throw new CommandError("network", "Can't reach the server");
  });
  const envelope = (await response.json().catch(() => null)) as Envelope<T> | null;
  if (envelope === null) throw new CommandError("http", `the server answered ${response.status} without an envelope`);
  if (!envelope.ok) throw new CommandError(envelope.error.code, envelope.error.message, envelope.error.details);
  return envelope.data;
}

/** Every item with its links: `item list` for the ids, then one `item show` for all of them. */
export async function items(fetcher: Fetch = fetch): Promise<Item[]> {
  const { items: rows } = await command<{ items: { id: string }[] }>({ item: "list" }, fetcher);
  if (rows.length === 0) return [];
  const shown = await command<RawItem[]>({ item: { show: { items: rows.map((row) => row.id) } } }, fetcher);
  return shown.map(settle);
}

/** A thread, with what the app reads of it. */
export interface Thread {
  id: string;
  title: string;
  /** The task this thread of one stands for while it has no title of its own: then its title is the task's. */
  task: string | null;
  /** The threads whose work its own waits on. */
  waitsFor: string[];
  /** Its member leaves by stage, as core counts them: as core counts them. */
  counts: Counts;
}

/**
 * Every thread, and those with work left in waves, first to last: each after the threads it waits on, unless they
 * wait on each other, when they share one. Those that wait on none and that none waits on can start any time.
 */
export interface Threads {
  threads: Thread[];
  waves: string[][];
  anytime: string[];
}

interface RawThreads {
  threads: {
    id: string;
    title: Raw<string>;
    task?: string | null;
    waits_for: string[];
    progress?: { counts: Counts };
  }[];
  waves: string[][];
  anytime: string[];
}

/** Every thread, in waves. */
export async function threads(fetcher: Fetch = fetch): Promise<Threads> {
  const raw = await command<RawThreads>({ thread: "list" }, fetcher);
  return {
    threads: raw.threads.map((row) => ({
      id: row.id,
      title: titleOr(row.title, row.id),
      task: row.task ?? null,
      waitsFor: row.waits_for,
      counts: row.progress?.counts ?? NO_COUNTS,
    })),
    waves: raw.waves,
    anytime: raw.anytime,
  };
}

/** A thread as its panel shows it. */
export interface ThreadDetail {
  id: string;
  title: string;
  /** The task a thread of one stands for, as in `Thread`. */
  task: string | null;
  notes: string | null;
  members: { id: string; title: string }[];
}

const text = (raw: Raw<string>, fallback: string) => (typeof raw === "string" ? raw : fallback);

export async function thread(id: string, fetcher: Fetch = fetch): Promise<ThreadDetail> {
  type Row = Omit<ThreadDetail, "title" | "task" | "notes" | "members"> & {
    task?: string | null;
    title: Raw<string>;
    notes: Raw<string>;
    members: { id: string; title: Raw<string> }[];
  };
  const [row] = await command<Row[]>({ thread: { show: { threads: [id] } } }, fetcher);
  if (row === undefined) throw new CommandError("not-found", `no thread ${id}`);
  return {
    ...row,
    title: text(row.title, row.id),
    task: row.task ?? null,
    notes: typeof row.notes === "string" ? row.notes : null,
    members: row.members.map((member) => ({ id: member.id, title: text(member.title, member.id) })),
  };
}

/** An initiative, the top level above the items. */
export interface Initiative {
  id: string;
  title: string;
  items: number;
}

/** Every initiative, and the items whose parent is unset or names nothing: in no initiative. */
export interface Initiatives {
  initiatives: Initiative[];
  noInitiative: string[];
}

/** Every initiative, with the ids of the items in none. */
export async function initiatives(fetcher: Fetch = fetch): Promise<Initiatives> {
  const raw = await command<{
    initiatives: { id: string; title: Raw<string>; counts?: { items?: number } }[];
    no_initiative: { items: string[] };
  }>({ initiative: "list" }, fetcher);
  return {
    initiatives: raw.initiatives.map((row) => ({
      id: row.id,
      title: titleOr(row.title, row.id),
      items: row.counts?.items ?? 0,
    })),
    noInitiative: raw.no_initiative.items,
  };
}

/** What needs the user, as `acyclic needs` lists it. */
export async function needs(fetcher: Fetch = fetch): Promise<({ kind: string } & Record<string, unknown>)[]> {
  return (await command<{ needs: ({ kind: string } & Record<string, unknown>)[] }>("needs", fetcher)).needs;
}
