// The server's API: one endpoint that runs a CLI command, given as JSON, and answers with the CLI's envelope.

export type Envelope<T> =
  | { ok: true; data: T }
  | { ok: false; error: { code: string; message: string; details: unknown } };

/** An item as `show` returns it, with the fields the app reads. */
export interface Item {
  id: string;
  title: string;
  parent: string | null;
  thread: string | null;
  state: string;
  archived: boolean;
  links: { waits_for: string[]; relates_to: string[] };
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

/** Runs one command as the paired browser's user; the session cookie rides along, since the page is same-origin. */
export async function command<T>(body: object, fetcher: Fetch = fetch): Promise<T> {
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
  const rows = await command<{ id: string }[]>({ list: {} }, fetcher);
  if (rows.length === 0) return [];
  return command<Item[]>({ show: { items: rows.map((row) => row.id) } }, fetcher);
}
