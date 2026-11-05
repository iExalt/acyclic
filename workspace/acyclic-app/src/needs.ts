// The needs-you list: what each entry says, what it names, and the actions the app can take on it. Entries the app
// has no command for say so and give the CLI's hint. Pure, so every kind is tested without a browser.

import { type Command, titleOr } from "./api";
import type { Line } from "./lines";

/** What the app says of what only a terminal settles; there, `acyclic needs` names the command. */
const IN_A_TERMINAL = "Settle this one in a terminal: run acyclic needs to see how";
const EDIT_THREAD = "Open it to choose its thread in its editor";

/** An entry as `acyclic needs` reports it: a kind, and fields that depend on it. */
export type Need = { kind: string; identity?: string } & Record<string, unknown>;

/** A value of a conflict and how it is kept, as the needs-you list gives it: the command, or why there is none. */
export interface Kept {
  value: unknown;
  command: Command | null;
  run: string | null;
  why: string | null;
}

/** The values a conflict entry holds, each with how it is kept. */
export function keepsIn(need: Need | undefined): Kept[] {
  return Array.isArray(need?.keeps) ? (need.keeps as Kept[]) : [];
}

/** The conflict entry for `field` of a record, by the key the list gives it. */
export function conflictOf(needs: readonly Need[], entity: string, id: string, field: string): Need | undefined {
  const key = `conflict:${entity}:${id}:${field}`;
  return needs.find((need) => need.key === key);
}

export interface Action {
  label: string;
  command: Command;
  /** Names the action for tests: `use`. */
  action: string;
  /** What the app says once it is saved, since the entry it settles goes. */
  done: string;
}

export interface NeedView {
  /** The entry's stable key, as the needs-you list gives it: its kind and what it is about. */
  key: string;
  /** The source kind, kept as data so views can adapt without parsing text or keys. */
  kind: string;
  text: string;
  /** What it names besides its text, for a list that shows the entry without its rows, such as an item's title. */
  detail: string | null;
  /** The line of the one thread its text names, for its badge, which the text leaves out. */
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

/** Whether the phone can collapse this terminal-only entry without hiding a decision or a single record. */
export function isCollapsible(view: NeedView): boolean {
  if (view.actions.length !== 0) return false;
  return view.kind === "thread-cycle" && view.threads.length > 1;
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

/** A count as the Needs you button shows it: exact to 99, then 99+. */
export function countText(count: number): string {
  return count > 99 ? "99+" : String(count);
}

/** A thread's row under an entry: its line, or, for a thread the list lacks, the title the entry gives. */
export function threadLabel(thread: { id: string; title: string }, lines: ReadonlyMap<string, Line>) {
  const line = lines.get(thread.id);
  return { line, text: line?.name ?? thread.title };
}

/** One entry as the app shows it, with each thread's line from `lines`. */
export function needView(
  need: Need,
  lines: ReadonlyMap<string, Line>,
  titleOf: (id: string) => string = (id) => id,
): NeedView {
  // A badge only where the text names its thread, and then the text picks the thread in place of its row.
  let named = false;
  const view = viewOf(need, lines, titleOf, () => {
    named = true;
  });
  if (!named) return { ...view, line: undefined, thread: null };
  const thread = view.threads[0]?.id ?? null;
  return { ...view, thread, threads: view.threads.filter((one) => one.id !== thread) };
}

/**
 * The entry, calling `named` when its text names the badge's thread by its line. Ids the entry gives bare, such as a
 * conflict's values, are named by the thread's line or `titleOf` the item.
 */
function viewOf(
  need: Need,
  lines: ReadonlyMap<string, Line>,
  titleOf: (id: string) => string,
  named: () => void,
): Omit<NeedView, "thread"> {
  const item = row(need.item);
  const thread = row(need.thread);
  const threads =
    need.kind === "thread-cycle"
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
    entity === "item" ? titleOf(id) : entity === "thread" ? threadName(id) : id;
  const base = {
    // The list's own key, which the server's pushes use too.
    key: str(need.key),
    kind: need.kind,
    line,
    items: item === null ? [] : [item.id],
    threads: threads.map((one) => ({ id: one.id, title: titled(one) })),
    actions: [] as Action[],
    note: null as string | null,
    detail: null as string | null,
  };
  switch (need.kind) {
    case "conflict": {
      const entity = str(need.entity);
      const id = str(need.id);
      const field = str(need.field);
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
      for (const kept of keepsIn(need)) {
        if (kept.command !== null)
          actions.push({
            label: `Use ${said(kept.value)}`,
            command: kept.command,
            action: "use",
            done: `Used ${said(kept.value)} for ${field}`,
          });
        else why.push(`${said(kept.value)}: ${kept.why ?? IN_A_TERMINAL}`);
      }
      return {
        ...base,
        text: `${nameOf(entity, id)}: its ${field} was written differently on two machines`,
        actions,
        note: why.length === 0 ? null : why.join("; "),
      };
    }
    case "thread-cycle":
      return {
        ...base,
        text: `Threads wait on each other in a cycle: ${threads.map(lineName).join(", ")}`,
        note: IN_A_TERMINAL,
      };
    case "dangling": {
      const name = nameOf(str(need.entity), str(need.id));
      // A waits-for naming an item that is gone reads as the plan does, with the CLI's own fix.
      if (str(need.entity) === "item" && str(need.field) === "waits-for")
        return {
          ...base,
          text: `${name} waits for ${shown(need.target)}, which doesn't exist`,
          note: str(need.fix) || IN_A_TERMINAL,
        };
      return {
        ...base,
        text: `${name}: its ${str(need.field)} names ${shown(need.target)}, which doesn't exist`,
        // An item in a thread that is gone is in none, which its editor puts right.
        note: str(need.entity) === "item" && str(need.field) === "thread" ? EDIT_THREAD : IN_A_TERMINAL,
      };
    }
    case "unthreaded": {
      return {
        ...base,
        text: "Needs a thread",
        detail: item === null ? null : titled(item),
        note: EDIT_THREAD,
      };
    }
    default: {
      const named = [...rows(need.items), ...rows(need.dependents), ...rows(need.open)];
      const hint = str(need.fix);
      return {
        ...base,
        items: [...base.items, ...named.map((one) => one.id)],
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

/** A JSON value's key with object fields sorted and array order kept. */
export function structuralKey(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(structuralKey).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const fields = Object.entries(value as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${fields.map(([key, child]) => `${JSON.stringify(key)}:${structuralKey(child)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? String(value);
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
    unthreaded: "Needs a thread",
    reopened: "Reopened, with work waiting on it",
    "done-early": "Done while work in it is open",
  };
  const said = words[kind] ?? kind;
  return subject === "" ? said : `${said}: ${subject}`;
}
