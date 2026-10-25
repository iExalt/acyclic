import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AddForm } from "./AddForm";
import { type Command, CommandError, command, globalLevel, type Item, items, needs, type Thread, threads } from "./api";
import { Badge, lineStyle } from "./Badge";
import { Choice } from "./Choice";
import {
  addCommand,
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
  withPending,
} from "./edits";
import { globalLevelCommand, type ItemDraft, LEVELS, type Level, levelLabel } from "./forms";
import { Inspector } from "./Inspector";
import { linesOf } from "./lines";
import { Menu } from "./Menu";
import { buildModel } from "./map/model";
import type { Focus } from "./map/PlanMap";
import { NeedsList } from "./NeedsList";
import type { Need } from "./needs";
import { ThreadPanel } from "./ThreadPanel";

// The Map's code (the renderer and the layout engine) is the bulk of the app; it loads while the tracker does.
const mapModule = import("./map/PlanMap");
const PlanMap = lazy(() => mapModule);

/** Titles in reading order, so "Thread 2" comes before "Thread 10". */
const byTitle = (a: Thread, b: Thread) => a.title.localeCompare(b.title, undefined, { numeric: true });

type Load =
  | { state: "loading" }
  | { state: "loaded"; items: Item[]; threads: Thread[]; level: string; needs: Need[] }
  | { state: "failed"; message: string };

/** What an edit selects afterwards, and which box holds still on screen while the Map redraws. */
interface After {
  /** The item to select and show, or null to leave the selection as it is, as a thread's edits do. */
  select: string | null;
  pin: string | null;
}

/** A refusal or failure to show, with what it offers: the command again when the server was only busy, or a refresh. */
interface Notice {
  text: string;
  offer: "retry" | "refresh" | null;
  retry: { body: Command; after: After } | null;
}

/** The add form, open under a parent or at the top level, with what the user wrote if a failed add reopened it. */
interface Adding {
  parent: string | null;
  idRefused: boolean;
  draft: NewItem | null;
}

/** An item's editor reopened, with what the user wrote, after their edit failed; `n` remounts the inspector. */
interface Reopened {
  id: string;
  draft: ItemDraft;
  n: number;
}

export function App() {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [showArchived, setShowArchived] = useState(false);
  const [thread, setThread] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [focus, setFocus] = useState<Focus | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "idle" });
  const [adding, setAdding] = useState<Adding | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  /** Set from a write's start to its end, so a second can't start before a render disables what sends it. */
  const sending = useRef(false);
  /** A write drawn before the server answers, until the plan read after it holds it. */
  const [pending, setPending] = useState<Pending | null>(null);
  const [reopened, setReopened] = useState<Reopened | null>(null);
  // A notification opens the app on `/?needs`, to the list it is about.
  const [showNeeds, setShowNeeds] = useState(() => new URLSearchParams(window.location.search).has("needs"));
  /** Counts edits sent and refreshes, so panels that read more than the plan read again after each. */
  const [version, setVersion] = useState(0);

  /**
   * Counts reads of the plan started, and the latest shown, so one overtaken by a later read is never shown; `items`
   * are the latest shown read's.
   */
  const reads = useRef<{ started: number; shown: number; items: Item[] }>({ started: 0, shown: 0, items: [] });
  /** Reads the plan, or null when a read started later has been shown already. */
  const fetchAll = useCallback(async () => {
    const n = ++reads.current.started;
    const [found, listed, level, needed] = await Promise.all([items(), threads(), globalLevel(), needs()]);
    if (n < reads.current.shown) return null;
    reads.current.shown = n;
    reads.current.items = found;
    return { state: "loaded" as const, items: found, threads: listed, level, needs: needed };
  }, []);
  const refresh = useCallback(() => {
    setVersion((was) => was + 1);
    fetchAll().then(
      (next) => {
        if (next === null) return;
        setLoad(next);
        // A plan read while a write is out may not hold it yet: the write's own read drops what it drew.
        if (!sending.current) setPending(null);
      },
      (err: unknown) => setLoad({ state: "failed", message: err instanceof Error ? err.message : String(err) }),
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
  const shown = useMemo(() => (loaded === null ? [] : withPending(loaded.items, pending)), [loaded, pending]);
  const sorted = useMemo(() => (loaded === null ? [] : [...loaded.threads].sort(byTitle)), [loaded]);
  const lines = useMemo(() => linesOf(sorted), [sorted]);
  const model = useMemo(() => (loaded === null ? null : buildModel(shown)), [loaded, shown]);
  const chosen = selected === null ? undefined : shown.find((one) => one.id === selected);
  // An item that went away, deleted or moved out of reach elsewhere, is no longer selected.
  useEffect(() => {
    if (model !== null && selected !== null && !model.items.has(selected)) setSelected(null);
  }, [model, selected]);

  // An editor opened again for one item stays closed on any other.
  useEffect(() => {
    setReopened((was) => (was === null || was.id === selected ? was : null));
  }, [selected]);

  const select = useCallback((id: string | null, pin: string | null = null) => {
    setSelected(id);
    if (id !== null) setFocus((was) => ({ id, pin, n: (was?.n ?? 0) + 1 }));
  }, []);

  /**
   * Sends an edit; once saved, reloads the plan and brings `after.select` into view with `after.pin` held still. An
   * edit `drawn` is drawn at once, its form closed, and if the write fails, the form opens again with what the user
   * wrote, unless the server turns out to hold it after all.
   */
  const send = useCallback(
    async (body: Command, after: After, drawn: Pending | null = null) => {
      if (sending.current) return;
      sending.current = true;
      setBusy(true);
      setNotice(null);
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
        try {
          const next = await fetchAll();
          if (next !== null) setLoad(next);
          setPending(null);
          setAdding(null);
          bringIn();
          sent = { write: "saved", proposals, reload: "done" };
        } catch (err) {
          // Saved, so what it drew joins the plan shown until a refresh reads it, where the next write can't undo it.
          if (drawn !== null) {
            setLoad((was) => (was.state === "loaded" ? { ...was, items: withPending(was.items, drawn) } : was));
            setPending(null);
          }
          setAdding(null);
          sent = { write: "saved", proposals, reload: { failed: err instanceof Error ? err.message : String(err) } };
        }
      } catch (err) {
        const code = err instanceof CommandError ? err.code : "network";
        sent = { write: "failed", code, message: err instanceof Error ? err.message : String(err) };
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
            setReopened((was) => ({ id: drawn.id, draft: drawn.draft, n: (was?.n ?? 0) + 1 }));
        }
      }
      const { text, offer, fixId } = told(sent);
      setNotice(text === null ? null : { text, offer, retry: offer === "retry" ? { body, after } : null });
      if (fixId) setAdding((was) => (was === null ? null : { ...was, idRefused: true }));
      setVersion((was) => was + 1);
      sending.current = false;
      setBusy(false);
    },
    [fetchAll],
  );

  const choose = useCallback(
    (target: Pick) => {
      if (model === null) return;
      const step = pick(model, mode, target);
      setMode(step.mode);
      setNotice(step.refusal === null ? null : { text: step.refusal, offer: null, retry: null });
      if (step.command === null || mode.kind === "idle") return;
      // A move pins the new parent, or the item itself when it goes to the top level; a link, the item linked from.
      const pin = mode.kind === "move" && target.kind === "item" ? target.id : mode.item;
      send(step.command, { select: mode.item, pin });
    },
    [model, mode, send],
  );
  useEffect(() => {
    if (mode.kind === "idle" && adding === null) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setMode({ kind: "idle" });
      setAdding(null);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [mode, adding]);

  if (load.state === "loading") return <p className="status">Loading…</p>;
  if (load.state === "failed")
    return (
      <p className="status" role="alert">
        {load.message}
      </p>
    );
  const titleOf = (id: string) => model?.items.get(id)?.title ?? id;
  const startAdd = (parent: string | null) => {
    setMode({ kind: "idle" });
    setAdding({ parent, idRefused: false, draft: null });
  };
  const add = (item: NewItem) => send(addCommand(item), { select: item.item, pin: item.parent }, { kind: "add", item });
  const unlink = (from: string, kind: LinkKind, to: string) =>
    send(unlinkCommand(from, kind, to), { select: selected ?? from, pin: selected });
  const prompt =
    mode.kind === "move"
      ? `Tap the box to put ${titleOf(mode.item)} in`
      : mode.kind === "link"
        ? `Tap the item ${titleOf(mode.item)} ${mode.link === "waits-for" ? "waits for" : "relates to"}`
        : null;
  return (
    <div className="shell">
      <header className="bar">
        <div className="brand">
          <h1>
            <span className="count" data-testid="item-count">
              {shown.length}
            </span>{" "}
            items
          </h1>
          <p title={`Autonomy: ${levelLabel(load.level)}`}>Autonomy: {levelLabel(load.level)}</p>
        </div>
        <button
          type="button"
          className="needs-button"
          data-testid="needs-button"
          data-none={load.needs.length === 0}
          aria-pressed={showNeeds}
          onClick={() => {
            setSelected(null);
            setShowNeeds((was) => !was);
          }}
        >
          <span className="badge" data-testid="needs-count">
            {load.needs.length}
          </span>
          Needs you
        </button>
        <button
          type="button"
          className="icon"
          aria-label="Add at the top level"
          onClick={() => startAdd(null)}
          data-action="add-top"
        >
          +
        </button>
        <Menu>
          <label>
            <input
              type="checkbox"
              checked={showArchived}
              onChange={(event) => setShowArchived(event.target.checked)}
              data-testid="show-archived"
            />
            Show archived items
          </label>
          <Choice
            label="Autonomy for every thread without its own"
            testid="global-level"
            value={load.level}
            disabled={busy}
            onWrite={(level) => send(globalLevelCommand(level as Level), { select: null, pin: null })}
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
        </Menu>
      </header>
      <nav className="threads" aria-label="Threads">
        {sorted.map((one) => (
          <button
            type="button"
            key={one.id}
            className="thread"
            style={lineStyle(lines.get(one.id))}
            aria-pressed={one.id === thread}
            data-thread={one.id}
            title={one.title}
            onClick={() => {
              setSelected(null);
              setThread(one.id === thread ? null : one.id);
            }}
          >
            <Badge line={lines.get(one.id)} />
            <span className="name">{lines.get(one.id)?.name ?? one.title}</span>
          </button>
        ))}
      </nav>
      <main className="stage" data-busy={busy}>
        <Suspense fallback={<p className="status">Loading the Map…</p>}>
          <PlanMap
            items={shown}
            showArchived={showArchived}
            thread={thread}
            lines={lines}
            selected={selected}
            onSelect={setSelected}
            focus={focus}
            onPick={
              prompt === null ? null : (id) => choose(id === null ? { kind: "background" } : { kind: "item", id })
            }
          />
        </Suspense>
        {notice === null ? null : (
          <div className="notice" role="alert" data-testid="notice">
            <span className="grow">{notice.text}</span>
            {notice.retry === null ? null : (
              <button
                type="button"
                disabled={busy}
                onClick={() => notice.retry && send(notice.retry.body, notice.retry.after)}
              >
                Try again
              </button>
            )}
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
            <button type="button" aria-label="Dismiss" onClick={() => setNotice(null)}>
              ×
            </button>
          </div>
        )}
        {prompt !== null ? (
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
        ) : adding !== null ? (
          <AddForm
            key={adding.parent ?? ""}
            parent={adding.parent}
            parentTitle={adding.parent === null ? null : titleOf(adding.parent)}
            thread={thread ?? (adding.parent === null ? null : (model?.items.get(adding.parent)?.thread ?? null))}
            threads={sorted}
            taken={new Set(model?.items.keys())}
            idRefused={adding.idRefused}
            draft={adding.draft}
            busy={busy}
            onSubmit={add}
            onCancel={() => setAdding(null)}
          />
        ) : chosen === undefined ? null : (
          <Inspector
            key={`${chosen.id}:${reopened?.id === chosen.id ? reopened.n : 0}`}
            item={chosen}
            reopened={reopened?.id === chosen.id ? reopened.draft : null}
            titleOf={titleOf}
            onSelect={(id) => select(id, selected)}
            onUnlink={unlink}
            onMove={() => setMode({ kind: "move", item: chosen.id })}
            onLink={(link) => setMode({ kind: "link", item: chosen.id, link })}
            onAdd={() => startAdd(chosen.id)}
            onClose={() => setSelected(null)}
            onCommand={(edit) => send(edit, { select: chosen.id, pin: chosen.id })}
            onSave={(edit, draft) =>
              send(edit, { select: chosen.id, pin: chosen.id }, { kind: "edit", id: chosen.id, draft })
            }
            threads={sorted}
            lines={lines}
            busy={busy}
          />
        )}
        {prompt === null && adding === null && chosen === undefined && showNeeds ? (
          <NeedsList
            needs={load.needs}
            titleOf={titleOf}
            lines={lines}
            busy={busy}
            onCommand={(edit) => send(edit, { select: null, pin: null })}
            onSelect={(id) => select(id, null)}
            onPickThread={(id) => {
              setShowNeeds(false);
              setThread(id);
            }}
            onClose={() => setShowNeeds(false)}
          />
        ) : null}
        {prompt === null && adding === null && chosen === undefined && !showNeeds && thread !== null ? (
          <ThreadPanel
            key={thread}
            id={thread}
            globalLevel={load.level}
            line={lines.get(thread)}
            titleOf={titleOf}
            version={version}
            busy={busy}
            onCommand={(edit) => send(edit, { select: null, pin: null })}
            onSelect={(id) => select(id, null)}
            onClose={() => setThread(null)}
          />
        ) : null}
      </main>
    </div>
  );
}
