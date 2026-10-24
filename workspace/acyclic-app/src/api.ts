// The server's API: one endpoint that runs a CLI command, given as JSON, and answers with the CLI's envelope.

export type Envelope<T> =
  | { ok: true; data: T }
  | { ok: false; error: { code: string; message: string; details: unknown } };

/** A scalar field as the tracker reports it: unset, one value, or concurrent writes that conflict. */
export type Raw<T> = T | null | { conflict: (T | null)[] };

/** An item as `show` returns it, with the fields the app reads. */
interface RawItem {
  id: string;
  title: Raw<string>;
  parent: Raw<string>;
  thread: Raw<string>;
  state: Raw<string>;
  archived: boolean;
  links: { waits_for: string[]; relates_to: string[] };
}

/** The fields whose conflicts the app settles for display. */
export type Settled = "title" | "parent" | "thread" | "state";

/**
 * An item with every field ready to draw: a conflicted field reads as its first value in the tracker's order, and
 * `conflicts` keeps each conflicted field's values, a clear among them as `null`, until the user resolves it.
 */
export interface Item extends Omit<RawItem, Settled> {
  title: string;
  parent: string | null;
  thread: string | null;
  state: string;
  conflicts: Partial<Record<Settled, (string | null)[]>>;
}

function isConflict<T>(raw: Raw<T>): raw is { conflict: (T | null)[] } {
  return typeof raw === "object" && raw !== null && "conflict" in raw;
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
  // An unset state reads as backlog, as the CLI reads it.
  const state = pick("state") ?? "backlog";
  return { ...raw, title, parent: pick("parent"), thread: pick("thread"), state, conflicts };
}

/** A refusal or failure, carrying the envelope's stable code. */
export class CommandError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

type Fetch = (input: string, init: RequestInit) => Promise<Response>;

/** A CLI command as the API takes it: a bare name for one without arguments, such as `"list"`, else `{name: args}`. */
export type Command = string | Record<string, unknown>;

/** Runs one command as the paired browser's user; the session cookie rides along, since the page is same-origin. */
export async function command<T>(body: Command, fetcher: Fetch = fetch): Promise<T> {
  const response = await fetcher("/api/command", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const envelope = (await response.json().catch(() => null)) as Envelope<T> | null;
  if (envelope === null) throw new CommandError("http", `the server answered ${response.status} without an envelope`);
  if (!envelope.ok) throw new CommandError(envelope.error.code, envelope.error.message);
  return envelope.data;
}

/** Every item with its links: `list` for the ids, then one `show` for all of them. */
export async function items(fetcher: Fetch = fetch): Promise<Item[]> {
  const { items: rows } = await command<{ items: { id: string }[] }>("list", fetcher);
  if (rows.length === 0) return [];
  const shown = await command<RawItem[]>({ show: { items: rows.map((row) => row.id) } }, fetcher);
  return shown.map(settle);
}
