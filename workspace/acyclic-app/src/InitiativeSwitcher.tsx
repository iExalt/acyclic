// The initiative switcher: one row per initiative with its open count, the one in context marked. Choosing a row puts
// that initiative in context; it opens nothing. It stands in the sidebar, and on a phone at the top of the Outline.

import type { InitiativeRow } from "./initiatives";
import { Prose } from "./Prose";

export interface SwitcherProps {
  rows: readonly InitiativeRow[];
  /** The initiative in context, marked. */
  context: string | null;
  onChoose: (id: string) => void;
}

export function InitiativeSwitcher({ rows, context, onChoose }: SwitcherProps) {
  if (rows.length === 0) return null;
  return (
    <div className="outline initiatives" data-testid="initiatives">
      <h2 className="heading">Initiatives</h2>
      <ul>
        {rows.map((row) => (
          <li key={row.id}>
            <button
              type="button"
              className="initiative"
              data-initiative={row.id}
              aria-current={row.id === context ? "true" : undefined}
              title={`${row.title}: ${row.open} open`}
              onClick={() => onChoose(row.id)}
            >
              <span className="initiative-name">
                <Prose text={row.title} />
              </span>
              {row.open > 0 && (
                <span className="initiative-count">
                  {row.open}
                  <span className="open"> open</span>
                </span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
