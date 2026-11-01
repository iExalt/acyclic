// A panel's one-press actions while their write is out: held rather than disabled, since a disabled button loses the
// focus it had, and once the write lands, the keyboard kept in the panel when the button went with what it settled.
// Only the pressed one looks held; the rest wait unchanged, as greying every button said they all were at work.

import { useEffect, useRef, useState } from "react";

/** Where focus goes once the write lands, if the pressed button has gone. */
type Next = () => HTMLElement | null | undefined;

/**
 * The props for the action `key`'s button: inert while `busy`, held-looking if it was the one pressed, and on press,
 * remembering where focus goes next. `settled`
 * changes when the panel shows what the write left, which may be later than the write itself, as a panel that reads
 * its subject again does.
 */
export function useHeld(busy: boolean, settled: unknown) {
  const pending = useRef<{ next: Next; settled: unknown; landed: boolean } | null>(null);
  const [pressed, setPressed] = useState<string | null>(null);
  useEffect(() => {
    if (!busy) setPressed(null);
    const was = pending.current;
    if (was === null) return;
    // A write after this one's has landed is another's: what this one pressed no longer stands to go.
    if (busy) {
      if (was.landed) pending.current = null;
      return;
    }
    was.landed = true;
    const active = document.activeElement;
    if (active === null || active === document.body) {
      pending.current = null;
      was.next()?.focus();
    } else if (settled !== was.settled) pending.current = null;
  }, [busy, settled]);
  return (key: string, act: () => void, next: Next) => ({
    "aria-disabled": (busy && pressed === key) || undefined,
    "aria-busy": (busy && pressed === key) || undefined,
    onClick: () => {
      if (busy) return;
      pending.current = { next, settled, landed: false };
      setPressed(key);
      act();
    },
  });
}
