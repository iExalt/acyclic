// The app's session with the server: the plan as last read, the write out and the one notice slot, with the Undo on
// offer kept in the tab over a reload. Writes go one at a time; reads are numbered, so one overtaken is never shown;
// what a write does to where the user is goes through `nav`. Plain TypeScript with its ports passed in, so every
// outcome of a write, a read and an undo is tested without a browser.

import { type Command, command, type Fetch, failure, type Item, items, needs, type Threads, threads } from "./api";
import { landed, type Pending, redraft, type Sent, told, unanswered } from "./edits";
import type { Intent } from "./nav";
import type { Need } from "./needs";
import { type NoticeEvent, noticeAfter, type Raised } from "./notices";
import { answered, NO_READS, type Reads, show, start, type Unread } from "./reads";
import { type Kept, keep, type Plan, type Undo, undoOf, unkeep } from "./undo";

export type Load =
  | { state: "loading" }
  | { state: "loaded"; items: Item[]; threads: Threads; needs: Need[]; unread: Unread[] }
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
  offer: "retry" | "refresh" | "hide-archived" | "undo" | null;
  /** The write again, offered when the server was only busy. */
  retry: { body: Command; after: After } | null;
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

export interface Ports {
  fetcher: Fetch;
  /** The tab's storage, or null where there is none. Any call may throw, as a full or blocked one does. */
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> | null;
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
  /** A write drawn before the server answers, until the plan read after it holds it. */
  pending: Pending | null;
  /** The latest read shown, as the server had it: what an undo's guard is checked against. */
  plan: Plan | null;
  /** Reads shown, so a browser journey knows a refresh has landed. */
  reads: number;
  /** Counts writes and refreshes, so panels that read more than the plan read again after each. */
  version: number;
}

export interface Session {
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

const planOf = (found: Item[]): Plan => ({ items: new Map(found.map((one) => [one.id, one])) });

export function createSession({ fetcher, storage, nav }: Ports): Session {
  const listeners = new Set<() => void>();
  let reads: Reads = NO_READS;
  /** Set from a write's start to its end, so a second can't start before a render disables what sends it. */
  let sending = false;
  let undoing = false;
  let snap: Snapshot = {
    load: { state: "loading" },
    notice: restore(),
    busy: false,
    pending: null,
    plan: null,
    reads: 0,
    version: 0,
  };
  /** The Undo stored, written once each rather than as the notice moves with the user, as a large plan is slow. */
  let kept = snap.notice?.kept;

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
    const [found, listed, needed] = await Promise.all([items(fetcher), threads(fetcher), needs(fetcher)]);
    const next = show(reads, n, found);
    if (next === null) return null;
    reads = next;
    return { state: "loaded" as const, items: found, threads: listed, needs: needed, unread: next.unread };
  }
  /** Shows a read: the plan, and the latest read the server gave. */
  const shown = (next: Extract<Load, { state: "loaded" }>) =>
    set({ load: next, plan: planOf(next.items), reads: reads.shown });

  async function refresh() {
    bump();
    let next: Awaited<ReturnType<typeof fetchAll>>;
    try {
      next = await fetchAll();
    } catch (err: unknown) {
      const { code, message } = failure(err);
      // Only a first read leaves nothing to show; a later one keeps the plan, out of date, and offers another, unless
      // the browser must pair first: coming back to the app after pairing reads again by itself.
      if (reads.shown === 0) set({ load: { state: "failed", code, message } });
      else {
        const unpaired = code === "no-session";
        const text = unpaired ? message : `Couldn't refresh: ${message}`;
        tell({ text, offer: unpaired ? null : "refresh", retry: null, lasts: "stale", at: null });
      }
      return;
    }
    if (next === null) return;
    shown(next);
    const now = noticeAfter(snap.notice, { kind: "read" });
    if (now?.restored !== true) tell(now);
    else {
      // A tab brought back hours later offers an Undo only while it can still take its write back.
      const { restored: _, ...checked } = now;
      tell(now.undo?.holds(planOf(next.items)) === true ? checked : null);
    }
    // A plan read while a write is out may not hold it yet: the write's own read drops what it drew.
    if (!sending) set({ pending: null });
  }

  async function write(body: Command, after: After, drawn: Pending | null = null): Promise<boolean> {
    // Any write sent while one is out was made through a control the busy state forgot to hold: dropped.
    if (sending) return false;
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
    let sent: Sent;
    try {
      await command(body, fetcher);
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
        retry: offer === "retry" ? { body, after } : null,
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
          lasts: "offer",
          at: null,
        });
        return;
      }
      if (next !== null) shown(next);
      const plan = snap.plan;
      if (plan === null || !offered.holds(plan))
        tell({ text: "Changed since, so it can't be undone", offer: null, retry: null, lasts: "offer", at: null });
      else await write(offered.command, { select: offered.shows ?? nav.item(), pin: null, undone: offered.undone });
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
      return again ? write(again.body, again.after) : Promise.resolve(false);
    },
    undo,
    tell,
    dismiss: () => tell(null),
    event: (event) => tell(noticeAfter(snap.notice, event)),
  };
}
