import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { AddForm } from "./AddForm";
import { type Command, CommandError, command, type Item, items, type Thread, threads } from "./api";
import { addCommand, type LinkKind, type Mode, type NewItem, type Pick, pick, recovery, unlinkCommand } from "./edits";
import { Inspector } from "./Inspector";
import { buildModel } from "./map/model";
import type { Focus } from "./map/PlanMap";

// The Map's code (the renderer and the layout engine) is the bulk of the app; it loads while the tracker does.
const mapModule = import("./map/PlanMap");
const PlanMap = lazy(() => mapModule);

/** Titles in reading order, so "Thread 2" comes before "Thread 10". */
const byTitle = (a: Thread, b: Thread) => a.title.localeCompare(b.title, undefined, { numeric: true });

type Load =
  | { state: "loading" }
  | { state: "loaded"; items: Item[]; threads: Thread[] }
  | { state: "failed"; message: string };

/** What an edit selects afterwards, and which box holds still on screen while the Map redraws. */
interface After {
  select: string;
  pin: string | null;
}

/** A refusal or failure to show, with the command to send again when the server was only busy. */
interface Notice {
  text: string;
  retry: { body: Command; after: After } | null;
}

/** The add form, open under a parent or at the top level. */
interface Adding {
  parent: string | null;
  idRefused: boolean;
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

  const fetchAll = useCallback(async () => {
    const [found, listed] = await Promise.all([items(), threads()]);
    return { state: "loaded" as const, items: found, threads: listed };
  }, []);
  const refresh = useCallback(() => {
    fetchAll().then(setLoad, (err: unknown) =>
      setLoad({ state: "failed", message: err instanceof Error ? err.message : String(err) }),
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
  const model = useMemo(() => (loaded === null ? null : buildModel(loaded.items)), [loaded]);
  const chosen = selected === null ? undefined : loaded?.items.find((one) => one.id === selected);
  // An item that went away, deleted or moved out of reach elsewhere, is no longer selected.
  useEffect(() => {
    if (model !== null && selected !== null && !model.items.has(selected)) setSelected(null);
  }, [model, selected]);

  const select = useCallback((id: string | null, pin: string | null = null) => {
    setSelected(id);
    if (id !== null) setFocus((was) => ({ id, pin, n: (was?.n ?? 0) + 1 }));
  }, []);

  /** Sends an edit; on success, reloads the plan and brings `after.select` into view with `after.pin` held still. */
  const send = useCallback(
    async (body: Command, after: After) => {
      setBusy(true);
      setNotice(null);
      try {
        const answer = await command<{ proposals?: unknown[] }>(body);
        const next = await fetchAll();
        setLoad(next);
        setAdding(null);
        setSelected(after.select);
        setFocus((was) => ({ id: after.select, pin: after.pin, n: (was?.n ?? 0) + 1 }));
        if ((answer?.proposals?.length ?? 0) > 0)
          setNotice({ text: "Sent as a proposal for you to keep", retry: null });
      } catch (err) {
        const code = err instanceof CommandError ? err.code : "network";
        const text = err instanceof Error ? err.message : String(err);
        const { retry, fixId } = recovery(code);
        setNotice({ text, retry: retry ? { body, after } : null });
        if (fixId) setAdding((was) => (was === null ? null : { ...was, idRefused: true }));
      } finally {
        setBusy(false);
      }
    },
    [fetchAll],
  );

  const choose = useCallback(
    (target: Pick) => {
      if (model === null) return;
      const step = pick(model, mode, target);
      setMode(step.mode);
      setNotice(step.refusal === null ? null : { text: step.refusal, retry: null });
      if (step.command === null || mode.kind === "idle") return;
      // A move pins the new parent; a link, the item linked from.
      const pin = mode.kind === "move" ? (target.kind === "item" ? target.id : null) : mode.item;
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
    setAdding({ parent, idRefused: false });
  };
  const add = (item: NewItem) => send(addCommand(item), { select: item.item, pin: item.parent });
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
        <h1>
          <span data-testid="item-count">{load.items.length}</span> items
        </h1>
        <label>
          <input
            type="checkbox"
            checked={showArchived}
            onChange={(event) => setShowArchived(event.target.checked)}
            data-testid="show-archived"
          />{" "}
          Archived
        </label>
        <button type="button" onClick={() => startAdd(null)} data-action="add-top">
          Add
        </button>
        <button type="button" onClick={refresh}>
          Refresh
        </button>
      </header>
      <nav className="threads" aria-label="Threads">
        {[...load.threads].sort(byTitle).map((one) => (
          <button
            type="button"
            key={one.id}
            className={one.id === thread ? "thread picked" : "thread"}
            aria-pressed={one.id === thread}
            data-thread={one.id}
            onClick={() => setThread(one.id === thread ? null : one.id)}
          >
            {one.title}
          </button>
        ))}
      </nav>
      <main className="stage" data-busy={busy}>
        <Suspense fallback={<p className="status">Loading the Map…</p>}>
          <PlanMap
            items={load.items}
            showArchived={showArchived}
            thread={thread}
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
              <button type="button" onClick={() => notice.retry && send(notice.retry.body, notice.retry.after)}>
                Try again
              </button>
            )}
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
            <button type="button" onClick={() => choose({ kind: "cancel" })}>
              Cancel
            </button>
          </div>
        ) : adding !== null ? (
          <AddForm
            key={adding.parent ?? ""}
            parent={adding.parent}
            parentTitle={adding.parent === null ? null : titleOf(adding.parent)}
            thread={thread ?? (adding.parent === null ? null : (model?.items.get(adding.parent)?.thread ?? null))}
            threads={[...load.threads].sort(byTitle)}
            taken={new Set(model?.items.keys())}
            idRefused={adding.idRefused}
            busy={busy}
            onSubmit={add}
            onCancel={() => setAdding(null)}
          />
        ) : chosen === undefined ? null : (
          <Inspector
            item={chosen}
            titleOf={titleOf}
            onSelect={(id) => select(id, selected)}
            onUnlink={unlink}
            onMove={() => setMode({ kind: "move", item: chosen.id })}
            onLink={(link) => setMode({ kind: "link", item: chosen.id, link })}
            onAdd={() => startAdd(chosen.id)}
            onClose={() => setSelected(null)}
          />
        )}
      </main>
    </div>
  );
}
