// Focus follows a panel: into it as it opens, so keys reach it without tabbing past the Map's thread rail, and back
// to what opened it as it closes, unless the user has moved on to something else meanwhile.

import { type RefObject, useEffect, useRef } from "react";

export function useFocusIn(panel: RefObject<HTMLElement | null>) {
  // Read while rendering, before the panel commits and a field in it takes focus for itself.
  const opened = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  useEffect(() => {
    const opener = opened.current;
    const element = panel.current;
    // Only while focus is still where it was, or lost: not once a field in the panel has taken it, nor once a sheet
    // closing over this one has given it back to what opened that sheet. Nor from a field being typed in elsewhere.
    // But if what had it went with a sheet that opened this panel as it closed, as a new item's form does, focus
    // given back to what opened that sheet is only on its way here.
    const now = document.activeElement;
    const gone = opener !== null && !opener.isConnected;
    const still = gone || now === opener || now === null || now === document.body;
    const typing = !gone && opener?.matches("input, textarea, select") === true;
    if (element !== null && still && !typing && !element.contains(opener)) element.focus({ preventScroll: true });
    return () => {
      // Closed, the panel took focus with it to the page; anywhere else, the user put it there.
      const lost = document.activeElement === null || document.activeElement === document.body;
      if (lost && opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [panel]);
}

/**
 * Keeps the field being typed in on screen as the view shrinks under it, as a keyboard rising does, where the browser
 * leaves it where it was; the sheet's scroll padding keeps it clear of the form's Save and Cancel.
 */
export function useFieldInView(form: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const keep = () => {
      const field = document.activeElement;
      if (field instanceof HTMLElement && form.current?.contains(field)) field.scrollIntoView({ block: "nearest" });
    };
    const viewport = window.visualViewport ?? window;
    viewport.addEventListener("resize", keep);
    return () => viewport.removeEventListener("resize", keep);
  }, [form]);
}
