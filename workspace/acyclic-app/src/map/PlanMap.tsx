// The Map: the plan drawn as nested boxes on a canvas, with Cytoscape.js placing nodes exactly where the scoped
// layout put them. Tapping a box's chevron collapses or expands it, and the box stays where it was on screen; tapping
// anything else selects it, lighting up its thread and drawing its links across boxes. While an edit waits for a
// target, a tap picks one instead. Dragging pans, the wheel zooms; nothing moves items.

import cytoscape, { type Core } from "cytoscape";
import { type RefObject, useEffect, useMemo, useRef, useState } from "react";
import type { Item } from "../api";
import { chain, expandTo } from "../edits";
import type { Line } from "../lines";
import type { Palette } from "../palette";
import { useWide } from "../wide";
import { onChevron, STRIP } from "./chevron";
import { browserLayouts } from "./client";
import { apply, type Drawn, elementsOf, land, prefersReducedMotion, settle, settledBox } from "./draw";
import { PORTRAIT, shapeAfter } from "./elk";
import { FRESH_WATCH, intentsPending, stepFrame } from "./framed";
import { createGesture } from "./gesture";
import { metricsOf, TILE_WIDTH } from "./labelmetrics";
import type { Placed } from "./layout";
import { buildModel, type Crossing, collapsedWith, type Model, sourceKey, topOf, type View, visible } from "./model";
import { busy, subscribe, track } from "./motion";
import {
  anchorPan,
  firstView,
  fitAll,
  fitPlan,
  focusReveal,
  framesAll,
  headed,
  newlyCovered,
  READABLE,
  type RevealIntent,
  reveal,
  startOf,
  uncovered,
} from "./reveal";
import { type Fonts, type Scene, type SceneNode, type Selection, scene, UNMEASURED } from "./scene";
import { styleFor, WEIGHT } from "./style";
import { stubTap, tapAction } from "./tap";
import { wheelZoom } from "./wheel";

/** The furthest the Map zooms in. */
const MAX_ZOOM = 2;
/** The first view's furthest zoom, on every screen: 1:1, titles at their 13 px, as a phone reads them too. */
const FIRST_ZOOM = 1;
/** What the pointer shows the links of: a tile, or a collapsed box, which holds its tiles' links. */
const HOVERABLE = "node.leaf, node.collapsed";
/** The face labels are drawn in, loaded first: the canvas draws with whatever is loaded. */
const FALLBACK_FONT = "system-ui, sans-serif";
const fontReady: Promise<string> =
  typeof document === "undefined" || !("fonts" in document)
    ? Promise.resolve(FALLBACK_FONT)
    : Promise.all([
        document.fonts.load(`${WEIGHT.regular} 13px Overpass`),
        document.fonts.load(`${WEIGHT.bold} 13px Overpass`),
      ])
        .then(([regular, bold]) =>
          regular.length > 0 &&
          bold.length > 0 &&
          document.fonts.check(`${WEIGHT.regular} 13px Overpass`) &&
          document.fonts.check(`${WEIGHT.bold} 13px Overpass`)
            ? "Overpass, system-ui, sans-serif"
            : FALLBACK_FONT,
        )
        .catch(() => FALLBACK_FONT);

/**
 * Measures labels as the renderer draws them, once the face is loaded, which the layout waits for before anything is
 * drawn. Each text is measured once.
 */
function measuring(family: string): Fonts {
  const context = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (context === null) return UNMEASURED;
  const face = (weight: number) => {
    const known = new Map<string, number>();
    return (text: string) => {
      let width = known.get(text);
      if (width === undefined) {
        context.font = `${weight} 13px ${family}`;
        width = context.measureText(text).width;
        known.set(text, width);
      }
      return width;
    };
  };
  return { regular: face(WEIGHT.regular), bold: face(WEIGHT.bold) };
}

export interface PlanMapProps {
  items: Item[];
  /** The thread picked in the sidebar, if any. */
  thread: string | null;
  /** Each thread's line, whose colour lights its items. */
  lines: ReadonlyMap<string, Line>;
  /** The colours to draw in, which change with the system's scheme. */
  palette: Palette;
  selected: string | null;
  onSelect: (item: string | null) => void;
  /**
   * An item to bring into view, expanding the boxes above it. Writes can preserve its existing root frame and zoom;
   * otherwise `pin` (or its nearest drawn ancestor) holds still on screen. A new object asks again.
   */
  focus?: Focus | undefined;
  /**
   * A thread to frame, its items fitted in the part of the Map nothing covers. A new object asks again, and an ask
   * waits for the Map to be drawn, even once the prop has moved on.
   */
  framed?: Framed | undefined;
  /** Set while an edit waits for a target: a tap picks an item, or the background (null), and toggles nothing. */
  onPick: ((item: string | null) => void) | null;
  /** The waits each item has on items outside `items`, drawn as stubs on its tile. */
  outside?: ReadonlyMap<string, Crossing[]> | undefined;
  /** A stub tapped: the item it names is opened, in its own initiative. */
  onOpen?: ((item: string) => void) | undefined;
  /** Whether the Map view is active, so its controls and loading message don't cover another view. */
  active: boolean;
}

export interface Framed {
  thread: string;
}

export interface Focus {
  id: string;
  pin: string | null;
  preserveFrame?: true;
}

export default function PlanMap({
  items,
  thread,
  lines,
  palette,
  selected,
  onSelect,
  focus,
  framed,
  onPick,
  outside,
  onOpen,
  active,
}: PlanMapProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const layouts = useMemo(browserLayouts, []);
  const model = useMemo(() => buildModel(items, outside), [items, outside]);
  // Only named threads have a membership label. Palette changes do not change layout.
  const names = JSON.stringify(
    [...lines].filter(([, line]) => line.task === null).map(([id, line]) => [id, line.name]),
  );
  const named = useMemo(() => new Map<string, string>(JSON.parse(names)), [names]);
  const source = useMemo(() => `${sourceKey(model)}:${names}`, [model, names]);
  /** The boxes the user has collapsed (true) or expanded (false); the rest follow the default view. */
  const [toggles, setToggles] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const [hovered, setHovered] = useState<string | null>(null);
  const [drawnOnce, setDrawnOnce] = useState(false);
  const [fontFamily, setFontFamily] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void fontReady.then((family) => {
      if (live) setFontFamily(family);
    });
    return () => {
      live = false;
    };
  }, []);
  /** The last layout, with the view and the top level's shape it was computed for. */
  /**
   * Whether a pan, pinch or wheel zoom is going on: a repack of the top level waits for it to end, so tiles never move
   * under the user's fingers. Selection and hover never wait.
   */
  const gesture = useMemo(() => {
    let hold: (() => void) | null = null;
    return createGesture({
      onPending: (pending) => {
        if (pending) hold = track();
        else {
          hold?.();
          hold = null;
        }
      },
    });
  }, []);
  const [laid, setLaid] = useState<{ view: View; aspect: number; source: string; placed: Placed } | null>(null);
  const aspect = useAspect(host);
  /**
   * The top-level box holding the box last toggled, and where its top-left corner was on screen, to hold it there: the
   * top level keeps its boxes where they were, so the rest of the plan stays put while the toggled box opens in place.
   */
  const anchor = useRef<{ id: string; x: number; y: number } | null>(null);
  /** An item brought into view: once drawn, it is panned out from under whatever covers the Map, if anything does. */
  const shown = useRef<RevealIntent | null>(null);
  const shownBeforeRender = shown.current;
  const anchorBeforeRender = anchor.current;
  const waitingFor = useRef(new WeakMap<Animation, RevealIntent>());
  const watchedAnimations = useRef(new WeakSet<Animation>());
  /** The reveal last found unable to be satisfied, which no longer holds the camera as unframed (see `framed.ts`). */
  const parked = useRef<RevealIntent | null>(null);
  /** Taps whose late reveal has not yet run. */
  const tapReveals = useRef(0);
  const watching = useRef<number | null>(null);
  /**
   * The motion in flight, which keeps the Map from settling: tiles gliding, links fading in, and a redraw held back by a
   * gesture (see `motion.ts`). A change in it, which `subscribe` reports, begins the watch again.
   */
  const animating = (): number => busy();
  const kick = useRef(() => {});
  useEffect(() => subscribe(() => kick.current()), []);
  const activeNow = useRef(active);
  activeNow.current = active;
  const threadNow = useRef(thread);
  threadNow.current = thread;
  /**
   * Says on the host, in `data-settled`, whether the Map has settled, its camera framed and no tile animating: "true"
   * once the plan is laid out and drawn, no camera intent can still move it (a reveal parked for want of an open
   * part of the Map does not count), and the zoom and pan have held for two animation frames; "false" while a layout,
   * an intent or a movement is in flight. Watched frame by frame from here until it is true, and begun again by
   * anything that may move the camera.
   */
  function kickFramed() {
    const element = host.current;
    if (element === null) return;
    element.dataset.settled = "false";
    if (watching.current !== null) return;
    let watch = FRESH_WATCH;
    const frame = () => {
      watching.current = null;
      const core = cy.current;
      const element = host.current;
      if (core === null || element === null || !activeNow.current) return;
      const laidOut = placedNow.current !== null || modelNow.current.items.size === 0;
      const pan = core.pan();
      const step = stepFrame(watch, {
        busy:
          !laidOut ||
          element.dataset.state !== "drawn" ||
          intentsPending({
            shown: shown.current,
            parked: parked.current,
            anchor: anchor.current !== null,
            toFrame: toFrame.current !== undefined && toFrame.current.thread === threadNow.current,
            tapReveals: tapReveals.current,
          }),
        animating: animating(),
        viewport: `${core.zoom()},${pan.x},${pan.y}`,
      });
      watch = step;
      if (step.settled) element.dataset.settled = "true";
      else watching.current = requestAnimationFrame(frame);
    };
    watching.current = requestAnimationFrame(frame);
  }
  kick.current = kickFramed;
  useEffect(
    () => () => {
      if (watching.current !== null) cancelAnimationFrame(watching.current);
      watching.current = null;
    },
    [],
  );
  /** The elements the renderer holds, by id. */
  const held = useRef<ReadonlyMap<string, Drawn>>(new Map());
  /** The model last drawn, to tell whether a focus will be drawn afresh. */
  const drawnModel = useRef<Model | null>(null);
  /** The full scene last applied, so a pending reveal never flushes against a stale view or source layout. */
  const applied = useRef<{
    model: Model;
    view: View;
    aspect: number | null;
    source: string;
    placed: Placed;
    drawn: Scene;
  } | null>(null);
  const collapsed = useMemo(() => collapsedWith(model, toggles), [model, toggles]);
  const view = useMemo(() => visible(model, { collapsed }), [model, collapsed]);
  const fonts = useMemo(() => (fontFamily === null ? UNMEASURED : measuring(fontFamily)), [fontFamily]);
  const labelMetrics = useMemo(
    () =>
      new Map(
        view.nodes.map(
          (node) =>
            [
              node.id,
              metricsOf(node, fonts, TILE_WIDTH, node.thread === null ? null : (named.get(node.thread) ?? null)),
            ] as const,
        ),
      ),
    [view, fonts, named],
  );
  const viewNow = useRef(view);
  viewNow.current = view;
  // Only the layout for the view on screen is drawn: one from before the last change would put boxes where they were.
  const placed =
    laid !== null && laid.view === view && laid.aspect === aspect && laid.source === source ? laid.placed : null;
  const suppressedTitles = useRef<Set<string>>(new Set());

  const clearSuppressedTitles = () => {
    const core = cy.current;
    if (core !== null)
      for (const id of suppressedTitles.current) {
        const node = core.getElementById(id);
        if (node.nonempty()) node.removeStyle("text-opacity");
      }
    suppressedTitles.current.clear();
  };

  const suppressTitles = (ids: Iterable<string>) => {
    const core = cy.current;
    if (core === null) return;
    for (const id of ids) {
      const node = core.getElementById(id);
      if (!node.empty()) {
        node.style("text-opacity", 0);
        suppressedTitles.current.add(id);
      }
    }
  };

  useEffect(() => {
    if (!active) clearSuppressedTitles();
  }, [active]);

  const fit = () => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null) return;
    land(core);
    cancelViewIntent();
    clearSuppressedTitles();
    const plan = core.elements().boundingBox({ includeLabels: true });
    const fitted = fitPlan(plan, openPart(element), MAX_ZOOM);
    if (fitted === null) return;
    const floor = Math.min(1, fitted.zoom * 0.9);
    core.scratch("floor", floor);
    core.minZoom(floor * PINCH_SLACK);
    core.viewport(fitted);
  };

  const previousSelected = useRef(selected);
  useEffect(() => {
    const changed = previousSelected.current !== selected;
    previousSelected.current = selected;
    if (!changed || focus?.id === selected) return;
    if (selected === null || shown.current !== null || anchor.current !== null) cancelViewIntent();
  }, [selected, focus]);

  // Brought into view: the boxes above it open, keeping a pinned box or a written item's prior root frame.
  useEffect(() => {
    const core = cy.current;
    if (focus === undefined) return;
    cancelViewIntent();
    const was = drawnModel.current;
    const preserveFrame =
      focus.preserveFrame === true &&
      (focus.pin === null || focus.pin === focus.id) &&
      was?.items.has(focus.id) === true;
    anchor.current = null;
    if (core === null) {
      // Focused before the renderer is made, as when a link opens the app on an item: shown once first drawn.
      shown.current = { id: focus.id, centered: true, picked: focus.pin === null };
      setToggles((was) => expandTo(model, focus.id, was));
      return;
    }
    const intent = focusReveal(focus, preserveFrame);
    const drawn = preserveFrame
      ? was === null
        ? undefined
        : [...chain(was, focus.id)].reverse().find((id) => core.getElementById(id).nonempty())
      : focus.pin === null
        ? undefined
        : chain(model, focus.pin).find((id) => core.getElementById(id).nonempty());
    if (drawn !== undefined) {
      const corner = settledBox(core, drawn);
      anchor.current = { id: drawn, x: corner.x1, y: corner.y1 };
    }
    const next = expandTo(model, focus.id, toggles);
    if (next === toggles && model === drawnModel.current) {
      // Nothing will be drawn afresh, so it is shown now; left for a later draw, it would pan there out of the blue.
      anchor.current = null;
      shown.current = intent;
      kickFramed();
      tryShow();
      return;
    }
    shown.current = intent;
    if (host.current !== null) host.current.dataset.state = "laying-out";
    kickFramed();
    setToggles(next);
  }, [focus]);

  useEffect(() => {
    let live = true;
    const start = performance.now();
    if (aspect === null || fontFamily === null) return;
    const sizes = new Map<string, { width: number; height: number }>(
      [...labelMetrics].map(([id, metric]) => [id, { width: metric.width, height: metric.height }] as const),
    );
    const headers = new Map<string, number>(
      [...labelMetrics].map(([id, metric]) => [id, metric.header || metric.height] as const),
    );
    void (async () => {
      await fontReady;
      if (!live) return;
      // Only submitted layout requests count; an unmounted font wait never enters the layout queue.
      if (host.current !== null) host.current.dataset.layouts = String(Number(host.current.dataset.layouts ?? 0) + 1);
      const result = await layouts.layout(view, aspect, source, sizes, headers, () => live);
      if (!live || result === null) return;
      if (host.current !== null) {
        host.current.dataset.layoutMs = String(Math.round(performance.now() - start));
        host.current.dataset.layoutCalls = String(result.laidOut.length);
      }
      setLaid({ view, aspect, source, placed: result });
    })();
    return () => {
      live = false;
    };
  }, [layouts, view, aspect, source, fontFamily, labelMetrics]);

  useEffect(() => {
    if (host.current === null || fontFamily === null) return;
    const core = cytoscape({
      container: host.current,
      style: styleFor(palette, fontFamily),
      autoungrabify: true,
      // The app marks the selected item itself; Cytoscape's own selection would only be a second, unstyled one.
      autounselectify: true,
      boxSelectionEnabled: false,
      maxZoom: MAX_ZOOM,
    });
    cy.current = core;
    // Browser journeys reach the renderer through its element, to find where a node is drawn on the canvas.
    Object.assign(host.current, { cy: core });
    // The wheel zooms by the app's own steps. Cytoscape listens on the Map itself while capturing, so only a listener
    // above it, capturing, runs first and can keep the event from it; a touch pinch is still the renderer's. It sits
    // on the Map's parent rather than the window, so scrolling a pane elsewhere never waits for it.
    const element = host.current;
    const above = element.parentElement ?? window;
    const wheel = (event: WheelEvent) => {
      if (!(event.target instanceof Node) || !element.contains(event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      cancelViewIntent();
      const at = element.getBoundingClientRect();
      core.zoom({
        level: wheelZoom(core.zoom(), event, floorOf(core), core.maxZoom(), element.clientHeight),
        renderedPosition: { x: event.clientX - at.left, y: event.clientY - at.top },
      });
    };
    above.addEventListener("wheel", wheel as EventListener, { capture: true, passive: false });
    // A pinch past the floor is held at it, about the fingers. Cytoscape's own limit drops the whole step of a pinch
    // that would cross it, its pan with it, so near the limit the plan stuck, then leapt as the fingers moved on: its
    // limit sits below the floor instead, and the floor is kept here, where the fingers are known.
    // Only while two fingers are down: a zoom left past a floor that rose, as after a box folds, stays until
    // the user zooms, rather than leaping on a pan.
    let fingers: { x: number; y: number } | null = null;
    const touch = (event: TouchEvent) => {
      const [one, two] = [event.touches[0], event.touches[1]];
      if (one === undefined || two === undefined) {
        fingers = null;
        return;
      }
      cancelViewIntent();
      const at = element.getBoundingClientRect();
      fingers = { x: (one.clientX + two.clientX) / 2 - at.left, y: (one.clientY + two.clientY) / 2 - at.top };
    };
    for (const kind of ["touchmove", "touchend", "touchcancel"] as const)
      element.addEventListener(kind, touch, { capture: true, passive: true });
    // What counts as a gesture: a pointer or touch down on the Map until it ends, and wheel steps until a pause. Hover
    // doesn't. A cancel, the window losing focus or the page hiding forgets them all, and the scheduler's watchdog
    // clears one whose end was never seen.
    const inside = (event: Event) => event.target instanceof Node && element.contains(event.target);
    const down = (event: PointerEvent) => {
      if (inside(event)) gesture.down(event.pointerId);
    };
    const up = (event: PointerEvent) => gesture.up(event.pointerId);
    // A pointer moving with no button down is hover, or a release outside the page that was never seen: not a gesture.
    const alive = (event: PointerEvent) => {
      if (event.pointerType !== "touch" && event.buttons === 0) gesture.up(event.pointerId);
      else gesture.move();
    };
    // Only the fingers on the Map count, wherever the touch that ended began.
    const touched = (event: TouchEvent) =>
      gesture.touches(
        [...event.touches].filter((touch) => touch.target instanceof Node && element.contains(touch.target)).length,
      );
    const forget = () => gesture.reset();
    const hiding = () => {
      if (document.hidden) gesture.reset();
    };
    const stepped = (event: WheelEvent) => {
      if (inside(event)) gesture.wheel();
    };
    element.addEventListener("pointerdown", down, { capture: true, passive: true });
    for (const kind of ["pointerup", "pointercancel"] as const) window.addEventListener(kind, up, { capture: true });
    window.addEventListener("pointermove", alive, { capture: true, passive: true });
    for (const kind of ["touchstart", "touchmove", "touchend", "touchcancel"] as const)
      window.addEventListener(kind, touched, { capture: true, passive: true });
    window.addEventListener("wheel", stepped, { capture: true, passive: true });
    window.addEventListener("blur", forget);
    document.addEventListener("visibilitychange", hiding);
    core.on("viewport", () => {
      clearSuppressedTitles();
      if (fingers !== null && core.zoom() < floorOf(core))
        core.zoom({ level: floorOf(core), renderedPosition: fingers });
    });
    return () => {
      clearSuppressedTitles();
      above.removeEventListener("wheel", wheel as EventListener, { capture: true });
      for (const kind of ["touchmove", "touchend", "touchcancel"] as const)
        element.removeEventListener(kind, touch, { capture: true });
      element.removeEventListener("pointerdown", down, { capture: true });
      for (const kind of ["pointerup", "pointercancel"] as const)
        window.removeEventListener(kind, up, { capture: true });
      window.removeEventListener("pointermove", alive, { capture: true });
      for (const kind of ["touchstart", "touchmove", "touchend", "touchcancel"] as const)
        window.removeEventListener(kind, touched, { capture: true });
      window.removeEventListener("wheel", stepped, { capture: true });
      window.removeEventListener("blur", forget);
      document.removeEventListener("visibilitychange", hiding);
      gesture.cancel();
      gesture.reset();
      settle(core);
      core.destroy();
      cy.current = null;
      held.current = new Map();
    };
  }, [fontFamily]);

  // A scheme changed restyles what is drawn in place, keeping the view, the selection and the boxes open; the elements'
  // own colours, their marks and lines, follow with the scene below.
  const styled = useRef(palette);
  useEffect(() => {
    const core = cy.current;
    if (core === null || styled.current === palette) return;
    styled.current = palette;
    core.style(styleFor(palette, fontFamily ?? FALLBACK_FONT));
    for (const id of suppressedTitles.current) {
      const node = core.getElementById(id);
      if (node.nonempty()) {
        node.style("text-opacity", 0);
      }
    }
  }, [palette]);

  // A phone's keyboard, or a window resized, changes the Map's size: the furthest zoom out follows. A strip taken from
  // the Map, such as the slot, or a window turned, shows the selection again if it hid it; a phone's keyboard, which
  // resizes the Map too, leaves it, as the user is typing.
  const placedNow = useRef<Placed | null>(null);
  placedNow.current = placed;
  const modelNow = useRef(model);
  modelNow.current = model;
  const sourceNow = useRef(source);
  sourceNow.current = source;
  const aspectNow = useRef(aspect);
  aspectNow.current = aspect;
  const drawnNow = useRef<Scene | null>(null);
  const selectedNow = useRef(selected);
  selectedNow.current = selected;
  useEffect(() => {
    const element = host.current;
    const stage = element?.parentElement;
    if (element === null || stage === null || stage === undefined) return;
    let size: { width: number; height: number } | null = null;
    const retryPending = () => {
      tryShow();
    };
    const panels = new Set<HTMLElement>();
    const observer = new ResizeObserver((entries) => {
      const hostChanged = entries.some((entry) => entry.target === element);
      const panelChanged = entries.some((entry) => entry.target !== element);
      if (!hostChanged) {
        if (panelChanged) retryPending();
        return;
      }
      const core = cy.current;
      const was = size;
      size = { width: element.clientWidth, height: element.clientHeight };
      if (core === null || placedNow.current === null) return;
      clearSuppressedTitles();
      core.resize();
      zoomOutTo(core, element, placedNow.current);
      const id = selectedNow.current;
      const node = id === null ? null : core.getElementById(id);
      if (shown.current !== null) retryPending();
      else if (was !== null && id !== null && node !== null && !node.empty() && !typing()) {
        if (newlyCovered(settledBox(core, id), was, size, coverOf(element))) {
          const request = { id, centered: false, picked: false };
          shown.current = request;
          kickFramed();
          tryShow();
        }
      }
    });
    observer.observe(element, { box: "border-box" });
    const reconcilePanels = () => {
      const current = new Set(panelsOver(element));
      for (const panel of panels)
        if (!current.has(panel)) {
          observer.unobserve(panel);
          panels.delete(panel);
        }
      for (const panel of current)
        if (!panels.has(panel)) {
          panels.add(panel);
          observer.observe(panel, { box: "border-box" });
        }
    };
    reconcilePanels();
    const mutations = new MutationObserver(() => {
      reconcilePanels();
      retryPending();
    });
    mutations.observe(stage, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["data-detent", "data-dragging"],
    });
    return () => {
      mutations.disconnect();
      observer.disconnect();
    };
  }, []);

  const drawn = useMemo(() => {
    if (placed === null) return null;
    const selection: Selection = { item: selected, thread, hovered };
    return scene(view, placed, selection, (id) => model.items.get(id)?.thread ?? null, fonts, labelMetrics);
  }, [view, placed, selected, thread, hovered, model, fonts, labelMetrics]);
  drawnNow.current = drawn;
  function sceneIsApplied() {
    const last = applied.current;
    return (
      last !== null &&
      placedNow.current !== null &&
      last.model === modelNow.current &&
      last.source === sourceNow.current &&
      last.view === viewNow.current &&
      last.aspect === aspectNow.current &&
      last.placed === placedNow.current &&
      last.drawn === drawnNow.current
    );
  }
  function tryShow(): boolean {
    const request = shown.current;
    const core = cy.current;
    const element = host.current;
    if (request === null || core === null || element === null) return false;
    if (!modelNow.current.items.has(request.id)) {
      if (shown.current === request) {
        shown.current = null;
        anchor.current = null;
      }
      return false;
    }
    if (!sceneIsApplied()) return false;
    if (selectedNow.current !== request.id) {
      if (shown.current === request) {
        shown.current = null;
        anchor.current = null;
      }
      return false;
    }
    const panels = panelsOver(element);
    if (panels.some((panel) => panel.dataset.dragging === "true")) return false;
    for (const panel of panels) {
      const animation = panel.getAnimations().find((active) => {
        const property = (active as CSSTransition).transitionProperty;
        return (
          (active.playState === "running" || active.pending) && (property === "height" || property === "max-height")
        );
      });
      if (animation !== undefined) {
        waitingFor.current.set(animation, request);
        if (!watchedAnimations.current.has(animation)) {
          watchedAnimations.current.add(animation);
          const retry = () => {
            const latest = waitingFor.current.get(animation);
            waitingFor.current.delete(animation);
            if (latest !== undefined && shown.current === latest) tryShow();
          };
          void animation.finished.then(retry, retry);
        }
        return false;
      }
    }
    const revealed = revealIn(core, element, request.id, request.centered, request.picked);
    if (revealed === null || revealed === undefined) {
      parked.current = request;
      kickFramed();
      return false;
    }
    if (shown.current === request) shown.current = null;
    return true;
  }

  // Only the lit thread's colour, so a reload that leaves it alone doesn't redraw.
  const colour = drawn === null || drawn.thread === null ? undefined : lines.get(drawn.thread)?.colour;
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null || drawn === null || placed === null) return;
    const first = core.elements().length === 0;
    const job = () => {
      // Only what changed: on a large plan, a write changes a few elements of thousands.
      clearSuppressedTitles();
      held.current = apply(core, held.current, elementsOf(drawn, colour, lines, palette), {
        reduced: prefersReducedMotion(),
        extent: core.extent(),
      });
      zoomOutTo(core, element, placed);
      if (first) {
        // At a zoom its titles can be read at, which on a phone shows only the start of a large plan: the first box's
        // corner, and its first item.
        const least = Math.min(...drawn.nodes.map((node) => node.layer));
        const open = openPart(element);
        const screen = { width: open.x2 - open.x1, height: open.y2 - open.y1 };
        const start = startOf(
          drawn.nodes.flatMap((node) => {
            const at = placed.nodes.get(node.id);
            return at === undefined ? [] : [{ ...at, box: node.kind === "box", top: node.layer === least }];
          }),
          screen.width,
          16,
          FIRST_ZOOM,
        );
        const view = firstView(placed, screen, 16, MAX_ZOOM, start, FIRST_ZOOM);
        core.viewport({ zoom: view.zoom, pan: { x: view.pan.x + open.x1, y: view.pan.y + open.y1 } });
      } else if (anchor.current !== null) {
        // Pan so the toggled box's top-level box has its corner where it was before the layout changed.
        const box = core.getElementById(anchor.current.id);
        if (box.nonempty()) {
          const now = settledBox(core, anchor.current.id);
          core.panBy(anchorPan({ x: anchor.current.x, y: anchor.current.y }, { x: now.x1, y: now.y1 }));
        }
        anchor.current = null;
      }
      drawnModel.current = model;
      applied.current = { model, view, aspect, source, placed, drawn };
      // Keep the request until the open Map can show it; a fully covered Map cannot satisfy a reveal.
      tryShow();
      // What the renderer holds, for the journeys to compare with the plan.
      element.dataset.nodes = String(core.nodes().not(".junction, .divider, .stub").length);
      element.dataset.edges = String(core.edges().length);
      element.dataset.lit = String(drawn.nodes.filter((node) => node.lit).length);
      element.dataset.litEdges = String(drawn.edges.filter((edge) => edge.lit).length);
      element.dataset.font = String(document.fonts?.check(`${WEIGHT.regular} 13px Overpass`) ?? false);
      // Drawn once the browser has painted the frame after this one's changes.
      const drawing = performance.now();
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          element.dataset.drawMs = String(Math.round(performance.now() - drawing));
          element.dataset.state = "drawn";
          setDrawnOnce(true);
        }),
      );
      kickFramed();
    };
    // A repack of the top level waits for a gesture, and the latest one wins; the first draw, and a redraw that moves
    // nothing, as a selection or a hover does, never wait.
    if (first) {
      gesture.cancel();
      job();
    } else if (applied.current?.placed === placed) {
      gesture.cancel();
      job();
    } else {
      gesture.defer(job);
    }
  }, [drawn, placed, colour, lines, palette, model, view, aspect, source]);

  // A change that will lay out or move the camera, noted before its layout is back, so the signal never reads a stale
  // "true"; and the Map becoming active, since the watch stops while it is not, and a Map drawn then is unsettled.
  useEffect(kickFramed, [model, view, aspect, source, fontFamily, framed, thread, focus, selected, active]);

  // A picked thread frames every visible member, clear of the panel the pick opens.
  const sought = useRef<string | null>(null);
  const toFrame = useRef(framed);
  const framedSeen = useRef(framed);
  useEffect(() => {
    if (framed !== undefined && framed !== framedSeen.current) {
      toFrame.current = framed;
      cancelViewIntent();
    }
    framedSeen.current = framed;
    const core = cy.current;
    const element = host.current;
    const asked = toFrame.current;
    if (asked === undefined || asked.thread !== thread) return;
    if (core === null || element === null || drawn === null) return;
    toFrame.current = undefined;
    land(core);
    clearSuppressedTitles();
    sought.current = thread;
    const boxOf = (kind: (node: SceneNode) => boolean) =>
      drawn.nodes.flatMap((node) => {
        const drawnNode = core.getElementById(node.id);
        return kind(node) && drawnNode.nonempty()
          ? [{ ...drawnNode.boundingBox({ includeLabels: false }), header: node.header }]
          : [];
      });
    const lit = boxOf((node) => node.lit);
    const open = openPart(element);
    // With the titles of the boxes they are in, when those fit too: else only the items, which matter more.
    const titled = headed(
      lit,
      boxOf((node) => node.kind === "box"),
      STRIP,
    );
    const titlesFit = framesAll(titled, open);
    const view = fitAll(titlesFit ? titled : lit, open, 1);
    if (view !== null) {
      core.viewport(view);
      if (!titlesFit) {
        const nodes = new Map(viewNow.current.nodes.map((node) => [node.id, node]));
        const ancestors = new Set<string>();
        for (const node of drawn.nodes) {
          if (!node.lit) continue;
          for (
            let parent = nodes.get(node.id)?.parent ?? null;
            parent !== null;
            parent = nodes.get(parent)?.parent ?? null
          )
            if (nodes.get(parent)?.kind === "box") ancestors.add(parent);
        }
        suppressTitles(ancestors);
      }
    }
  }, [framed, thread, drawn]);
  const priorThread = useRef(thread);
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    const changed = priorThread.current !== thread;
    priorThread.current = thread;
    if (
      changed &&
      (shownBeforeRender !== null || anchorBeforeRender !== null) &&
      shown.current === shownBeforeRender &&
      anchor.current === anchorBeforeRender
    )
      cancelViewIntent();
    if (thread === null) {
      sought.current = null;
    }
    if (thread === null || sought.current === thread || core === null || element === null || drawn === null) return;
    sought.current = thread;
    if (focus?.id === selected) return;
    const lit = drawn.nodes.flatMap((node) => {
      const drawnNode = core.getElementById(node.id);
      return node.lit && drawnNode.nonempty() ? [drawnNode.boundingBox({ includeLabels: false })] : [];
    });
    const open = openPart(element);
    const fitted = fitAll(lit, open, 1);
    if (fitted !== null) {
      cancelViewIntent();
      core.viewport(fitted);
    }
  }, [thread, drawn, focus, selected]);

  // The latest handlers, so the tap listener is added once.
  const handlers = useRef({ onSelect, onPick, onOpen });
  handlers.current = { onSelect, onPick, onOpen };
  /** Counts taps, so a tap's late reveal gives way to a newer tap's. */
  const taps = useRef(0);
  function cancelViewIntent() {
    shown.current = null;
    anchor.current = null;
    taps.current++;
  }
  useEffect(() => {
    const core = cy.current;
    if (core === null) return;
    const tap = (event: cytoscape.EventObject) => {
      const { onSelect, onPick, onOpen } = handlers.current;
      const now = ++taps.current;
      const target = event.target as cytoscape.NodeSingular | Core;
      let node = target !== core && "isNode" in target && target.isNode() ? target : null;
      // A stub opens what it names; while an edit waits for a target, the tap is the tile's under it.
      if (node?.hasClass("stub")) {
        const through = stubTap(onPick !== null, {
          node: String(node.data("node")),
          target: String(node.data("target")),
        });
        if (through.action === "open") {
          cancelViewIntent();
          onOpen?.(through.item);
          return;
        }
        node = core.getElementById(through.node);
      }
      if (node === null) {
        cancelViewIntent();
        if (onPick !== null) onPick(null);
        else onSelect(null);
        return;
      }
      const id = node.id();
      const kind = node.data("kind") as string;
      const action = tapAction({
        picking: onPick !== null,
        chevron: onChevron(
          kind,
          { ...node.boundingBox({ includeLabels: false }), header: Number(node.data("header") ?? STRIP) },
          event.position,
          core.zoom(),
        ),
      });
      shown.current = null;
      anchor.current = null;
      if (action === "pick") {
        onPick?.(id);
        return;
      }
      if (action === "toggle") {
        const top = topOf(viewNow.current, id);
        const corner = settledBox(core, top);
        anchor.current = { id: top, x: corner.x1, y: corner.y1 };
        if (host.current !== null) host.current.dataset.state = "laying-out";
        kickFramed();
        setToggles((was) => new Map(was).set(id, kind === "box"));
        return;
      }
      onSelect(id);
      // The panel a tap opens may cover what was tapped: once it is drawn, the Map pans the least that shows it,
      // unless another tap came since, which the Map now answers to, or the Map has moved since, which leaves it
      // where it was moved to.
      const pan = core.pan();
      const zoom = core.zoom();
      tapReveals.current++;
      kickFramed();
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          tapReveals.current--;
          const still = cy.current?.zoom() === zoom && cy.current.pan().x === pan.x && cy.current.pan().y === pan.y;
          if (taps.current === now && still && cy.current !== null && host.current !== null) {
            shown.current = { id, centered: false, picked: false };
            tryShow();
          }
        }),
      );
    };
    // The renderer pans only from the background; a drag from an item or a box pans too, since nothing moves items. A
    // second finger makes it a pinch, which the renderer zooms.
    let from: { x: number; y: number } | null = null;
    const press = (event: cytoscape.EventObject) => {
      from = { x: event.renderedPosition.x, y: event.renderedPosition.y };
    };
    const drag = (event: cytoscape.EventObject) => {
      const touches = (event.originalEvent as Partial<TouchEvent> | undefined)?.touches;
      if (touches !== undefined && touches.length > 1) from = null;
      if (from === null) return;
      cancelViewIntent();
      const at = event.renderedPosition;
      core.panBy({ x: at.x - from.x, y: at.y - from.y });
      from = { x: at.x, y: at.y };
    };
    const release = () => {
      from = null;
    };
    const dragged = () => cancelViewIntent();
    // A pointer over an item shows its links; touch has no hover, and gives none of these.
    const enter = (event: cytoscape.EventObject) => setHovered((event.target as cytoscape.NodeSingular).id());
    const leave = () => setHovered(null);
    // A stub's full names show as the Map's hover title, as the canvas has no element to hold one, and its pointer is a
    // hand: it opens what it names.
    const over = (event: cytoscape.EventObject) => {
      const element = host.current;
      if (element === null) return;
      element.title = String((event.target as cytoscape.NodeSingular).data("full"));
      element.style.cursor = "pointer";
    };
    const off = () => {
      const element = host.current;
      if (element === null) return;
      element.removeAttribute("title");
      element.style.removeProperty("cursor");
    };
    core.on("tap", tap);
    core.on("viewport", kickFramed);
    core.on("tapstart", "node", press);
    core.on("tapdrag", drag);
    core.on("dragpan", dragged);
    core.on("tapend", release);
    core.on("mouseover", HOVERABLE, enter);
    core.on("mouseout", HOVERABLE, leave);
    core.on("mouseover", "node.stub", over);
    core.on("mouseout", "node.stub", off);
    // Leaving the Map from over an item gives the renderer no move off it.
    const element = host.current;
    element?.addEventListener("mouseleave", leave);
    return () => {
      element?.removeEventListener("mouseleave", leave);
      core.off("mouseover", HOVERABLE, enter);
      core.off("mouseout", HOVERABLE, leave);
      core.off("mouseover", "node.stub", over);
      core.off("mouseout", "node.stub", off);
      core.off("tap", tap);
      core.off("viewport", kickFramed);
      core.off("tapstart", "node", press);
      core.off("tapdrag", drag);
      core.off("dragpan", dragged);
      core.off("tapend", release);
    };
  }, [fontFamily]);

  return (
    <>
      <div
        ref={host}
        className="map"
        data-testid="map"
        data-state="laying-out"
        data-settled="false"
        aria-busy={active && !drawnOnce && items.length > 0 ? true : undefined}
      />
      {active && drawnOnce && placed !== null && items.length > 0 ? (
        <button type="button" className="map-fit" aria-label="Fit the whole plan" onClick={fit}>
          Fit
        </button>
      ) : null}
      {/* Until the plan is first drawn, which on a phone takes a second or two, a blank canvas would read as no plan. */}
      {active && !drawnOnce && items.length > 0 ? (
        <p className="status map-status" role="status">
          Laying out the Map…
        </p>
      ) : null}
    </>
  );
}

/**
 * The top level's shape. Where the panes sit side by side, roughly the Map's own, rounded to a fifth so that resizing
 * a window lays the top level out again only now and then; on a phone, portrait, which a keyboard opening never
 * changes.
 */
function useAspect(host: RefObject<HTMLDivElement | null>): number | null {
  const wide = useWide();
  // Unknown until the Map is measured, so the first layout is the one at its real shape.
  const [shape, setShape] = useState<number | null>(null);
  useEffect(() => {
    const element = host.current;
    if (element === null) return;
    const observer = new ResizeObserver(() =>
      setShape((was) => shapeAfter(was, element.clientWidth, element.clientHeight)),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [host]);
  return wide ? shape : PORTRAIT;
}

/** How far below the floor Cytoscape's own limit sits, so a pinch's step past the floor is held rather than dropped. */
const PINCH_SLACK = 0.5;

/** The furthest out the Map zooms: the whole plan in view. */
const floorOf = (core: Core): number => (core.scratch("floor") as number | undefined) ?? core.minZoom();

/**
 * Lets the Map zoom out until the whole plan fits it, and no further: the wheel stops there and a pinch is held there
 * (see the viewport listener); a zoom already further out stays until moved.
 */
function zoomOutTo(core: Core, element: HTMLElement, placed: Placed) {
  const fit = Math.min(
    element.clientWidth / Math.max(1, placed.width),
    element.clientHeight / Math.max(1, placed.height),
  );
  const floor = Math.min(1, fit * 0.9);
  core.scratch("floor", floor);
  core.minZoom(floor * PINCH_SLACK);
}

/**
 * Pans the least that shows a drawn node clear of the sheet or pane over the Map, if one covers it; `centred`, to the
 * middle of what is left, for one brought in from elsewhere. One the user `picked`, on a Map zoomed out past reading,
 * comes in at a zoom its title reads at, wherever it was; one a write moved keeps the user's zoom unless it must move.
 */
function revealIn(
  core: Core,
  element: HTMLElement,
  id: string,
  centred = false,
  picked = false,
): { x: number; y: number } | null | undefined {
  const node = core.getElementById(id);
  if (node.empty()) return undefined;
  const open = openPart(element);
  const box = () => settledBox(core, id);
  const pan = reveal(box(), open, 8, centred);
  if (pan === null) return null;
  if ((picked || pan.x !== 0 || pan.y !== 0) && centred && core.zoom() < READABLE) {
    const at = box();
    core.zoom({ level: READABLE, renderedPosition: { x: (at.x1 + at.x2) / 2, y: (at.y1 + at.y2) / 2 } });
    const zoomed = reveal(box(), open, 8, true);
    if (zoomed === null) return null;
    core.panBy(zoomed);
    return zoomed;
  }
  core.panBy(pan);
  return pan;
}

/** Whether the user is typing, as a phone's keyboard, which takes from the Map's height, is up for. */
function typing(): boolean {
  const at = document.activeElement;
  return at instanceof HTMLElement && (at.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(at.tagName));
}

/**
 * The part of the Map no sheet or pane covers, in its own pixels: of the panels open, the one covering most, where it
 * comes to rest, without the rise it may still be animating.
 */
function openPart(element: HTMLElement) {
  const map = element.getBoundingClientRect();
  const open = uncovered(map.width, map.height, coverOf(element));
  // The Fit button sits 12 px from the top and leaves 8 px after its touch target.
  const hit = Number.parseFloat(getComputedStyle(element).getPropertyValue("--hit")) || 44;
  const top = 12 + hit + 8;
  return { ...open, y1: Math.min(open.y2, Math.max(open.y1, top)) };
}

/** The sheets and panes that may cover the Map. */
function panelsOver(element: HTMLElement): HTMLElement[] {
  return [...(element.parentElement?.querySelectorAll<HTMLElement>(".inspector, .sheet") ?? [])];
}

/** The panel covering most of the Map, in its pixels, where it comes to rest; null with none open. */
function coverOf(element: HTMLElement) {
  const map = element.getBoundingClientRect();
  const covers = panelsOver(element).map((sheet) => {
    const box = sheet.getBoundingClientRect();
    const risen = new DOMMatrixReadOnly(getComputedStyle(sheet).transform).m42;
    return {
      x1: box.left - map.left,
      y1: box.top - risen - map.top,
      x2: box.right - map.left,
      y2: box.bottom - risen - map.top,
    };
  });
  const most = covers.reduce<(typeof covers)[number] | null>(
    (best, one) =>
      best === null || (one.x2 - one.x1) * (one.y2 - one.y1) > (best.x2 - best.x1) * (best.y2 - best.y1) ? one : best,
    null,
  );
  return most;
}
