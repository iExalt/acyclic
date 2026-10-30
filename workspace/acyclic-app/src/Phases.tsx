// The sidebar's outline: phases, each opening into the threads at work in it, then the threads in none. A selected
// item marks its phase and its thread's row there, and opens that phase; so does a thread picked elsewhere.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { Badge, lineStyle } from "./Badge";
import { useFocusIn } from "./focus";
import type { Line } from "./lines";
import type { Outline } from "./outline";
import { Prose } from "./Prose";

export interface PhasesProps {
  outline: Outline;
  lines: ReadonlyMap<string, Line>;
  /** The thread picked, whose rows are pressed. */
  picked: string | null;
  /** The rows the selected item marks. */
  current: { phase: string | null; thread: string | null };
  onPickThread: (id: string) => void;
  onSelectPhase: (id: string) => void;
}

export function Phases({ outline, lines, picked, current, onPickThread, onSelectPhase }: PhasesProps) {
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const reveal = (phase: string | undefined) =>
    phase !== undefined && setOpen((was) => (was.has(phase) ? was : new Set([...was, phase])));
  useEffect(() => {
    reveal(current.phase ?? undefined);
  }, [current.phase]);
  // A thread picked with none of its rows in view opens the first phase it works in.
  useEffect(() => {
    if (picked === null) return;
    const homes = outline.phases.filter((phase) => phase.threads.some((one) => one.id === picked));
    if (!homes.some((phase) => open.has(phase.id))) reveal(homes[0]?.id);
  }, [picked]);
  const title = (id: string) => outline.phases.find((phase) => phase.id === id)?.title ?? id;

  const row = (thread: string, phase: string | null, also: string[] = []) => {
    const line = lines.get(thread);
    return (
      <li key={thread}>
        <button
          type="button"
          className="thread"
          style={lineStyle(line)}
          aria-pressed={thread === picked}
          aria-current={thread === current.thread && phase === current.phase ? "true" : undefined}
          data-thread={thread}
          data-phase={phase ?? ""}
          title={line?.name}
          onClick={() => onPickThread(thread)}
        >
          <Badge line={line} />
          <span className="name">
            <Prose text={line?.name ?? thread} />
            {also.length === 0 ? null : <span className="also">also in {also.map(title).join(", ")}</span>}
          </span>
        </button>
      </li>
    );
  };

  return (
    <div className="outline">
      <h2 className="heading">
        Phases <span className="count">{outline.phases.length}</span>
      </h2>
      <ul>
        {outline.phases.map((phase) => {
          const expanded = open.has(phase.id);
          return (
            <li key={phase.id} className="phase" aria-current={phase.id === current.phase ? "true" : undefined}>
              <div className="phase-row">
                <button
                  type="button"
                  className="chevron"
                  aria-expanded={expanded}
                  aria-label={`${expanded ? "Close" : "Open"} ${phase.title}`}
                  data-toggle-phase={phase.id}
                  onClick={() =>
                    setOpen((was) => {
                      const next = new Set(was);
                      if (expanded) next.delete(phase.id);
                      else next.add(phase.id);
                      return next;
                    })
                  }
                />
                <button
                  type="button"
                  className="phase-title"
                  data-select-phase={phase.id}
                  onClick={() => onSelectPhase(phase.id)}
                >
                  <Prose text={phase.title} />
                </button>
                <span className="count">{phase.threads.length}</span>
              </div>
              {expanded ? (
                <>
                  <h3 className="subheading">Threads in this phase</h3>
                  <ul>{phase.threads.map((one) => row(one.id, phase.id, one.also))}</ul>
                </>
              ) : null}
            </li>
          );
        })}
      </ul>
      {outline.others.length === 0 ? null : (
        <>
          <h2 className="heading">
            Other threads <span className="count">{outline.others.length}</span>
          </h2>
          <ul>{outline.others.map((thread) => row(thread, null))}</ul>
        </>
      )}
    </div>
  );
}

/** The outline as a sheet over the Map, as a phone shows it. */
export function PhasesSheet({ onClose, children }: { onClose: () => void; children: ReactNode }) {
  const panel = useRef<HTMLElement>(null);
  useFocusIn(panel);
  return (
    <aside ref={panel} tabIndex={-1} className="inspector" data-testid="phases" aria-label="Phases">
      <header>
        <h2>Phases</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </header>
      {children}
    </aside>
  );
}
