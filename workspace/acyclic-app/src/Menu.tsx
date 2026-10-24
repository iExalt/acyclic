// The bar's menu of what changes rarely. A disclosure, so it needs no script to open and browsers tell assistive
// technology whether it is expanded. While it is open, a backdrop drawn by its summary takes a tap anywhere else, so
// the tap closes the menu and nothing under it; Escape closes it too, and only it.

import { type ReactNode, useEffect, useRef } from "react";

export function Menu({ children }: { children: ReactNode }) {
  const details = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    // Once, and asking the element whether it's open, so a key pressed as it opens is never missed; in the capture
    // phase, to keep the Escape from also cancelling a form or a mode underneath.
    const onKey = (event: KeyboardEvent) => {
      const menu = details.current;
      if (event.key !== "Escape" || menu === null || !menu.open) return;
      event.stopPropagation();
      menu.open = false;
      menu.querySelector("summary")?.focus();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  return (
    <details className="menu" ref={details}>
      <summary aria-label="More" data-testid="menu">
        <span aria-hidden="true">⋯</span>
      </summary>
      <div className="menu-body">{children}</div>
    </details>
  );
}
