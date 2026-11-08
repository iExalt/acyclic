// Dependency waves come first, a column per wave, followed by an independent grid of threads that can start any time.
// A tile names what it waits on; side by side, arrows show the picked or hovered thread's links.

import { useLayoutEffect, useMemo, useState } from "react";
import { type Box, boundsOf, type Column, gapWidths, headOf, pathOf, type Route, routes, type Wait } from "./arrows";
import { Badge, lineStyle } from "./Badge";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import { StageCounts } from "./Stage";
import { projectLabels, type ThreadTile, type WavesView } from "./waves";

/** The room between columns when wide, as app.css sets it. */
const GAP = 64;

export interface ThreadsViewProps {
  view: WavesView;
  lines: ReadonlyMap<string, Line>;
  /** The thread picked on the Map, whose tile and arrows stand out. */
  picked: string | null;
  /** Whether there is room for the arrows: columns side by side, rather than waves stacked on a phone. */
  wide: boolean;
  /** Goes to the Map for the thread. */
  onPick: (id: string) => void;
  /** Opens a task in another initiative, which puts that initiative in context. */
  onOpen: (id: string) => void;
}

/** The most tasks in other initiatives a tile names; the rest are counted. */
const MOST_TASKS = 3;

export function ThreadsView({ view, lines, picked, wide, onPick, onOpen }: ThreadsViewProps) {
  const [routedHeight, setRoutedHeight] = useState(0);
  const [hovered, setHovered] = useState<string | null>(null);
  // A state, not a ref: the arrows measure the columns once they are there, which a ref wouldn't tell them.
  const [columns, setColumns] = useState<HTMLDivElement | null>(null);
  // Each a dot, named on hover and to a screen reader, as a badge alone says nothing in words.
  const badges = (ids: string[]) =>
    ids.map((id) => {
      const name = lines.get(id)?.name ?? id;
      return (
        <span key={id} style={lineStyle(lines.get(id))} title={name}>
          <Badge line={lines.get(id)} />
          <span className="visually-hidden">{name}</span>
        </span>
      );
    });
  const tile = (one: ThreadTile) => {
    const line = lines.get(one.id);
    const projects = projectLabels(one.projects, line?.name ?? one.id);
    const { after, looped, tasks } = one.outside;
    const away = after.length + looped.length + tasks.length > 0;
    // A thread's name for a stub, plain, as a chip has no room for its code spans.
    const nameOf = (id: string) => (lines.get(id)?.name ?? id).replace(/`([^`]+)`/g, "$1");
    // A stub opens what it names, in its own initiative; the full name is its title, where the text is cut short.
    const stub = (key: string, lead: string, where: string, name: string, open: () => void) => (
      <button key={key} type="button" className="stub" title={`${lead}${where}: ${name}`} onClick={open}>
        <span aria-hidden="true">{lead}</span>
        <span className="arrow" aria-hidden="true">
          ↗{" "}
        </span>
        <span className="visually-hidden">{lead}</span>
        <span className="where">{where}: </span>
        <span className="named">{name}</span>
      </button>
    );
    return (
      <li key={one.id} className={away ? "has-away" : undefined}>
        <button
          type="button"
          className="thread-tile"
          data-thread-tile={one.id}
          data-task={line?.task ?? undefined}
          data-thread={one.id}
          aria-current={one.id === picked || undefined}
          style={lineStyle(line)}
          onClick={() => onPick(one.id)}
          onPointerEnter={() => setHovered(one.id)}
          onPointerLeave={() => setHovered((was) => (was === one.id ? null : was))}
        >
          <span className="head">
            <Badge line={line} />
            {projects.length === 0 ? null : <span className="projects">{projects.join(", ")}</span>}
          </span>
          <span className="name">
            <Prose text={line?.name ?? one.id} />
          </span>
          <span className="counts">
            <StageCounts counts={one.counts} none="No items shown" />
          </span>
          {one.after.length === 0 ? null : <span className="after">after {badges(one.after)}</span>}
          {one.looped.length === 0 ? null : <span className="looped">waits in a cycle with {badges(one.looped)}</span>}
        </button>
        {away ? (
          <div className="away" style={lineStyle(line)}>
            {tasks
              .slice(0, MOST_TASKS)
              .map((task) => stub(`task ${task.id}`, "", task.initiative, task.title, () => onOpen(task.id)))}
            {tasks.length > MOST_TASKS ? (
              <span className="more">+{tasks.length - MOST_TASKS} more in other initiatives</span>
            ) : null}
            {after.map((other) =>
              stub(`after ${other.id}`, "after ", other.initiative, nameOf(other.id), () => onPick(other.id)),
            )}
            {looped.map((other) =>
              stub(`looped ${other.id}`, "waits in a cycle with ", other.initiative, nameOf(other.id), () =>
                onPick(other.id),
              ),
            )}
          </div>
        ) : null}
      </li>
    );
  };
  const empty = view.waves.length === 0 && view.anytime.length === 0;
  const waits = useMemo(
    () =>
      view.waves
        .flat()
        .flatMap((tile) => [
          ...tile.after.map((from) => ({ from, to: tile.id, looped: false })),
          ...tile.looped.map((from) => ({ from, to: tile.id, looped: true })),
        ]),
    [view],
  );
  // Each gap as wide as its lanes need: the columns stand GAP apart, and a crowded gap takes the rest as a margin.
  const widths = useMemo(
    () =>
      gapWidths(
        view.waves.map((tiles) => tiles.map((tile) => tile.id)),
        waits,
        GAP,
      ),
    [view, waits],
  );
  return (
    <section className="waves" data-testid="threads-view" aria-label="Threads in waves">
      {empty ? <p className="status">No thread has work left.</p> : null}
      {view.waves.length === 0 ? null : (
        <div className="columns" ref={setColumns} style={{ minHeight: wide ? routedHeight : undefined }}>
          {wide ? (
            <Arrows
              view={view}
              waits={waits}
              widths={widths}
              lines={lines}
              lit={hovered ?? picked}
              within={columns}
              onHeight={setRoutedHeight}
            />
          ) : null}
          {view.waves.map((tiles, at) => (
            <section
              key={at}
              className="wave"
              data-wave={at + 1}
              aria-label={`Wave ${at + 1}`}
              style={
                wide
                  ? {
                      marginRight: (widths[at] ?? GAP) - (at + 1 < view.waves.length ? GAP : 0),
                    }
                  : undefined
              }
            >
              <h2>
                Wave {at + 1} <small>{at === 0 ? "start here" : `after wave ${at}`}</small>
              </h2>
              <ul>{tiles.map(tile)}</ul>
            </section>
          ))}
        </div>
      )}
      {view.anytime.length === 0 ? null : (
        <section className="wave anytime" data-wave="anytime" aria-label="Any time">
          <h2>
            Any time <small>No threads to wait for or hold up</small>
          </h2>
          <ul>{view.anytime.map(tile)}</ul>
        </section>
      )}
    </section>
  );
}

/**
 * An arrow from each thread to each thread after it that waits on it, and between two in one wave, which wait on each
 * other, routed through the gaps so none crosses a tile. Measured from the tiles as drawn, again whenever the
 * columns change size.
 */
function Arrows({
  view,
  waits,
  widths,
  lines,
  lit,
  within,
  onHeight,
}: {
  view: WavesView;
  waits: Wait[];
  widths: number[];
  lines: ReadonlyMap<string, Line>;
  lit: string | null;
  within: HTMLDivElement | null;
  onHeight: (height: number) => void;
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
      const columns: Column[] = view.waves.map((tiles, at) => {
        const wave = host.querySelector(`[data-wave="${at + 1}"]`);
        const heading = wave?.querySelector("h2");
        return {
          heading: heading ? box(heading) : { left: 0, top: 0, right: 0, bottom: 0 },
          tiles: tiles.flatMap((tile) => {
            // The tile's whole place, its stubs under it too, which no arrow may cross.
            const drawn = wave?.querySelector(`[data-thread-tile="${CSS.escape(tile.id)}"]`)?.closest("li");
            return drawn ? [{ id: tile.id, box: box(drawn) }] : [];
          }),
        };
      });
      const last = host.querySelector(`[data-wave="${view.waves.length}"]`);
      const beyond = (last === null ? 0 : box(last).right) + (widths[view.waves.length - 1] ?? GAP);
      const routed = routes(columns, waits, beyond);
      const bounds = boundsOf(routed);
      setFound(routed);
      // Reserve the routes before hover too, so revealing one never moves the independent grid.
      onHeight(bounds === null ? 0 : bounds.bottom + 12);
      // Room for routes that run below the columns or past them, which the scroll then reaches.
      setSize({
        width: Math.max(host.offsetWidth, (bounds?.right ?? 0) + 12),
        height: Math.max(host.offsetHeight, (bounds?.bottom ?? 0) + 12),
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(host);
    for (const wave of host.querySelectorAll(".wave")) watch.observe(wave);
    return () => watch.disconnect();
  }, [view, waits, widths, within, onHeight]);
  const on = (route: Route) => lit !== null && (route.from === lit || route.to === lit);
  const ordered = found.filter(on);
  return (
    <svg className="arrows" aria-hidden="true" width={size.width} height={size.height}>
      {ordered.map((route) => {
        const colour = on(route) && lit !== null ? lines.get(lit)?.colour : undefined;
        return (
          <g
            key={`${route.from}->${route.to}`}
            data-from={route.from}
            data-to={route.to}
            className={[route.looped ? "looped" : "", "lit"].join(" ")}
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
