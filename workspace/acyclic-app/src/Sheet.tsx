// A phone's sheet: its handle, which says the sheet is a layer and switches it between half and full height, and the
// drag on the handle or the header that follows the finger and lands where sheets.ts says. Closing by a drag goes the
// way Escape and Back do, so a form holding typing stays open and says how to leave it. Wide, the panel is a pane and
// none of this shows.

import { type PointerEvent, type ReactNode, type RefObject, useRef, useState } from "react";
import { type Detent, detentAfter, draggableFrom, type Sample, SLOP, velocity } from "./sheets";

export interface SheetDrag {
  detent: Detent;
  /** The handle, first in the sheet. */
  handle: ReactNode;
  /** For the sheet's header: a drag from anywhere in it but its controls. */
  header: {
    onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  };
}

/** The sheet in `panel`, opening at `initial`; `onClose` closes it as Escape does, and may refuse. */
export function useSheet(panel: RefObject<HTMLElement | null>, initial: Detent, onClose: () => void): SheetDrag {
  const [detent, setDetent] = useState<Detent>(initial);
  /** A press that may become a drag: where it started, the sheet's height then, and its moves since. */
  const press = useRef<{ id: number; y: number; height: number; dragging: boolean; samples: Sample[] } | null>(null);
  /** A drag just ended on the handle: the click that follows it isn't a tap. */
  const dragged = useRef(false);

  const start = (event: PointerEvent<HTMLElement>) => {
    const sheet = panel.current;
    if (sheet === null || event.button !== 0 || !phone()) return;
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
      dragged.current = true;
      delete sheet.dataset.dragging;
      sheet.style.removeProperty("--dragged");
      if (ended.type === "pointercancel") return;
      const room = sheet.parentElement?.getBoundingClientRect().height ?? now.height;
      const half = halfOf(room);
      const landed = detentAfter(detent, ended.clientY - now.y, velocity(now.samples), room - half, half);
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
        setDetent((was) => (was === "full" ? "half" : "full"));
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

/** The half sheet's height in a stage `room` tall, as the stylesheet sets it: half, but at least 320 px. */
function halfOf(room: number): number {
  return Math.min(room, Math.max(320, room / 2));
}

const phone = () => window.matchMedia("(max-width: 899px)").matches;
