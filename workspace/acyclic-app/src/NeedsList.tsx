// The needs-you list: each entry in a line, with what it names to select under it, then the actions the app can take
// on it, so the button comes after what it acts on.

import { type ReactNode, useEffect, useRef, useState } from "react";
import type { Command } from "./api";
import { Badge, lineStyle } from "./Badge";
import { useFocusIn } from "./focus";
import { useHeld } from "./held";
import { Icon, X } from "./icons";
import type { Line } from "./lines";
import { GROUPS, groupOf, type Need, type NeedGroup, needView, threadLabel } from "./needs";
import { Prose } from "./Prose";
import { useSheet } from "./Sheet";

/** The actions that say yes, set as the entry's main one; dropping and the rest stay outlined. */
const PRIMARY = new Set(["keep", "approve"]);

export interface NeedsListProps {
  needs: Need[];
  titleOf: (id: string) => string;
  lines: ReadonlyMap<string, Line>;
  /** A thread's items, as an approval covers them. */
  membersOf?: (thread: string) => string[];
  busy: boolean;
  /** Sends an entry's action, with what to say once it is saved. */
  onCommand: (command: Command, done: string) => void;
  onSelect: (id: string) => void;
  onPickThread: (id: string) => void;
  onClose: () => void;
  /** Closes it as Escape does, for a drag down on a phone: a form holding typing stays, saying how to leave it. */
  onDismiss: () => void;
  /** A refusal or failure of what this panel sent, shown in it. */
  notice?: ReactNode;
  /** The entry to open at, as the Overview's row for it asks: scrolled to, marked and focused, but none of its actions,
   *  so a stray Enter decides nothing. */
  at?: string | null;
}

export function NeedsList({
  needs,
  titleOf,
  lines,
  membersOf,
  busy,
  onCommand,
  onSelect,
  onPickThread,
  onClose,
  onDismiss,
  notice,
  at = null,
}: NeedsListProps) {
  const all = needs.map((need) => ({ group: groupOf(need.kind), view: needView(need, lines, membersOf) }));
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
  // A task list: full height on a phone, where half would show a small phone no decision.
  const sheet = useSheet(panel, "full", onDismiss);
  useFocusIn(panel);
  // Once an entry is settled, the keyboard goes to the one that took its place, so settling the list is pressing Enter
  // down it; with nothing left to settle, to the panel's Close.
  const held = useHeld(busy, views.map((view) => view.key).join("\n"));
  const after = (index: number) => () => {
    const entries = panel.current?.querySelectorAll<HTMLElement>("[data-need]") ?? [];
    const entry = entries[Math.min(index, entries.length - 1)];
    // An entry settled only in a terminal has no action: its first row, or the entry itself.
    const into = entry?.querySelector<HTMLElement>(".need-actions button") ?? entry?.querySelector("button") ?? entry;
    return into ?? panel.current?.querySelector<HTMLElement>(".close");
  };
  useEffect(() => {
    if (at === null) return;
    const entry = panel.current?.querySelector<HTMLElement>(`[data-need="${CSS.escape(at)}"]`);
    entry?.scrollIntoView({ block: "start" });
    entry?.focus({ preventScroll: true });
  }, [at]);
  return (
    <aside
      ref={panel}
      tabIndex={-1}
      className="inspector"
      data-testid="needs"
      aria-label="Needs you"
      data-detent={sheet.detent}
    >
      <header {...sheet.header}>
        {sheet.handle}
        <h2>Needs you</h2>
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {notice}
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
      {views.length === 0 ? <p className="empty">Nothing needs you. New approvals and blockers show here.</p> : null}
      <ul className="needs">
        {views.map((view, index) => (
          <li
            key={view.key}
            data-need={view.key}
            data-at={view.key === at || undefined}
            // A row J and K move to, and the Overview opens the list at: focusable, but no control, so Enter on it does
            // nothing; Tab reaches its actions.
            data-row
            tabIndex={-1}
          >
            {view.thread === null ? (
              <p className="facts line need-head" style={lineStyle(view.line)}>
                <Badge line={view.line} />
                <span>{view.text}</span>
              </p>
            ) : (
              <button
                type="button"
                className="row line need-head"
                style={lineStyle(view.line)}
                data-pick-thread={view.thread}
                onClick={() => view.thread !== null && onPickThread(view.thread)}
              >
                <Badge line={view.line} />
                <span>{view.text}</span>
              </button>
            )}
            {view.note === null ? null : <p className="why need-why">{view.note}</p>}
            <ul className="rows need-items">
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
              {view.more === 0 ? null : (
                <li>
                  {view.thread === null ? (
                    <span className="row more">and {view.more} more</span>
                  ) : (
                    <button
                      type="button"
                      className="row more"
                      data-pick-thread={view.thread}
                      onClick={() => view.thread !== null && onPickThread(view.thread)}
                    >
                      and {view.more} more
                    </button>
                  )}
                </li>
              )}
            </ul>
            {view.actions.length === 0 ? null : (
              <div className="actions need-actions">
                {view.actions.map((action) => (
                  <button
                    type="button"
                    key={action.label}
                    className={PRIMARY.has(action.action) ? "primary" : "quiet"}
                    data-action={action.action}
                    {...held(() => onCommand(action.command, action.done), after(index))}
                  >
                    {action.label}
                  </button>
                ))}
              </div>
            )}
          </li>
        ))}
      </ul>
    </aside>
  );
}
