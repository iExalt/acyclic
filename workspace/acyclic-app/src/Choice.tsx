// A setting picked from a list, written as choosing.ts decides: a pick at once, arrow keys as a draft until Enter.

import { type ReactNode, useState } from "react";
import { type ChoiceEvent, choose, UNCHOSEN } from "./choosing";

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
        onPointerDown={() => step({ kind: "pointer" })}
        onKeyDown={(event) => {
          if (event.key === "Enter") step({ kind: "enter" });
          else if (event.key === "Escape") step({ kind: "cancel" });
          else step({ kind: "key" });
        }}
        onChange={(event) => {
          // A draft isn't an act: it keeps a menu around it open.
          if (step({ kind: "change", value: event.target.value }).write === null) event.stopPropagation();
        }}
        onBlur={() => step({ kind: "cancel" })}
      >
        {children}
      </select>
    </label>
  );
}
