// The needs-you list: what each entry says, what it names, and the actions the app can take on it. Entries the app
// has no command for say so and give the CLI's hint. Pure, so every kind is tested without a browser.

import { type Command, titleOr } from "./api";
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

/** The most items an approval lists before it says how many more. */
export const LISTED = 5;

/** What the app says of what only a terminal settles; there, `acyclic needs` names the command. */
const IN_A_TERMINAL = "Settle this one in a terminal: run acyclic needs to see how";

/** An entry as `acyclic needs` reports it: a kind, and fields that depend on it. */
export type Need = { kind: string } & Record<string, unknown>;

export interface Action {
  label: string;
  command: Command;
  /** Names the action for tests: `keep`, `drop`, `approve` or `use`. */
  action: string;
  /** What the app says once it is saved, since the entry it settles goes. */
  done: string;
  /** Offered back, as a first approval is, while no agent has started its work. */
  undo?: true;
}

export interface NeedView {
  /** A stable key for the entry: its kind and what it names. */
  key: string;
  text: string;
  /**
   * What it asks, where the text is a name alone, for a list that shows the entry without its actions: an approval's
   * text names its thread, and this says it waits and for how many items, the count its Approve button gives.
   */
  detail: string | null;
  /** The line of the one thread its text names, for its badge, which carries the number the text leaves out. */
  line: Line | undefined;
  /** What it names, to select: items by id, and threads with the title the entry gives, for one the list lacks. */
  items: string[];
  /** How many more items it covers than it lists, as an approval of a large thread lists only its first few. */
  more: number;
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
const titled = (one: Row) => titleOr(one.title, one.id);
const str = (value: unknown) => (typeof value === "string" ? value : "");

/** What the user does about an entry, by which the needs-you list filters: say yes or no, answer, or resolve. */
export type NeedGroup = "approve" | "answer" | "resolve";

/** Each group's filter, named as a noun: "Approve 20" over a list of Approve buttons read as one more of them. */
export const GROUPS: { group: NeedGroup; label: string }[] = [
  { group: "approve", label: "Approvals" },
  { group: "answer", label: "Questions" },
  { group: "resolve", label: "Problems" },
];

/**
 * An entry's group: scope waiting for a yes or no, a blocker approving the thread again acknowledges, and an agent's
 * question; every other kind, including one the app doesn't know yet, is something in the way to settle.
 */
export function groupOf(kind: string): NeedGroup {
  if (["proposal", "unapproved", "awaiting-approval", "new-blocker"].includes(kind)) return "approve";
  return kind === "question" ? "answer" : "resolve";
}

/** The threads Needs you asks the user to approve. */
export function awaitingApproval(needs: readonly Need[]): Set<string> {
  const ids = needs
    .filter((need) => need.kind === "awaiting-approval")
    .map((need) => row(need.thread)?.id)
    .filter((id) => id !== undefined);
  return new Set(ids);
}

/** A count as the Needs you button shows it: exact to 99, then 99+. */
export function countText(count: number): string {
  return count > 99 ? "99+" : String(count);
}

/** A thread's row under an entry: its line, or, for a thread the list lacks, the title the entry gives. */
export function threadLabel(thread: { id: string; title: string }, lines: ReadonlyMap<string, Line>) {
  const line = lines.get(thread.id);
  return { line, text: line?.name ?? thread.title };
}

/**
 * One entry as the app shows it, with each thread's line from `lines`, and a thread's items from `members`, in the
 * order its work runs, where the plan holds them.
 */
export function needView(
  need: Need,
  lines: ReadonlyMap<string, Line>,
  members: (thread: string) => string[] = () => [],
  titleOf: (id: string) => string = (id) => id,
): NeedView {
  // A badge only where the text names its thread, and then the text picks the thread in place of its row.
  let named = false;
  const view = viewOf(need, lines, members, titleOf, () => {
    named = true;
  });
  if (!named) return { ...view, line: undefined, thread: null };
  const thread = view.threads[0]?.id ?? null;
  return { ...view, thread, threads: view.threads.filter((one) => one.id !== thread) };
}

/**
 * The entry, calling `named` when its text names the badge's thread by its line. Ids the entry gives bare, such as a
 * proposal's threads or a conflict's values, are named by the thread's line or `titleOf` the item.
 */
function viewOf(
  need: Need,
  lines: ReadonlyMap<string, Line>,
  members: (thread: string) => string[],
  titleOf: (id: string) => string,
  named: () => void,
): Omit<NeedView, "thread"> {
  const item = row(need.item);
  const thread = row(need.thread);
  const holders = Array.isArray(need.holders) ? need.holders : [];
  const threads =
    need.kind === "resource-clash"
      ? holders.map((one) => row((one as { thread?: unknown }).thread)).filter((one) => one !== null)
      : need.kind === "number-clash"
        ? (Array.isArray(need.threads) ? need.threads : []).map(row).filter((one) => one !== null)
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
  const threadName = (id: string) => lines.get(id)?.name ?? id;
  /** An entity the entry names by id, by its name. */
  const nameOf = (entity: string, id: string) =>
    entity === "item" ? titleOf(id) : entity === "thread" ? threadName(id) : entity === "settings" ? "Settings" : id;
  const base = {
    line,
    more: 0,
    items: item === null ? [] : [item.id],
    threads: threads.map((one) => ({ id: one.id, title: titled(one) })),
    actions: [] as Action[],
    note: null as string | null,
    detail: null as string | null,
  };
  switch (need.kind) {
    case "proposal": {
      const proposal = need.proposal as { id?: string; actor?: string; edit?: Record<string, unknown> } | null;
      const id = proposal?.id ?? "";
      const edit = proposal?.edit ?? {};
      const change = `${proposal?.actor ?? "an agent"}'s ${str(edit.field)} change`;
      // A member's thread by its name, or no thread; a repository list joined, or none.
      const value = (one: unknown) =>
        edit.field === "thread"
          ? typeof one === "string"
            ? threadName(one)
            : "no thread"
          : Array.isArray(one)
            ? one.map(shown).join(", ") || "none"
            : shown(one);
      const subject =
        item !== null
          ? titled(item)
          : thread !== null
            ? lineName(thread)
            : typeof edit.thread === "string"
              ? threadName(edit.thread)
              : null;
      return {
        ...base,
        key: `proposal:${id}`,
        text: `${proposal?.actor ?? "An agent"} proposes ${str(edit.field)}: ${value(edit.before)} → ${value(edit.after)}${subject === null ? "" : `, for ${subject}`}`,
        actions: [
          { label: "Keep", command: keepCommand(id), action: "keep", done: `Kept ${change}` },
          { label: "Drop", command: dropCommand(id), action: "drop", done: `Dropped ${change}` },
        ],
      };
    }
    case "unapproved": {
      const id = item?.id ?? "";
      const to = thread?.id ?? "";
      const [title, name] = [item === null ? id : titled(item), thread === null ? to : lineName(thread)];
      return {
        ...base,
        key: `unapproved:${id}:${to}`,
        text: `${title} joined ${name} after its approval`,
        actions: [
          { label: "Keep", command: keepMemberCommand(id, to), action: "keep", done: `Kept ${title} in ${name}` },
          { label: "Drop", command: dropMemberCommand(id), action: "drop", done: `Dropped ${title} from ${name}` },
        ],
      };
    }
    case "awaiting-approval": {
      const id = thread?.id ?? "";
      // All its members, as the thread's panel counts them: approval covers the thread's whole scope, so the entry
      // lists the items it covers, as the plan holds them (only where the plan has none, the heads it names), the
      // first few, and says how many more.
      const count = Number((need.thread as { counts?: { members?: number } } | undefined)?.counts?.members ?? 0);
      const heads = Array.isArray((need.thread as { heads?: unknown })?.heads)
        ? ((need.thread as { heads: string[] }).heads ?? [])
        : [];
      const known = members(id);
      const listed = (known.length > 0 ? known : heads).slice(0, LISTED);
      const name = thread === null ? id : lineName(thread);
      const items = `${count} ${count === 1 ? "item" : "items"}`;
      return {
        ...base,
        key: `awaiting-approval:${id}`,
        items: listed,
        more: Math.max(0, count - listed.length),
        // The name alone: every entry here waits for approval, which the list's filter and the button say, and a
        // sentence on each row read as noise.
        text: name,
        detail: `${items} to approve`,
        actions: [
          {
            label: `Approve ${items}`,
            command: approveCommand(id),
            action: "approve",
            done: `Approved ${name}: ${items}`,
            undo: true,
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
      // A value naming an item or a thread by its name, as the command still sends the id.
      const said = (value: unknown) =>
        typeof value !== "string"
          ? field === "parent" && value === null
            ? "the top level"
            : field === "thread" && value === null
              ? "no thread"
              : shown(value)
          : field === "parent"
            ? titleOf(value)
            : field === "thread"
              ? threadName(value)
              : value;
      for (const value of values) {
        const command = known ? resolveCommand(entity as Entity, id, field, value) : null;
        if (command !== null)
          actions.push({
            label: `Use ${said(value)}`,
            command,
            action: "use",
            done: `Used ${said(value)} for ${field}`,
          });
        else
          why.push(`${said(value)}: ${known ? unsettledWhy(entity as Entity, field, value) : "settled in a terminal"}`);
      }
      return {
        ...base,
        key: `conflict:${entity}:${id}:${field}`,
        text: `${nameOf(entity, id)}: its ${field} was written differently on two machines`,
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
      // Approving the thread again acknowledges what its approval didn't see.
      // Its row names the thread under the action, so the name here mustn't move it to a badge the text never names.
      const name = thread === null ? null : threadName(thread.id);
      return {
        ...base,
        items: [...base.items, ...(blocker === null ? [] : [blocker.id])],
        key: `new-blocker:${item?.id ?? ""}:${blocker?.id ?? ""}`,
        text: `${item === null ? "An item" : titled(item)} gained a new blocker since its approval: ${blocker === null ? "?" : titled(blocker)}`,
        actions:
          thread === null
            ? []
            : [
                {
                  label: "Approve again",
                  command: approveCommand(thread.id),
                  action: "approve",
                  done: `Approved ${name} again`,
                },
              ],
        note: thread === null ? IN_A_TERMINAL : null,
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
    case "number-clash":
      return {
        ...base,
        key: `number-clash:${shown(need.number)}`,
        text: `Threads two machines numbered alike share ${shown(need.number)}: ${threads.map(lineName).join(", ")}`,
        note: str(need.fix) || IN_A_TERMINAL,
      };
    case "dangling":
      return {
        ...base,
        key: `dangling:${str(need.entity)}:${str(need.id)}:${str(need.field)}`,
        text: `${nameOf(str(need.entity), str(need.id))}: its ${str(need.field)} names ${shown(need.target)}, which doesn't exist`,
        note: IN_A_TERMINAL,
      };
    case "unverified": {
      const repository = str(need.repository);
      const reason = str(need.reason);
      // The item is its row, so the text doesn't name it twice; the Overview, which shows no rows, gives it as detail.
      return {
        ...base,
        key: `unverified:${item?.id ?? ""}:${repository}`,
        text: `Can't confirm ${item === null ? "an item" : "its"} merge into ${repository}`,
        detail: item === null ? null : titled(item),
        note: unverifiedWhy(repository, reason),
      };
    }
    case "unthreaded": {
      return {
        ...base,
        key: `unthreaded:${item?.id ?? ""}`,
        text: "Needs a thread",
        detail: item === null ? null : titled(item),
        note: "Open it to choose its thread in its editor",
      };
    }
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

/**
 * Why a done item's merge couldn't be confirmed, and what fixes it: acyclic looks for the item's commit on the
 * repository's default branch, in this machine's clone of it.
 */
function unverifiedWhy(repository: string, reason: string): string {
  if (reason === "no clone of the repository is mapped")
    return `This machine has no clone of ${repository} to look for its commit in: add the clone's path to ~/.config/acyclic/clones.toml`;
  if (reason === "not a commit id") return "Its merge evidence isn't a commit id: record the merged commit on the item";
  return `Looking for its commit in this machine's clone of ${repository} failed (${reason}): fetch in that clone, or check its path in ~/.config/acyclic/clones.toml`;
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
    unthreaded: "Needs a thread",
    "run-clash": "Two runs work one thread",
    "new-blocker": "Gained a new blocker",
    reopened: "Reopened, with work waiting on it",
    "done-early": "Done while work in it is open",
  };
  const said = words[kind] ?? kind;
  return subject === "" ? said : `${said}: ${subject}`;
}
