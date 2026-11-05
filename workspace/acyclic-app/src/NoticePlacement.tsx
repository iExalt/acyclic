import { type ReactNode, type RefObject, useCallback, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { onMac, undoes } from "./commands";
import { SIDE_BY_SIDE } from "./wide";

type Mode = "none" | "slot" | "floating" | "disclosure";

/** One notice DOM, outside forms; pressure changes only its presentation. */
export function NoticePlacement({
  notice,
  phone,
  mapActive,
  stage,
  chrome,
  context,
  suspended,
}: {
  notice: ReactNode;
  phone: boolean;
  mapActive: boolean;
  stage: RefObject<HTMLElement | null>;
  chrome: RefObject<HTMLDivElement | null>;
  context: string;
  suspended: boolean;
}) {
  const id = useId();
  const origin = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const [container] = useState(() => {
    const node = document.createElement("div");
    node.className = "notice-portal";
    node.id = id;
    node.tabIndex = -1;
    node.setAttribute("role", "region");
    node.setAttribute("aria-label", "Notice");
    node.hidden = true;
    node.inert = true;
    return node;
  });
  const [mode, setMode] = useState<Mode>("none");
  const [open, setOpen] = useState(false);
  const modeNow = useRef<Mode>(mode);
  const openNow = useRef(open);
  const current = useRef({ notice, phone, mapActive, suspended });
  current.current = { notice, phone, mapActive, suspended };
  const lastFocus = useRef<HTMLElement | null>(null);
  const returnFocus = useRef(false);
  const opening = useRef(false);
  const previousContext = useRef(context);
  const update = useRef<(() => void) | null>(null);
  const ownsFocus = useCallback(() => {
    const active = document.activeElement;
    return (
      container.contains(active) ||
      active === trigger.current ||
      (active === document.body && lastFocus.current !== null && !lastFocus.current.isConnected)
    );
  }, [container]);
  const close = useCallback(
    (restore: boolean) => {
      if (restore && ownsFocus()) returnFocus.current = true;
      openNow.current = false;
      opening.current = false;
      setOpen(false);
    },
    [ownsFocus],
  );
  const triggerRef = useCallback((node: HTMLButtonElement | null) => {
    if (node === null && document.activeElement === trigger.current) returnFocus.current = true;
    trigger.current = node;
  }, []);

  useLayoutEffect(() => {
    let observed = new Set<Element>();
    const resize = new ResizeObserver(() => place());
    const place = () => {
      const host = origin.current;
      const map = stage.current;
      if (host === null || map === null) return;
      const panels = [...map.querySelectorAll<HTMLElement>(":scope > .inspector, :scope > .sheet")].filter(
        (panel) => panel.getBoundingClientRect().height > 0,
      );
      const targets = new Set<Element>([map, ...panels]);
      for (const panel of panels) {
        const header = panel.querySelector(":scope > header, :scope > .form-head");
        if (header !== null) targets.add(header);
      }
      const card = container.querySelector(".notice");
      if (card !== null) targets.add(card);
      for (const target of observed) if (!targets.has(target)) resize.unobserve(target);
      for (const target of targets) if (!observed.has(target)) resize.observe(target);
      observed = targets;
      const state = current.current;
      const area = map.getBoundingClientRect();
      const edge = Math.min(area.bottom, ...panels.map((panel) => panel.getBoundingClientRect().top));
      const fit = map.querySelector(".map-fit")?.getBoundingClientRect();
      const top = Math.max(area.top + 8, fit === undefined ? area.top + 8 : fit.bottom + 8);
      const bottom = edge - 8;
      const room = bottom - top;
      const next: Mode = !state.notice
        ? "none"
        : !state.phone || (!state.mapActive && panels.length === 0)
          ? "slot"
          : state.mapActive && room >= 44
            ? "floating"
            : "disclosure";
      if (next !== modeNow.current) {
        if (next === "disclosure" && container.contains(document.activeElement)) returnFocus.current = true;
        close(false);
        modeNow.current = next;
        setMode(next);
      }
      host.className = next === "slot" ? "slot" : "notice-origin";
      host.dataset.noticeMode = next;
      container.dataset.mode = next;
      const hidden = next === "none" || (next === "disclosure" && (!openNow.current || state.suspended));
      if (hidden && !state.suspended && container.contains(document.activeElement)) returnFocus.current = true;
      container.hidden = hidden;
      container.inert = container.hidden;
      if (next === "floating") {
        Object.assign(container.style, {
          position: "fixed",
          left: `${area.left + 8}px`,
          width: `${Math.max(0, area.width - 16)}px`,
          maxHeight: `${room}px`,
        });
        container.style.top = `${bottom - Math.min(container.scrollHeight, room)}px`;
      } else if (next === "disclosure") {
        const bar = chrome.current?.closest(".bar")?.getBoundingClientRect();
        const width = Math.min(260, window.innerWidth - 16);
        const left = Math.max(
          8,
          Math.min(chrome.current?.getBoundingClientRect().left ?? 14, window.innerWidth - width - 8),
        );
        const y = (bar?.bottom ?? 44) + 6;
        Object.assign(container.style, {
          position: "fixed",
          left: `${left}px`,
          top: `${y}px`,
          width: `${width}px`,
          maxHeight: `${Math.max(0, (window.visualViewport?.height ?? window.innerHeight) - y - 8)}px`,
        });
      } else container.removeAttribute("style");
    };
    update.current = place;
    origin.current?.append(container);
    const changes = new MutationObserver(place);
    if (stage.current !== null)
      changes.observe(stage.current, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["class", "data-detent", "data-dragging"],
      });
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || !openNow.current || modeNow.current !== "disclosure") return;
      event.preventDefault();
      event.stopPropagation();
      close(true);
    };
    const keys = (event: KeyboardEvent) => {
      if (modeNow.current === "disclosure" && openNow.current && !undoes(event, onMac())) event.stopPropagation();
    };
    const action = (event: MouseEvent) => {
      if (
        modeNow.current === "disclosure" &&
        openNow.current &&
        event.target instanceof Element &&
        event.target.closest("button") !== null
      )
        close(true);
    };
    const focus = (event: FocusEvent) => {
      if (!(event.target instanceof HTMLElement)) return;
      if (container.contains(event.target) || event.target === trigger.current) lastFocus.current = event.target;
      else {
        if (event.target !== document.body) lastFocus.current = null;
        if (openNow.current) close(false);
      }
    };
    const onResize = () => {
      // The responsive layout effect owns breakpoint changes after its phone prop commits.
      if (current.current.phone !== !matchMedia(SIDE_BY_SIDE).matches) return;
      if (modeNow.current === "disclosure" && openNow.current) close(false);
      place();
    };
    window.addEventListener("keydown", onEscape, true);
    window.addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("resize", onResize);
    document.addEventListener("focusin", focus);
    container.addEventListener("keydown", keys);
    container.addEventListener("click", action);
    place();
    return () => {
      update.current = null;
      resize.disconnect();
      changes.disconnect();
      window.removeEventListener("keydown", onEscape, true);
      window.removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("resize", onResize);
      document.removeEventListener("focusin", focus);
      container.removeEventListener("keydown", keys);
      container.removeEventListener("click", action);
      container.remove();
    };
  }, [container, stage, chrome, close]);

  useLayoutEffect(() => {
    if (context !== previousContext.current || suspended) close(!suspended);
    previousContext.current = context;
    // A replacement may remove the focused action without removing the notice itself.
    if (notice && document.activeElement === document.body && lastFocus.current && !lastFocus.current.isConnected)
      close(true);
    update.current?.();
    if (opening.current && openNow.current && modeNow.current === "disclosure") {
      opening.current = false;
      const target =
        container.querySelector<HTMLElement>(
          "input:not(:disabled), button:not(:disabled), select:not(:disabled), a[href]",
        ) ?? container;
      target.focus({ preventScroll: true });
      const bounds = container.getBoundingClientRect();
      const control = target.getBoundingClientRect();
      const top = bounds.top + container.clientTop;
      const bottom = top + container.clientHeight;
      if (control.bottom > bottom) container.scrollTop += control.bottom - bottom;
      else if (control.top < top) container.scrollTop -= top - control.top;
    }
    if (!notice && ownsFocus()) returnFocus.current = true;
    if (returnFocus.current && !suspended) {
      returnFocus.current = false;
      if (document.activeElement === document.body || ownsFocus()) {
        const target =
          modeNow.current === "disclosure"
            ? trigger.current
            : notice
              ? container
              : (chrome.current ?? document.querySelector<HTMLElement>("[data-action=search]"));
        target?.focus({ preventScroll: true });
      }
    }
  }, [notice, phone, mapActive, open, mode, context, suspended, container, chrome, close, ownsFocus]);

  return (
    <>
      <div ref={origin} className="notice-origin" />
      {createPortal(notice, container)}
      {mode === "disclosure" && notice && chrome.current !== null
        ? createPortal(
            <button
              ref={triggerRef}
              type="button"
              className="notice-trigger"
              data-testid="notice-trigger"
              aria-expanded={open}
              aria-controls={id}
              onKeyDown={(event) => {
                if (event.key !== "Escape" && !undoes(event.nativeEvent, onMac())) event.stopPropagation();
              }}
              onClick={() => {
                if (openNow.current) close(true);
                else {
                  opening.current = true;
                  openNow.current = true;
                  setOpen(true);
                }
              }}
            >
              Notice
            </button>,
            chrome.current,
          )
        : null}
      {open && mode === "disclosure" && !suspended ? (
        <button
          type="button"
          className="notice-backdrop"
          aria-label="Close notice details"
          tabIndex={-1}
          onPointerDown={(event) => event.preventDefault()}
          onClick={() => close(true)}
        />
      ) : null}
    </>
  );
}
