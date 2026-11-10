import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { AddForm } from "./AddForm";
import { CommandMenu } from "./CommandMenu";
import { entries as commandEntries, type OnItem, onMac, opensMenu, type Run, targets, undoes } from "./commands";
import { addCommand, type NewItem, type Pick, pick, unlinkCommand } from "./edits";
import {
  boundary,
  type Cursor,
  type HistoryHint,
  type HistoryPlan,
  plan as planHistory,
  readHistory,
  replaceDepth,
} from "./history";
import { InitiativeSwitcher } from "./InitiativeSwitcher";
import { Inspector } from "./Inspector";
import { Icon, X } from "./icons";
import {
  contextOf,
  contextTitle,
  crossingWaits,
  emptyThreadIds,
  inContext,
  type ListScope,
  needInitiatives,
  rowLabel,
  startProject,
  switcherRows,
  threadInitiatives,
  threadsIn,
} from "./initiatives";
import { type KeyAction, nextRow, SHORTCUTS, step, typingIn, type Waiting } from "./keys";
import { byTitle, linesOf } from "./lines";
import { buildModel } from "./map/model";
import { NeedsList } from "./NeedsList";
import { NoticePlacement } from "./NoticePlacement";
import { Notify } from "./Notify";
import {
  addressOf,
  type Intent,
  landing,
  landingPlace,
  NOTHING,
  navigate,
  openLayers,
  openOf,
  type Place,
  parse,
  placesByDepth,
  type Requests,
  START,
  type Step,
  samePlace,
  stored,
  threadIntent,
  topLayer,
  type Wanted,
  type World,
} from "./nav";
import type { Seen } from "./notices";
import { Overview } from "./Overview";
import { currentRows, outlineOf } from "./outline";
import { PickBar } from "./PickBar";
import { Projects, ProjectsSheet } from "./Projects";
import { PALETTES, setTheme, type Theme, useScheme, useTheme } from "./palette";
import { drawn as drawnOver } from "./reads";
import { Shell, type View, Wordmark } from "./Shell";
import { ThreadPanel } from "./ThreadPanel";
import { ThreadsView } from "./ThreadsView";
import { workingThreads } from "./threadLists";
import { createTracker, type Notice } from "./tracker";
import { outsideOf, scopeThreads, wavesView } from "./waves";
import { useWide } from "./wide";

// The Map's code (the renderer and the layout engine) is the bulk of the app; it loads while the tracker does.
const mapModule = import("./map/PlanMap");
const PlanMap = lazy(() => mapModule);

/** Where this device keeps whether single-key shortcuts are on. */
const SHORTCUTS_KEY = "acyclic.shortcuts";

/** What a form holding typing says when the user tries to leave it. */
const UNSAVED: Notice = {
  text: "Unsaved changes: save them, or cancel",
  offer: null,
  retry: null,
  lasts: "form",
  at: null,
};

/** What opens as a link does: an address, or the place a history entry holds. */
type Opening = Wanted & {
  place?: Place;
  source: "initial" | "pop" | "message";
  cursor: Cursor | null;
  fresh: boolean;
  wide: boolean | null;
};

function historyWide(raw: unknown): boolean | null {
  if (typeof raw !== "object" || raw === null || !("wide" in raw)) return null;
  return typeof raw.wide === "boolean" ? raw.wide : null;
}

function openingAt(source: Opening["source"], raw: unknown, search: string): Opening {
  const address = parse(search);
  const entry = readHistory(raw);
  const place = entry?.place;
  const matchingPlace = place !== undefined && place !== null && addressOf(place) === addressOf(address) ? place : null;
  return {
    ...address,
    ...(matchingPlace === null ? {} : { place: matchingPlace }),
    source,
    cursor: matchingPlace === null ? null : (entry?.cursor ?? null),
    fresh: source === "initial" && raw === null,
    wide: historyWide(raw),
  };
}

/** The tab's storage, where the Undo on offer is kept over a reload, or null where the browser blocks it. */
function storageOrNull(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

/** Whether a form holds what the user typed, which nothing closes unasked. */
const dirtyForm = () => document.querySelector("form[data-dirty='true']") !== null;

/** The rows J and K step through: those of the view in the page's main area, the Overview's or Needs you's. */
function rowsInView(): HTMLElement[] {
  return [...(document.querySelector("main")?.querySelectorAll<HTMLElement>("[data-row]") ?? [])];
}

export function App() {
  const noticeStage = useRef<HTMLElement>(null);
  const noticeChrome = useRef<HTMLDivElement>(null);
  const selectNoticeLink = useCallback(
    (field: HTMLInputElement | null) => field?.setSelectionRange(0, field.value.length),
    [],
  );
  /**
   * Where the user is, and what the moves since the last render asked of what is drawn. `placeNow` is the place as of
   * the latest move, read by the next one before a render, so moves made one after another, in a handler or across a
   * write's awaits, each start where the last left; and the requests they make before a render all reach it, the
   * latest of each kind winning.
   */
  const [nav, setNav] = useState<Step>({ place: START, requests: {} });
  const placeNow = useRef(START);
  const askedNow = useRef<Requests>({});
  useLayoutEffect(() => {
    askedNow.current = {};
  });
  /** What the moves read of the plan and the screen: kept current at each render. */
  const worldNow = useRef<Omit<World, "dirty">>({
    has: () => false,
    threadOf: () => null,
    initiativeOf: () => null,
    initiativesOf: () => [],
    contextOf: (chosen) => chosen,
    wide: false,
  });
  const historyMove = useRef<(place: Place, hint: HistoryHint, retainPurpose?: boolean) => void>(() => undefined);
  const go = useCallback((intent: Intent, hint?: HistoryHint): Step => {
    const before = placeNow.current;
    const step = navigate(before, intent, { ...worldNow.current, dirty: dirtyForm() });
    const { unsaved: _, ...asked } = step.requests;
    placeNow.current = step.place;
    askedNow.current = { ...askedNow.current, ...asked };
    setNav({ place: step.place, requests: askedNow.current });
    if (!step.requests.unsaved) {
      const purpose =
        hint ?? (intent.kind === "follow" && !samePlace(stored(before), stored(step.place)) ? "visit" : "ordinary");
      historyMove.current(step.place, purpose, ["wrote", "leaveNeeds", "edit", "reopen"].includes(intent.kind));
    }
    return step;
  }, []);
  const [preview, setPreview] = useState<string | null>(null);
  const { place, requests } = nav;
  const { view, item: selected, thread, needsAt } = place;
  const { command: showCommand, mode, add: adding, projects: showProjects } = place.overlays;
  /** The command menu lists only what has a key, as ? asks. */
  const [shortcutsOnly, setShortcutsOnly] = useState(false);
  /** What the menu starts with: the key typed at a pick bar, which finds the target. */
  const [findFirst, setFindFirst] = useState("");
  /** Single-key shortcuts, on unless switched off on this device. */
  const [shortcuts, setShortcuts] = useState(() => localStorage.getItem(SHORTCUTS_KEY) !== "off");
  const wide = useWide();
  /**
   * The tracker: the plan, the write out, and the one notice slot. Its moves of where the user is go
   * through `goNow`, so they start from where the user is by then.
   */
  const goNow = useRef<(intent: Intent) => void>(go);
  goNow.current = go;
  const [tracker] = useState(() =>
    createTracker({
      fetcher: (input, init) => fetch(input, init),
      storage: storageOrNull(),
      nav: { go: (intent) => goNow.current(intent), item: () => placeNow.current.item },
    }),
  );
  const { load, notice, busy, pending, version, reads } = useSyncExternalStore(tracker.subscribe, tracker.snapshot);
  const { refresh, send, tell } = tracker;
  /**
   * An address to open: the one the app was opened at, one a notification sent the app already open, or the place a
   * history entry Forward landed on holds, with what it names. It waits for the plan to load, and for a form holding
   * what the user typed to close.
   */
  const [wanted, setWanted] = useState<Opening | null>(() =>
    openingAt("initial", window.history.state, window.location.search),
  );
  const initialFresh = useRef(window.history.state === null);
  const waitingLink = useRef(wanted !== null);
  waitingLink.current = wanted !== null;
  /** Counts forms closed, so a link waiting on one opens once it has. */
  const [formsClosed, setFormsClosed] = useState(0);

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
  const shown = useMemo(
    () => (loaded === null ? [] : drawnOver(loaded.items, loaded.unread, pending)),
    [loaded, pending],
  );
  const sorted = useMemo(() => (loaded === null ? [] : [...loaded.threads.threads].sort(byTitle)), [loaded]);
  const palette = PALETTES[useScheme()];
  const theme = useTheme();
  const lines = useMemo(() => linesOf(sorted, palette), [sorted, palette]);
  const model = useMemo(() => (loaded === null ? null : buildModel(shown)), [loaded, shown]);
  const chosen = selected === null ? undefined : shown.find((one) => one.id === selected);
  const outline = useMemo(
    () =>
      model === null
        ? null
        : outlineOf(
            model,
            sorted.map((one) => one.id),
          ),
    [model, sorted],
  );
  /** Each thread's leaves by stage, as core counts them: read with the plan. */
  const counts = useMemo(
    () => new Map(loaded === null ? [] : loaded.threads.threads.map((one) => [one.id, one.counts])),
    [loaded],
  );
  const initiativeTitles = useMemo(
    () => new Map((loaded?.initiatives ?? []).map((one) => [one.id, one.title])),
    [loaded],
  );
  /** The switcher's rows, and the initiative in context: the one chosen, else the first. */
  const switcher = useMemo(() => switcherRows(loaded?.initiatives ?? [], model), [loaded, model]);
  const context = contextOf(place.initiative, switcher);
  const ofThreads = useMemo(
    () => (model === null ? new Map<string, Set<string | null>>() : threadInitiatives(model)),
    [model],
  );
  const scope = useMemo<ListScope>(() => {
    const none: ReadonlySet<string | null> = new Set([null]);
    return {
      order: switcher.map((one) => one.id),
      context,
      titles: new Map(switcher.map((one) => [one.id, one.title])),
      ofThread: (id) => ofThreads.get(id) ?? none,
      ofNeed: (need) => needInitiatives(need, model, ofThreads),
      ofProject: (id) => new Set([model?.items.get(id)?.initiative ?? null]),
    };
  }, [switcher, context, model, ofThreads]);
  // The Map, the Threads view and the sidebar's epics show the initiative in context alone; the lists show every one.
  const inScope = useMemo(() => inContext(shown, context), [shown, context]);
  const scopedModel = useMemo(() => (loaded === null ? null : buildModel(inScope)), [loaded, inScope]);
  const scopedThreads = useMemo(() => threadsIn(sorted, ofThreads, context), [sorted, ofThreads, context]);
  const scopedOutline = useMemo(
    () =>
      scopedModel === null
        ? null
        : outlineOf(
            scopedModel,
            scopedThreads.map((one) => one.id),
          ),
    [scopedModel, scopedThreads],
  );
  const chooseInitiative = (id: string) => {
    if (go({ kind: "initiative", id }).requests.unsaved) tell(UNSAVED);
  };
  const itemsById = useMemo(() => new Map(shown.map((item) => [item.id, item])), [shown]);
  /** The waits of items in the initiative in context on items in another, which the Map draws as stubs. */
  const outsideWaits = useMemo(
    () => crossingWaits(shown, context, initiativeTitles),
    [shown, context, initiativeTitles],
  );
  const waves = useMemo(() => {
    if (loaded === null || scopedOutline === null || model === null) return null;
    const keep = new Set(scopedThreads.map((one) => one.id));
    const none: ReadonlySet<string | null> = new Set([null]);
    // A thread in another initiative is named by where it is; a tile names its tasks there and the waves it comes after.
    const outside = outsideOf(
      loaded.threads,
      keep,
      model,
      context,
      (id) => rowLabel(ofThreads.get(id) ?? none, initiativeTitles, context),
      initiativeTitles,
    );
    return wavesView(
      scopeThreads(loaded.threads, keep),
      scopedOutline,
      outside,
      emptyThreadIds(scopedThreads, ofThreads),
    );
  }, [loaded, scopedOutline, scopedThreads, model, context, ofThreads, initiativeTitles]);
  const working = useMemo(() => new Set(workingThreads(counts)), [counts]);
  worldNow.current = {
    has: (id) => model?.items.has(id) ?? false,
    threadOf: (id) => model?.items.get(id)?.thread ?? null,
    initiativeOf: (id) => model?.items.get(id)?.initiative ?? null,
    initiativesOf: (thread) => [...(ofThreads.get(thread) ?? [])].filter((id): id is string => id !== null),
    contextOf: (chosen) => contextOf(chosen, switcher),
    wide,
  };
  const current = useMemo(
    () =>
      model === null || scopedOutline === null
        ? { project: null, thread: null }
        : currentRows(model, scopedOutline, selected),
    [model, scopedOutline, selected],
  );
  // An item that went away, deleted or moved out of reach elsewhere, is no longer
  // selected: what's selected is drawn, so the panel and the address never name an item the Map doesn't show.
  useEffect(() => {
    if (model === null || selected === null) return;
    if (!model.items.has(selected)) go({ kind: "gone" });
  }, [model, selected, go]);

  /** An item chosen anywhere but on the Map: to the Map for it, brought into view with `pin` held still. */
  const select = useCallback((id: string, pin: string | null = null) => go({ kind: "select", id, pin }), [go]);
  // A thread of one is a task alone: its row opens the task, as any item's does, not a thread's panel.
  const taskOf = useMemo(
    () => new Map((loaded?.threads.threads ?? []).flatMap((one) => (one.task === null ? [] : [[one.id, one.task]]))),
    [loaded],
  );
  /** What a thread's row asks for: the Map for the thread, or for the task a thread of one stands for. */
  const forThread = useCallback(
    (id: string, kind: "thread" | "outline" = "thread") => threadIntent(taskOf, id, kind),
    [taskOf],
  );
  /** To the Map for a thread, as its row or tile anywhere does: picked, its panel open, and its items framed. */
  const toThread = useCallback((id: string) => go(forThread(id)), [go, forThread]);

  /** A stub's item opened, from the Map or the Threads view: in its own initiative, which is put in context. */
  const openOutside = (id: string) => {
    if (go({ kind: "follow", id, pin: selected }).requests.unsaved) tell(UNSAVED);
  };

  const choose = useCallback(
    (target: Pick) => {
      if (model === null) return;
      const step = pick(model, mode, target);
      go({ kind: "mode", mode: step.mode });
      const refusal = step.refusal;
      // A target refused says why where the user chose it; one taken drops what an earlier choice was refused.
      const was = tracker.snapshot().notice;
      tell(
        refusal !== null
          ? { text: refusal, offer: null, retry: null, lasts: "refusal", at: null }
          : was?.lasts === "refusal"
            ? null
            : was,
      );
      if (step.command === null || mode.kind === "idle") return;
      // A move pins the new parent; a link, the item linked from.
      const pin = mode.kind === "move" && target.kind === "item" ? target.id : mode.item;
      send(step.command, { select: mode.item, pin, undo: true });
    },
    [model, mode, send, go, tracker, tell],
  );
  // The key that opened the find is for that opening only: the next menu starts empty.
  useEffect(() => {
    if (!showCommand) setFindFirst("");
  }, [showCommand]);
  const open = openOf(place, worldNow.current);
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
      name: lines.get(one.id)?.name ?? one.id,
    }));
    const item = panel === "item" && selected !== null ? model.items.get(selected) : undefined;
    const plan = { model, threads, working, item: item === undefined ? null : item };
    return mode.kind === "idle" ? commandEntries(plan) : targets(plan, mode.kind, mode.item);
  }, [showCommand, model, sorted, lines, working, mode, panel, selected]);
  /** Drops what was said of an unsaved form, once the form has gone. */
  const formClosed = useCallback(() => {
    tracker.event({ kind: "form-closed" });
    setFormsClosed((was) => was + 1);
  }, [tracker]);
  // What's on top changing takes the add form, or the item's editor, with it.
  useEffect(formClosed, [panel]);
  /** Where the user is, which says how long a notice stays. */
  const shownItem = chosen?.id ?? null;
  const seen: Seen = useMemo(() => ({ layer: panel, item: shownItem, thread, view }), [panel, shownItem, thread, view]);
  // A notice is first shown where the user is, and kept or dropped as they move. As an update of the notice as it
  // is by then, so it never brings back one another effect of the same move cleared, such as a closed form's.
  useEffect(() => {
    if (notice !== null) tracker.event({ kind: "moved", to: seen });
  }, [notice, seen, tracker]);
  /**
   * Closes `count` layers from the top, as Escape (one) and Back (one per entry it went back over) do, but never a
   * form holding what the user typed: that says how to leave it instead. How many it closed.
   */
  const closeTop = useRef<(count: number) => number>(() => 0);
  closeTop.current = (count) => {
    const closing = openLayers(openOf(placeNow.current, worldNow.current)).slice(0, count);
    if (go({ kind: "close", count }).requests.unsaved) {
      // A link waiting for the form says so already, and dismissing that drops it.
      if (!tracker.snapshot().notice?.held) tell(UNSAVED);
      return 0;
    }
    return closing.length;
  };
  /** What the keys read of the app as it is: kept current at each render, for the one listener. */
  const keysSee = useRef({ shortcuts, panel, menu: showCommand });
  keysSee.current = { shortcuts, panel, menu: showCommand };
  /** Whether an Undo is on offer, its notice shown and nothing being sent: what ⌘Z takes back. */
  const undoNow = useRef(false);
  undoNow.current = notice?.offer === "undo" && notice.undo !== undefined && !notice.restored && !busy;
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
        const blocked = undo === null || keysSee.current.menu || typingIn(event.target) || dirtyForm();
        if (!blocked) {
          event.preventDefault();
          tracker.undo();
        }
      } else if (opensMenu(event, mac)) {
        event.preventDefault();
        setShortcutsOnly(false);
        go({ kind: "command", open: !placeNow.current.overlays.command });
      } else {
        const seen = keysSee.current;
        const next = step(pending, event, {
          enabled: seen.shortcuts,
          panel: seen.panel,
          menu: seen.menu,
          dirty: dirtyForm(),
          typing: typingIn(event.target),
          picking: event.target instanceof HTMLElement && event.target.dataset.pickBar !== undefined,
          rows: rowsInView().length,
        });
        if (next.action !== null || next.pending !== null) event.preventDefault();
        pending = next.pending;
        if (next.action !== null) onKeyAction.current(next.action);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [go]);
  // Browser positions count visits; parent positions say which visit closing a layer returns to.
  const cursor = useRef<Cursor | null>(null);
  const ownBack = useRef<{ place: Place; hint: HistoryHint; expectedPosition: number } | null>(null);
  const writeEntry = (kind: "replace" | "push", at: Cursor, place: Place) => {
    const state = { ...at, place, wide: worldNow.current.wide };
    const url = addressOf(place, window.location.search);
    if (kind === "replace") window.history.replaceState({ ...window.history.state, ...state }, "", url);
    else window.history.pushState(state, "", url);
  };
  const applyHistory = (planned: HistoryPlan, desired: Place, hint: HistoryHint) => {
    cursor.current = planned.cursor;
    for (const effect of planned.effects) {
      if (effect.kind === "go") {
        ownBack.current = { place: desired, hint, expectedPosition: planned.cursor.position + effect.offset };
        window.history.go(effect.offset);
      } else writeEntry(effect.kind, effect.cursor, effect.place);
    }
  };
  const driveHistory = (desired: Place, hint: HistoryHint, retainPurpose = false) => {
    if (model === null || waitingLink.current || cursor.current === null) return;
    if (ownBack.current !== null) {
      ownBack.current = { ...ownBack.current, place: desired, hint: retainPurpose ? ownBack.current.hint : hint };
      return;
    }
    // A write can name a new item before the refreshed model draws it.
    const drawable =
      desired.item !== null && !worldNow.current.has(desired.item) ? { ...desired, item: null } : desired;
    const projected = placesByDepth(drawable, { ...worldNow.current, dirty: false });
    const source = readHistory(window.history.state)?.place;
    const sourceWide = historyWide(window.history.state);
    const sourceProjected =
      source !== undefined && source !== null && sourceWide !== null
        ? placesByDepth(source, { ...worldNow.current, wide: sourceWide, dirty: false })
        : undefined;
    const planned = planHistory(cursor.current, projected, hint, window.history.length, sourceProjected);
    if (planned !== null) applyHistory(planned, desired, hint);
  };
  historyMove.current = driveHistory;
  useLayoutEffect(() => {
    historyMove.current(placeNow.current, "ordinary", true);
  }, [model, wide, place, wanted]);
  useEffect(() => {
    const onMove = (event: PopStateEvent) => {
      const pending = ownBack.current;
      const landed = openingAt("pop", event.state, window.location.search);
      ownBack.current = null;
      if (
        pending !== null &&
        landed.cursor !== null &&
        landed.wide !== null &&
        landed.cursor.position === pending.expectedPosition
      ) {
        cursor.current = landed.cursor;
        // A queued visit pushes from the landed entry; it must not overwrite that entry first.
        historyMove.current(pending.place, pending.hint);
      } else {
        waitingLink.current = true;
        setWanted(landed);
      }
    };
    window.addEventListener("popstate", onMove);
    return () => window.removeEventListener("popstate", onMove);
  }, []);

  const here: Wanted = {
    view,
    item: chosen?.id ?? null,
    thread,
    needsAt: view === "needs" ? needsAt : null,
    initiative: place.initiative,
  };
  const hereNow = useRef(here);
  hereNow.current = here;

  // A notification's link, sent to the app already open rather than reloading it: answered at once, so the service
  // worker knows not to, then opened as any link is.
  useEffect(() => {
    const container = navigator.serviceWorker;
    if (container === undefined) return;
    const onMessage = (event: MessageEvent) => {
      if (event.data?.kind !== "open" || typeof event.data.search !== "string") return;
      event.ports[0]?.postMessage("opened");
      // A newer link replaces one still waiting. One naming no view, such as a thread's, opens over the view shown, so
      // closing it goes back to where the user was; Needs you's, bare `needs`, names its view.
      const asked = parse(event.data.search);
      const query = new URLSearchParams(event.data.search);
      const names = ["view", "item", "needs"].some((key) => query.has(key));
      waitingLink.current = true;
      setWanted({
        ...asked,
        view: names ? asked.view : hereNow.current.view,
        source: "message",
        cursor: null,
        fresh: false,
        wide: null,
      });
    };
    container.addEventListener("message", onMessage);
    container.startMessages();
    return () => container.removeEventListener("message", onMessage);
  }, []);

  // A link opens once the plan is loaded, over home, with each layer it opens its own entry above, so Back closes them
  // one by one as if the user had opened them. A form holding what the user typed is never closed for it: the link
  // waits, said so, until the form is saved or cancelled.
  const wantedOpens = useRef<(link: Opening) => void>(() => undefined);
  wantedOpens.current = (link) => {
    if (model === null) return;
    const name =
      link.item !== null
        ? (model.items.get(link.item)?.title ?? link.item)
        : link.thread !== null
          ? (lines.get(link.thread)?.name ?? link.thread)
          : link.view === "needs"
            ? "Needs you"
            : link.view === "map"
              ? "the Map"
              : link.view === "threads"
                ? "Threads"
                : "the Overview";
    if (dirtyForm()) {
      tell({
        text: `Unsaved changes: save or cancel, then ${name} opens`,
        offer: null,
        retry: null,
        held: true,
        lasts: "form",
        at: null,
      });
      return;
    }
    const has = {
      item: (id: string) => model.items.has(id),
      thread: (id: string) => sorted.some((one) => one.id === id),
    };
    // A place an entry held lands as a link does, gone since.
    const found = link.place === undefined ? landing(link, has) : landingPlace(link.place, has);
    if ("place" in found) {
      go({ kind: "restore", place: found.place });
    } else {
      go({ kind: "link", wanted: found.wanted });
    }
    const projected = placesByDepth(placeNow.current, { ...worldNow.current, dirty: false });
    const actualDepth = projected.length - 1;
    const destination = projected[actualDepth];
    if (destination !== undefined && (link.source !== "message" || cursor.current === null)) {
      if (link.fresh || (link.source === "message" && cursor.current === null && initialFresh.current)) {
        const base = boundary(0);
        if (base !== null) {
          const planned = planHistory(base, projected, "ordinary", window.history.length);
          if (planned !== null) applyHistory(planned, placeNow.current, "ordinary");
        }
      } else {
        cursor.current =
          link.cursor !== null && link.wide === worldNow.current.wide && actualDepth <= link.cursor.layers
            ? replaceDepth(link.cursor, actualDepth)
            : boundary(actualDepth);
        if (cursor.current !== null) writeEntry("replace", cursor.current, destination);
      }
    }
    waitingLink.current = false;
    setWanted(null);
    const gone = found.missing.join(" and ");
    // An offered Undo waits wherever the user goes, a link included, and one a reload kept is offered again.
    const was = tracker.snapshot().notice;
    tell(
      found.missing.length > 0
        ? {
            text: `That ${gone} ${found.missing.length > 1 ? "aren't" : "isn't"} in the plan any more`,
            offer: null,
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
      tell({ text: "Copy this link:", offer: null, retry: null, link: url, lasts: "refusal", at: null });
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
        <p>
          Check that <code>acyclic up</code> is still running, then try again.
        </p>
        <p className="muted">{load.message}</p>
        <button type="button" className="primary" onClick={refresh}>
          Try again
        </button>
      </div>
    );
  const titleOf = (id: string) => model?.items.get(id)?.title ?? initiativeTitles.get(id) ?? id;
  const startAdd = (parent: string | null) => go({ kind: "add", parent });
  /**
   * Adds an item and, once saved, shows it on the Map, selected where it landed; closing it goes back to the view it was
   * added from. A refused add reopens its form where the user was.
   */
  const add = async (item: NewItem) => {
    const from = placeNow.current.view;
    const saved = await send(addCommand(item), { select: item.id, pin: item.parent }, { kind: "add", item });
    if (saved) go({ kind: "added", id: item.id, from });
    return saved;
  };
  /**
   * Runs what the command menu chose, once it has closed. Going somewhere never closes a form holding what the user
   * typed: that says how to leave it instead, as Escape does.
   */
  const runCommand = (run: Run) => {
    if (run.kind === "target" || run.kind === "refresh" || dirtyForm()) {
      go({ kind: "shut", layer: "command" });
      if (run.kind === "target") return choose({ kind: "item", id: run.id });
      if (run.kind === "refresh") return refresh();
      return tell(UNSAVED);
    }
    const under = {
      ...placeNow.current,
      overlays: { ...placeNow.current.overlays, command: false },
    };
    const move = (intent: Intent, newOverlay = false) => {
      const next = navigate(under, intent, { ...worldNow.current, dirty: false }).place;
      const different = newOverlay || !samePlace(stored(under), stored(next));
      // An outstanding close has already given its transient entry back; its result branches after arrival.
      const hint: HistoryHint = different ? (ownBack.current === null ? "replaceTransient" : "visit") : "ordinary";
      placeNow.current = under;
      go(intent, hint);
    };
    if (run.kind === "view") move({ kind: "goTo", view: run.view, at: null });
    else if (run.kind === "add") move({ kind: "add", parent: null }, true);
    else if (run.kind === "on-item") {
      placeNow.current = under;
      onItem(run.action, run.action === "edit" ? "ordinary" : ownBack.current === null ? "replaceTransient" : "visit");
      if (chosen === undefined || busy) go({ kind: "shut", layer: "command" });
    } else if (run.kind === "thread") move(forThread(run.id));
    else move({ kind: "select", id: run.id, pin: null });
  };
  /**
   * Acts on the item whose panel shows, as its buttons do, while they would be enabled; a move or link, asked for
   * from the keyboard or the menu, finds its target in the menu, as the keyboard has no Map to tap.
   */
  const onItem = (action: OnItem, hint?: HistoryHint) => {
    if (chosen === undefined || busy) return;
    if (action === "edit") go({ kind: "edit" }, hint);
    else if (action === "add-inside") go({ kind: "add", parent: chosen.id }, hint);
    else {
      go(
        {
          kind: "mode",
          mode: action === "move" ? { kind: "move", item: chosen.id } : { kind: "link", item: chosen.id },
        },
        hint,
      );
      setShortcutsOnly(false);
      go({ kind: "command", open: true });
    }
  };
  /** Moves the focus to the next or previous row of the list in view: the Overview's rows, or Needs you's entries. */
  const moveRow = (by: 1 | -1) => {
    const rows = rowsInView();
    const here = document.activeElement instanceof HTMLElement ? document.activeElement.closest("[data-row]") : null;
    rows[nextRow(rows.length, here === null ? -1 : rows.indexOf(here as HTMLElement), by)]?.focus();
  };
  onKeyAction.current = (action) => {
    if (action.kind === "go") goTo(action.view);
    else if (action.kind === "add") startAdd(null);
    else if (action.kind === "search" || action.kind === "help" || action.kind === "find") {
      setShortcutsOnly(action.kind === "help");
      setFindFirst(action.kind === "find" ? action.text : "");
      go({ kind: "command", open: true });
    } else if (action.kind === "edit") onItem("edit");
    else if (action.kind === "move") onItem("move");
    else if (action.kind === "link") onItem("waits-for");
    else if (action.kind === "addInside") onItem("add-inside");
    else moveRow(action.by);
  };
  const picking = mode.kind === "idle" ? null : mode;
  const commandMenu = showCommand ? (
    <CommandMenu
      entries={menuEntries}
      keys={shortcuts ? SHORTCUTS : null}
      shortcutsOnly={shortcutsOnly && picking === null}
      initial={findFirst}
      placeholder={
        shortcutsOnly && picking === null
          ? "Keyboard shortcuts"
          : picking === null
            ? "Search the plan, or type a command"
            : picking.kind === "move"
              ? `Find where to put ${titleOf(picking.item)}`
              : `Find what ${titleOf(picking.item)} waits for`
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
  const unlink = (from: string, to: string) =>
    send(unlinkCommand(from, to), { select: selected ?? from, pin: selected, undo: true });
  /** A refusal or failure, shown in the form or item it is about, else over the Map. */

  const noticeView =
    // One a reload brought back shows once the plan read since says it can still be undone.
    notice === null || notice.restored ? null : (
      <div className="notice" data-testid="notice" data-lasts={notice.lasts}>
        <span className={notice.link === undefined ? "grow" : undefined} title={notice.text}>
          {notice.text}
        </span>
        {notice.retry === null ? null : (
          <button type="button" disabled={busy} onClick={() => tracker.retry()}>
            Try again
          </button>
        )}
        {notice.link === undefined ? null : (
          <input
            key={notice.link}
            className="grow"
            readOnly
            value={notice.link}
            aria-label="Link"
            data-testid="link-to-copy"
            // Selected, so a long press or ⌘C copies it.
            ref={selectNoticeLink}
            onClick={(event) => event.currentTarget.select()}
          />
        )}
        {notice.offer === "undo" && notice.undo !== undefined ? (
          <button
            type="button"
            disabled={busy}
            aria-keyshortcuts={onMac() ? "Meta+Z" : "Control+Z"}
            onClick={() => tracker.undo()}
          >
            Undo
          </button>
        ) : null}
        {notice.offer === "refresh" ? (
          <button
            type="button"
            onClick={() => {
              tracker.dismiss();
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
            if (notice.held) {
              waitingLink.current = false;
              setWanted(null);
              const projected = placesByDepth(placeNow.current, { ...worldNow.current, dirty: false });
              const destination = projected.at(-1);
              cursor.current = boundary(projected.length - 1);
              if (destination !== undefined && cursor.current !== null)
                writeEntry("replace", cursor.current, destination);
            }
            tracker.dismiss();
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
        ? `Choose the item that ${titleOf(mode.item)} waits for`
        : null;
  const menu = (
    <>
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
      <button type="button" onClick={refresh}>
        Refresh
      </button>
    </>
  );
  /** Goes to a view, every way the user asks to: the tabs and sidebar, keys, the command menu, the Overview's links. */
  const goTo = (to: View, at: string | null = null) => go({ kind: "goTo", view: to, at });
  return (
    <Shell
      noticeHost={noticeChrome}
      wide={wide}
      count={shown.length}
      view={view}
      onView={goTo}
      needs={load.needs.length}
      onAdd={() => startAdd(null)}
      menu={menu}
      initiatives={
        !wide || switcher.length === 0 ? null : (
          <InitiativeSwitcher rows={switcher} context={context} onChoose={chooseInitiative} />
        )
      }
      // With one initiative there is nothing to tell apart, so the bar leaves it unsaid.
      context={wide || switcher.length < 2 ? null : contextTitle(context, switcher)}
      outline={
        scopedOutline === null || !wide ? null : (
          <Projects
            outline={scopedOutline}
            initiative={contextTitle(context, switcher)}
            lines={lines}
            picked={thread}
            current={current}
            onPickThread={(id) => go(forThread(id, "outline"))}
            onSelectProject={(id) => select(id, null)}
          />
        )
      }
      outlineOpen={showProjects}
      onOutline={() => go({ kind: "projects", open: !placeNow.current.overlays.projects })}
      onSearch={() => {
        setShortcutsOnly(false);
        go({ kind: "command", open: true });
      }}
      shortcuts={shortcuts}
    >
      {/* Outside the stage, whose busy state would hold it back. */}
      <p className="visually-hidden" role="status">
        {notice?.polite ? notice.text : ""}
      </p>
      <p className="visually-hidden" role="alert">
        {notice !== null && !notice.polite ? notice.text : ""}
      </p>
      {/* The plan reads shown, for browser journeys to know a refresh has landed. */}
      <main ref={noticeStage} className="stage" data-busy={busy} aria-busy={busy} data-reads={reads}>
        <Suspense
          fallback={
            <p className="status" role="status">
              Loading the Map…
            </p>
          }
        >
          <PlanMap
            items={inScope}
            scope={context}
            thread={thread}
            lines={lines}
            palette={palette}
            selected={selected}
            preview={preview}
            onSelect={(id) => go({ kind: "tap", id })}
            outside={outsideWaits}
            onOpen={openOutside}
            focus={requests.focus}
            framed={requests.frame}
            active={view === "map"}
            onPick={
              prompt === null ? null : (id) => choose(id === null ? { kind: "background" } : { kind: "item", id })
            }
          />
        </Suspense>
        {/* Wider, the sidebar or tabs show the view's name; it heads the page for assistive technology all the same. The
            Overview and Needs you title their own pages. */}
        {wide && (view === "map" || view === "threads") ? (
          <h1 className="visually-hidden">{view === "map" ? "Map" : "Threads"}</h1>
        ) : null}
        {view === "threads" && waves !== null ? (
          <ThreadsView view={waves} lines={lines} picked={thread} wide={wide} onPick={toThread} onOpen={openOutside} />
        ) : null}
        {view === "overview" ? (
          <Overview
            needs={load.needs}
            lines={lines}
            titleOf={titleOf}
            counts={counts}
            outline={outline}
            scope={scope}
            items={itemsById}
            onNeeds={(key) => goTo("needs", key ?? null)}
            onPickThread={toThread}
            onSelectProject={(id) => select(id, null)}
            onView={goTo}
          />
        ) : null}
        {view === "needs" ? (
          <NeedsList
            needs={load.needs}
            scope={scope}
            titleOf={titleOf}
            lines={lines}
            busy={busy}
            onCommand={(edit, done) => send(edit, { select: null, pin: null, done })}
            // To the Map, as the Overview's rows go: the item selected, or the thread framed with its panel, and closing
            // either comes back here.
            onSelect={(id) => select(id, null)}
            onPickThread={toThread}
            at={needsAt}
            onLeave={(left) => go({ kind: "leaveNeeds", left })}
            restore={requests.restoreNeeds}
          />
        ) : null}
        {view === "map" && inScope.length === 0 ? (
          <div className="empty-plan" data-testid="empty-plan">
            <h2>{context === null ? "No items yet" : "No items in this initiative"}</h2>
            <p className="muted">
              {context === null
                ? "Add an item, then the work inside it. What agents import shows here too."
                : "Add an item here, or choose another initiative."}
            </p>
            <button type="button" className="primary" onClick={() => startAdd(null)}>
              New item
            </button>
          </div>
        ) : null}
        {commandMenu}
        {panel === "mode" ? (
          <PickBar>
            <span className="grow">{prompt}</span>
            <button type="button" className="quiet" onClick={() => choose({ kind: "cancel" })}>
              Cancel
            </button>
          </PickBar>
        ) : panel === "add" && adding !== null ? (
          <AddForm
            key={adding.parent ?? ""}
            parent={adding.parent}
            parentTitle={adding.parent === null ? null : titleOf(adding.parent)}
            // The thread picked, else the one of the item it goes in, else of the item selected, as the project is.
            thread={thread ?? model?.items.get(adding.parent ?? selected ?? "")?.thread ?? null}
            projects={outline?.projects ?? []}
            initiative={context === null ? null : { id: context, title: contextTitle(context, switcher) ?? context }}
            initiativeTitles={scope.titles}
            // The project of the item selected, when it is in the initiative in context: the one in context wins.
            project={startProject(current.project, (id) => model?.items.get(id)?.initiative, context)}
            projectThreads={outline?.projects ?? []}
            threads={sorted.map((one) => ({ id: one.id, label: one.title }))}
            draft={adding.draft}
            busy={busy}
            onSubmit={add}
            onCancel={() => go({ kind: "shut", layer: "add" })}
          />
        ) : panel !== "item" || chosen === undefined ? null : (
          <Inspector
            onPreview={setPreview}
            key={chosen.id}
            item={chosen}
            isProject={outline?.projects.some((project) => project.id === chosen.id) ?? false}
            reopen={requests.reopenEditor?.id === chosen.id ? requests.reopenEditor : undefined}
            titleOf={titleOf}
            stageOf={(id) => model?.items.get(id)?.stage ?? undefined}
            onSelect={(id) => {
              if (go({ kind: "follow", id, pin: selected }).requests.unsaved) tell(UNSAVED);
            }}
            edit={requests.edit}
            shortcuts={shortcuts}
            // To the Map for the thread, framed, as a thread goes from anywhere; the item stays under its panel, so
            // closing the panel, or Back, opens the item again.
            onPickThread={(id) => go({ kind: "itemThread", id })}
            onUnlink={unlink}
            onMove={() => go({ kind: "mode", mode: { kind: "move", item: chosen.id } })}
            onLink={() => go({ kind: "mode", mode: { kind: "link", item: chosen.id } })}
            onAdd={() => startAdd(chosen.id)}
            onClose={() => go({ kind: "shut", layer: "item" })}
            onDismiss={() => closeTop.current(1)}
            onCopyLink={() =>
              copyLink({ ...NOTHING, view: "map", item: chosen.id }).then((copied) => {
                if (copied)
                  tell({ text: "Link copied", offer: null, retry: null, polite: true, lasts: "news", at: null });
              })
            }
            onEditorClosed={formClosed}
            onCommand={(edit) => send(edit, { select: chosen.id, pin: chosen.id })}
            onSave={(edit, edited) =>
              send(edit, { select: chosen.id, pin: chosen.id, undo: true }, { kind: "edit", id: chosen.id, edited })
            }
            threads={sorted}
            initiativeTitles={initiativeTitles}
            lines={lines}
            needs={load.needs}
            busy={busy}
          />
        )}
        {panel === "projects" && scopedOutline !== null ? (
          <ProjectsSheet onClose={() => go({ kind: "shut", layer: "projects" })} onDismiss={() => closeTop.current(1)}>
            <InitiativeSwitcher rows={switcher} context={context} onChoose={chooseInitiative} />
            <Projects
              outline={scopedOutline}
              initiative={contextTitle(context, switcher)}
              lines={lines}
              picked={thread}
              current={current}
              onPickThread={(id) => {
                go({ kind: "shut", layer: "projects" });
                go(forThread(id, "outline"));
              }}
              onSelectProject={(id) => {
                go({ kind: "shut", layer: "projects" });
                select(id, null);
              }}
            />
          </ProjectsSheet>
        ) : null}
        {panel === "thread" && thread !== null ? (
          <ThreadPanel
            key={thread}
            id={thread}
            line={lines.get(thread)}
            counts={counts.get(thread)}
            titleOf={titleOf}
            stageOf={(id) => model?.items.get(id)?.stage ?? undefined}
            waitsFor={(id) => model?.items.get(id)?.waitsFor ?? []}
            version={version}
            onSelect={(id) => select(id, null)}
            onClose={() => go({ kind: "shut", layer: "thread" })}
            onDismiss={() => closeTop.current(1)}
            onCopyLink={() => copyLink({ ...NOTHING, view: "threads", thread })}
          />
        ) : null}
      </main>
      <NoticePlacement
        notice={noticeView}
        phone={!wide}
        mapActive={view === "map"}
        stage={noticeStage}
        chrome={noticeChrome}
        context={`${view}/${panel}/${selected}/${thread}`}
        suspended={showCommand}
      />
    </Shell>
  );
}
