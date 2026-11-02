// A phone's sheet: its handle, which says the sheet is a layer and switches it between its first and full height, and
// the drag on the handle or the header that follows the finger and lands where sheets.ts says. Closing by a drag goes
// the way Escape and Back do, so a form holding typing stays open and says how to leave it. Wide, the panel is a pane
// and none of this shows.

import { type PointerEvent, type ReactNode, type RefObject, useLayoutEffect, useRef, useState } from "react";
import { type Detent, detentAfter, draggableFrom, firstHeight, type Sample, SLOP, velocity } from "./sheets";

export interface SheetDrag {
  detent: Detent;
  /** The handle, first in the sheet. */
  handle: ReactNode;
  /** For the sheet's header: a drag from anywhere in it but its controls. */
  header: {
    onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  };
}

/**
 * The sheet in `panel`, opening at its first height, which is short for a panel and `half` the room for the outline (as
 * app.css sets them); `onClose` closes it as Escape does, and may refuse. A panel's main action, marked `data-main`,
 * shows at the first height however its title and status wrap: the sheet measures where it ends, for app.css (as
 * `--first`) and for where a drag lands.
 */
export function useSheet(panel: RefObject<HTMLElement | null>, onClose: () => void, half = false): SheetDrag {
  const [detent, setDetent] = useState<Detent>("first");
  /** How tall the first height must be to show the main action, 0 without one. */
  const needs = useRef(0);
  // After every render, as what the panel holds changes, and whenever its header rewraps, as when the window turns.
  useLayoutEffect(() => {
    const sheet = panel.current;
    if (sheet === null || half) return;
    const measure = () => {
      const main = sheet.querySelector<HTMLElement>(":scope > [data-main]");
      const measured =
        main === null
          ? 0
          : Math.ceil(
              sheet.clientTop +
                main.offsetTop +
                main.offsetHeight +
                Number.parseFloat(getComputedStyle(sheet).paddingBottom),
            );
      // Written only when it changes: the sheet's rise to it is a transition, which a write each render would hold back.
      if (measured === needs.current && sheet.style.getPropertyValue("--first") !== "") return;
      needs.current = measured;
      sheet.style.setProperty("--first", `${measured}px`);
    };
    measure();
    const header = sheet.querySelector(":scope > header");
    if (header === null) return;
    const watch = new ResizeObserver(measure);
    watch.observe(header);
    return () => watch.disconnect();
  });
  /** A press that may become a drag: where it started, the sheet's height then, and its moves since. */
  const press = useRef<{ id: number; y: number; height: number; dragging: boolean; samples: Sample[] } | null>(null);
  /** A drag just ended on the handle: the click that follows it isn't a tap. */
  const dragged = useRef(false);

  const start = (event: PointerEvent<HTMLElement>) => {
    const sheet = panel.current;
    // One press at a time: a second finger while one drags is the first's to finish.
    if (sheet === null || event.button !== 0 || !phone() || press.current !== null) return;
    dragged.current = false;
    /** Only a drag on the handle is followed by a click on it, which mustn't then switch the height. */
    const onHandle = event.currentTarget.classList.contains("sheet-handle");
    press.current = {
      id: event.pointerId,
      y: event.clientY,
      height: sheet.getBoundingClientRect().height,
      dragging: false,
      samples: [{ t: event.timeStamp, y: event.clientY }],
    };
    const target = event.currentTarget;
    const move = (moved: globalThis.PointerEvent) => {
      const now = press.current;
      if (now === null || moved.pointerId !== now.id) return;
      const dy = moved.clientY - now.y;
      if (!now.dragging) {
        if (Math.abs(dy) < SLOP) return;
        now.dragging = true;
        target.setPointerCapture(now.id);
        sheet.dataset.dragging = "true";
      }
      now.samples.push({ t: moved.timeStamp, y: moved.clientY });
      // Follows the finger: taller while it rises, lower while it falls, never past the top.
      sheet.style.setProperty("--dragged", `${Math.max(0, now.height - dy)}px`);
    };
    const end = (ended: globalThis.PointerEvent) => {
      const now = press.current;
      if (now === null || ended.pointerId !== now.id) return;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      press.current = null;
      if (!now.dragging) return;
      dragged.current = onHandle;
      // The release is the last sample: a finger held still before it lifts has stopped, whatever it did before.
      now.samples.push({ t: ended.timeStamp, y: ended.clientY });
      delete sheet.dataset.dragging;
      sheet.style.removeProperty("--dragged");
      if (ended.type === "pointercancel") return;
      const room = sheet.parentElement?.getBoundingClientRect().height ?? now.height;
      const first = firstHeight(room, half, needs.current);
      const landed = detentAfter(detent, ended.clientY - now.y, velocity(now.samples), room - first, first);
      if (landed === "close") onClose();
      else setDetent(landed);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  const handle = (
    <button
      type="button"
      className="sheet-handle"
      aria-label="Sheet size"
      aria-expanded={detent === "full"}
      onPointerDown={start}
      onClick={() => {
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        setDetent((was) => (was === "full" ? "first" : "full"));
      }}
    />
  );
  return {
    detent,
    handle,
    header: {
      onPointerDown: (event) => {
        if (draggableFrom(event.target instanceof Element ? event.target : null)) start(event);
      },
    },
  };
}

const phone = () => window.matchMedia("(max-width: 899px)").matches;
