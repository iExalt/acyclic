// The bar's menu of what changes rarely. A disclosure, so it needs no script to open and browsers tell assistive
// technology whether it is expanded. While it is open, a backdrop drawn by its summary takes a tap anywhere else, so
// the tap closes the menu and nothing under it; Escape closes it too, and only it, and so does acting in it, so the
// next tap reaches what it lands on.

import { type ReactNode, useEffect, useRef } from "react";
import { Icon, MoreHorizontal } from "./icons";

/** Closes the menu, keeping focus where it was unless closing would hide it. */
function close(menu: HTMLDetailsElement | null) {
  if (menu === null || !menu.open) return;
  // Read first: once closed, the browser moves focus from what the menu hid to the page.
  const hidden = menu.contains(document.activeElement) || document.activeElement === document.body;
  menu.open = false;
  if (hidden) menu.querySelector("summary")?.focus();
}

/**
 * `up` opens the menu above its button, as at the foot of a sidebar; `list` makes it a list of actions, a row each,
 * rather than settings.
 */
export function Menu({
  children,
  up = false,
  list = false,
  label = "More",
  testid = "menu",
}: {
  children: ReactNode;
  up?: boolean;
  list?: boolean;
  label?: string;
  testid?: string;
}) {
  const details = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    // Once, and asking the element whether it's open, so a key pressed as it opens is never missed; in the capture
    // phase, to keep the Escape from also cancelling a form or a mode underneath.
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || details.current?.open !== true) return;
      event.stopPropagation();
      close(details.current);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  return (
    <details className={up ? "menu up" : "menu"} ref={details}>
      <summary aria-label={label} data-testid={testid}>
        <Icon of={MoreHorizontal} size={20} />
      </summary>
      {/* Its buttons close it on a click, its fields on a change, and its lists on Enter; keys reach each control. */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: listens for what its buttons, fields and lists already handle. */}
      <div
        className={list ? "menu-body menu-list" : "menu-body"}
        onClick={(event) => {
          if ((event.target as Element).closest("button") !== null) close(details.current);
        }}
        onChange={() => close(details.current)}
        // Enter on a list writes its draft, which acts as a pick does; a list with no draft keeps the key to itself.
        onKeyDown={(event) => {
          if (event.key !== "Enter" || !(event.target instanceof HTMLSelectElement)) return;
          // Its default would reach the summary that closing focuses, and open the menu again.
          event.preventDefault();
          close(details.current);
        }}
      >
        {children}
      </div>
    </details>
  );
}
