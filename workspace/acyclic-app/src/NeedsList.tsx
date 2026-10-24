// The needs-you list: each entry in a line, with what it names to select and the actions the app can take on it.

import type { Command } from "./api";
import { Notify } from "./Notify";
import { type Need, needView } from "./needs";

export interface NeedsListProps {
  needs: Need[];
  titleOf: (id: string) => string;
  busy: boolean;
  onCommand: (command: Command) => void;
  onSelect: (id: string) => void;
  onPickThread: (id: string) => void;
  onClose: () => void;
}

export function NeedsList({ needs, titleOf, busy, onCommand, onSelect, onPickThread, onClose }: NeedsListProps) {
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
        {views.map((view) => (
          <li key={view.key} data-need={view.key}>
            <p className="facts">{view.text}</p>
            <div className="actions">
              {view.items.map((id) => (
                <button type="button" className="chip" key={`i-${id}`} data-select={id} onClick={() => onSelect(id)}>
                  {titleOf(id)}
                </button>
              ))}
              {view.threads.map((id) => (
                <button type="button" className="chip" key={`t-${id}`} onClick={() => onPickThread(id)}>
                  {id}
                </button>
              ))}
              {view.actions.map((action) => (
                <button
                  type="button"
                  key={action.label}
                  data-action={action.action}
                  disabled={busy}
                  onClick={() => onCommand(action.command)}
                >
                  {action.label}
                </button>
              ))}
            </div>
            {view.note === null ? null : <p className="why">{view.note}</p>}
          </li>
        ))}
      </ul>
    </aside>
  );
}
