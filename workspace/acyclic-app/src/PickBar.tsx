// The bar a move or a link waits in, for a target to be tapped on the Map. It takes the focus as it shows, so typing
// goes to it: the keys find the target in the command menu (keys.ts), rather than reaching shortcuts under the Map.

import { type ReactNode, useRef } from "react";
import { useFocusIn } from "./focus";

export function PickBar({ children }: { children: ReactNode }) {
  const bar = useRef<HTMLDivElement>(null);
  useFocusIn(bar);
  return (
    <div ref={bar} className="sheet prompt" data-testid="prompt" data-pick-bar="" tabIndex={-1}>
      {children}
    </div>
  );
}
