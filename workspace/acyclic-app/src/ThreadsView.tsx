// The Threads view: the threads with work left in waves of what to start first, a column a wave, and those that can
// start any time apart. Side by side, an arrow runs from each thread to every one waiting on it; the picked or
// hovered thread's stand out. A card always says what it comes after, so the arrows are never the only telling.

import { useLayoutEffect, useMemo, useState } from "react";
import { countsText } from "./api";
import { type Box, boundsOf, type Column, gapWidths, headOf, pathOf, type Route, routes, type Wait } from "./arrows";
import { Badge, lineStyle } from "./Badge";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import type { Card, WavesView } from "./waves";

/** The room between columns when wide, as app.css sets it. */
const GAP = 64;

export interface ThreadsViewProps {
  view: WavesView;
  lines: ReadonlyMap<string, Line>;
  picked: string | null;
  /** Whether there is room for the arrows: columns side by side, rather than waves stacked on a phone. */
  wide: boolean;
  onPick: (id: string) => void;
}

export function ThreadsView({ view, lines, picked, wide, onPick }: ThreadsViewProps) {
  const [hovered, setHovered] = useState<string | null>(null);
  // A state, not a ref: the arrows measure the columns once they are there, which a ref wouldn't tell them.
  const [columns, setColumns] = useState<HTMLDivElement | null>(null);
  const badges = (ids: string[]) =>
    ids.map((id) => (
      <span key={id} style={lineStyle(lines.get(id))}>
        <Badge line={lines.get(id)} />
      </span>
    ));
  const card = (one: Card) => {
    const line = lines.get(one.id);
    return (
      <li key={one.id}>
        <button
          type="button"
          className="card"
          data-card={one.id}
          data-thread={one.id}
          aria-pressed={one.id === picked}
          style={lineStyle(line)}
          onClick={() => onPick(one.id)}
          onPointerEnter={() => setHovered(one.id)}
          onPointerLeave={() => setHovered((was) => (was === one.id ? null : was))}
        >
          <span className="head">
            <Badge line={line} />
            <span className="phases">{one.phases.join(", ")}</span>
          </span>
          <span className="name">
            <Prose text={line?.name ?? one.id} />
          </span>
          <span className="counts">{countsText(one.counts) || "no tasks shown"}</span>
          {one.after.length === 0 ? null : <span className="after">after {badges(one.after)}</span>}
          {one.looped.length === 0 ? null : <span className="looped">waits in a cycle with {badges(one.looped)}</span>}
        </button>
      </li>
    );
  };
  const empty = view.waves.length === 0 && view.anytime.length === 0;
  const waits = useMemo(
    () =>
      view.waves
        .flat()
        .flatMap((card) => [
          ...card.after.map((from) => ({ from, to: card.id, looped: false })),
          ...card.looped.map((from) => ({ from, to: card.id, looped: true })),
        ]),
    [view],
  );
  // Each gap as wide as its lanes need: the columns stand GAP apart, and a crowded gap takes the rest as a margin.
  const widths = useMemo(
    () =>
      gapWidths(
        view.waves.map((cards) => cards.map((card) => card.id)),
        waits,
        GAP,
      ),
    [view, waits],
  );
  return (
    <section className="waves" data-testid="threads-view" aria-label="Threads in waves">
      {empty ? <p className="status">No thread has work left.</p> : null}
      <div className="columns" ref={setColumns}>
        {wide ? (
          <Arrows view={view} waits={waits} widths={widths} lines={lines} lit={hovered ?? picked} within={columns} />
        ) : null}
        {view.waves.map((cards, at) => (
          <section
            key={at}
            className="wave"
            data-wave={at + 1}
            aria-label={`Wave ${at + 1}`}
            style={
              wide
                ? {
                    marginRight:
                      (widths[at] ?? GAP) - (at + 1 < view.waves.length || view.anytime.length > 0 ? GAP : 0),
                  }
                : undefined
            }
          >
            <h2>
              Wave {at + 1} <small>{at === 0 ? "start here" : `after wave ${at}`}</small>
            </h2>
            <ul>{cards.map(card)}</ul>
          </section>
        ))}
        {view.anytime.length === 0 ? null : (
          <section className="wave anytime" data-wave="anytime" aria-label="Any time">
            <h2>
              Any time <small>waits on no thread, and none on it</small>
            </h2>
            <ul>{view.anytime.map(card)}</ul>
          </section>
        )}
      </div>
    </section>
  );
}

/**
 * An arrow from each thread to each thread after it that waits on it, and between two in one wave, which wait on each
 * other, routed through the gaps so none crosses a card. Measured from the cards as drawn, again whenever the
 * columns change size.
 */
function Arrows({
  view,
  waits,
  widths,
  lines,
  lit,
  within,
}: {
  view: WavesView;
  waits: Wait[];
  widths: number[];
  lines: ReadonlyMap<string, Line>;
  lit: string | null;
  within: HTMLDivElement | null;
}) {
  const [found, setFound] = useState<Route[]>([]);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const host = within;
    if (host === null) return;
    const measure = () => {
      const origin = host.getBoundingClientRect();
      const box = (element: Element): Box => {
        const at = element.getBoundingClientRect();
        return {
          left: at.left - origin.left,
          top: at.top - origin.top,
          right: at.right - origin.left,
          bottom: at.bottom - origin.top,
        };
      };
      const columns: Column[] = view.waves.map((cards, at) => {
        const wave = host.querySelector(`[data-wave="${at + 1}"]`);
        const heading = wave?.querySelector("h2");
        return {
          heading: heading ? box(heading) : { left: 0, top: 0, right: 0, bottom: 0 },
          cards: cards.flatMap((card) => {
            const drawn = wave?.querySelector(`[data-card="${CSS.escape(card.id)}"]`);
            return drawn ? [{ id: card.id, box: box(drawn) }] : [];
          }),
        };
      });
      const last = host.querySelector(`[data-wave="${view.waves.length}"]`);
      const beyond = last === null ? null : box(last).right + (widths[view.waves.length - 1] ?? 0);
      const routed = routes(columns, waits, beyond);
      const bounds = boundsOf(routed);
      setFound(routed);
      // Room for routes that run below the columns or past them, which the scroll then reaches.
      setSize({
        width: Math.max(host.offsetWidth, (bounds?.right ?? 0) + 12),
        height: Math.max(host.offsetHeight, (bounds?.bottom ?? 0) + 12),
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(host);
    return () => watch.disconnect();
  }, [view, waits, widths, within]);
  const on = (route: Route) => lit !== null && (route.from === lit || route.to === lit);
  // The lit arrows last, so they cross over the rest.
  const ordered = [...found.filter((route) => !on(route)), ...found.filter(on)];
  return (
    <svg className="arrows" aria-hidden="true" width={size.width} height={size.height}>
      {ordered.map((route) => {
        const colour = on(route) && lit !== null ? lines.get(lit)?.colour : undefined;
        return (
          <g
            key={`${route.from}->${route.to}`}
            data-from={route.from}
            data-to={route.to}
            className={[route.looped ? "looped" : "", on(route) ? "lit" : lit === null ? "" : "dim"].join(" ")}
            style={colour === undefined ? undefined : { color: colour }}
          >
            <path d={pathOf(route.points)} />
            <path className="head" d={headOf(route.points)} />
          </g>
        );
      })}
    </svg>
  );
}
