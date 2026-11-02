// The frame around the views: where the views, Needs you among them, adding and the menu are reached. A phone has one
// bar on top and the views along the bottom; a wider screen has a quiet sidebar.

import { type ReactNode, useState } from "react";
import { onMac } from "./commands";
import {
  Icon,
  Inbox,
  LayoutDashboard,
  ListTree,
  MapIcon,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Search,
  Waypoints,
} from "./icons";
import { Menu } from "./Menu";
import { countText } from "./needs";

export type View = "overview" | "map" | "threads" | "needs";

const MAC = onMac();
/** Where this device remembers the sidebar folded. */
const FOLDED_KEY = "acyclic.sidebar";

/** The view the app opens on, and Back on a phone returns to before it leaves the app. */
export const HOME: View = "overview";

const VIEWS: { view: View; label: string; icon: typeof MapIcon; key: string }[] = [
  { view: "overview", label: "Overview", icon: LayoutDashboard, key: "O" },
  { view: "map", label: "Map", icon: MapIcon, key: "M" },
  { view: "threads", label: "Threads", icon: Waypoints, key: "T" },
  { view: "needs", label: "Needs you", icon: Inbox, key: "N" },
];

export interface ShellProps {
  wide: boolean;
  /** How many items the plan shows. */
  count: number;
  /** The global autonomy level, as a label: said at the sidebar's foot; a phone's menu says it. */
  autonomy: string;
  view: View;
  onView: (view: View) => void;
  /** How many entries Needs you holds, which its tab counts. */
  needs: number;
  onAdd: () => void;
  /** The menu's controls. */
  menu: ReactNode;
  /** The outline beside the views on a wider screen, or null while the plan loads. */
  outline: ReactNode;
  /** On a phone, the outline is a sheet: whether it is open, and what opens or closes it. */
  outlineOpen: boolean;
  onOutline: () => void;
  /** Opens the command menu. */
  onSearch: () => void;
  /** Whether single-key shortcuts are on, so the controls they stand for say their keys. */
  shortcuts: boolean;
  /** The views and what opens over them. */
  children: ReactNode;
}

export function Shell(props: ShellProps) {
  if (!props.wide) return <PhoneShell {...props} />;
  return <SidebarShell {...props} />;
}

/** The views, each a destination: the one shown is the current page. Needs you's carries its count. */
function ViewLinks({ view, onView, shortcuts, needs }: Pick<ShellProps, "view" | "onView" | "shortcuts" | "needs">) {
  return VIEWS.map((one) => {
    const isNeeds = one.view === "needs";
    return (
      <button
        key={one.view}
        type="button"
        className={isNeeds ? "needs-button view" : "view"}
        data-view={one.view}
        data-none={isNeeds ? needs === 0 : undefined}
        aria-current={view === one.view ? "page" : undefined}
        aria-label={isNeeds ? `Needs you: ${needs} waiting` : undefined}
        // A sequence, which aria-keyshortcuts can't say: the title names it. Without it, the title still names the
        // icon, all a folded sidebar shows.
        title={shortcuts ? `${one.label} (G then ${one.key})` : one.label}
        onClick={() => onView(one.view)}
      >
        <Icon of={one.icon} />
        {isNeeds ? (
          // The count alone says something waits, as an inbox's does; with nothing waiting, none shows.
          <span className="label">
            <span className="text">{one.label}</span>
            {needs > 0 && (
              <span className="count needs-n" data-testid="needs-count">
                {countText(needs)}
              </span>
            )}
          </span>
        ) : (
          <span className="text">{one.label}</span>
        )}
      </button>
    );
  });
}

function Count({ count }: { count: number }) {
  return (
    <span className="items">
      <span className="count" data-testid="item-count">
        {count}
      </span>{" "}
      items
    </span>
  );
}

export function Wordmark() {
  return (
    <span className="wordmark">
      <svg viewBox="0 0 512 512" aria-hidden="true" focusable="false">
        <path d="M150 170 L256 340 L362 170" fill="none" stroke="currentColor" strokeWidth="40" strokeLinecap="round" />
        <circle cx="150" cy="170" r="56" />
        <circle cx="362" cy="170" r="56" />
        <circle cx="256" cy="340" r="56" />
      </svg>
      <span className="text">acyclic</span>
    </span>
  );
}

/**
 * A quiet sidebar holding the views, adding and the outline, the menu at its foot; all but the foot scroll
 * as one. Folded, a rail of the controls' icons, Needs you's count kept, its fold remembered on this device.
 */
function SidebarShell(props: ShellProps) {
  const [folded, setFolded] = useState(() => localStorage.getItem(FOLDED_KEY) === "folded");
  const fold = () => {
    localStorage.setItem(FOLDED_KEY, folded ? "open" : "folded");
    setFolded(!folded);
  };
  return (
    <div className="shell sidebar-shell" data-folded={folded}>
      <aside className="sidebar">
        <div className="workspace">
          <Wordmark />
          {/* In the same place folded or not, so a second press undoes the first. */}
          <button
            type="button"
            className="icon fold"
            aria-label={folded ? "Unfold the sidebar" : "Fold the sidebar"}
            title={folded ? "Unfold the sidebar" : "Fold the sidebar"}
            aria-expanded={!folded}
            data-action="fold"
            onClick={fold}
          >
            <Icon of={folded ? PanelLeftOpen : PanelLeftClose} />
          </button>
        </div>
        <div className="sidebar-body">
          <button
            type="button"
            className="search"
            onClick={props.onSearch}
            data-action="search"
            title="Search or jump to…"
            aria-keyshortcuts={`${MAC ? "Meta+K" : "Control+K"}${props.shortcuts ? " /" : ""}`}
          >
            <Icon of={Search} />
            <span className="grow text">Search or jump to…</span>
            {/* The shortcut, for the eye; assistive technology has it from aria-keyshortcuts. */}
            <span className="kbd" aria-hidden="true">
              {MAC ? "⌘K" : "Ctrl K"}
            </span>
          </button>
          <nav className="views" aria-label="Views">
            <ViewLinks view={props.view} onView={props.onView} shortcuts={props.shortcuts} needs={props.needs} />
          </nav>
          <button
            type="button"
            className="new"
            onClick={props.onAdd}
            data-action="add-top"
            aria-keyshortcuts={props.shortcuts ? "C" : undefined}
            title={props.shortcuts ? "New item (C)" : "New item"}
          >
            <Icon of={Plus} />
            <span className="text">New item</span>
          </button>
          {folded ? null : (
            <nav className="threads" aria-label="Projects and threads">
              {props.outline}
            </nav>
          )}
        </div>
        <footer className="sidebar-foot">
          <div className="about">
            <Count count={props.count} />
            <Autonomy level={props.autonomy} />
          </div>
          <Menu up>{props.menu}</Menu>
        </footer>
      </aside>
      {props.children}
    </div>
  );
}

/**
 * The level, which a word alone leaves unexplained: pressed, it opens the menu at the setting, which says in words what
 * the level lets reach the user.
 */
function Autonomy({ level }: { level: string }) {
  const open = (from: HTMLElement) => {
    const menu = from.closest(".shell")?.querySelector("summary[data-testid=menu]")?.parentElement;
    if (!(menu instanceof HTMLDetailsElement)) return;
    menu.open = true;
    requestAnimationFrame(() => menu.querySelector<HTMLElement>("[data-testid=global-level]")?.focus());
  };
  return (
    <button
      type="button"
      className="autonomy"
      title="What agents ask you about: change it in the menu"
      aria-haspopup="true"
      onClick={(event) => open(event.currentTarget)}
    >
      Autonomy: {level}
    </button>
  );
}

/** A phone: the view's name with the outline, adding and the menu on top; the views along the bottom. */
function PhoneShell(props: ShellProps) {
  const title = VIEWS.find((one) => one.view === props.view)?.label;
  return (
    <div className="shell phone-shell">
      <header className="bar">
        {/* The view's name alone: the menu's Autonomy says the level, as the sidebar's foot does wide. */}
        <div className="brand">
          <h1>{title}</h1>
        </div>
        <button type="button" className="icon" aria-label="Search" onClick={props.onSearch} data-action="search">
          <Icon of={Search} size={20} />
        </button>
        <button
          type="button"
          className="icon projects-button"
          data-testid="projects-button"
          aria-label="Outline"
          aria-pressed={props.outlineOpen}
          onClick={props.onOutline}
        >
          <Icon of={ListTree} size={20} />
        </button>
        <button type="button" className="icon" aria-label="New item" onClick={props.onAdd} data-action="add-top">
          <Icon of={Plus} size={20} />
        </button>
        <Menu>{props.menu}</Menu>
      </header>
      {props.children}
      <nav className="tabbar" aria-label="Views">
        <ViewLinks view={props.view} onView={props.onView} shortcuts={props.shortcuts} needs={props.needs} />
      </nav>
    </div>
  );
}
