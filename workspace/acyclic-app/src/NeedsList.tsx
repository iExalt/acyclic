// Needs you's view: the needs-you list as a page, as the Overview is, its entries in columns as wide as the page
// allows. Each entry is a line, with what it names to select under it, then the actions the app can take on it, so
// the button comes after what it acts on.

import { useId, useLayoutEffect, useRef, useState } from "react";
import type { Command } from "./api";
import { Badge, lineStyle, NeedBadge } from "./Badge";
import { useHeld } from "./held";
import type { Line } from "./lines";
import type { NeedsLeft } from "./nav";
import { isCollapsible, type Need, needView, threadLabel } from "./needs";
import { Prose } from "./Prose";
import { useWide } from "./wide";

export interface NeedsListProps {
  needs: Need[];
  titleOf: (id: string) => string;
  lines: ReadonlyMap<string, Line>;
  busy: boolean;
  /** Sends an entry's action, with what to say once it is saved. */
  onCommand: (command: Command, done: string) => void;
  onSelect: (id: string) => void;
  onPickThread: (id: string) => void;
  /** The entry to open at, as the Overview's row for it asks: scrolled to, marked and focused, but none of its actions,
   *  so a stray Enter decides nothing. */
  at?: string | null;
  /** Says where the view is left, as an entry opens what it names, for the app to keep while it isn't shown. */
  onLeave: (left: NeedsLeft) => void;
  /** Where the view was left, asked back as it shows again: each new one taken back once. */
  restore?: NeedsLeft | undefined;
}

export function NeedsList({
  needs,
  titleOf,
  lines,
  busy,
  onCommand,
  onSelect,
  onPickThread,
  at = null,
  onLeave,
  restore,
}: NeedsListProps) {
  const views = needs.map((need) => needView(need, lines, titleOf));
  const target = at;
  const wide = useWide();
  const [disclosures, setDisclosures] = useState<{
    target: string | null;
    restore: NeedsLeft | undefined;
    expanded: Set<string>;
  }>({ target: null, restore: undefined, expanded: new Set() });
  // Apply a new target or restoration during render, before the focus/scroll layout effect below sees the DOM.
  if (disclosures.target !== target || disclosures.restore !== restore) {
    const expanded = new Set(disclosures.expanded);
    const include = (key: string | null | undefined) => {
      const view = views.find((one) => one.key === key);
      if (view !== undefined && isCollapsible(view)) expanded.add(view.key);
    };
    include(target);
    include(restore?.entry);
    setDisclosures({ target, restore, expanded });
  }
  const disclosureId = useId();
  const expanded = disclosures.expanded;
  const toggle = (key: string) => {
    setDisclosures((was) => {
      const next = new Set(was.expanded);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return { ...was, expanded: next };
    });
  };
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
  const seen = useRef<{ target: string | null; restore: NeedsLeft | undefined } | null>(null);
  useLayoutEffect(() => {
    const before = seen.current;
    seen.current = { target, restore };
    const was = restore !== undefined && restore !== before?.restore ? restore : null;
    // Only a new ask moves the list: the request going, as the app moves on, or the same target again, leaves it.
    if (was === null && before !== null && before.target === target) return;
    const scroller = page.current;
    if (scroller === null) return;
    if (was !== null) {
      scroller.scrollTop = was.top;
      const entry = scroller.querySelector<HTMLElement>(`[data-need="${CSS.escape(was.entry)}"]`);
      const control = entry === null ? null : safeControl(entry, was.control);
      (control ?? entry)?.focus({ preventScroll: true });
      return;
    }
    if (target === null) return;
    const entry = scroller.querySelector<HTMLElement>(`[data-need="${CSS.escape(target)}"]`);
    entry?.scrollIntoView({ block: "start" });
    entry?.focus({ preventScroll: true });
  }, [target, restore]);
  /** Keeps where the view is, for the item or thread an entry opens, then opens it. */
  const leave = (entry: string, control: string, open: () => void) => () => {
    onLeave({ top: page.current?.scrollTop ?? 0, entry, control });
    open();
  };
  return (
    <section ref={page} tabIndex={-1} className="needs-view" data-testid="needs" aria-labelledby="needs-title">
      <h1 id="needs-title" className="view-title">
        Needs you
      </h1>
      {views.length === 0 ? (
        <p className="empty">Nothing needs you. Conflicts, cycles and reopened work show here.</p>
      ) : null}
      <ul className="needs">
        {views.map((view, index) => {
          const eligible = isCollapsible(view);
          const isExpanded = wide || expanded.has(view.key);
          const collapsed = eligible && !isExpanded;
          const detailsId = `${disclosureId}-${index}`;
          return (
            <li
              key={view.key}
              data-need={view.key}
              data-at={view.key === target || undefined}
              // A row J and K move to, and the Overview opens the list at: focusable, but no control, so Enter on it does
              // nothing; Tab reaches its actions.
              data-row
              tabIndex={-1}
            >
              {view.thread === null ? (
                <p
                  className="facts line need-head"
                  style={lineStyle(view.line)}
                  data-collapsed={collapsed || undefined}
                >
                  <NeedBadge line={view.line} />
                  <Prose text={view.text} />
                </p>
              ) : (
                <button
                  type="button"
                  className="row line need-head"
                  style={lineStyle(view.line)}
                  data-pick-thread={view.thread}
                  data-collapsed={collapsed || undefined}
                  onClick={leave(
                    view.key,
                    ":scope > [data-pick-thread]",
                    () => view.thread && onPickThread(view.thread),
                  )}
                >
                  <Badge line={view.line} />
                  <Prose text={view.text} />
                </button>
              )}
              {eligible && !wide ? (
                <button
                  type="button"
                  className="need-disclosure-toggle"
                  aria-expanded={isExpanded}
                  aria-controls={detailsId}
                  onClick={() => toggle(view.key)}
                >
                  {isExpanded ? "Hide details" : "Show details"}
                </button>
              ) : null}
              <div id={detailsId} className="need-details" hidden={collapsed}>
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
                      <span className="row more">and {view.more} more</span>
                    </li>
                  )}
                </ul>
                {view.actions.length === 0 ? null : (
                  <div className="actions need-actions">
                    {view.actions.map((action) => (
                      <button
                        type="button"
                        key={action.label}
                        className="quiet"
                        data-action={action.action}
                        {...held(
                          `${view.key} ${action.action}`,
                          () => onCommand(action.command, action.done),
                          after(index),
                        )}
                      >
                        {action.label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function safeControl(entry: HTMLElement, selector: string): HTMLElement | null {
  try {
    return entry.querySelector<HTMLElement>(selector);
  } catch {
    return null;
  }
}
