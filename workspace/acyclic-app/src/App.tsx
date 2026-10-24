import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { type Item, items, type Thread, threads } from "./api";

// The Map's code (the renderer and the layout engine) is the bulk of the app; it loads while the tracker does.
const mapModule = import("./map/PlanMap");
const PlanMap = lazy(() => mapModule);

/** Titles in reading order, so "Thread 2" comes before "Thread 10". */
const byTitle = (a: Thread, b: Thread) => a.title.localeCompare(b.title, undefined, { numeric: true });

type Load =
  | { state: "loading" }
  | { state: "loaded"; items: Item[]; threads: Thread[] }
  | { state: "failed"; message: string };

export function App() {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [showArchived, setShowArchived] = useState(false);
  const [thread, setThread] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const refresh = useCallback(() => {
    Promise.all([items(), threads()]).then(
      ([found, listed]) => setLoad({ state: "loaded", items: found, threads: listed }),
      (err: unknown) => setLoad({ state: "failed", message: err instanceof Error ? err.message : String(err) }),
    );
  }, []);
  useEffect(() => {
    refresh();
    // Coming back to the app shows what changed meanwhile.
    const visible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    document.addEventListener("visibilitychange", visible);
    return () => document.removeEventListener("visibilitychange", visible);
  }, [refresh]);

  if (load.state === "loading") return <p className="status">Loading…</p>;
  if (load.state === "failed")
    return (
      <p className="status" role="alert">
        {load.message}
      </p>
    );
  const chosen = selected === null ? undefined : load.items.find((one) => one.id === selected);
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
      <main className="stage">
        <Suspense fallback={<p className="status">Loading the Map…</p>}>
          <PlanMap items={load.items} showArchived={showArchived} thread={thread} onSelect={setSelected} />
        </Suspense>
        {chosen === undefined ? null : (
          <aside className="inspector" data-testid="inspector">
            <h2>{chosen.title}</h2>
            <p>
              {chosen.id} · {chosen.state}
            </p>
          </aside>
        )}
      </main>
    </div>
  );
}
