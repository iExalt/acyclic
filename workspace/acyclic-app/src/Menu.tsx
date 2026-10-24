// The bar's menu of what changes rarely. A disclosure, so it needs no script to open and browsers tell assistive
// technology whether it is expanded; Escape and a tap outside close it.

import { type ReactNode, useEffect, useRef, useState } from "react";

export function Menu({ children }: { children: ReactNode }) {
  const details = useRef<HTMLDetailsElement>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const close = () => {
      if (details.current !== null) details.current.open = false;
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      close();
      details.current?.querySelector("summary")?.focus();
    };
    const onPointer = (event: PointerEvent) => {
      if (!details.current?.contains(event.target as Node)) close();
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("pointerdown", onPointer);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onPointer);
    };
  }, [open]);
  return (
    <details className="menu" ref={details} onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary aria-label="More" data-testid="menu">
        <span aria-hidden="true">⋯</span>
      </summary>
      <div className="menu-body">{children}</div>
    </details>
  );
}
