// The app's tracker: the plan as last read, the write out and the one notice slot, with the Undo on
// offer kept in the tab over a reload. Writes go one at a time; reads are numbered, so one overtaken is never shown;
// what a write does to where the user is goes through `nav`. Plain TypeScript with its ports passed in, so every
// outcome of a write, a read and an undo is tested without a browser.

import {
  type Command,
  command,
  type Expected,
  type Fetch,
  failure,
  type Initiative,
  type Item,
  initiatives,
  items,
  needs,
  type Threads,
  threads,
} from "./api";
import { landed, type Pending, redraft, type Sent, told, unanswered } from "./edits";
import type { Intent } from "./nav";
import type { Need } from "./needs";
import { type NoticeEvent, noticeAfter, type Raised } from "./notices";
import { answered, NO_READS, type Reads, show, start, type Unread } from "./reads";
import { type Kept, keep, type Plan, type Undo, undoOf, unkeep } from "./undo";

export type Load =
  | { state: "loading" }
  | {
      state: "loaded";
      items: Item[];
      threads: Threads;
      needs: Need[];
      /** The initiatives, and the ids of the items in none. */
      initiatives: Initiative[];
      noInitiative: string[];
      unread: Unread[];
    }
  | { state: "failed"; code: string; message: string };

/** What an edit selects afterwards, and which box holds still on screen while the Map redraws. */
export interface After {
  /** The item to select and show, or null to leave the selection as it is, as a thread's edits do. */
  select: string | null;
  pin: string | null;
  /** Offer to undo it, once saved: for the user's own moves, links and field edits; never a decision. */
  undo?: true;
  /** Said once saved, for an undo: what it did. */
  undone?: string;
  /** Said once saved, for a write that takes away what it was done from, as settling a need does. */
  done?: string;
}

/** What the app tells the user, which `notices.ts` says how long to keep and where to show. */
export interface Notice extends Raised {
  text: string;
  offer: "retry" | "refresh" | "undo" | null;
  /** The write again, offered when the server was only busy. */
  retry: { body: Command; after: After; expected?: readonly Expected[]; observed?: readonly string[] } | null;
  /** Said of a link waiting for a form to close: dismissing it drops the link. */
  held?: true;
  /** A link to copy by hand, where the browser offers no clipboard. */
  link?: string;
  /** What Undo takes back, with the offer "undo". */
  undo?: Undo;
  /** That Undo as it is kept over a reload. */
  kept?: Kept;
  /** Brought back by a reload, and not yet checked against the plan read since. */
  restored?: true;
  /** News, not a failure: told politely, as a status. */
  polite?: true;
}

export const UNDOING: Notice = { text: "Undoing…", offer: null, retry: null, lasts: "offer", at: null };

/** Where the Undo on offer is kept in this tab over a reload. */
export const UNDO_KEY = "acyclic.undo";

/** Where the last plan shown is kept on this device, so the next start has one to show while the first read is out. */
export const PLAN_KEY = "acyclic.plan";

/** The shape the plan is kept in: bumped whenever `Load`, or what any item or thread holds, changes. */
export const PLAN_SCHEMA = 1;

/** The server's refusals of a user it doesn't serve: whoever they are, the plan kept from before isn't theirs to see. */
const NOT_THE_USER = ["no-identity", "wrong-identity"];

export interface Ports {
  fetcher: Fetch;
  /** The tab's storage, or null where there is none. Any call may throw, as a full or blocked one does. */
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> | null;
  /** Where the last plan shown is kept over the app's closing, or null where there is none; as `storage`, it may throw. */
  planStorage: Pick<Storage, "getItem" | "setItem" | "removeItem"> | null;
  nav: {
    go(intent: Intent): void;
    /** The item selected now, which an undo that moves nothing brings back into view. */
    item(): string | null;
  };
}

export interface Snapshot {
  load: Load;
  notice: Notice | null;
  /** A write is out: what sends one is held. */
  busy: boolean;
  /** The plan shown is the one kept from before, until the first read of this start lands: read-only, as `busy` is. */
  stale: boolean;
  /** A write drawn before the server answers, until the plan read after it holds it. */
  pending: Pending | null;
  /** The latest read shown, as the server had it: what an undo's guard is checked against. */
  plan: Plan | null;
  /** Reads shown, so a browser journey knows a refresh has landed. */
  reads: number;
  /** Counts writes and refreshes, so panels that read more than the plan read again after each. */
  version: number;
}

export interface Tracker {
  /** The same object until something in it changes. */
  snapshot(): Snapshot;
  subscribe(listener: () => void): () => void;
  /** Reads the plan again: a first read that fails fails the page; a later one keeps the plan and says it is stale. */
  refresh(): Promise<void>;
  /**
   * Sends an edit; once saved, reloads the plan and brings `after.select` into view with `after.pin` held still. An
   * edit `drawn` is drawn at once, its form closed, and if the write fails, the form opens again with what the user
   * wrote, unless the server turns out to hold it after all. Whether it was saved; false for one sent while another
   * is out.
   */
  send(body: Command, after: After, drawn?: Pending | null): Promise<boolean>;
  /** Sends again the write the notice offers to retry. */
  retry(): Promise<boolean>;
  /**
   * Takes back the write the Undo on offer came from: reads the plan first, so the guard is checked against the
   * server's latest, and sends nothing when what the write left has changed since. Once at a time.
   */
  undo(): Promise<void>;
  /** Shows `notice` in the slot, or empties it: whatever takes an Undo's place drops the one kept. */
  tell(notice: Notice | null): void;
  dismiss(): void;
  event(event: NoticeEvent): void;
}

const isObject = (one: unknown): one is Record<string, unknown> => typeof one === "object" && one !== null;
const isString = (one: unknown): one is string => typeof one === "string";
const isList = (one: unknown, each: (one: unknown) => boolean) => Array.isArray(one) && one.every(each);
const orNull = (one: unknown) => one === null || isString(one);

/** Whether a kept item has what drawing it reads unconditionally: one from an older build may not. */
function isItem(one: unknown): boolean {
  if (!isObject(one) || !isObject(one.links) || !isObject(one.conflicts)) return false;
  return (
    isString(one.id) &&
    isString(one.title) &&
    isString(one.state) &&
    orNull(one.parent) &&
    orNull(one.thread) &&
    orNull(one.notes) &&
    orNull(one.initiative) &&
    isList(one.links.waits_for, isString) &&
    isList(one.links.waited_on_by, isString)
  );
}

function isThread(one: unknown): boolean {
  return (
    isObject(one) && isString(one.id) && isString(one.title) && isList(one.waitsFor, isString) && isObject(one.counts)
  );
}

const planOf = (found: Item[]): Plan => ({ items: new Map(found.map((one) => [one.id, one])) });

export function createTracker({ fetcher, storage, planStorage, nav }: Ports): Tracker {
  const listeners = new Set<() => void>();
  let reads: Reads = NO_READS;
  /** Set from a write's start to its end, so a second can't start before a render disables what sends it. */
  let sending = false;
  let undoing = false;
  const cached = cachedPlan();
  let snap: Snapshot = {
    load: cached ?? { state: "loading" },
    notice: restore(),
    busy: false,
    stale: cached !== null,
    pending: null,
    plan: null,
    reads: 0,
    version: 0,
  };
  /** A restored Undo a failed read put aside, until a read says whether it still holds. */
  let shelved: Notice | null = null;
  /** The Undo stored, written once each rather than as the notice moves with the user, as a large plan is slow. */
  let kept = snap.notice?.kept;

  /** The plan the last start showed, or null when there is none, or it isn't whole and of this schema. */
  function cachedPlan(): Extract<Load, { state: "loaded" }> | null {
    try {
      const raw = planStorage?.getItem(PLAN_KEY) ?? null;
      const was = raw === null ? null : JSON.parse(raw);
      if (!isObject(was) || was.v !== PLAN_SCHEMA || !isObject(was.plan)) return null;
      const { items, threads, needs, initiatives, noInitiative } = was.plan;
      if (!isObject(threads) || !Array.isArray(threads.threads)) return null;
      const whole =
        Array.isArray(items) &&
        items.every(isItem) &&
        threads.threads.every(isThread) &&
        isList(threads.waves, Array.isArray) &&
        isList(threads.anytime, isString) &&
        isList(needs, (one) => isObject(one) && isString(one.kind)) &&
        isList(initiatives, (one) => isObject(one) && isString(one.id) && isString(one.title)) &&
        isList(noInitiative, isString);
      if (!whole) return null;
      // Checked as far as drawing reads them above.
      return {
        state: "loaded",
        unread: [],
        ...(was.plan as Omit<Extract<Load, { state: "loaded" }>, "state" | "unread">),
      };
    } catch {
      return null;
    }
  }

  /** The Undo a reload left on offer in this tab, offered again once a read says it still holds; or none. */
  function restore(): Notice | null {
    try {
      const raw = storage?.getItem(UNDO_KEY) ?? null;
      const was: Kept | null = raw === null ? null : JSON.parse(raw);
      const undo = was === null ? null : unkeep(was);
      if (was === null || undo === null) return null;
      return { text: was.text, offer: "undo", retry: null, undo, kept: was, restored: true, lasts: "offer", at: null };
    } catch {
      return null;
    }
  }

  // Kept while it is on offer, and dropped once anything else takes its place; "Undoing…" passes it on to what follows.
  function persist(notice: Notice | null) {
    const next = notice?.kept;
    // A restored one put aside by a read that failed is offered again once a read confirms it.
    if (next === undefined && shelved !== null && notice?.offer === "refresh") return;
    if (next === kept || (next === undefined && notice === UNDOING)) return;
    kept = next;
    try {
      if (next !== undefined) storage?.setItem(UNDO_KEY, JSON.stringify(next));
      else storage?.removeItem(UNDO_KEY);
    } catch {
      // Too big a plan for the tab's storage: the Undo is offered until a reload, as without it.
      try {
        storage?.removeItem(UNDO_KEY);
      } catch {
        // No storage at all: nothing kept.
      }
    }
  }

  function set(change: Partial<Snapshot>) {
    const keys = Object.keys(change) as (keyof Snapshot)[];
    if (keys.every((key) => change[key] === snap[key])) return;
    snap = { ...snap, ...change };
    if ("notice" in change) persist(snap.notice);
    for (const listener of listeners) listener();
  }
  const tell = (notice: Notice | null) => set({ notice });
  const bump = () => set({ version: snap.version + 1 });

  /** Reads the plan, or null when a read started later has been shown already. */
  async function fetchAll() {
    reads = start(reads);
    const n = reads.started;
    const [found, listed, needed, tops] = await Promise.all([
      items(fetcher),
      threads(fetcher),
      needs(fetcher),
      initiatives(fetcher),
    ]).catch((err: unknown) => {
      if (NOT_THE_USER.includes(failure(err).code)) forget();
      throw err;
    });
    const next = show(reads, n, found);
    if (next === null) return null;
    reads = next;
    return {
      state: "loaded" as const,
      items: found,
      threads: listed,
      needs: needed,
      initiatives: tops.initiatives,
      noInitiative: tops.noInitiative,
      unread: next.unread,
    };
  }
  /** Drops the plan kept, as a server that won't serve this user says it isn't theirs. */
  function forget() {
    try {
      planStorage?.removeItem(PLAN_KEY);
    } catch {
      // Nothing kept that can be reached.
    }
  }
  /** Shows a read: the plan, and the latest read the server gave. */
  const shown = (next: Extract<Load, { state: "loaded" }>) => {
    set({ load: next, plan: planOf(next.items), reads: reads.shown, stale: false });
    const { unread: _, ...plan } = next;
    try {
      planStorage?.setItem(PLAN_KEY, JSON.stringify({ v: PLAN_SCHEMA, plan }));
    } catch {
      // Too big for the tab's storage, or none: the next start shows nothing until its first read, as without it.
    }
  };

  async function refresh() {
    bump();
    let next: Awaited<ReturnType<typeof fetchAll>>;
    try {
      next = await fetchAll();
    } catch (err: unknown) {
      const { code, message } = failure(err);
      // Only a first read leaves nothing to show; a later one, or one after the kept plan, keeps it, out of date.
      if (NOT_THE_USER.includes(code)) set({ load: { state: "failed", code, message }, stale: false, plan: null });
      else if (reads.shown === 0 && !snap.stale) set({ load: { state: "failed", code, message } });
      else {
        if (snap.notice?.restored === true) shelved = snap.notice;
        tell({ text: `Couldn't refresh: ${message}`, offer: "refresh", retry: null, lasts: "stale", at: null });
      }
      return;
    }
    if (next === null) return;
    shown(next);
    const put = shelved !== null && snap.notice?.offer === "refresh" ? shelved : snap.notice;
    shelved = null;
    const now = noticeAfter(put, { kind: "read" });
    if (now?.restored !== true) tell(now);
    else {
      // A tab brought back hours later offers an Undo only while it can still take its write back.
      const { restored: _, ...checked } = now;
      tell(now.undo?.holds(planOf(next.items)) === true ? checked : null);
    }
    // A plan read while a write is out may not hold it yet: the write's own read drops what it drew.
    if (!sending) set({ pending: null });
  }

  async function write(
    body: Command,
    after: After,
    drawn: Pending | null = null,
    expected?: readonly Expected[],
    previouslyObserved?: readonly string[],
  ): Promise<boolean> {
    // Any write sent while one is out was made through a control the busy state forgot to hold: dropped.
    // The kept plan is read-only until a read of this start replaces it.
    if (sending || snap.stale) return false;
    sending = true;
    set({ busy: true });
    tell(after.undone === undefined ? noticeAfter(snap.notice, { kind: "write" }) : UNDOING);
    // The plan as the server last gave it, without writes still on their way: what an undo sets back to.
    const before = snap.load.state === "loaded" ? planOf(snap.load.items) : null;
    const bringIn = () => nav.go({ kind: "wrote", select: after.select, pin: after.pin });
    if (drawn !== null) {
      set({ pending: drawn });
      bringIn();
    }
    const observed =
      previouslyObserved ??
      (snap.load.state === "loaded"
        ? snap.load.needs.flatMap((need) => (typeof need.identity === "string" ? [need.identity] : []))
        : []);
    let sent: Sent;
    try {
      try {
        await command(body, fetcher, expected, observed);
      } catch (err) {
        const { code } = failure(err);
        const item = typeof body === "object" ? body.item : null;
        const fields = typeof item === "object" && item !== null ? Object.entries(item)[0] : undefined;
        if (
          code !== "redundant-dependency" ||
          fields === undefined ||
          !["add", "edit"].includes(fields[0]) ||
          typeof fields[1] !== "object" ||
          fields[1] === null ||
          !window.confirm(
            "This task already waits for that dependency through other tasks. Add the direct dependency anyway?",
          )
        )
          throw err;
        body = {
          ...(body as Record<string, unknown>),
          item: { [fields[0]]: { ...fields[1], "allow-redundant": true } },
        };
        await command(body, fetcher, expected, observed);
      }
      // Saved: what it drew stays drawn until a read started from now is shown, whatever reads still out say.
      if (drawn !== null) reads = answered(reads, drawn);
      try {
        const next = await fetchAll();
        if (next !== null) shown(next);
        set({ pending: null });
        bringIn();
        sent = { write: "saved", reload: "done" };
      } catch (err) {
        // Saved, so what it drew stays, as unread, where the next write can't undo it.
        if (drawn !== null) {
          const { unread } = reads;
          if (snap.load.state === "loaded") set({ load: { ...snap.load, unread } });
          set({ pending: null });
        }
        nav.go({ kind: "wrote", select: null, pin: null });
        sent = { write: "saved", reload: { failed: failure(err).message } };
      }
    } catch (err) {
      const { code, message } = failure(err);
      sent = { write: "failed", code, message };
      if (drawn !== null) {
        // No answer may still mean saved: the plan says, and the form opens again only if it isn't. What was drawn
        // stays until then, so the item stays selected if it was saved.
        let held = false;
        if (unanswered(code)) {
          const next = await fetchAll().catch(() => undefined);
          if (next !== undefined) {
            // A read overtaken by one already shown: that one says.
            if (next !== null) shown(next);
            held = landed(drawn, next?.items ?? reads.items);
          }
        }
        set({ pending: null });
        if (!held && drawn.kind === "add")
          nav.go({ kind: "redraft", adding: { parent: drawn.item.parent, draft: redraft(drawn.item, code) } });
        if (!held && drawn.kind === "edit") nav.go({ kind: "reopen", id: drawn.id, edited: drawn.edited });
      }
    }
    const { text, offer } = told(sent);
    const saved = sent.write === "saved" && sent.reload === "done";
    const undo = saved && after.undo && before !== null ? undoOf(body, before) : null;
    if (text !== null)
      tell({
        text,
        offer,
        retry: offer === "retry" ? { body, after, ...(expected === undefined ? {} : { expected }), observed } : null,
        // A plan saved but not read again is out of date wherever the user goes; a refusal is the surface's.
        lasts: offer === "refresh" ? "stale" : "refusal",
        at: null,
      });
    else if (undo !== null)
      tell({
        text: after.done ?? undo.text,
        offer: "undo",
        retry: null,
        undo,
        ...(before === null ? {} : { kept: keep(body, before, after.done ?? undo.text, undo.undone) }),
        // A decision's outcome is said politely, as it is without an Undo.
        ...(after.done === undefined ? {} : { polite: true }),
        lasts: "offer",
        at: null,
      });
    else if (saved && after.undone !== undefined)
      tell({ text: after.undone, offer: null, retry: null, lasts: "news", at: null });
    else if (saved && after.done !== undefined)
      tell({ text: after.done, offer: null, retry: null, polite: true, lasts: "news", at: null });
    else tell(noticeAfter(snap.notice, { kind: "write" }));
    bump();
    sending = false;
    set({ busy: false });
    return sent.write === "saved";
  }

  async function undo() {
    const offered = snap.notice?.offer === "undo" ? snap.notice.undo : undefined;
    /** How it is kept over a reload, which it stays while it is still on offer. */
    const keeping = snap.notice?.kept;
    if (offered === undefined || undoing || sending) return;
    undoing = true;
    // Said at once and until it is done: the plan is read again before the undo is sent.
    tell(UNDOING);
    try {
      // Never without a fresh read: one overtaken by a later read shown leaves that one, as fresh, to check against.
      let next: Awaited<ReturnType<typeof fetchAll>>;
      try {
        next = await fetchAll();
      } catch (err: unknown) {
        // Still on offer: Undo tries again.
        const { message } = failure(err);
        tell({
          text: `Couldn't check the plan, so nothing was undone: ${message}`,
          offer: "undo",
          retry: null,
          undo: offered,
          ...(keeping === undefined ? {} : { kept: keeping }),
          lasts: "offer",
          at: null,
        });
        return;
      }
      if (next !== null) shown(next);
      const plan = snap.plan;
      if (plan === null || !offered.holds(plan))
        tell({ text: "Changed since, so it can't be undone", offer: null, retry: null, lasts: "offer", at: null });
      else
        await write(
          offered.command,
          { select: offered.shows ?? nav.item(), pin: null, undone: offered.undone },
          null,
          offered.expect,
        );
    } finally {
      undoing = false;
    }
  }

  return {
    snapshot: () => snap,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    refresh,
    send: write,
    retry() {
      const again = snap.notice?.retry;
      return again ? write(again.body, again.after, null, again.expected, again.observed) : Promise.resolve(false);
    },
    undo,
    tell,
    // Nothing to act on while the plan kept is the one shown, so dismissing the offer to refresh it makes the read.
    dismiss() {
      const offered = snap.notice?.offer === "refresh";
      tell(null);
      if (offered && snap.stale) void refresh();
    },
    event: (event) => tell(noticeAfter(snap.notice, event)),
  };
}
