// A setting picked from a list, written as choosing.ts decides: a pick at once, arrow keys as a draft until Enter.

import { type ReactNode, useRef, useState } from "react";
import { type ChoiceEvent, choose, UNCHOSEN } from "./choosing";

/** The keys that open a closed list rather than step through it. */
const opens = (event: { key: string; altKey: boolean }) =>
  event.key === " " || event.key === "F4" || (event.altKey && (event.key === "ArrowDown" || event.key === "ArrowUp"));

export interface ChoiceProps {
  /** What the list sets, said before it. */
  label: ReactNode;
  className?: string;
  value: string;
  disabled: boolean;
  onWrite: (value: string) => void;
  name?: string;
  testid?: string;
  children: ReactNode;
}

export function Choice({ label, className, value, disabled, onWrite, name, testid, children }: ChoiceProps) {
  const [state, setState] = useState(UNCHOSEN);
  /** Set while a key's own task runs: a change then is an arrow key's on the closed list, not a pick. */
  const keying = useRef(false);
  const step = (event: ChoiceEvent) => {
    const next = choose(state, value, event);
    setState(next.state);
    if (next.write !== null) onWrite(next.write);
    return next;
  };
  return (
    <label className={className}>
      {label}
      <select
        name={name}
        data-testid={testid}
        value={state.draft ?? value}
        disabled={disabled}
        onKeyDown={(event) => {
          if (event.key === "Enter") step({ kind: "enter" });
          else if (event.key === "Escape") step({ kind: "cancel" });
          // A key that only opens the list changes nothing itself: the pick comes later, from the list.
          if (opens(event)) return;
          keying.current = true;
          setTimeout(() => {
            keying.current = false;
          }, 0);
        }}
        onChange={(event) => {
          // A draft isn't an act: it keeps a menu around it open.
          if (step({ kind: "change", value: event.target.value, keyed: keying.current }).write === null)
            event.stopPropagation();
        }}
        onBlur={() => step({ kind: "cancel" })}
      >
        {children}
      </select>
    </label>
  );
}
