// The needs-you list: each entry in a line, with what it names to select and the actions the app can take on it.

import type { Command } from "./api";
import { Badge, lineStyle } from "./Badge";
import type { Line } from "./lines";
import { Notify } from "./Notify";
import { type Need, needView } from "./needs";

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
  const views = needs.map(needView);
  return (
    <aside className="inspector" data-testid="needs" aria-label="Needs you">
      <header>
        <h2>Needs you</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          ×
        </button>
      </header>
      <Notify />
      {views.length === 0 ? <p className="facts">Nothing needs you.</p> : null}
      <ul className="needs">
        {views.map((view) => {
          const first = view.threads.length === 1 ? lines.get(view.threads[0] ?? "") : undefined;
          return (
            <li key={view.key} data-need={view.key}>
              <p className="facts line" style={lineStyle(first)}>
                <Badge line={first} />
                <span>{view.text}</span>
              </p>
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
                {view.threads.map((id) => (
                  <li key={`t-${id}`}>
                    <button
                      type="button"
                      className="row line"
                      style={lineStyle(lines.get(id))}
                      onClick={() => onPickThread(id)}
                    >
                      <Badge line={lines.get(id)} />
                      {lines.get(id)?.name ?? id}
                    </button>
                  </li>
                ))}
                {view.items.map((id) => (
                  <li key={`i-${id}`}>
                    <button type="button" className="row" data-select={id} onClick={() => onSelect(id)}>
                      {titleOf(id)}
                    </button>
                  </li>
                ))}
              </ul>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
