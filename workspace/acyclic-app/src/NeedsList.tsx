// The needs-you list: each entry in a line, with what it names to select and the actions the app can take on it.

import { useRef, useState } from "react";
import type { Command } from "./api";
import { Badge, lineStyle } from "./Badge";
import { useFocusIn } from "./focus";
import { Icon, X } from "./icons";
import type { Line } from "./lines";
import { Notify } from "./Notify";
import { GROUPS, groupOf, type Need, type NeedGroup, needView, threadLabel } from "./needs";
import { Prose } from "./Prose";

/** The actions that say yes, set as the entry's main one; dropping and the rest stay outlined. */
const PRIMARY = new Set(["keep", "approve"]);

export interface NeedsListProps {
  needs: Need[];
  titleOf: (id: string) => string;
  lines: ReadonlyMap<string, Line>;
  busy: boolean;
  onCommand: (command: Command) => void;
  onSelect: (id: string) => void;
  onPickThread: (id: string) => void;
  onClose: () => void;
}

export function NeedsList({ needs, titleOf, lines, busy, onCommand, onSelect, onPickThread, onClose }: NeedsListProps) {
  const all = needs.map((need) => ({ group: groupOf(need.kind), view: needView(need, lines) }));
  // Filters for the groups present, only when there are two or more to choose between.
  const groups = GROUPS.map((one) => ({
    ...one,
    count: all.filter((entry) => entry.group === one.group).length,
  })).filter((one) => one.count > 0);
  const [filter, setFilter] = useState<NeedGroup | null>(null);
  // A filter whose entries have all been settled shows everything again.
  const shown = groups.some((one) => one.group === filter) ? filter : null;
  const views = all.filter((entry) => shown === null || entry.group === shown).map((entry) => entry.view);
  const panel = useRef<HTMLElement>(null);
  useFocusIn(panel);
  return (
    <aside ref={panel} tabIndex={-1} className="inspector" data-testid="needs" aria-label="Needs you">
      <header>
        <h2>Needs you</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      <Notify />
      {groups.length < 2 ? null : (
        <fieldset className="filters" aria-label="Show">
          {[{ group: null, label: "All", count: all.length }, ...groups].map((one) => (
            <button
              type="button"
              key={one.label}
              className="chip"
              aria-pressed={shown === one.group}
              data-filter={one.group ?? "all"}
              onClick={() => setFilter(one.group)}
            >
              {one.label} <span className="count">{one.count}</span>
            </button>
          ))}
        </fieldset>
      )}
      {views.length === 0 ? <p className="facts">Nothing to review.</p> : null}
      <ul className="needs">
        {views.map((view) => (
          <li key={view.key} data-need={view.key}>
            {view.thread === null ? (
              <p className="facts line" style={lineStyle(view.line)}>
                <Badge line={view.line} />
                <span>{view.text}</span>
              </p>
            ) : (
              <button
                type="button"
                className="row line"
                style={lineStyle(view.line)}
                data-pick-thread={view.thread}
                onClick={() => view.thread !== null && onPickThread(view.thread)}
              >
                <Badge line={view.line} />
                <span>{view.text}</span>
              </button>
            )}
            {view.actions.length === 0 ? null : (
              <div className="actions">
                {view.actions.map((action) => (
                  <button
                    type="button"
                    key={action.label}
                    className={PRIMARY.has(action.action) ? "primary" : undefined}
                    data-action={action.action}
                    disabled={busy}
                    onClick={() => onCommand(action.command)}
                  >
                    {action.label}
                  </button>
                ))}
              </div>
            )}
            {view.note === null ? null : <p className="why">{view.note}</p>}
            <ul className="rows">
              {view.threads.map((thread) => {
                const label = threadLabel(thread, lines);
                return (
                  <li key={`t-${thread.id}`}>
                    <button
                      type="button"
                      className="row line"
                      style={lineStyle(label.line)}
                      onClick={() => onPickThread(thread.id)}
                    >
                      <Badge line={label.line} />
                      <Prose text={label.text} />
                    </button>
                  </li>
                );
              })}
              {view.items.map((id) => (
                <li key={`i-${id}`}>
                  <button type="button" className="row" data-select={id} onClick={() => onSelect(id)}>
                    <Prose text={titleOf(id)} />
                  </button>
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </aside>
  );
}
