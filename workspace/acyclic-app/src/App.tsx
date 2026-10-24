import { useEffect, useState } from "react";
import { type Item, items } from "./api";

type Load = { state: "loading" } | { state: "loaded"; items: Item[] } | { state: "failed"; message: string };

export function App() {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  useEffect(() => {
    items().then(
      (found) => setLoad({ state: "loaded", items: found }),
      (err: unknown) => setLoad({ state: "failed", message: err instanceof Error ? err.message : String(err) }),
    );
  }, []);
  if (load.state === "loading") return <p>Loading…</p>;
  if (load.state === "failed") return <p role="alert">{load.message}</p>;
  return (
    <main>
      <h1>
        <span data-testid="item-count">{load.items.length}</span> items
      </h1>
      <ul>
        {load.items.map((item) => (
          <li key={item.id}>{item.title}</li>
        ))}
      </ul>
    </main>
  );
}
