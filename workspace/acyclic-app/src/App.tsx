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
import { Inspector } from "./Inspector";
import { Icon, X } from "./icons";
import { type KeyAction, nextRow, SHORTCUTS, step, typingIn, type Waiting } from "./keys";
import { byTitle, linesOf } from "./lines";
import { buildModel, inArchived } from "./map/model";
import { NeedsList } from "./NeedsList";
import { Notify } from "./Notify";
import {
  addressOf,
  arrived,
  depth,
  type Entries,
  type Found,
  follow,
  type HistoryAction,
  type Intent,
  landing,
  landingPlace,
  NOTHING,
  navigate,
  openLayers,
  openOf,
  type Place,
  parse,
  placeIn,
  type Requests,
  restored,
  START,
  type Step,
  samePlace,
  stored,
  topLayer,
  type Wanted,
  type World,
} from "./nav";
import type { Seen } from "./notices";
import { Overview } from "./Overview";
import { currentRows, outlineOf } from "./outline";
import { Projects, ProjectsSheet } from "./Projects";
import { PALETTES, setTheme, type Theme, useScheme, useTheme } from "./palette";
import { drawn as drawnOver } from "./reads";
import { Shell, type View, Wordmark } from "./Shell";
import { createSession, type Notice } from "./session";
import { ThreadPanel } from "./ThreadPanel";
import { ThreadsView } from "./ThreadsView";
import { workingThreads } from "./threadLists";
import { wavesView } from "./waves";
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
type Opening = Wanted & { place?: Place };

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
  const [showArchived, setShowArchived] = useState(false);
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
    archived: () => false,
    threadOf: () => null,
    wide: false,
  });
  const go = useCallback((intent: Intent): Step => {
    const step = navigate(placeNow.current, intent, { ...worldNow.current, dirty: dirtyForm() });
    const { unsaved: _, ...asked } = step.requests;
    placeNow.current = step.place;
    askedNow.current = { ...askedNow.current, ...asked };
    setNav({ place: step.place, requests: askedNow.current });
    return step;
  }, []);
  const { place, requests } = nav;
  const { view, item: selected, thread, needsAt } = place;
  const { command: showCommand, mode, add: adding, projects: showProjects } = place.overlays;
  /** The command menu lists only what has a key, as ? asks. */
  const [shortcutsOnly, setShortcutsOnly] = useState(false);
  /** Single-key shortcuts, on unless switched off on this device. */
  const [shortcuts, setShortcuts] = useState(() => localStorage.getItem(SHORTCUTS_KEY) !== "off");
  const wide = useWide();
  /**
   * The session with the server: the plan, the write out, and the one notice slot. Its moves of where the user is go
   * through `goNow`, so they start from where the user is by then.
   */
  const goNow = useRef<(intent: Intent) => void>(go);
  goNow.current = go;
  const [session] = useState(() =>
    createSession({
      fetcher: (input, init) => fetch(input, init),
      storage: storageOrNull(),
      nav: { go: (intent) => goNow.current(intent), item: () => placeNow.current.item },
    }),
  );
  const { load, notice, busy, pending, version, reads } = useSyncExternalStore(session.subscribe, session.snapshot);
  const { refresh, send, tell } = session;
  /**
   * An address to open: the one the app was opened at, one a notification sent the app already open, or the place a
   * history entry Forward landed on holds, with what it names. It waits for the plan to load, and for a form holding
   * what the user typed to close.
   */
  const [wanted, setWanted] = useState<Opening | null>(() => {
    const asked = parse(window.location.search);
    return addressOf(asked) === "/" ? null : asked;
  });
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
            showArchived,
          ),
    [model, sorted, showArchived],
  );
  /** Each thread's leaves by stage, as core counts them: every one but those archived, read with the plan. */
  const counts = useMemo(
    () => new Map(loaded === null ? [] : loaded.threads.threads.map((one) => [one.id, one.counts])),
    [loaded],
  );
  const itemsById = useMemo(() => new Map(shown.map((item) => [item.id, item])), [shown]);
  const waves = useMemo(
    () => (loaded === null || outline === null ? null : wavesView(loaded.threads, outline)),
    [loaded, outline],
  );
  const working = useMemo(() => new Set(workingThreads(counts)), [counts]);
  worldNow.current = {
    has: (id) => model?.items.has(id) ?? false,
    archived: (id) => model !== null && !showArchived && inArchived(model, id),
    threadOf: (id) => model?.items.get(id)?.thread ?? null,
    wide,
  };
  const current = useMemo(
    () =>
      model === null || outline === null ? { project: null, thread: null } : currentRows(model, outline, selected),
    [model, outline, selected],
  );
  // An item that went away, deleted or moved out of reach elsewhere, or hidden with archived items, is no longer
  // selected: what's selected is drawn, so the panel and the address never name an item the Map doesn't show.
  useEffect(() => {
    if (model === null || selected === null) return;
    if (!model.items.has(selected) || (!showArchived && inArchived(model, selected))) go({ kind: "gone" });
  }, [model, selected, showArchived, go]);

  /** An item chosen anywhere but on the Map: to the Map for it, brought into view with `pin` held still. */
  const select = useCallback((id: string, pin: string | null = null) => go({ kind: "select", id, pin }), [go]);
  /** To the Map for a thread, as its row or tile anywhere does: picked, its panel open, and its items framed. */
  const toThread = useCallback((id: string) => go({ kind: "thread", id }), [go]);

  const choose = useCallback(
    (target: Pick) => {
      if (model === null) return;
      const step = pick(model, mode, target);
      go({ kind: "mode", mode: step.mode });
      const refusal = step.refusal;
      // A target refused says why where the user chose it; one taken drops what an earlier choice was refused.
      const was = session.snapshot().notice;
      tell(
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
    [model, mode, send, go, session, tell],
  );
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
    const plan = { model, threads, working, showArchived, item: item === undefined ? null : item };
    return mode.kind === "idle" ? commandEntries(plan) : targets(plan, mode.kind, mode.item);
  }, [showCommand, model, sorted, lines, working, showArchived, mode, panel, selected]);
  const layers = depth(open);
  /** Drops what was said of an unsaved form, once the form has gone. */
  const formClosed = useCallback(() => {
    session.event({ kind: "form-closed" });
    setFormsClosed((was) => was + 1);
  }, [session]);
  // What's on top changing takes the add form, or the item's editor, with it.
  useEffect(formClosed, [panel]);
  /** Where the user is, which says how long a notice stays. */
  const shownItem = chosen?.id ?? null;
  const seen: Seen = useMemo(() => ({ layer: panel, item: shownItem, thread, view }), [panel, shownItem, thread, view]);
  // A notice is first shown where the user is, and kept or dropped as they move. As an update of the notice as it
  // is by then, so it never brings back one another effect of the same move cleared, such as a closed form's.
  useEffect(() => {
    if (notice !== null) session.event({ kind: "moved", to: seen });
  }, [notice, seen, session]);
  /**
   * Closes `count` layers from the top, as Escape (one) and Back (one per entry it went back over) do, but never a
   * form holding what the user typed: that says how to leave it instead. How many it closed.
   */
  const closeTop = useRef<(count: number) => number>(() => 0);
  closeTop.current = (count) => {
    const closing = openLayers(openOf(placeNow.current, worldNow.current)).slice(0, count);
    if (go({ kind: "close", count }).requests.unsaved) {
      // A link waiting for the form says so already, and dismissing that drops it.
      if (!session.snapshot().notice?.held) tell(UNSAVED);
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
          session.undo();
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
  // One history entry per layer open, so a phone's Back closes them one by one before it leaves the app.
  const entries = useRef<Entries>({ pushed: 0, waiting: false });
  const layersNow = useRef(layers);
  layersNow.current = layers;
  const act = useCallback((step: { entries: Entries; action: HistoryAction }) => {
    entries.current = step.entries;
    const { action } = step;
    if (action === null) return;
    if (action.kind === "push") {
      // Each with the address and the place of what is open, so Forward onto any of them opens it again; while a link
      // waits, the address it was opened at stays, and no place says otherwise. The entry under them, unless it is the
      // app's first, takes them too: what it stood for may have closed as these opened, as the command menu does
      // choosing an item, and its address with it.
      const url = waitingLink.current ? undefined : addressOf(hereNow.current, window.location.search);
      const place = url === undefined ? {} : { place: stored(placeShown.current) };
      if (url !== undefined && action.from > 0)
        window.history.replaceState({ ...window.history.state, ...place }, "", url);
      for (let n = action.from + 1; n <= action.to; n++) window.history.pushState({ layers: n, ...place }, "", url);
    } else if (action.kind === "back") window.history.go(-action.by);
    else if (action.kind === "relabel")
      window.history.replaceState({ ...window.history.state, layers: action.layers }, "");
    else if (action.kind === "restore") {
      // Read now, before the address is written over with what was open: Forward opens the place the entry holds,
      // where it came from too, or, on an entry written before entries held places, its address, as a link does.
      waitingLink.current = true;
      const place = placeIn(window.history.state);
      const address = parse(window.location.search);
      setWanted(
        place === null
          ? address
          : { view: place.view, item: place.item, thread: place.thread, needsAt: place.needsAt, place },
      );
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
    needsAt: view === "needs" ? needsAt : null,
  };
  const hereNow = useRef(here);
  hereNow.current = here;
  /** The place shown, as the address says it: what an entry holds, written with its address. */
  const placeShown = useRef(place);
  placeShown.current = place;
  const waitingLink = useRef(wanted !== null);
  waitingLink.current = wanted !== null;
  const writeAddress = useCallback(() => {
    if (entries.current.waiting || waitingLink.current) return;
    const next = addressOf(hereNow.current, window.location.search);
    const place = stored(placeShown.current);
    const state = window.history.state;
    if (next !== window.location.pathname + window.location.search || !samePlace(placeIn(state), place))
      window.history.replaceState({ ...state, place }, "", next);
  }, []);
  const address = addressOf(here);
  // Written again whenever what's open changes, and once a link has opened.
  useEffect(writeAddress, [writeAddress, address, wanted, layers, place]);
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
      // A newer link replaces one still waiting. One naming no view, such as a thread's, opens over the view shown, so
      // closing it goes back to where the user was; Needs you's, bare `needs`, names its view.
      const asked = parse(event.data.search);
      const query = new URLSearchParams(event.data.search);
      const names = ["view", "item", "needs"].some((key) => query.has(key));
      setWanted(names ? asked : { ...asked, view: hereNow.current.view });
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
      // An entry Forward landed on stands for what is open meanwhile, so no later move goes back over it unasked.
      if (entries.current.restoring) act(restored(entries.current, layersNow.current));
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
      item: (id: string) => (!model.items.has(id) ? null : inArchived(model, id) ? "archived" : "shown") as Found,
      thread: (id: string) => sorted.some((one) => one.id === id),
    };
    // A place an entry held lands as a link does, gone or archived since.
    const found = link.place === undefined ? landing(link, has) : landingPlace(link.place, has);
    setWanted(null);
    // Landing from outside, nothing open has an entry yet: the one landed on becomes home.
    if (entries.current.pushed === 0)
      window.history.replaceState(window.history.state, "", addressOf(NOTHING, window.location.search));
    if ("place" in found) {
      if (found.place.item !== null && found.archived) setShowArchived(true);
      go({ kind: "restore", place: found.place });
    } else {
      if (found.wanted.item !== null && found.archived) setShowArchived(true);
      go({ kind: "link", wanted: found.wanted });
    }
    const gone = found.missing.join(" and ");
    // An offered Undo waits wherever the user goes, a link included, and one a reload kept is offered again.
    const was = session.snapshot().notice;
    tell(
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
  const titleOf = (id: string) => model?.items.get(id)?.title ?? id;
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
    go({ kind: "shut", layer: "command" });
    if (run.kind === "target") return choose(run.id === null ? { kind: "top" } : { kind: "item", id: run.id });
    if (run.kind === "archived") return setShowArchived((was) => !was);
    if (run.kind === "refresh") return refresh();
    if (dirtyForm()) return tell(UNSAVED);
    if (run.kind === "view") goTo(run.view);
    else if (run.kind === "add") startAdd(null);
    else if (run.kind === "on-item") onItem(run.action);
    else if (run.kind === "thread") toThread(run.id);
    else select(run.id, null);
  };
  /**
   * Acts on the item whose panel shows, as its buttons do, while they would be enabled; a move or link, asked for
   * from the keyboard or the menu, finds its target in the menu, as the keyboard has no Map to tap.
   */
  const onItem = (action: OnItem) => {
    if (chosen === undefined || busy) return;
    if (action === "edit") go({ kind: "edit" });
    else if (action === "add-inside") startAdd(chosen.id);
    else {
      go({
        kind: "mode",
        mode: action === "move" ? { kind: "move", item: chosen.id } : { kind: "link", item: chosen.id },
      });
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
    else if (action.kind === "search" || action.kind === "help") {
      setShortcutsOnly(action.kind === "help");
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
      // News is told by the status line below, always there, as a live region must be to be heard.
      <div className="notice" role={notice.polite ? undefined : "alert"} data-testid="notice" data-lasts={notice.lasts}>
        <span className={notice.link === undefined ? "grow" : undefined} title={notice.text}>
          {notice.text}
        </span>
        {notice.retry === null ? null : (
          <button type="button" disabled={busy} onClick={() => session.retry()}>
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
            onClick={() => session.undo()}
          >
            Undo
          </button>
        ) : null}
        {notice.offer === "hide-archived" ? (
          <button
            type="button"
            onClick={() => {
              session.dismiss();
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
              session.dismiss();
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
            session.dismiss();
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
      <button type="button" onClick={refresh}>
        Refresh
      </button>
    </>
  );
  /** Goes to a view, every way the user asks to: the tabs and sidebar, keys, the command menu, the Overview's links. */
  const goTo = (to: View, at: string | null = null) => go({ kind: "goTo", view: to, at });
  return (
    <Shell
      wide={wide}
      count={shown.length}
      view={view}
      onView={goTo}
      needs={load.needs.length}
      onAdd={() => startAdd(null)}
      menu={menu}
      outline={
        outline === null || !wide ? null : (
          <Projects
            outline={outline}
            lines={lines}
            picked={thread}
            current={current}
            onPickThread={(id) => go({ kind: "outline", id })}
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
      {/* The plan reads shown, for browser journeys to know a refresh has landed. */}
      <main className="stage" data-busy={busy} aria-busy={busy} data-reads={reads}>
        <Suspense
          fallback={
            <p className="status" role="status">
              Loading the Map…
            </p>
          }
        >
          <PlanMap
            items={shown}
            showArchived={showArchived}
            thread={thread}
            lines={lines}
            palette={palette}
            selected={selected}
            onSelect={(id) => go({ kind: "tap", id })}
            onThread={toThread}
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
          <ThreadsView view={waves} lines={lines} picked={thread} wide={wide} onPick={toThread} />
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
            onNeeds={(key) => goTo("needs", key ?? null)}
            onPickThread={toThread}
            onSelectProject={(id) => select(id, null)}
            onView={goTo}
          />
        ) : null}
        {view === "needs" ? (
          <NeedsList
            needs={load.needs}
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
            threads={sorted.map((one) => ({ id: one.id, label: one.title }))}
            draft={adding.draft}
            busy={busy}
            onSubmit={add}
            onCancel={() => go({ kind: "shut", layer: "add" })}
          />
        ) : panel !== "item" || chosen === undefined ? null : (
          <Inspector
            key={chosen.id}
            item={chosen}
            reopen={requests.reopenEditor?.id === chosen.id ? requests.reopenEditor : undefined}
            titleOf={titleOf}
            stageOf={(id) => model?.items.get(id)?.stage ?? undefined}
            onSelect={(id) => select(id, selected)}
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
            lines={lines}
            needs={load.needs}
            busy={busy}
          />
        )}
        {panel === "projects" && outline !== null ? (
          <ProjectsSheet onClose={() => go({ kind: "shut", layer: "projects" })} onDismiss={() => closeTop.current(1)}>
            <Projects
              outline={outline}
              lines={lines}
              picked={thread}
              current={current}
              onPickThread={(id) => {
                go({ kind: "shut", layer: "projects" });
                go({ kind: "outline", id });
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
      {/* The slot, where every notice shows: at the stage's foot, in its flow, so it covers nothing and a sheet rises
          above it. */}
      {noticeView === null ? null : <div className="slot">{noticeView}</div>}
    </Shell>
  );
}
