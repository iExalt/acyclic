// Focus follows a panel: into it as it opens, so keys reach it without tabbing past the Map's thread rail, and back
// to what opened it as it closes, unless the user has moved on to something else meanwhile.

import { type RefObject, useEffect } from "react";

export function useFocusIn(panel: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const element = panel.current;
    // Not from a field the user is typing in elsewhere, nor from inside the panel itself.
    const typing = opener?.matches("input, textarea, select") === true;
    if (element !== null && !typing && !element.contains(opener)) element.focus({ preventScroll: true });
    return () => {
      // Closed, the panel took focus with it to the page; anywhere else, the user put it there.
      const lost = document.activeElement === null || document.activeElement === document.body;
      if (lost && opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [panel]);
}
