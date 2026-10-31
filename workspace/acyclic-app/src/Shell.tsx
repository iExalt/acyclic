// The frame around the views: where the views, Needs you, adding and the menu are reached. A phone has one bar on top
// and the views along the bottom; a wider screen has look A's sidebar or look B's header and tabs. Only the frame
// differs between the looks: what is inside it is the same.

import type { ReactNode } from "react";
import { Icon, Inbox, ListTree, MapIcon, Plus, Waypoints } from "./icons";
import type { Look } from "./look";
import { Menu } from "./Menu";
import { countText } from "./needs";

export type View = "map" | "threads";

const VIEWS: { view: View; label: string; icon: typeof MapIcon }[] = [
  { view: "map", label: "Map", icon: MapIcon },
  { view: "threads", label: "Threads", icon: Waypoints },
];

export interface ShellProps {
  look: Look;
  wide: boolean;
  /** How many items the plan shows. */
  count: number;
  /** The global autonomy level, as a label: safety-relevant, so always in view. */
  autonomy: string;
  view: View;
  onView: (view: View) => void;
  needs: number;
  needsOpen: boolean;
  onNeeds: () => void;
  onAdd: () => void;
  /** The menu's controls. */
  menu: ReactNode;
  /** The outline beside the views on a wider screen, or null while the plan loads. */
  outline: ReactNode;
  /** On a phone, the outline is a sheet: whether it is open, and what opens or closes it. */
  outlineOpen: boolean;
  onOutline: () => void;
  /** The views and what opens over them. */
  children: ReactNode;
}

export function Shell(props: ShellProps) {
  if (!props.wide) return <PhoneShell {...props} />;
  return props.look === "a" ? <SidebarShell {...props} /> : <HeaderShell {...props} />;
}

/** The views, each a destination: the one shown is the current page. */
function ViewLinks({ view, onView, short }: Pick<ShellProps, "view" | "onView"> & { short?: boolean }) {
  return VIEWS.map((one) => (
    <button
      key={one.view}
      type="button"
      className="view"
      data-view={one.view}
      aria-current={view === one.view ? "page" : undefined}
      onClick={() => onView(one.view)}
    >
      {short ? null : <Icon of={one.icon} />}
      <span>{one.label}</span>
    </button>
  ));
}

/** Needs you: a layer over the views, so a toggle rather than a destination. */
function NeedsButton({
  needs,
  needsOpen,
  onNeeds,
  compact,
}: Pick<ShellProps, "needs" | "needsOpen" | "onNeeds"> & {
  compact?: boolean;
}) {
  return (
    <button
      type="button"
      className="needs-button view"
      data-testid="needs-button"
      data-none={needs === 0}
      aria-pressed={needsOpen}
      aria-label={`Needs you: ${needs} waiting`}
      onClick={onNeeds}
    >
      <Icon of={Inbox} />
      {/* A "!" rather than the count beside the icon, so the signal never looks like a thread's numbered badge; with
          nothing waiting, neither shows. */}
      {needs > 0 && (
        <span className="badge needs-mark" data-testid="needs-badge" aria-hidden="true">
          !
        </span>
      )}
      <span className={compact ? "label short" : "label"}>
        Needs you
        {needs > 0 && (
          <span className="count needs-n" data-testid="needs-count">
            {countText(needs)}
          </span>
        )}
      </span>
    </button>
  );
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

function Wordmark() {
  return (
    <span className="wordmark">
      <svg viewBox="0 0 512 512" aria-hidden="true" focusable="false">
        <path d="M150 170 L256 340 L362 170" fill="none" stroke="currentColor" strokeWidth="40" strokeLinecap="round" />
        <circle cx="150" cy="170" r="56" />
        <circle cx="362" cy="170" r="56" />
        <circle cx="256" cy="340" r="56" />
      </svg>
      acyclic
    </span>
  );
}

/** Look A: a quiet sidebar holding the views, Needs you, adding and the outline, the menu at its foot. */
function SidebarShell(props: ShellProps) {
  return (
    <div className="shell sidebar-shell">
      <aside className="sidebar">
        <div className="workspace">
          <Wordmark />
        </div>
        <nav className="views" aria-label="Views">
          <ViewLinks view={props.view} onView={props.onView} />
          <NeedsButton needs={props.needs} needsOpen={props.needsOpen} onNeeds={props.onNeeds} />
        </nav>
        <button type="button" className="new" onClick={props.onAdd} data-action="add-top">
          <Icon of={Plus} />
          New item
        </button>
        <nav className="threads" aria-label="Phases and threads">
          {props.outline}
        </nav>
        <footer className="sidebar-foot">
          <div className="about">
            <Count count={props.count} />
            <span className="autonomy">Autonomy: {props.autonomy}</span>
          </div>
          <Menu up>{props.menu}</Menu>
        </footer>
      </aside>
      {props.children}
    </div>
  );
}

/** Look B: a header with the product, Needs you, adding and the menu, over tabs for the views; the outline at the left. */
function HeaderShell(props: ShellProps) {
  return (
    <div className="shell header-shell">
      <header className="bar">
        <Wordmark />
        <Count count={props.count} />
        <span className="grow" />
        <span className="autonomy">Autonomy: {props.autonomy}</span>
        <NeedsButton needs={props.needs} needsOpen={props.needsOpen} onNeeds={props.onNeeds} compact />
        <button type="button" className="primary new" onClick={props.onAdd} data-action="add-top">
          <Icon of={Plus} />
          New item
        </button>
        <Menu>{props.menu}</Menu>
      </header>
      <nav className="tabs" aria-label="Views">
        <ViewLinks view={props.view} onView={props.onView} />
      </nav>
      <nav className="threads" aria-label="Phases and threads">
        {props.outline}
      </nav>
      {props.children}
    </div>
  );
}

/** A phone: the view's name with the outline, adding and the menu on top; the views and Needs you along the bottom. */
function PhoneShell(props: ShellProps) {
  const title = VIEWS.find((one) => one.view === props.view)?.label;
  return (
    <div className="shell phone-shell">
      <header className="bar">
        <div className="brand">
          <h1>{title}</h1>
          <p>
            <Count count={props.count} />, autonomy {props.autonomy}
          </p>
        </div>
        <button
          type="button"
          className="icon phases-button"
          data-testid="phases-button"
          aria-label="Outline"
          aria-pressed={props.outlineOpen}
          onClick={props.onOutline}
        >
          <Icon of={ListTree} size={22} />
        </button>
        <button type="button" className="icon" aria-label="New item" onClick={props.onAdd} data-action="add-top">
          <Icon of={Plus} size={22} />
        </button>
        <Menu>{props.menu}</Menu>
      </header>
      {props.children}
      <nav className="tabbar" aria-label="Views">
        <ViewLinks view={props.view} onView={props.onView} />
        <NeedsButton needs={props.needs} needsOpen={props.needsOpen} onNeeds={props.onNeeds} />
      </nav>
    </div>
  );
}
