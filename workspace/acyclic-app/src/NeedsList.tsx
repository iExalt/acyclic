// Needs you's view: the needs-you list as a page, as the Overview is, its entries in columns as wide as the page
// allows. Each entry is a line, with what it names to select under it, then the actions the app can take on it, so
// the button comes after what it acts on.

import { type RefObject, useLayoutEffect, useRef, useState } from "react";
import type { Command } from "./api";
import { Badge, lineStyle, NeedBadge } from "./Badge";
import { useHeld } from "./held";
import type { Line } from "./lines";
import { GROUPS, groupOf, type Need, type NeedGroup, needView, threadLabel } from "./needs";
import { Prose } from "./Prose";

/** The actions that say yes, set as the entry's main one; dropping and the rest stay outlined. */
const PRIMARY = new Set(["keep", "approve"]);

/**
 * Where the view was left for an item or a thread an entry named, which shows on the Map: the filter shown, how far it
 * was scrolled, and the entry and control picked, so closing what it opened comes back to the list as it was, the
 * focus on that control. The filter comes back with the rest, as the scroll and the entry only hold under it.
 */
export interface LeftAt {
  filter: NeedGroup | null;
  top: number;
  entry: string;
  /** A selector for the control within the entry. */
  control: string;
}

export interface NeedsListProps {
  needs: Need[];
  titleOf: (id: string) => string;
  lines: ReadonlyMap<string, Line>;
  /** A thread's items, as an approval covers them. */
  membersOf?: (thread: string) => string[];
  busy: boolean;
  /** Sends an entry's action, with what to say once it is saved. */
  onCommand: (command: Command, done: string, undo?: true) => void;
  onSelect: (id: string) => void;
  onPickThread: (id: string) => void;
  /** The entry to open at, as the Overview's row for it asks: scrolled to, marked and focused, but none of its actions,
   *  so a stray Enter decides nothing. */
  at?: string | null;
  /** Where the view was left, kept by the app while it isn't shown: set as an entry opens what it names, and taken
   *  back, then cleared, as the view shows again. */
  left: RefObject<LeftAt | null>;
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
  at = null,
  left,
}: NeedsListProps) {
  const all = needs.map((need) => ({ group: groupOf(need.kind), view: needView(need, lines, membersOf, titleOf) }));
  // Filters for the groups present, only when there are two or more to choose between.
  const groups = GROUPS.map((one) => ({
    ...one,
    count: all.filter((entry) => entry.group === one.group).length,
  })).filter((one) => one.count > 0);
  // Read while rendering, before the effect below takes the rest of where the view was left and clears it.
  const [filter, setFilter] = useState<NeedGroup | null>(() => left.current?.filter ?? null);
  // A filter whose entries have all been settled shows everything again.
  const shown = groups.some((one) => one.group === filter) ? filter : null;
  const views = all.filter((entry) => shown === null || entry.group === shown).map((entry) => entry.view);
  const page = useRef<HTMLElement>(null);
  // Once an entry is settled, the keyboard goes to the one that took its place, so settling the list is pressing Enter
  // down it; with nothing left to settle, to the page.
  const held = useHeld(busy, views.map((view) => view.key).join("\n"));
  const after = (index: number) => () => {
    const entries = page.current?.querySelectorAll<HTMLElement>("[data-need]") ?? [];
    const entry = entries[Math.min(index, entries.length - 1)];
    // An entry settled only in a terminal has no action: its first row, or the entry itself.
    const into = entry?.querySelector<HTMLElement>(".need-actions button") ?? entry?.querySelector("button") ?? entry;
    return into ?? page.current;
  };
  // Back from what an entry opened, the list is as it was left; else at the entry asked for, if any. Before paint, so
  // the list never shows at its top first.
  useLayoutEffect(() => {
    const was = left.current;
    left.current = null;
    const scroller = page.current;
    if (scroller === null) return;
    if (was !== null) {
      scroller.scrollTop = was.top;
      const entry = scroller.querySelector<HTMLElement>(`[data-need="${CSS.escape(was.entry)}"]`);
      (entry?.querySelector<HTMLElement>(was.control) ?? entry)?.focus({ preventScroll: true });
      return;
    }
    if (at === null) return;
    const entry = scroller.querySelector<HTMLElement>(`[data-need="${CSS.escape(at)}"]`);
    entry?.scrollIntoView({ block: "start" });
    entry?.focus({ preventScroll: true });
  }, [at, left]);
  /** Keeps where the view is, for the item or thread an entry opens, then opens it. */
  const leave = (entry: string, control: string, open: () => void) => () => {
    left.current = { filter: shown, top: page.current?.scrollTop ?? 0, entry, control };
    open();
  };
  return (
    <section ref={page} tabIndex={-1} className="needs-view" data-testid="needs" aria-labelledby="needs-title">
      <h1 id="needs-title" className="view-title">
        Needs you
      </h1>
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
                <NeedBadge line={view.line} />
                <Prose text={view.text} />
              </p>
            ) : (
              <button
                type="button"
                className="row line need-head"
                style={lineStyle(view.line)}
                data-pick-thread={view.thread}
                onClick={leave(view.key, ":scope > [data-pick-thread]", () => view.thread && onPickThread(view.thread))}
              >
                <Badge line={view.line} />
                <Prose text={view.text} />
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
                      data-pick-thread={thread.id}
                      onClick={leave(view.key, `.need-items [data-pick-thread="${CSS.escape(thread.id)}"]`, () =>
                        onPickThread(thread.id),
                      )}
                    >
                      <Badge line={label.line} />
                      <Prose text={label.text} />
                    </button>
                  </li>
                );
              })}
              {view.items.map((id) => (
                <li key={`i-${id}`}>
                  <button
                    type="button"
                    className="row"
                    data-select={id}
                    onClick={leave(view.key, `[data-select="${CSS.escape(id)}"]`, () => onSelect(id))}
                  >
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
                      onClick={leave(view.key, ".more", () => view.thread && onPickThread(view.thread))}
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
                    {...held(
                      `${view.key} ${action.action}`,
                      () => onCommand(action.command, action.done, action.undo),
                      after(index),
                    )}
                  >
                    {action.label}
                  </button>
                ))}
              </div>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
