import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AddForm } from "./AddForm";
import { addressOf, landing, NOTHING, parse, type Wanted } from "./address";
import { type Command, command, failure, globalLevel, type Item, items, needs, type Threads, threads } from "./api";
import { Choice } from "./Choice";
import { CommandMenu } from "./CommandMenu";
import { entries as commandEntries, type OnItem, onMac, opensMenu, type Run, targets, undoes } from "./commands";
import {
  addCommand,
  type Edited,
  type LinkKind,
  landed,
  type Mode,
  type NewItem,
  type Pending,
  type Pick,
  pick,
  type Sent,
  told,
  unanswered,
  unlinkCommand,
} from "./edits";
import { globalLevelCommand, LEVELS, type Level, levelHint, levelLabel } from "./forms";
import { Inspector } from "./Inspector";
import { Icon, X } from "./icons";
import { type KeyAction, nextRow, SHORTCUTS, step, typingIn, type Waiting } from "./keys";
import {
  arrived,
  depth,
  type Entries,
  follow,
  type HistoryAction,
  type Open,
  openLayers,
  restored,
  topLayer,
} from "./layers";
import { byNumber, lineLabel, linesOf } from "./lines";
import { buildModel, inArchived } from "./map/model";
import type { Focus, Framed } from "./map/PlanMap";
import { NeedsList } from "./NeedsList";
import { Notify } from "./Notify";
import { awaitingApproval, type Need } from "./needs";
import { noticeAfter, type Place, type Raised } from "./notices";
import { Overview } from "./Overview";
import { currentRows, membersOf, outlineOf, pickedAfter } from "./outline";
import { Projects, ProjectsSheet } from "./Projects";
import { PALETTES, setTheme, type Theme, useScheme, useTheme } from "./palette";
import { answered, drawn as drawnOver, NO_READS, type Reads, show, start, type Unread } from "./reads";
import { HOME, Shell, type View, Wordmark } from "./Shell";
import { ThreadPanel } from "./ThreadPanel";
import { ThreadsView } from "./ThreadsView";
import { workingThreads } from "./threadLists";
import { type Kept, keep, type Undo, undoOf, unkeep } from "./undo";
import { threadCounts, wavesView } from "./waves";
import { useWide } from "./wide";

// The Map's code (the renderer and the layout engine) is the bulk of the app; it loads while the tracker does.
const mapModule = import("./map/PlanMap");
const PlanMap = lazy(() => mapModule);

type Load =
  | { state: "loading" }
  | { state: "loaded"; items: Item[]; threads: Threads; level: string; needs: Need[]; unread: Unread[] }
  | { state: "failed"; code: string; message: string };

/** Where this device keeps whether single-key shortcuts are on. */
const SHORTCUTS_KEY = "acyclic.shortcuts";
/** The Undo on offer, kept in this tab over a reload. */
const UNDO_KEY = "acyclic.undo";

/** What an edit selects afterwards, and which box holds still on screen while the Map redraws. */
interface After {
  /** The item to select and show, or null to leave the selection as it is, as a thread's edits do. */
  select: string | null;
  pin: string | null;
  /**
   * Offer to undo it, once saved: for the user's own moves, links, field edits and autonomy, and a first approval;
   * never another decision.
   */
  undo?: true;
  /** Said once saved, for an undo: what it did. */
  undone?: string;
  /** Said once saved, for a write that takes away what it was done from, as settling a need does. */
  done?: string;
  /**
   * A setting, which its control shows at once: saved without the busy bar or holding anything else, and, sent while
   * another write is out, sent after it rather than dropped.
   */
  background?: true;
}

/** A refusal or failure to show, with what it offers: the command again when the server was only busy, or a refresh. */
/** What the app tells the user, which `notices.ts` says how long to keep and where to show. */
interface Notice extends Raised {
  text: string;
  offer: "retry" | "refresh" | "hide-archived" | "undo" | null;
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

const UNDOING: Notice = { text: "Undoing…", offer: null, retry: null, lasts: "offer", at: null };
/** What a form holding typing says when the user tries to leave it. */
const UNSAVED: Notice = {
  text: "Unsaved changes: save them, or cancel",
  offer: null,
  retry: null,
  lasts: "form",
  at: null,
};

/** The add form, open under a parent or at the top level, with what the user wrote if a failed add reopened it. */
interface Adding {
  parent: string | null;
  idRefused: boolean;
  draft: NewItem | null;
}

/** An item's editor reopened, with what the user wrote, after their edit failed; `n` remounts the inspector. */
interface Reopened {
  id: string;
  edited: Edited;
  n: number;
}

/** The rows J and K step through: Needs you's entries, else the rows of the view in the page's main area. */
function rowsInView(): HTMLElement[] {
  const scope = document.querySelector("[data-testid=needs]") ?? document.querySelector("main");
  return [...(scope?.querySelectorAll<HTMLElement>("[data-row]") ?? [])];
}

/** The Undo a reload left on offer in this tab, offered again; or none. */
function kept(): Notice | null {
  try {
    const raw = sessionStorage.getItem(UNDO_KEY);
    const was: Kept | null = raw === null ? null : JSON.parse(raw);
    const undo = was === null ? null : unkeep(was);
    if (was === null || undo === null) return null;
    return { text: was.text, offer: "undo", retry: null, undo, kept: was, restored: true, lasts: "offer", at: null };
  } catch {
    return null;
  }
}

export function App() {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [showArchived, setShowArchived] = useState(false);
  const [thread, setThread] = useState<string | null>(null);
  const threadNow = useRef(thread);
  threadNow.current = thread;
  /** The projects' sheet, open over the Map on a phone; side by side, the outline is always in view. */
  const [showProjects, setShowProjects] = useState(false);
  /** The command menu, over everything; listing only what has a key, as ? asks. */
  const [showCommand, setShowCommand] = useState(false);
  const [shortcutsOnly, setShortcutsOnly] = useState(false);
  /** Single-key shortcuts, on unless switched off on this device. */
  const [shortcuts, setShortcuts] = useState(() => localStorage.getItem(SHORTCUTS_KEY) !== "off");
  /** Counts the keyboard's asks to edit the item shown, which its panel answers. */
  const [editAsked, setEditAsked] = useState(0);
  /** The Map, or the threads in waves over it. */
  const [view, setView] = useState<View>(HOME);
  const viewNow = useRef(view);
  viewNow.current = view;
  const wide = useWide();
  const [selected, setSelected] = useState<string | null>(null);
  /** The selection now, for a write's callback: an undo brings it back into view, as the Map may move it. */
  const selectedNow = useRef(selected);
  selectedNow.current = selected;
  /**
   * The view an item or a thread was opened from, which closing the one opened from it returns to: both are shown on
   * the Map, but the user came from there. An item chosen from a thread's panel leaves the thread's, for the thread's
   * close. Dropped once the user selects on the Map or goes to another view, which makes that where they are.
   */
  const [cameFrom, setCameFrom] = useState<{ view: View; by: "item" | "thread" } | null>(null);
  /** A thread the command menu opened while Needs you showed, so shown over it. */
  const [threadOver, setThreadOver] = useState<string | null>(null);
  /** The item Needs you was opened over, which shows again once it closes. */
  const [needsOver, setNeedsOver] = useState<string | null>(null);
  const [focus, setFocus] = useState<Focus | null>(null);
  const [framed, setFramed] = useState<Framed | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "idle" });
  const [adding, setAdding] = useState<Adding | null>(null);
  const [notice, setNotice] = useState<Notice | null>(kept);
  const [busy, setBusy] = useState(false);
  // Kept while it is on offer, and dropped once anything else takes its place; "Undoing…" passes it on to what follows.
  // Written once each, not as the notice moves with the user, as a large plan takes a while to write.
  const keptNow = useRef<Kept | undefined>(notice?.kept);
  useEffect(() => {
    const next = notice?.kept;
    if (next === keptNow.current || (next === undefined && notice === UNDOING)) return;
    keptNow.current = next;
    try {
      if (next !== undefined) sessionStorage.setItem(UNDO_KEY, JSON.stringify(next));
      else sessionStorage.removeItem(UNDO_KEY);
    } catch {
      // Too big a plan for the tab's storage: the Undo is offered until a reload, as without it.
      sessionStorage.removeItem(UNDO_KEY);
    }
  }, [notice]);
  /** Set from a write's start to its end, so a second can't start before a render disables what sends it. */
  const sending = useRef(false);
  /** A write drawn before the server answers, until the plan read after it holds it. */
  const [pending, setPending] = useState<Pending | null>(null);
  const [reopened, setReopened] = useState<Reopened | null>(null);
  const [showNeeds, setShowNeeds] = useState(false);
  /** The entry Needs you opens at, when the Overview opened it from one. */
  const [needsAt, setNeedsAt] = useState<string | null>(null);
  /**
   * An address to open: the one the app was opened at, or one a notification sent the app already open. It waits for
   * the plan to load, and for a form holding what the user typed to close.
   */
  const [wanted, setWanted] = useState<Wanted | null>(() => {
    const asked = parse(window.location.search);
    return addressOf(asked) === "/" ? null : asked;
  });
  /** Counts forms closed, so a link waiting on one opens once it has. */
  const [formsClosed, setFormsClosed] = useState(0);
  /** Counts edits sent and refreshes, so panels that read more than the plan read again after each. */
  const [version, setVersion] = useState(0);
  /** The global level just picked, shown until the plan read back says it, or put back if it was refused. */
  const [levelChosen, setLevelChosen] = useState<string | null>(null);

  /** The plan's reads, so one overtaken by a later read is never shown, and the writes saved that none holds yet. */
  const reads = useRef<Reads>(NO_READS);
  /** The latest read shown, as the server had it: what an undo's guard is checked against. */
  const lastRead = useRef<{ items: ReadonlyMap<string, Item>; level: string } | null>(null);
  /** Reads the plan, or null when a read started later has been shown already. */
  const fetchAll = useCallback(async () => {
    reads.current = start(reads.current);
    const n = reads.current.started;
    const [found, listed, level, needed] = await Promise.all([items(), threads(), globalLevel(), needs()]);
    const next = show(reads.current, n, found);
    if (next === null) return null;
    reads.current = next;
    lastRead.current = { items: new Map(found.map((one) => [one.id, one])), level };
    return { state: "loaded" as const, items: found, threads: listed, level, needs: needed, unread: next.unread };
  }, []);
  const refresh = useCallback(() => {
    setVersion((was) => was + 1);
    fetchAll().then(
      (next) => {
        if (next === null) return;
        setLoad(next);
        setNotice((was) => {
          const now = noticeAfter(was, { kind: "read" });
          if (now?.restored !== true) return now;
          // A tab brought back hours later offers an Undo only while it can still take its write back.
          const { restored: _, ...checked } = now;
          const plan = { items: new Map(next.items.map((one) => [one.id, one])), level: next.level };
          return now.undo?.holds(plan) === true ? checked : null;
        });
        // A plan read while a write is out may not hold it yet: the write's own read drops what it drew.
        if (!sending.current) setPending(null);
      },
      (err: unknown) => {
        const { code, message } = failure(err);
        // Only a first read leaves nothing to show; a later one keeps the plan, out of date, and offers another, unless
        // the browser must pair first: coming back to the app after pairing reads again by itself.
        if (reads.current.shown === 0) setLoad({ state: "failed", code, message });
        else {
          const unpaired = code === "no-session";
          const text = unpaired ? message : `Couldn't refresh: ${message}`;
          setNotice({ text, offer: unpaired ? null : "refresh", retry: null, lasts: "stale", at: null });
        }
      },
    );
  }, [fetchAll]);
  useEffect(() => {
    refresh();
    // Coming back to the app shows what changed meanwhile.
    const visible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    document.addEventListener("visibilitychange", visible);
    return () => document.removeEventListener("visibilitychange", visible);
  }, [refresh]);

  const loaded = load.state === "loaded" ? load : null;
  /** The plan as the server last gave it, without writes still on their way: what an undo sets back to. */
  const serverPlan = useRef<{ items: ReadonlyMap<string, Item>; level: string } | null>(null);
  serverPlan.current =
    loaded === null ? null : { items: new Map(loaded.items.map((one) => [one.id, one])), level: loaded.level };
  const shown = useMemo(
    () => (loaded === null ? [] : drawnOver(loaded.items, loaded.unread, pending)),
    [loaded, pending],
  );
  const sorted = useMemo(() => (loaded === null ? [] : [...loaded.threads.threads].sort(byNumber)), [loaded]);
  const palette = PALETTES[useScheme()];
  const theme = useTheme();
  const lines = useMemo(() => linesOf(sorted, palette), [sorted, palette]);
  const linesNow = useRef(lines);
  linesNow.current = lines;
  const model = useMemo(() => (loaded === null ? null : buildModel(shown)), [loaded, shown]);
  const chosen = selected === null ? undefined : shown.find((one) => one.id === selected);
  const outline = useMemo(
    () =>
      model === null
        ? null
        : outlineOf(
            model,
            sorted.map((one) => one.id),
            showArchived,
          ),
    [model, sorted, showArchived],
  );
  const counts = useMemo(() => threadCounts(shown), [shown]);
  const itemsById = useMemo(() => new Map(shown.map((item) => [item.id, item])), [shown]);
  const waves = useMemo(
    () => (loaded === null || outline === null ? null : wavesView(loaded.threads, shown, outline)),
    [loaded, shown, outline],
  );
  const working = useMemo(() => new Set(workingThreads(counts)), [counts]);
  const awaiting = useMemo(() => awaitingApproval(loaded?.needs ?? []), [loaded]);
  const current = useMemo(
    () =>
      model === null || outline === null ? { project: null, thread: null } : currentRows(model, outline, selected),
    [model, outline, selected],
  );
  // An item that went away, deleted or moved out of reach elsewhere, or hidden with archived items, is no longer
  // selected: what's selected is drawn, so the panel and the address never name an item the Map doesn't show.
  useEffect(() => {
    if (model === null || selected === null) return;
    if (!model.items.has(selected) || (!showArchived && inArchived(model, selected))) setSelected(null);
  }, [model, selected, showArchived]);

  // Closed by any other way than its own close, such as a tap on the Map's background, it leaves the user there.
  useEffect(() => {
    if (selected === null && thread === null) setCameFrom(null);
  }, [selected, thread]);
  // A view the user goes to is where they are; opening an item or a thread goes to the Map, which keeps it.
  useEffect(() => {
    if (view !== "map") setCameFrom(null);
  }, [view]);
  // A thread the command menu put over Needs you, once closed, opens again as any other does.
  useEffect(() => {
    if (thread === null) setThreadOver(null);
  }, [thread]);
  /** Goes back to the view what is closing was opened from. */
  const goBack = () => {
    if (cameFrom !== null) setView(cameFrom.view);
    setCameFrom(null);
  };
  /** Closes the item: one chosen from a thread's panel goes back to the thread, whose own close goes back further. */
  const closeItem = () => {
    setSelected(null);
    if (cameFrom?.by === "item") goBack();
  };
  const closeThread = () => {
    setThread(null);
    goBack();
  };

  // An editor opened again for one item stays closed on any other.
  useEffect(() => {
    setReopened((was) => (was === null || was.id === selected ? was : null));
  }, [selected]);

  // One thing in focus: an item selected outside the picked thread unpicks it, in the same update, so the outline never
  // marks a project and a thread the selection isn't in at once, and the layers swap without a step through the
  // history. One in the thread keeps it, its panel over the thread's.
  const modelNow = useRef(model);
  modelNow.current = model;
  const selectOnly = useCallback((id: string | null) => {
    setSelected(id);
    if (id !== null) setThread((was) => pickedAfter(modelNow.current, was, id));
  }, []);
  const select = useCallback(
    (id: string | null, pin: string | null = null) => {
      selectOnly(id);
      if (id === null) return;
      setFocus((was) => ({ id, pin, n: (was?.n ?? 0) + 1 }));
      // An item is shown on the Map, so selecting one from another view goes to it, and closing it goes back. One
      // chosen from a thread's panel keeps where the thread came from, for the thread's close.
      // Read now: the update runs at the next render, by when the view is the Map.
      const from = viewNow.current;
      const inThread = threadNow.current !== null;
      setCameFrom((was) =>
        from !== "map" ? { view: from, by: "item" } : inThread && was?.by === "thread" ? was : null,
      );
      setView("map");
    },
    [selectOnly],
  );
  /**
   * Takes the user to the Map for a thread: picked, its panel open, and its items framed in the part of the Map the
   * panel leaves, unlike `pickThread`.
   */
  const toThread = useCallback((id: string) => {
    setSelected(null);
    setThread(id);
    setCameFrom(viewNow.current === "map" ? null : { view: viewNow.current, by: "thread" });
    setView("map");
    setFramed((was) => ({ thread: id, n: (was?.n ?? 0) + 1 }));
  }, []);
  /**
   * A thread's row in the outline: on the Map, the thread picked already is unpicked; any other goes to the Map for
   * it, framed, as the Overview's rows do.
   */
  const pickFromOutline = (id: string) => {
    if (thread === id && view === "map") setThread(null);
    else toThread(id);
  };
  /**
   * Picks a thread, as a tile in the Threads view does, or unpicks the one picked, leaving the Map's view alone
   * unless nothing of the thread is in it.
   */
  const pickThread = useCallback((id: string) => {
    setSelected(null);
    setThread((was) => (was === id ? null : id));
  }, []);

  /**
   * Sends an edit; once saved, reloads the plan and brings `after.select` into view with `after.pin` held still. An
   * edit `drawn` is drawn at once, its form closed, and if the write fails, the form opens again with what the user
   * wrote, unless the server turns out to hold it after all.
   */
  const queued = useRef<{ body: Command; after: After; saved: (saved: boolean) => void }[]>([]);
  /** The write out is a setting's, which holds nothing, so a write the user makes meanwhile waits for it. */
  const quietOut = useRef(false);
  const send = useCallback(
    async (body: Command, after: After, drawn: Pending | null = null): Promise<boolean> => {
      if (sending.current) {
        // Any other write sent while one is out was made through a control the busy state forgot to hold: dropped.
        if (after.background === undefined && !quietOut.current) return false;
        return new Promise((saved) => queued.current.push({ body, after, saved }));
      }
      sending.current = true;
      quietOut.current = after.background !== undefined;
      if (after.background === undefined) setBusy(true);
      setNotice((was) => (after.undone === undefined ? noticeAfter(was, { kind: "write" }) : UNDOING));
      const before = serverPlan.current;
      const bringIn = () => {
        const select = after.select;
        if (select === null) return;
        setSelected(select);
        setFocus((was) => ({ id: select, pin: after.pin, n: (was?.n ?? 0) + 1 }));
      };
      if (drawn !== null) {
        setPending(drawn);
        setAdding(null);
        setReopened(null);
        bringIn();
      }
      let sent: Sent;
      try {
        const answer = await command<{ proposals?: unknown[] } | null>(body);
        const proposals = answer?.proposals?.length ?? 0;
        // Saved: what it drew stays drawn until a read started from now is shown, whatever reads still out say.
        if (drawn !== null) reads.current = answered(reads.current, drawn);
        try {
          const next = await fetchAll();
          if (next !== null) setLoad(next);
          setPending(null);
          setAdding(null);
          bringIn();
          sent = { write: "saved", proposals, reload: "done" };
        } catch (err) {
          // Saved, so what it drew stays, as unread, where the next write can't undo it.
          if (drawn !== null) {
            const { unread } = reads.current;
            setLoad((was) => (was.state === "loaded" ? { ...was, unread } : was));
            setPending(null);
          }
          setAdding(null);
          sent = { write: "saved", proposals, reload: { failed: failure(err).message } };
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
              if (next !== null) setLoad(next);
              held = landed(drawn, next?.items ?? reads.current.items);
            }
          }
          setPending(null);
          if (!held && drawn.kind === "add")
            setAdding({ parent: drawn.item.parent, idRefused: told(sent).fixId, draft: drawn.item });
          if (!held && drawn.kind === "edit")
            setReopened((was) => ({ id: drawn.id, edited: drawn.edited, n: (was?.n ?? 0) + 1 }));
        }
      }
      const { text, offer, fixId } = told(sent);
      const saved = sent.write === "saved" && sent.reload === "done" && sent.proposals === 0;
      const undo =
        saved && after.undo && before !== null
          ? undoOf(body, before, levelLabel, (id) => {
              const line = linesNow.current.get(id);
              // A thread titled only with its number is named by it.
              return line?.name || (line?.number ? `thread ${line.number}` : id);
            })
          : null;
      if (text !== null)
        setNotice({
          text,
          offer,
          retry: offer === "retry" ? { body, after } : null,
          // A plan saved but not read again is out of date wherever the user goes; a refusal is the surface's.
          lasts: offer === "refresh" ? "stale" : "refusal",
          at: null,
        });
      else if (undo !== null)
        setNotice({
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
        setNotice({ text: after.undone, offer: null, retry: null, lasts: "news", at: null });
      else if (saved && after.done !== undefined)
        setNotice({ text: after.done, offer: null, retry: null, polite: true, lasts: "news", at: null });
      else setNotice((was) => noticeAfter(was, { kind: "write" }));
      if (fixId) setAdding((was) => (was === null ? null : { ...was, idRefused: true }));
      setVersion((was) => was + 1);
      sending.current = false;
      setBusy(false);
      // A setting changed while this was out goes now, in the order chosen.
      const next = queued.current.shift();
      if (next !== undefined) void send(next.body, next.after).then(next.saved);
      return sent.write === "saved";
    },
    [fetchAll],
  );
  // The level picked is the plan's own once a read says it.
  useEffect(() => {
    if (load.state === "loaded" && load.level === levelChosen) setLevelChosen(null);
  }, [load, levelChosen]);

  /**
   * Takes back the write `undo` came from: reads the plan first, so the guard is checked against the server's latest,
   * and sends nothing when what the write left has changed since. Once at a time.
   */
  const undoing = useRef(false);
  const runUndo = useCallback(
    async (undo: Undo) => {
      if (undoing.current || sending.current) return;
      undoing.current = true;
      // Said at once and until it is done: the plan is read again before the undo is sent.
      setNotice(UNDOING);
      try {
        // Never without a fresh read: one overtaken by a later read shown leaves that one, as fresh, to check against.
        let next: Awaited<ReturnType<typeof fetchAll>>;
        try {
          next = await fetchAll();
        } catch (err: unknown) {
          // Still on offer: Undo tries again.
          const { message } = failure(err);
          setNotice({
            text: `Couldn't check the plan, so nothing was undone: ${message}`,
            offer: "undo",
            retry: null,
            undo,
            lasts: "offer",
            at: null,
          });
          return;
        }
        if (next !== null) setLoad(next);
        const plan = lastRead.current;
        if (plan === null || !undo.holds(plan))
          setNotice({
            text: "Changed since, so it can't be undone",
            offer: null,
            retry: null,
            lasts: "offer",
            at: null,
          });
        else await send(undo.command, { select: undo.shows ?? selectedNow.current, pin: null, undone: undo.undone });
      } finally {
        undoing.current = false;
      }
    },
    [fetchAll, send],
  );

  const choose = useCallback(
    (target: Pick) => {
      if (model === null) return;
      const step = pick(model, mode, target);
      setMode(step.mode);
      const refusal = step.refusal;
      // A target refused says why where the user chose it; one taken drops what an earlier choice was refused.
      setNotice((was) =>
        refusal !== null
          ? { text: refusal, offer: null, retry: null, lasts: "refusal", at: null }
          : was?.lasts === "refusal"
            ? null
            : was,
      );
      if (step.command === null || mode.kind === "idle") return;
      // A move pins the new parent, or the item itself when it goes to the top level; a link, the item linked from.
      const pin = mode.kind === "move" && target.kind === "item" ? target.id : mode.item;
      send(step.command, { select: mode.item, pin, undo: true });
    },
    [model, mode, send],
  );
  const open: Open = {
    command: showCommand,
    mode: mode.kind !== "idle",
    adding: adding !== null,
    item: chosen !== undefined,
    needs: showNeeds,
    projects: showProjects && !wide,
    thread: thread !== null,
    away: !wide && view !== HOME,
    threadOverNeeds: showNeeds && thread !== null && threadOver === thread,
    needsOverItem: showNeeds && chosen !== undefined && needsOver === chosen.id,
  };
  /**
   * The layer shown under the command menu, or on top without it: the menu opens over a panel without closing it, so
   * a form under it keeps what the user typed.
   */
  const panel = topLayer({ ...open, command: false });
  // What the command menu finds, built once as it opens rather than at every render while it shows.
  const menuEntries = useMemo(() => {
    if (!showCommand || model === null) return [];
    const threads = sorted.map((one) => ({
      id: one.id,
      number: lines.get(one.id)?.number ?? null,
      name: lines.get(one.id)?.name ?? one.id,
    }));
    const item = panel === "item" && selected !== null ? model.items.get(selected) : undefined;
    const plan = { model, threads, working, showArchived, item: item === undefined ? null : item };
    return mode.kind === "idle" ? commandEntries(plan) : targets(plan, mode.kind, mode.item);
  }, [showCommand, model, sorted, lines, working, showArchived, mode, panel, selected]);
  const layers = depth(open);
  /** Drops what was said of an unsaved form, once the form has gone. */
  const formClosed = useCallback(() => {
    setNotice((was) => noticeAfter(was, { kind: "form-closed" }));
    setFormsClosed((was) => was + 1);
  }, []);
  // What's on top changing takes the add form, or the item's editor, with it.
  useEffect(formClosed, [panel]);
  /** Where the user is, which says how long a notice stays. */
  const shownItem = chosen?.id ?? null;
  const place: Place = useMemo(
    () => ({ layer: panel, item: shownItem, thread, view }),
    [panel, shownItem, thread, view],
  );
  // A notice is first shown where the user is, and kept or dropped as they move. As an update of the notice as it
  // is by then, so it never brings back one another effect of the same move cleared, such as a closed form's.
  useEffect(() => {
    if (notice !== null) setNotice((was) => noticeAfter(was, { kind: "moved", to: place }));
  }, [notice, place]);
  /**
   * Closes `count` layers from the top, as Escape (one) and Back (one per entry it went back over) do, but never a
   * form holding what the user typed: that says how to leave it instead. How many it closed.
   */
  const closeTop = useRef<(count: number) => number>(() => 0);
  closeTop.current = (count) => {
    const closing = openLayers(open).slice(0, count);
    // The command menu closes over a form without touching it.
    if (closing.some((layer) => layer !== "command") && document.querySelector("form[data-dirty='true']") !== null) {
      // A link waiting for the form says so already, and dismissing that drops it.
      setNotice((was) => (was?.held ? was : UNSAVED));
      return 0;
    }
    for (const layer of closing) {
      if (layer === "command") setShowCommand(false);
      else if (layer === "mode") setMode({ kind: "idle" });
      else if (layer === "add") setAdding(null);
      else if (layer === "item") closeItem();
      else if (layer === "needs") setShowNeeds(false);
      else if (layer === "projects") setShowProjects(false);
      else if (layer === "away") setView(HOME);
      else closeThread();
    }
    return closing.length;
  };
  /** What the keys read of the app as it is: kept current at each render, for the one listener. */
  const keysSee = useRef({ shortcuts, panel, menu: showCommand });
  keysSee.current = { shortcuts, panel, menu: showCommand };
  /** The undo on offer, while its notice shows and nothing is being sent: what ⌘Z takes back. */
  const undoNow = useRef<Undo | null>(null);
  undoNow.current = notice?.offer === "undo" && !notice.restored && !busy ? (notice.undo ?? null) : null;
  const runUndoNow = useRef(runUndo);
  runUndoNow.current = runUndo;
  /** What a single-key shortcut does; set once the plan has loaded. */
  const onKeyAction = useRef<(action: KeyAction) => void>(() => undefined);
  useEffect(() => {
    const mac = onMac();
    let pending: Waiting = null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) closeTop.current(1);
      // Taken from the browser only when it opens or closes the menu: the listener is the app's page's, so only while
      // the app has the focus.
      else if (undoes(event, mac)) {
        // Only an undo the user can see, and never over typing, the menu or a changed form, where ⌘Z is the field's.
        const undo = undoNow.current;
        const blocked =
          undo === null ||
          keysSee.current.menu ||
          typingIn(event.target) ||
          document.querySelector("form[data-dirty='true']") !== null;
        if (!blocked) {
          event.preventDefault();
          runUndoNow.current(undo);
        }
      } else if (opensMenu(event, mac)) {
        event.preventDefault();
        setShortcutsOnly(false);
        setShowCommand((was) => !was);
      } else {
        const seen = keysSee.current;
        const next = step(pending, event, {
          enabled: seen.shortcuts,
          panel: seen.panel,
          menu: seen.menu,
          dirty: document.querySelector("form[data-dirty='true']") !== null,
          typing: typingIn(event.target),
          rows: rowsInView().length,
        });
        if (next.action !== null || next.pending !== null) event.preventDefault();
        pending = next.pending;
        if (next.action !== null) onKeyAction.current(next.action);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  // One history entry per layer open, so a phone's Back closes them one by one before it leaves the app.
  const entries = useRef<Entries>({ pushed: 0, waiting: false });
  const layersNow = useRef(layers);
  layersNow.current = layers;
  const act = useCallback((step: { entries: Entries; action: HistoryAction }) => {
    entries.current = step.entries;
    const { action } = step;
    if (action === null) return;
    if (action.kind === "push") {
      // Each with the address of what is open, so Forward onto any of them opens it again; while a link waits, the
      // address it was opened at stays. The entry under them, unless it is the app's first, takes it too: what it
      // stood for may have closed as these opened, as the command menu does choosing an item, and its address with it.
      const url = waitingLink.current ? undefined : addressOf(hereNow.current, window.location.search);
      if (url !== undefined && action.from > 0) window.history.replaceState(window.history.state, "", url);
      for (let n = action.from + 1; n <= action.to; n++) window.history.pushState({ layers: n }, "", url);
    } else if (action.kind === "back") window.history.go(-action.by);
    else if (action.kind === "relabel")
      window.history.replaceState({ ...window.history.state, layers: action.layers }, "");
    else if (action.kind === "restore") {
      // Read now, before the address is written over with what was open: Forward opens it as a link does.
      waitingLink.current = true;
      setWanted(parse(window.location.search));
    }
    // A form left open by Back takes its entries again.
    else if (closeTop.current(action.by) === 0) act(follow(entries.current, layersNow.current));
  }, []);
  // What Forward restores settles once it has opened: until then the layers open are the ones before it.
  useEffect(() => {
    if (!(entries.current.restoring && wanted !== null)) act(follow(entries.current, layers));
  }, [act, layers, wanted]);
  useEffect(() => {
    const onMove = (event: PopStateEvent) => {
      const at = typeof event.state?.layers === "number" ? event.state.layers : 0;
      act(arrived(entries.current, at, layersNow.current));
    };
    window.addEventListener("popstate", onMove);
    return () => window.removeEventListener("popstate", onMove);
  }, [act]);

  // The address always says what's open, on the entry shown: written over it as what's open changes, never while the
  // app's own move back is on its way (it would land on the entry being left), nor before a link asked for is open.
  const here: Wanted = {
    view,
    item: chosen?.id ?? null,
    thread,
    needs: showNeeds,
    needsAt: showNeeds ? needsAt : null,
  };
  const hereNow = useRef(here);
  hereNow.current = here;
  const waitingLink = useRef(wanted !== null);
  waitingLink.current = wanted !== null;
  const writeAddress = useCallback(() => {
    if (entries.current.waiting || waitingLink.current) return;
    const next = addressOf(hereNow.current, window.location.search);
    if (next !== window.location.pathname + window.location.search)
      window.history.replaceState(window.history.state, "", next);
  }, []);
  const address = addressOf(here);
  // Written again whenever what's open changes, and once a link has opened.
  useEffect(writeAddress, [writeAddress, address, wanted, layers]);
  useEffect(() => {
    window.addEventListener("popstate", writeAddress);
    return () => window.removeEventListener("popstate", writeAddress);
  }, [writeAddress]);

  // A notification's link, sent to the app already open rather than reloading it: answered at once, so the service
  // worker knows not to, then opened as any link is.
  useEffect(() => {
    const container = navigator.serviceWorker;
    if (container === undefined) return;
    const onMessage = (event: MessageEvent) => {
      if (event.data?.kind !== "open" || typeof event.data.search !== "string") return;
      event.ports[0]?.postMessage("opened");
      // A newer link replaces one still waiting. One naming no view, such as Needs you's, opens over the view shown, so
      // closing it goes back to where the user was.
      const asked = parse(event.data.search);
      const query = new URLSearchParams(event.data.search);
      setWanted(query.has("view") || query.has("item") ? asked : { ...asked, view: hereNow.current.view });
    };
    container.addEventListener("message", onMessage);
    container.startMessages();
    return () => container.removeEventListener("message", onMessage);
  }, []);

  // A link opens once the plan is loaded, over home, with each layer it opens its own entry above, so Back closes them
  // one by one as if the user had opened them. A form holding what the user typed is never closed for it: the link
  // waits, said so, until the form is saved or cancelled.
  const wantedOpens = useRef<(link: Wanted) => void>(() => undefined);
  wantedOpens.current = (link) => {
    if (model === null) return;
    const name =
      link.item !== null
        ? (model.items.get(link.item)?.title ?? link.item)
        : link.thread !== null
          ? (lines.get(link.thread)?.name ?? link.thread)
          : link.needs
            ? "Needs you"
            : link.view === "map"
              ? "the Map"
              : link.view === "threads"
                ? "Threads"
                : "the Overview";
    if (document.querySelector("form[data-dirty='true']") !== null) {
      // An entry Forward landed on stands for what is open meanwhile, so no later move goes back over it unasked.
      if (entries.current.restoring) act(restored(entries.current, layersNow.current));
      setNotice({
        text: `Unsaved changes: save or cancel, then ${name} opens`,
        offer: null,
        retry: null,
        held: true,
        lasts: "form",
        at: null,
      });
      return;
    }
    const found = landing(link, {
      item: (id) => (!model.items.has(id) ? null : inArchived(model, id) ? "archived" : "shown"),
      thread: (id) => sorted.some((one) => one.id === id),
    });
    setWanted(null);
    // Landing from outside, nothing open has an entry yet: the one landed on becomes home.
    if (entries.current.pushed === 0)
      window.history.replaceState(window.history.state, "", addressOf(NOTHING, window.location.search));
    const to = found.wanted;
    setShowCommand(false);
    setMode({ kind: "idle" });
    setAdding(null);
    setShowProjects(false);
    setView(to.view);
    setSelected(null);
    if (to.item !== null) {
      if (found.archived) setShowArchived(true);
      // On the Map, brought into view; over another view, left open there, as it was when the address was written.
      if (to.view === "map") select(to.item, null);
      else selectOnly(to.item);
    }
    if (to.view === "map" && to.item === null && to.thread !== null) toThread(to.thread);
    else setThread(to.thread);
    // A link names what it opens where it is, so the user came from nowhere else.
    setCameFrom(null);
    setNeedsAt(to.needsAt);
    setShowNeeds(to.needs);
    const gone = found.missing.join(" and ");
    // An offered Undo waits wherever the user goes, a link included, and one a reload kept is offered again.
    setNotice((was) =>
      found.missing.length > 0
        ? {
            text: `That ${gone} ${found.missing.length > 1 ? "aren't" : "isn't"} in the plan any more`,
            offer: null,
            retry: null,
            lasts: "news",
            at: null,
          }
        : found.archived
          ? {
              text: "That item is archived, so archived items are showing",
              offer: "hide-archived",
              retry: null,
              lasts: "news",
              at: null,
            }
          : was?.lasts === "offer"
            ? was
            : null,
    );
  };
  // Tried again as the plan loads and as each form closes.
  useEffect(() => {
    if (wanted !== null) wantedOpens.current(wanted);
  }, [wanted, model, formsClosed]);

  /**
   * Copies the address of what `link` names, or, where there's no clipboard (a plain-http address away from this
   * machine), shows it selected to copy by hand. Whether it copied.
   */
  const copyLink = async (link: Wanted) => {
    const url = window.location.origin + addressOf(link);
    try {
      await navigator.clipboard.writeText(url);
      return true;
    } catch {
      setNotice({ text: "Copy this link:", offer: null, retry: null, link: url, lasts: "refusal", at: null });
      return false;
    }
  };

  if (load.state === "loading")
    return (
      <div className="status-page" aria-busy="true">
        <Wordmark />
        <p className="muted">Loading the plan…</p>
      </div>
    );
  if (load.state === "failed")
    return (
      <div className="status-page" role="alert">
        <Wordmark />
        <h1>Couldn't load the plan</h1>
        {load.code === "no-session" ? (
          // Nothing to try again until the browser is paired; coming back to the app after that reads again.
          <p>{load.message}.</p>
        ) : (
          <>
            <p>
              Check that <code>acyclic up</code> is still running where you paired this app, then try again.
            </p>
            <p className="muted">{load.message}</p>
            <button type="button" className="primary" onClick={refresh}>
              Try again
            </button>
          </>
        )}
      </div>
    );
  const titleOf = (id: string) => model?.items.get(id)?.title ?? id;
  const startAdd = (parent: string | null) => {
    setMode({ kind: "idle" });
    setAdding({ parent, idRefused: false, draft: null });
  };
  const add = (item: NewItem) => send(addCommand(item), { select: item.item, pin: item.parent }, { kind: "add", item });
  /**
   * Runs what the command menu chose, once it has closed. Going somewhere never closes a form holding what the user
   * typed: that says how to leave it instead, as Escape does.
   */
  const runCommand = (run: Run) => {
    setShowCommand(false);
    if (run.kind === "target") return choose(run.id === null ? { kind: "top" } : { kind: "item", id: run.id });
    if (run.kind === "archived") return setShowArchived((was) => !was);
    if (run.kind === "refresh") return refresh();
    if (document.querySelector("form[data-dirty='true']") !== null) return setNotice(UNSAVED);
    if (run.kind === "view") setView(run.view);
    else if (run.kind === "needs") openNeeds();
    else if (run.kind === "add") startAdd(null);
    else if (run.kind === "on-item") onItem(run.action);
    else if (run.kind === "thread") {
      // To the Map, framed, as every thread goes; over Needs you if it shows, so closing the thread comes back to it.
      setThreadOver(showNeeds ? run.id : null);
      toThread(run.id);
    } else select(run.id, null);
  };
  /**
   * Acts on the item whose panel shows, as its buttons do, while they would be enabled; a move or link, asked for
   * from the keyboard or the menu, finds its target in the menu, as the keyboard has no Map to tap.
   */
  const onItem = (action: OnItem) => {
    if (chosen === undefined || busy) return;
    if (action === "edit") setEditAsked((was) => was + 1);
    else if (action === "add-inside") startAdd(chosen.id);
    else {
      setMode(action === "move" ? { kind: "move", item: chosen.id } : { kind: "link", item: chosen.id, link: action });
      setShortcutsOnly(false);
      setShowCommand(true);
    }
  };
  /** Moves the focus to the next or previous row of the list in view: Needs you's entries, else the Overview's rows. */
  const moveRow = (by: 1 | -1) => {
    const rows = rowsInView();
    const here = document.activeElement instanceof HTMLElement ? document.activeElement.closest("[data-row]") : null;
    rows[nextRow(rows.length, here === null ? -1 : rows.indexOf(here as HTMLElement), by)]?.focus();
  };
  onKeyAction.current = (action) => {
    if (action.kind === "go") setView(action.view);
    else if (action.kind === "needs") openNeeds();
    else if (action.kind === "add") startAdd(null);
    else if (action.kind === "search" || action.kind === "help") {
      setShortcutsOnly(action.kind === "help");
      setShowCommand(true);
    } else if (action.kind === "edit") onItem("edit");
    else if (action.kind === "move") onItem("move");
    else if (action.kind === "link") onItem(action.link);
    else if (action.kind === "addInside") onItem("add-inside");
    else moveRow(action.by);
  };
  const picking = mode.kind === "idle" ? null : mode;
  const commandMenu = showCommand ? (
    <CommandMenu
      entries={menuEntries}
      keys={shortcuts ? SHORTCUTS : null}
      shortcutsOnly={shortcutsOnly && picking === null}
      placeholder={
        shortcutsOnly && picking === null
          ? "Keyboard shortcuts"
          : picking === null
            ? "Search the plan, or type a command"
            : picking.kind === "move"
              ? `Find where to put ${titleOf(picking.item)}`
              : `Find what ${titleOf(picking.item)} ${picking.link === "waits-for" ? "waits for" : "relates to"}`
      }
      working={working}
      lines={lines}
      kindOf={(id) => {
        const found = model?.items.get(id);
        return found?.progress != null ? "box" : (found?.stage ?? undefined);
      }}
      onRun={runCommand}
      onDismiss={() => closeTop.current(1)}
    />
  ) : null;
  const unlink = (from: string, kind: LinkKind, to: string) =>
    send(unlinkCommand(from, kind, to), { select: selected ?? from, pin: selected, undo: true });
  /** A refusal or failure, shown in the form or item it is about, else over the Map. */

  const noticeView =
    // One a reload brought back shows once the plan read since says it can still be undone.
    notice === null || notice.restored ? null : (
      // News is told by the status line below, always there, as a live region must be to be heard.
      <div className="notice" role={notice.polite ? undefined : "alert"} data-testid="notice" data-lasts={notice.lasts}>
        <span className={notice.link === undefined ? "grow" : undefined} title={notice.text}>
          {notice.text}
        </span>
        {notice.retry === null ? null : (
          <button
            type="button"
            disabled={busy}
            onClick={() => notice.retry && send(notice.retry.body, notice.retry.after)}
          >
            Try again
          </button>
        )}
        {notice.link === undefined ? null : (
          <input
            className="grow"
            readOnly
            value={notice.link}
            aria-label="Link"
            data-testid="link-to-copy"
            // Selected, so a long press or ⌘C copies it.
            ref={(field) => field?.select()}
            onFocus={(event) => event.target.select()}
          />
        )}
        {notice.offer === "undo" && notice.undo !== undefined ? (
          <button
            type="button"
            disabled={busy}
            aria-keyshortcuts={onMac() ? "Meta+Z" : "Control+Z"}
            onClick={() => notice.undo && runUndo(notice.undo)}
          >
            Undo
          </button>
        ) : null}
        {notice.offer === "hide-archived" ? (
          <button
            type="button"
            onClick={() => {
              setNotice(null);
              setShowArchived(false);
            }}
          >
            Hide archived
          </button>
        ) : null}
        {notice.offer === "refresh" ? (
          <button
            type="button"
            onClick={() => {
              setNotice(null);
              refresh();
            }}
          >
            Refresh
          </button>
        ) : null}
        <button
          type="button"
          className="close"
          aria-label="Dismiss"
          onClick={() => {
            if (notice.held) setWanted(null);
            setNotice(null);
          }}
        >
          <Icon of={X} />
        </button>
      </div>
    );
  const prompt =
    mode.kind === "move"
      ? `Choose the box to put ${titleOf(mode.item)} in`
      : mode.kind === "link"
        ? `Choose the item that ${titleOf(mode.item)} ${mode.link === "waits-for" ? "waits for" : "relates to"}`
        : null;
  const menu = (
    <>
      <label>
        <input
          type="checkbox"
          checked={showArchived}
          onChange={(event) => setShowArchived(event.target.checked)}
          data-testid="show-archived"
        />
        Show archived items
      </label>
      <Notify />
      <label>
        <input
          type="checkbox"
          checked={shortcuts}
          onChange={(event) => {
            setShortcuts(event.target.checked);
            localStorage.setItem(SHORTCUTS_KEY, event.target.checked ? "on" : "off");
          }}
          data-testid="shortcuts"
        />
        Single-key shortcuts
      </label>
      <label>
        Theme
        <select value={theme} onChange={(event) => setTheme(event.target.value as Theme)} data-testid="theme">
          <option value="system">System</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </select>
      </label>
      <Choice
        label="Autonomy, for threads without their own"
        testid="global-level"
        hint={levelHint(levelChosen ?? load.level)}
        value={levelChosen ?? load.level}
        disabled={false}
        onWrite={(level) => {
          setLevelChosen(level);
          void send(globalLevelCommand(level as Level), { select: null, pin: null, undo: true, background: true }).then(
            (saved) => {
              if (!saved) setLevelChosen(null);
            },
          );
        }}
      >
        {LEVELS.map((level) => (
          <option key={level} value={level}>
            {levelLabel(level)}
          </option>
        ))}
      </Choice>
      <button type="button" onClick={refresh}>
        Refresh
      </button>
    </>
  );
  /**
   * Needs you is a layer: pressed while another covers it, it comes back to the top, closing what was over it; on top,
   * it closes. Opening it closes the item and the outline's sheet, which would cover it.
   */
  const toggleNeeds = () => {
    const above = openLayers(open).indexOf("needs");
    if (above > 0) {
      closeTop.current(above);
      return;
    }
    // Over the item open, which shows again once it closes.
    setNeedsOver(showNeeds ? null : selected);
    setShowProjects(false);
    setThreadOver(null);
    setShowNeeds((was) => !was);
  };
  /** Opens Needs you, at the entry `key` names if given. */
  const openNeeds = (key?: string) => {
    setNeedsAt(key ?? null);
    if (!showNeeds || topLayer(open) !== "needs") toggleNeeds();
  };
  return (
    <Shell
      wide={wide}
      count={shown.length}
      autonomy={levelLabel(levelChosen ?? load.level)}
      view={view}
      onView={(to) => {
        // On a phone, Needs you fills the screen, so a view picked from the tabs would change nothing in sight.
        if (!wide) setShowNeeds(false);
        // An item is shown on the Map: going to another view leaves it, rather than its panel standing over a view
        // that doesn't show it.
        if (to !== "map") setSelected(null);
        setView(to);
      }}
      needs={load.needs.length}
      needsOpen={showNeeds}
      onNeeds={() => {
        setNeedsAt(null);
        toggleNeeds();
      }}
      onAdd={() => startAdd(null)}
      menu={menu}
      outline={
        outline === null || !wide ? null : (
          <Projects
            outline={outline}
            lines={lines}
            picked={thread}
            current={current}
            onPickThread={pickFromOutline}
            onSelectProject={(id) => select(id, null)}
          />
        )
      }
      outlineOpen={showProjects}
      onOutline={() => setShowProjects((was) => !was)}
      onSearch={() => {
        setShortcutsOnly(false);
        setShowCommand(true);
      }}
      shortcuts={shortcuts}
    >
      {/* Outside the stage, whose busy state would hold it back. */}
      <p className="visually-hidden" role="status">
        {notice?.polite ? notice.text : ""}
      </p>
      {/* The plan reads shown, for browser journeys to know a refresh has landed. */}
      <main className="stage" data-busy={busy} aria-busy={busy} data-reads={reads.current.shown}>
        <Suspense fallback={<p className="status">Loading the Map…</p>}>
          <PlanMap
            items={shown}
            showArchived={showArchived}
            thread={thread}
            lines={lines}
            palette={palette}
            selected={selected}
            onSelect={(id) => {
              setCameFrom(null);
              selectOnly(id);
            }}
            focus={focus}
            framed={framed}
            onPick={
              prompt === null ? null : (id) => choose(id === null ? { kind: "background" } : { kind: "item", id })
            }
          />
        </Suspense>
        {/* Wider, the sidebar or tabs show the view's name; it heads the page for assistive technology all the same. */}
        {wide && view !== "overview" ? <h1 className="visually-hidden">{view === "map" ? "Map" : "Threads"}</h1> : null}
        {view === "threads" && waves !== null ? (
          <ThreadsView view={waves} lines={lines} picked={thread} wide={wide} onPick={pickThread} />
        ) : null}
        {view === "overview" ? (
          <Overview
            needs={load.needs}
            lines={lines}
            titleOf={titleOf}
            counts={counts}
            waves={waves}
            outline={outline}
            items={itemsById}
            onNeeds={openNeeds}
            needsOpen={showNeeds}
            onPickThread={toThread}
            onSelectProject={(id) => select(id, null)}
            onView={setView}
          />
        ) : null}
        {view === "map" && shown.length === 0 ? (
          <div className="empty-plan" data-testid="empty-plan">
            <h2>No items yet</h2>
            <p className="muted">
              Add an item at the top level, then the work inside it. What agents import shows here too.
            </p>
            <button type="button" className="primary" onClick={() => startAdd(null)}>
              New item
            </button>
          </div>
        ) : null}
        {commandMenu}
        {panel === "mode" ? (
          <div className="sheet prompt" data-testid="prompt">
            <span className="grow">{prompt}</span>
            {mode.kind === "move" ? (
              <button type="button" onClick={() => choose({ kind: "top" })} data-action="top-level">
                Top level
              </button>
            ) : null}
            <button type="button" className="quiet" onClick={() => choose({ kind: "cancel" })}>
              Cancel
            </button>
          </div>
        ) : panel === "add" && adding !== null ? (
          <AddForm
            key={adding.parent ?? ""}
            parent={adding.parent}
            parentTitle={adding.parent === null ? null : titleOf(adding.parent)}
            // The thread picked, else the one of the item it goes in, else of the item selected, as the project is.
            thread={thread ?? model?.items.get(adding.parent ?? selected ?? "")?.thread ?? null}
            projects={outline?.projects ?? []}
            project={current.project}
            threads={sorted.map((one) => ({ id: one.id, label: lineLabel(lines.get(one.id), one.title) }))}
            taken={new Set(model?.items.keys())}
            idRefused={adding.idRefused}
            draft={adding.draft}
            busy={busy}
            onSubmit={add}
            onCancel={() => setAdding(null)}
          />
        ) : panel !== "item" || chosen === undefined ? null : (
          <Inspector
            key={`${chosen.id}:${reopened?.id === chosen.id ? reopened.n : 0}`}
            item={chosen}
            reopened={reopened?.id === chosen.id ? reopened.edited : null}
            titleOf={titleOf}
            stageOf={(id) => model?.items.get(id)?.stage ?? undefined}
            onSelect={(id) => select(id, selected)}
            editAsked={editAsked}
            shortcuts={shortcuts}
            // The item closes so its thread's panel, the bottom layer, shows; Back from the panel doesn't bring the item
            // back, as each layer has its one history entry.
            onPickThread={(id) => {
              setSelected(null);
              setThread(id);
            }}
            onUnlink={unlink}
            onMove={() => setMode({ kind: "move", item: chosen.id })}
            onLink={(link) => setMode({ kind: "link", item: chosen.id, link })}
            onAdd={() => startAdd(chosen.id)}
            onClose={closeItem}
            onDismiss={() => closeTop.current(1)}
            onCopyLink={() =>
              copyLink({ ...NOTHING, view: "map", item: chosen.id }).then((copied) => {
                if (copied)
                  setNotice({ text: "Link copied", offer: null, retry: null, polite: true, lasts: "news", at: null });
              })
            }
            onEditorClosed={formClosed}
            onCommand={(edit) => send(edit, { select: chosen.id, pin: chosen.id })}
            onSave={(edit, edited) =>
              send(edit, { select: chosen.id, pin: chosen.id, undo: true }, { kind: "edit", id: chosen.id, edited })
            }
            threads={sorted}
            lines={lines}
            busy={busy}
          />
        )}
        {panel === "needs" ? (
          <NeedsList
            needs={load.needs}
            titleOf={titleOf}
            lines={lines}
            membersOf={(id) => (model === null ? [] : membersOf(model, id))}
            busy={busy}
            onCommand={(edit, done, undo) => send(edit, { select: null, pin: null, done, ...(undo ? { undo } : {}) })}
            onSelect={(id) => {
              // Picked from the list, an item shows over it, even the one the list was opened over.
              setNeedsOver(null);
              select(id, null);
            }}
            onPickThread={(id) => {
              // To the Map, framed, as an item picked from the list goes, over the list, so closing it comes back.
              setThreadOver(id);
              toThread(id);
            }}
            onClose={() => setShowNeeds(false)}
            onDismiss={() => closeTop.current(1)}
            at={needsAt}
          />
        ) : null}
        {panel === "projects" && outline !== null ? (
          <ProjectsSheet onClose={() => setShowProjects(false)} onDismiss={() => closeTop.current(1)}>
            <Projects
              outline={outline}
              lines={lines}
              picked={thread}
              current={current}
              onPickThread={(id) => {
                setShowProjects(false);
                pickFromOutline(id);
              }}
              onSelectProject={(id) => {
                setShowProjects(false);
                select(id, null);
              }}
            />
          </ProjectsSheet>
        ) : null}
        {panel === "thread" && thread !== null ? (
          <ThreadPanel
            key={thread}
            id={thread}
            globalLevel={levelChosen ?? load.level}
            awaiting={awaiting.has(thread)}
            line={lines.get(thread)}
            counts={counts.get(thread)}
            titleOf={titleOf}
            stageOf={(id) => model?.items.get(id)?.stage ?? undefined}
            waitsFor={(id) => model?.items.get(id)?.waitsFor ?? []}
            version={version}
            busy={busy}
            onCommand={(edit, done, undo) =>
              send(edit, {
                select: null,
                pin: null,
                ...(done === undefined ? {} : { done }),
                ...(undo ? { undo } : {}),
              })
            }
            onSetting={(edit) => send(edit, { select: null, pin: null, background: true })}
            onSelect={(id) => select(id, null)}
            onClose={closeThread}
            onDismiss={() => closeTop.current(1)}
            onCopyLink={() => copyLink({ ...NOTHING, view: "threads", thread })}
          />
        ) : null}
      </main>
      {/* The slot, where every notice shows: at the stage's foot, in its flow, so it covers nothing and a sheet rises
          above it. */}
      {noticeView === null ? null : <div className="slot">{noticeView}</div>}
    </Shell>
  );
}
