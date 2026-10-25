// The needs-you list: what each entry says, what it names, and the actions the app can take on it. Entries the app
// has no command for say so and give the CLI's hint. Pure, so every kind is tested without a browser.

import type { Command } from "./api";
import type { Entity } from "./forms";
import {
  approveCommand,
  dropCommand,
  dropMemberCommand,
  keepCommand,
  keepMemberCommand,
  resolveCommand,
  unsettledWhy,
} from "./forms";
import type { Line } from "./lines";

/** What the app says of what only a terminal settles; there, `acyclic needs` names the command. */
const IN_A_TERMINAL = "Settle this one in a terminal: acyclic needs says how";

/** An entry as `acyclic needs` reports it: a kind, and fields that depend on it. */
export type Need = { kind: string } & Record<string, unknown>;

export interface Action {
  label: string;
  command: Command;
  /** Names the action for tests: `keep`, `drop`, `approve` or `use`. */
  action: string;
}

export interface NeedView {
  /** A stable key for the entry: its kind and what it names. */
  key: string;
  text: string;
  /** The line of the one thread its text names, for its badge, which carries the number the text leaves out. */
  line: Line | undefined;
  /** What it names, to select: items by id, and threads with the title the entry gives, for one the list lacks. */
  items: string[];
  threads: { id: string; title: string }[];
  /** The thread the badge-led text names, which the text picks: its row would only repeat it. */
  thread: string | null;
  actions: Action[];
  /** Why there is no action here, or how to settle it elsewhere. */
  note: string | null;
}

interface Row {
  id: string;
  title?: unknown;
}

const row = (value: unknown): Row | null =>
  typeof value === "object" && value !== null && typeof (value as Row).id === "string" ? (value as Row) : null;
const rows = (value: unknown): Row[] => (Array.isArray(value) ? value.map(row).filter((one) => one !== null) : []);
const titled = (one: Row) => (typeof one.title === "string" ? one.title : one.id);
const str = (value: unknown) => (typeof value === "string" ? value : "");

/** A thread's row under an entry: its line, or, for a thread the list lacks, the title the entry gives. */
export function threadLabel(thread: { id: string; title: string }, lines: ReadonlyMap<string, Line>) {
  const line = lines.get(thread.id);
  return { line, text: line?.name ?? thread.title };
}

/** One entry as the app shows it, with each thread's line from `lines`. */
export function needView(need: Need, lines: ReadonlyMap<string, Line>): NeedView {
  // A badge only where the text names its thread, and then the text picks the thread in place of its row.
  let named = false;
  const view = viewOf(need, lines, () => {
    named = true;
  });
  if (!named) return { ...view, line: undefined, thread: null };
  const thread = view.threads[0]?.id ?? null;
  return { ...view, thread, threads: view.threads.filter((one) => one.id !== thread) };
}

/** The entry, calling `named` when its text names the badge's thread by its line. */
function viewOf(need: Need, lines: ReadonlyMap<string, Line>, named: () => void): Omit<NeedView, "thread"> {
  const item = row(need.item);
  const thread = row(need.thread);
  const holders = Array.isArray(need.holders) ? need.holders : [];
  const threads =
    need.kind === "resource-clash"
      ? holders.map((one) => row((one as { thread?: unknown }).thread)).filter((one) => one !== null)
      : thread === null
        ? []
        : [thread];
  // A badge only for an entry naming one thread, and a thread named by its line's name only beside that badge.
  const line = threads.length === 1 ? lines.get(threads[0]?.id ?? "") : undefined;
  const lineName = (one: Row) => {
    if (line === undefined || one.id !== threads[0]?.id) return titled(one);
    named();
    return line.name;
  };
  const base = {
    line,
    items: item === null ? [] : [item.id],
    threads: threads.map((one) => ({ id: one.id, title: titled(one) })),
    actions: [] as Action[],
    note: null as string | null,
  };
  switch (need.kind) {
    case "proposal": {
      const proposal = need.proposal as { id?: string; actor?: string; edit?: Record<string, unknown> } | null;
      const id = proposal?.id ?? "";
      const edit = proposal?.edit ?? {};
      return {
        ...base,
        key: `proposal:${id}`,
        text: `${proposal?.actor ?? "An agent"} proposes ${str(edit.field)} ${shown(edit.before)} → ${shown(edit.after)}${item === null ? "" : ` for ${titled(item)}`}`,
        actions: [
          { label: "Keep", command: keepCommand(id), action: "keep" },
          { label: "Drop", command: dropCommand(id), action: "drop" },
        ],
      };
    }
    case "unapproved": {
      const id = item?.id ?? "";
      const to = thread?.id ?? "";
      return {
        ...base,
        key: `unapproved:${id}:${to}`,
        text: `${item === null ? id : titled(item)} joined ${thread === null ? to : lineName(thread)} after its approval`,
        actions: [
          { label: "Keep", command: keepMemberCommand(id, to), action: "keep" },
          { label: "Drop", command: dropMemberCommand(id), action: "drop" },
        ],
      };
    }
    case "awaiting-approval": {
      const id = thread?.id ?? "";
      // All its members, as the thread's panel counts them: approval covers the thread's whole scope.
      const members = Number((need.thread as { counts?: { members?: number } } | undefined)?.counts?.members ?? 0);
      const heads = Array.isArray((need.thread as { heads?: unknown })?.heads)
        ? ((need.thread as { heads: string[] }).heads ?? [])
        : [];
      return {
        ...base,
        key: `awaiting-approval:${id}`,
        items: heads,
        text: `${thread === null ? id : lineName(thread)} waits for your approval`,
        actions: [
          {
            label: `Approve ${members} ${members === 1 ? "item" : "items"}`,
            command: approveCommand(id),
            action: "approve",
          },
        ],
      };
    }
    case "conflict": {
      const entity = str(need.entity);
      const id = str(need.id);
      const field = str(need.field);
      const values = Array.isArray(need.values) ? need.values : [];
      const known = entity === "item" || entity === "thread" || entity === "settings";
      const actions: Action[] = [];
      const why: string[] = [];
      for (const value of values) {
        const command = known ? resolveCommand(entity as Entity, id, field, value) : null;
        if (command !== null) actions.push({ label: `Use ${shown(value)}`, command, action: "use" });
        else
          why.push(
            `${shown(value)}: ${known ? unsettledWhy(entity as Entity, field, value) : "settled in a terminal"}`,
          );
      }
      return {
        ...base,
        key: `conflict:${entity}:${id}:${field}`,
        text: `${entity} ${id}: ${field} was written differently on two machines`,
        actions,
        note: why.length === 0 ? null : [...why, str(need.resolve)].filter((one) => one !== "").join("; "),
      };
    }
    case "question": {
      const id = str(need.id);
      return {
        ...base,
        key: `question:${id}`,
        text: str(need.text) || "An agent asks a question",
        note: "The app can't answer questions yet",
      };
    }
    case "new-blocker": {
      const blocker = row(need.blocker);
      return {
        ...base,
        items: [...base.items, ...(blocker === null ? [] : [blocker.id])],
        key: `new-blocker:${item?.id ?? ""}:${blocker?.id ?? ""}`,
        text: `${item === null ? "An item" : titled(item)} gained a new blocker since its approval: ${blocker === null ? "?" : titled(blocker)}`,
      };
    }
    case "resource-clash": {
      return {
        ...base,
        key: `resource-clash:${str(need.resource)}`,
        text: `Threads held by different holders need ${str(need.resource)}: ${threads.map(lineName).join(", ")}`,
        note: IN_A_TERMINAL,
      };
    }
    case "dangling":
      return {
        ...base,
        key: `dangling:${str(need.entity)}:${str(need.id)}:${str(need.field)}`,
        text: `${str(need.entity)} ${str(need.id)}: ${str(need.field)} names ${shown(need.target)}, which doesn't exist`,
        note: IN_A_TERMINAL,
      };
    case "unverified":
      return {
        ...base,
        key: `unverified:${item?.id ?? ""}:${str(need.repository)}`,
        text: `Can't check ${item === null ? "an item" : titled(item)}'s merge in ${str(need.repository)}: ${str(need.reason)}`,
        note: IN_A_TERMINAL,
      };
    default: {
      const named = [...rows(need.items), ...rows(need.dependents), ...rows(need.open)];
      const hint = str(need.fix) || str(need.resolve);
      const ids = [
        str(need.id),
        ...base.items,
        ...base.threads.map((one) => one.id),
        ...named.map((one) => one.id),
        str(need.remote),
      ];
      return {
        ...base,
        items: [...base.items, ...named.map((one) => one.id)],
        key: `${need.kind}:${ids.filter((one) => one !== "").join(":")}`,
        text: describe(need.kind, item, thread, named, lineName),
        note: hint === "" ? IN_A_TERMINAL : hint,
      };
    }
  }
}

function shown(value: unknown): string {
  if (value === null || value === undefined) return "none";
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** A line for a kind the app only shows. */
function describe(
  kind: string,
  item: Row | null,
  thread: Row | null,
  named: Row[],
  name: (thread: Row) => string,
): string {
  const subject = item !== null ? titled(item) : thread !== null ? name(thread) : named.map(titled).join(", ");
  const words: Record<string, string> = {
    unpushed: "This clone holds writes the remote lacks",
    "link-cycle": "Links form a cycle",
    "parent-cycle": "Parents form a cycle",
    contest: "Two holders claim one thread",
    "clock-ahead": "A claim's clock runs ahead",
    "unknown-run": "A run nothing knows of claims a thread",
    unthreaded: "In no thread",
    "run-clash": "Two runs work one thread",
    "new-blocker": "Gained a new blocker",
    reopened: "Reopened, with work waiting on it",
    "done-early": "Done while work in it is open",
  };
  const said = words[kind] ?? kind;
  return subject === "" ? said : `${said}: ${subject}`;
}
