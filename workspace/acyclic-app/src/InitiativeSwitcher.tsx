// The initiative switcher: one row per initiative with its open count, the one in context marked. Choosing a row puts
// that initiative in context; it opens nothing. It stands in the sidebar, and on a phone at the top of the Outline.

import { Icon, Trash2 } from "./icons";
import type { InitiativeRow } from "./initiatives";
import { Menu } from "./Menu";
import { Prose } from "./Prose";

export interface SwitcherProps {
  rows: readonly InitiativeRow[];
  /** The initiative in context, marked. */
  context: string | null;
  onChoose: (id: string) => void;
  onDelete?: (id: string, title: string) => void;
  busy?: boolean;
}

export function InitiativeSwitcher({ rows, context, onChoose, onDelete, busy }: SwitcherProps) {
  if (rows.length === 0) return null;
  return (
    <div className="outline initiatives" data-testid="initiatives">
      <h2 className="heading">Initiatives</h2>
      <ul>
        {rows.map((row) => (
          <li key={row.id} className="initiative-row">
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
            {onDelete === undefined ? null : (
              <Menu label={`Actions for ${row.title}`} testid={`initiative-menu-${row.id}`} list>
                <button
                  type="button"
                  className="menu-action danger"
                  disabled={busy}
                  onClick={() => onDelete(row.id, row.title)}
                >
                  <Icon of={Trash2} size={16} />
                  <span>Delete initiative…</span>
                </button>
              </Menu>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
