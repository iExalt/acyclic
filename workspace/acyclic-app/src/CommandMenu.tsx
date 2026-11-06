// The command menu: one field that finds anything in the plan, or something to do, and goes there; or, while a move or
// a link waits for its target, finds the target. A combobox over a list, so the field keeps the focus while ↑ and ↓
// choose; Enter or a tap runs the one chosen. Modal: drawn over the page, which is inert while it shows, so neither Tab
// nor a screen reader reaches what's under it. Escape and Back close it as they do any layer, a tap outside it too,
// and the focus goes back to where it was.

import { type ReactNode, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Stage } from "./api";
import { Badge, lineStyle } from "./Badge";
import { type Entry, onMac, type Run, rank } from "./commands";
import {
  ArrowUpLeft,
  FolderInput,
  Icon,
  Inbox,
  LayoutDashboard,
  ListTree,
  MapIcon,
  Pencil,
  Plus,
  RotateCw,
  Search,
  Waypoints,
  X,
} from "./icons";
import { keyReference } from "./keys";
import type { Line } from "./lines";
import { StageIcon } from "./Stage";

interface Props {
  entries: readonly Entry[];
  /** What the field asks for. */
  placeholder: string;
  /** What the field starts with: the key that opened the menu from a pick bar. */
  initial?: string;
  /** Each entry's single-key shortcut, by key, or null while they're switched off. */
  keys: Readonly<Record<string, string>> | null;
  /** Lists only what has a shortcut, as ? asks. */
  shortcutsOnly: boolean;
  /** The threads at work, listed before anything is typed. */
  working: ReadonlySet<string>;
  lines: ReadonlyMap<string, Line>;
  /** An item's stage, or "box" for a parent. */
  kindOf: (id: string) => Stage | "box" | undefined;
  onRun: (run: Run) => void;
  /** Closes the menu, as Escape does. */
  onDismiss: () => void;
}

export function CommandMenu({
  entries,
  placeholder,
  initial = "",
  keys,
  shortcutsOnly,
  working,
  lines,
  kindOf,
  onRun,
  onDismiss,
}: Props) {
  const [query, setQuery] = useState(initial);
  const keyboardSelection = useRef(
    window.matchMedia("(pointer: fine) and (hover: hover)").matches ||
      document.documentElement.dataset.keyboard === "true",
  );
  const [active, setActive] = useState(keyboardSelection.current ? 0 : -1);
  const pool = useMemo(
    () => (shortcutsOnly ? entries.filter((entry) => keys?.[entry.key] !== undefined) : entries),
    [entries, keys, shortcutsOnly],
  );
  const found = useMemo(() => rank(query, pool, working), [query, pool, working]);
  const list = useId();
  const field = useRef<HTMLInputElement>(null);
  /** What had the focus before the menu opened. */
  const before = useRef<Element | null>(null);
  const shown = found.length === 0 ? -1 : Math.min(active, found.length - 1);

  // The app under the menu inert while it shows. Leaving, the focus goes back to where it was, before anything runs,
  // so a panel what runs opens takes the focus from there, and gives it back there as it closes.
  const leave = () => {
    // Out of the field first, so a panel opening next doesn't take the field for where the user was typing.
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest(".command") !== null) active.blur();
    const app = document.getElementById("root");
    if (app !== null) app.inert = false;
    const was = before.current;
    if (was instanceof HTMLElement && was.isConnected) was.focus({ preventScroll: true });
  };
  useEffect(() => {
    // Once: run again, as React's strict mode does, the field would be where the user was.
    before.current ??= document.activeElement;
    const app = document.getElementById("root");
    if (app !== null) app.inert = true;
    field.current?.focus();
    return leave;
  }, []);
  const run = (entry: Entry | undefined) => {
    if (entry === undefined) return;
    leave();
    onRun(entry.run);
  };
  // The option chosen stays in view as ↑ and ↓ move through a long list.
  useEffect(() => {
    if (shown >= 0) document.getElementById(`${list}-${shown}`)?.scrollIntoView({ block: "nearest" });
  }, [list, shown]);

  const chooseByKeyboard = (next: number) => {
    keyboardSelection.current = true;
    setActive(next);
  };
  const move = (by: number) => {
    if (found.length === 0) return;
    setActive((was) => {
      const next = was < 0 ? (by > 0 ? 0 : found.length - 1) : (was + by + found.length) % found.length;
      return next;
    });
    keyboardSelection.current = true;
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    const to = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (to !== undefined) {
      event.preventDefault();
      move(to);
    } else if (event.key === "Home" && found.length > 0) {
      event.preventDefault();
      chooseByKeyboard(0);
    } else if (event.key === "End" && found.length > 0) {
      event.preventDefault();
      chooseByKeyboard(found.length - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (shown >= 0) run(found[shown]);
      else if (query.trim() !== "") run(found[0]);
    }
  };

  // Consecutive ranked results share a heading; when ranking returns to a group, its heading returns too.
  const groups: { name: string; heading: string; rows: ReactNode[] }[] = [];
  found.forEach((entry, n) => {
    if (groups.at(-1)?.name !== entry.group)
      groups.push({ name: entry.group, heading: `${list}-group-${entry.key}`, rows: [] });
    groups.at(-1)?.rows.push(
      // biome-ignore lint/a11y/useKeyWithClickEvents: the field's keys choose and run; a pointer runs one directly.
      <div
        key={entry.key}
        id={`${list}-${n}`}
        role="option"
        tabIndex={-1}
        aria-selected={n === shown}
        className="command-row"
        data-command={entry.key}
        onPointerMove={(event) => {
          if (event.pointerType === "mouse") {
            keyboardSelection.current = true;
            setActive(n);
          }
        }}
        onClick={() => run(entry)}
      >
        <span className="command-icon">{icon(entry, lines, kindOf)}</span>
        <span className="command-text">
          <span className="command-title">{entry.title}</span>
          {entry.detail === null ? null : <span className="command-detail">{entry.detail}</span>}
        </span>
        {/* For the eye: the buttons the keys stand for carry them for assistive technology. */}
        {keys?.[entry.key] === undefined ? null : (
          <span className="kbd command-key" aria-hidden="true">
            {keys[entry.key]}
          </span>
        )}
      </div>,
    );
  });

  // Over the page rather than in it, so it stays out of the app it makes inert.
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: a tap outside closes it; Escape is the keyboard's way.
    // biome-ignore lint/a11y/noStaticElementInteractions: the backdrop, not a control.
    <div
      className="command-backdrop"
      onClick={(event) => {
        if (event.target === event.currentTarget) onDismiss();
      }}
    >
      <div className="command" role="dialog" aria-modal="true" aria-label="Command menu" data-testid="command-menu">
        <div className="command-field">
          <Icon of={Search} />
          <input
            ref={field}
            type="text"
            role="combobox"
            aria-expanded="true"
            aria-controls={list}
            aria-activedescendant={shown < 0 ? undefined : `${list}-${shown}`}
            aria-autocomplete="list"
            aria-label={placeholder}
            placeholder={`${placeholder}…`}
            autoComplete="off"
            spellCheck={false}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(keyboardSelection.current ? 0 : -1);
            }}
            onKeyDown={onKeyDown}
          />
          {/* The way out without a keyboard, where a phone has no Back. */}
          <button type="button" className="close" aria-label="Close" onClick={onDismiss}>
            <Icon of={X} />
          </button>
        </div>
        <div id={list} role="listbox" aria-label="Results" className="command-list">
          {groups.map((group) => (
            // biome-ignore lint/a11y/useSemanticElements: a group of a listbox's options; a fieldset groups form controls.
            <div key={group.heading} role="group" aria-labelledby={group.heading}>
              <div id={group.heading} className="command-group">
                {group.name}
              </div>
              {group.rows}
            </div>
          ))}
        </div>
        {shortcutsOnly && keys !== null && query.trim() === "" ? (
          <section className="key-reference" aria-label="Other keys">
            <div className="command-group">Other keys</div>
            <dl>
              {keyReference(new Set(pool.map((entry) => entry.key)), onMac()).map((row) => (
                <div key={row.what}>
                  <dt>{row.what}</dt>
                  <dd>{row.keys}</dd>
                </div>
              ))}
            </dl>
          </section>
        ) : null}
        {found.length > 0 ? null : shortcutsOnly && keys === null ? (
          <p className="command-none muted" role="status">
            Single-key shortcuts are off: turn them on in the menu.
          </p>
        ) : (
          <p className="command-none muted" role="status">
            No matches for “{query.trim()}”
          </p>
        )}
      </div>
    </div>,
    document.body,
  );
}

/** What each entry is, at a glance: a thread's badge, an item's stage or a box's icon, an action's own icon. */
function icon(entry: Entry, lines: ReadonlyMap<string, Line>, kindOf: (id: string) => Stage | "box" | undefined) {
  const { run } = entry;
  if (run.kind === "thread") {
    const line = lines.get(run.id);
    return (
      <span className="line" style={lineStyle(line)}>
        <Badge line={line} />
      </span>
    );
  }
  if (run.kind === "project" || (run.kind === "target" && run.id === null)) return <Icon of={ListTree} size={16} />;
  if (run.kind === "item" || run.kind === "target") {
    const kind = run.id === null ? undefined : kindOf(run.id);
    if (kind === "box") return <Icon of={ListTree} size={16} />;
    return kind === undefined ? null : <StageIcon stage={kind} />;
  }
  if (run.kind === "view") {
    const of = { overview: LayoutDashboard, map: MapIcon, threads: Waypoints, needs: Inbox };
    return <Icon of={of[run.view]} size={16} />;
  }
  if (run.kind === "on-item") {
    const of = { edit: Pencil, move: FolderInput, "waits-for": ArrowUpLeft, "add-inside": Plus };
    return <Icon of={of[run.action]} size={16} />;
  }
  if (run.kind === "add") return <Icon of={Plus} size={16} />;
  return <Icon of={RotateCw} size={16} />;
}
