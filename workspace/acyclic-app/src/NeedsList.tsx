// The needs-you list: each entry in a line, with what it names to select and the actions the app can take on it.

import type { Command } from "./api";
import { Badge, lineStyle } from "./Badge";
import type { Line } from "./lines";
import { Notify } from "./Notify";
import { type Need, needView, threadLabel } from "./needs";

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
  const views = needs.map((need) => needView(need, lines));
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
                data-thread={view.thread}
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
                      {label.text}
                    </button>
                  </li>
                );
              })}
              {view.items.map((id) => (
                <li key={`i-${id}`}>
                  <button type="button" className="row" data-select={id} onClick={() => onSelect(id)}>
                    {titleOf(id)}
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
