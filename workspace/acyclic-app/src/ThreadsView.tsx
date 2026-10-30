// The Threads view: the threads with work left in waves of what to start first, a column a wave, and those that can
// start any time apart. Side by side, an arrow runs from each thread to every one waiting on it; the picked or
// hovered thread's stand out. A card always says what it comes after, so the arrows are never the only telling.

import { useLayoutEffect, useState } from "react";
import { countsText } from "./api";
import { Badge, lineStyle } from "./Badge";
import type { Line } from "./lines";
import { Prose } from "./Prose";
import type { Card, WavesView } from "./waves";

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
  return (
    <section className="waves" data-testid="threads-view" aria-label="Threads in waves">
      {empty ? <p className="status">No thread has work left.</p> : null}
      <div className="columns" ref={setColumns}>
        {wide ? <Arrows view={view} lines={lines} lit={hovered ?? picked} within={columns} /> : null}
        {view.waves.map((cards, at) => (
          <section key={at} className="wave" data-wave={at + 1} aria-label={`Wave ${at + 1}`}>
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

interface Arrow {
  key: string;
  from: string;
  to: string;
  path: string;
  /** Its head, a small triangle at its end, pointing the way it goes in. */
  head: string;
  looped: boolean;
}

/** A head at (x, y), pointing right, or left when `back`. */
const head = (x: number, y: number, back: boolean) => {
  const tail = back ? x + 7 : x - 7;
  return `M ${x} ${y} L ${tail} ${y - 4} L ${tail} ${y + 4} z`;
};

/**
 * An arrow from each thread to each thread after it that waits on it, from the one's right edge to the other's left,
 * and between two in one wave, which wait on each other, along their right edges. Measured from the cards as drawn,
 * again whenever the columns change size.
 */
function Arrows({
  view,
  lines,
  lit,
  within,
}: {
  view: WavesView;
  lines: ReadonlyMap<string, Line>;
  lit: string | null;
  within: HTMLDivElement | null;
}) {
  const [arrows, setArrows] = useState<Arrow[]>([]);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const host = within;
    if (host === null) return;
    const measure = () => {
      const origin = host.getBoundingClientRect();
      const box = (id: string) => {
        const at = host.querySelector(`[data-card="${CSS.escape(id)}"]`)?.getBoundingClientRect();
        return at === undefined
          ? null
          : {
              left: at.left - origin.left + host.scrollLeft,
              right: at.right - origin.left + host.scrollLeft,
              middle: (at.top + at.bottom) / 2 - origin.top + host.scrollTop,
            };
      };
      const found: Arrow[] = [];
      for (const card of view.waves.flat()) {
        const to = box(card.id);
        if (to === null) continue;
        for (const [first, looped] of [
          ...card.after.map((id) => [id, false] as const),
          ...card.looped.map((id) => [id, true] as const),
        ]) {
          const from = box(first);
          if (from === null) continue;
          const path = looped
            ? `M ${from.right} ${from.middle} C ${from.right + 28} ${from.middle}, ${to.right + 28} ${to.middle}, ${to.right} ${to.middle}`
            : `M ${from.right} ${from.middle} C ${from.right + 40} ${from.middle}, ${to.left - 40} ${to.middle}, ${to.left} ${to.middle}`;
          const end = looped ? head(to.right, to.middle, true) : head(to.left, to.middle, false);
          found.push({ key: `${first}->${card.id}`, from: first, to: card.id, path, head: end, looped });
        }
      }
      setArrows(found);
      setSize({ width: host.scrollWidth, height: host.scrollHeight });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(host);
    return () => watch.disconnect();
  }, [view, within]);
  return (
    <svg className="arrows" aria-hidden="true" width={size.width} height={size.height}>
      {arrows.map((arrow) => {
        const on = lit !== null && (arrow.from === lit || arrow.to === lit);
        const colour = on ? lines.get(lit)?.colour : undefined;
        return (
          <g
            key={arrow.key}
            className={[arrow.looped ? "looped" : "", on ? "lit" : lit === null ? "" : "dim"].join(" ")}
            style={colour === undefined ? undefined : { color: colour }}
          >
            <path d={arrow.path} />
            <path className="head" d={arrow.head} />
          </g>
        );
      })}
    </svg>
  );
}
