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
import { apply, type Drawn, elementsOf } from "./draw";
import { PORTRAIT, shapeAfter } from "./elk";
import type { Placed } from "./layout";
import { buildModel, collapsedWith, type Model, type View, visible } from "./model";
import {
  firstView,
  frame,
  framesAll,
  headed,
  nearest,
  newlyCovered,
  READABLE,
  reveal,
  startOf,
  uncovered,
} from "./reveal";
import { type Fonts, type SceneNode, type Selection, scene, UNMEASURED } from "./scene";
import { FONT, styleFor, WEIGHT } from "./style";
import { wheelZoom } from "./wheel";

/** The furthest the Map zooms in. */
const MAX_ZOOM = 2;
/** The first view's furthest zoom, on every screen: 1:1, titles at their 13 px, as a phone reads them too. */
const FIRST_ZOOM = 1;
/** What the pointer shows the links of: a tile, or a collapsed box, which holds its tiles' links. */
const HOVERABLE = "node.leaf, node.collapsed";

/** The face labels are drawn in, loaded first: the canvas draws with whatever is loaded. */
const fontLoaded: Promise<unknown> =
  typeof document === "undefined" || !("fonts" in document)
    ? Promise.resolve()
    : Promise.all([
        document.fonts.load(`${WEIGHT.regular} 13px Overpass`),
        document.fonts.load(`${WEIGHT.bold} 13px Overpass`),
      ]).catch(() => undefined);

/**
 * Measures labels as the renderer draws them, once the face is loaded, which the layout waits for before anything is
 * drawn. Each text is measured once.
 */
function measuring(): Fonts {
  const context = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (context === null) return UNMEASURED;
  const face = (weight: number) => {
    const known = new Map<string, number>();
    return (text: string) => {
      let width = known.get(text);
      if (width === undefined) {
        context.font = `${weight} 13px ${FONT}`;
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
  showArchived: boolean;
  /** The thread picked in the sidebar, if any. */
  thread: string | null;
  /** Each thread's line, whose colour lights its items. */
  lines: ReadonlyMap<string, Line>;
  /** The colours to draw in, which change with the system's scheme. */
  palette: Palette;
  selected: string | null;
  onSelect: (item: string | null) => void;
  /**
   * An item to bring into view, expanding the boxes above it, while `pin` (or its nearest drawn ancestor) holds still
   * on screen. A new `n` asks again.
   */
  focus: Focus | null;
  /** A thread to frame, its items fitted in the part of the Map nothing covers. A new `n` asks again. */
  framed?: Framed | null;
  /** Set while an edit waits for a target: a tap picks an item, or the background (null), and toggles nothing. */
  onPick: ((item: string | null) => void) | null;
}

export interface Framed {
  thread: string;
  n: number;
}

export interface Focus {
  id: string;
  pin: string | null;
  n: number;
}

export default function PlanMap({
  items,
  showArchived,
  thread,
  lines,
  palette,
  selected,
  onSelect,
  focus,
  framed = null,
  onPick,
}: PlanMapProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const layouts = useMemo(browserLayouts, []);
  const model = useMemo(() => buildModel(items), [items]);
  /** The boxes the user has collapsed (true) or expanded (false); the rest follow the default view. */
  const [toggles, setToggles] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const [hovered, setHovered] = useState<string | null>(null);
  const [drawnOnce, setDrawnOnce] = useState(false);
  /** The last layout, with the view and the top level's shape it was computed for. */
  const [laid, setLaid] = useState<{ view: View; aspect: number; placed: Placed } | null>(null);
  const aspect = useAspect(host);
  /** The box last toggled, and where its top-left corner was on screen, to hold it there. */
  const anchor = useRef<{ id: string; x: number; y: number } | null>(null);
  /** An item brought into view: once drawn, it is panned out from under whatever covers the Map, if anything does. */
  const shown = useRef<string | null>(null);
  // Whether that item was picked, which zooms a Map too far out to read in to it, rather than moved by a write.
  const picked = useRef(false);
  /** The elements the renderer holds, by id. */
  const held = useRef<ReadonlyMap<string, Drawn>>(new Map());
  /** The model last drawn, to tell whether a focus will be drawn afresh. */
  const drawnModel = useRef<Model | null>(null);
  const collapsed = useMemo(() => collapsedWith(model, toggles), [model, toggles]);
  const view = useMemo(() => visible(model, { collapsed, showArchived }), [model, collapsed, showArchived]);
  // Only the layout for the view on screen is drawn: one from before the last change would put boxes where they were.
  const placed = laid !== null && laid.view === view && laid.aspect === aspect ? laid.placed : null;

  // Brought into view: the boxes above it open, and the pinned box held where it is now drawn.
  useEffect(() => {
    const core = cy.current;
    if (focus === null) return;
    if (core === null) {
      // Focused before the renderer is made, as when a link opens the app on an item: shown once first drawn.
      shown.current = focus.id;
      picked.current = focus.pin === null;
      setToggles((was) => expandTo(model, focus.id, was));
      return;
    }
    const drawn =
      focus.pin === null ? undefined : chain(model, focus.pin).find((id) => core.getElementById(id).nonempty());
    if (drawn !== undefined) {
      const corner = core.getElementById(drawn).renderedBoundingBox({ includeLabels: false });
      anchor.current = { id: drawn, x: corner.x1, y: corner.y1 };
    }
    const next = expandTo(model, focus.id, toggles);
    if (next === toggles && model === drawnModel.current) {
      // Nothing will be drawn afresh, so it is shown now; left for a later draw, it would pan there out of the blue.
      anchor.current = null;
      if (host.current !== null) revealIn(core, host.current, focus.id, true, focus.pin === null);
      return;
    }
    shown.current = focus.id;
    picked.current = focus.pin === null;
    if (host.current !== null) host.current.dataset.state = "laying-out";
    setToggles(next);
  }, [focus]);

  useEffect(() => {
    let live = true;
    const start = performance.now();
    if (aspect === null) return;
    // How many layouts the Map has asked for, so a journey can tell that a first load asks for one; a development
    // build's StrictMode runs effects twice, and counts two.
    if (host.current !== null) host.current.dataset.layouts = String(Number(host.current.dataset.layouts ?? 0) + 1);
    Promise.all([layouts.layout(view, aspect), fontLoaded]).then(([result]) => {
      if (!live || result === null) return;
      // How long the layout took and how many boxes it laid out afresh, for timing runs.
      if (host.current !== null) {
        host.current.dataset.layoutMs = String(Math.round(performance.now() - start));
        host.current.dataset.layoutCalls = String(result.laidOut.length);
      }
      setLaid({ view, aspect, placed: result });
    });
    return () => {
      live = false;
    };
  }, [layouts, view, aspect]);

  useEffect(() => {
    if (host.current === null) return;
    const core = cytoscape({
      container: host.current,
      style: styleFor(palette),
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
    // Only while two fingers are down: a zoom left past a floor that rose, as after hiding archived items, stays until
    // the user zooms, rather than leaping on a pan.
    let fingers: { x: number; y: number } | null = null;
    const touch = (event: TouchEvent) => {
      const [one, two] = [event.touches[0], event.touches[1]];
      if (one === undefined || two === undefined) {
        fingers = null;
        return;
      }
      const at = element.getBoundingClientRect();
      fingers = { x: (one.clientX + two.clientX) / 2 - at.left, y: (one.clientY + two.clientY) / 2 - at.top };
    };
    for (const kind of ["touchmove", "touchend", "touchcancel"] as const)
      element.addEventListener(kind, touch, { capture: true, passive: true });
    core.on("viewport", () => {
      if (fingers !== null && core.zoom() < floorOf(core))
        core.zoom({ level: floorOf(core), renderedPosition: fingers });
    });
    return () => {
      above.removeEventListener("wheel", wheel as EventListener, { capture: true });
      for (const kind of ["touchmove", "touchend", "touchcancel"] as const)
        element.removeEventListener(kind, touch, { capture: true });
      core.destroy();
      cy.current = null;
      held.current = new Map();
    };
  }, []);

  // A scheme changed restyles what is drawn in place, keeping the view, the selection and the boxes open; the elements'
  // own colours, their marks and lines, follow with the scene below.
  const styled = useRef(palette);
  useEffect(() => {
    const core = cy.current;
    if (core === null || styled.current === palette) return;
    styled.current = palette;
    core.style(styleFor(palette));
  }, [palette]);

  // A phone's keyboard, or a window resized, changes the Map's size: the furthest zoom out follows. A strip taken from
  // the Map, such as the slot, or a window turned, shows the selection again if it hid it; a phone's keyboard, which
  // resizes the Map too, leaves it, as the user is typing.
  const placedNow = useRef<Placed | null>(null);
  placedNow.current = placed;
  const selectedNow = useRef(selected);
  selectedNow.current = selected;
  useEffect(() => {
    const element = host.current;
    if (element === null) return;
    let size: { width: number; height: number } | null = null;
    const observer = new ResizeObserver(() => {
      const core = cy.current;
      const was = size;
      size = { width: element.clientWidth, height: element.clientHeight };
      if (core === null || placedNow.current === null) return;
      core.resize();
      zoomOutTo(core, element, placedNow.current);
      const id = selectedNow.current;
      const node = id === null ? null : core.getElementById(id);
      if (was === null || id === null || node === null || node.empty() || typing()) return;
      if (newlyCovered(node.renderedBoundingBox({ includeLabels: false }), was, size, coverOf(element)))
        revealIn(core, element, id);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const fonts = useMemo(measuring, []);
  const drawn = useMemo(() => {
    if (placed === null) return null;
    const selection: Selection = { item: selected, thread, hovered };
    return scene(view, placed, selection, (id) => model.items.get(id)?.thread ?? null, fonts);
  }, [view, placed, selected, thread, hovered, model, fonts]);

  // Only the lit thread's colour, so a reload that leaves it alone doesn't redraw.
  const colour = drawn === null || drawn.thread === null ? undefined : lines.get(drawn.thread)?.colour;
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null || drawn === null || placed === null) return;
    const first = core.elements().length === 0;
    // Only what changed: on a large plan, a write changes a few elements of thousands.
    held.current = apply(core, held.current, elementsOf(drawn, colour, lines, palette));
    zoomOutTo(core, element, placed);
    if (first) {
      // At a zoom its titles can be read at, which on a phone shows only the start of a large plan: the first box's
      // corner, and its first item.
      const least = Math.min(...drawn.nodes.map((node) => node.layer));
      const screen = { width: element.clientWidth, height: element.clientHeight };
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
      core.viewport({ zoom: view.zoom, pan: view.pan });
    } else if (anchor.current !== null) {
      // Pan so the toggled box's corner is where it was before the layout changed.
      const box = core.getElementById(anchor.current.id);
      if (box.nonempty()) {
        const now = box.renderedBoundingBox({ includeLabels: false });
        core.panBy({ x: anchor.current.x - now.x1, y: anchor.current.y - now.y1 });
      }
      anchor.current = null;
    }
    if (shown.current !== null) {
      // The pin holds still unless that leaves the item hidden.
      revealIn(core, element, shown.current, true, picked.current);
      shown.current = null;
    }
    drawnModel.current = model;
    // What the renderer holds, for the journeys to compare with the plan.
    element.dataset.nodes = String(core.nodes().not(".junction, .divider").length);
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
  }, [drawn, placed, colour, lines, palette]);

  // A thread picked with none of its items in view brings the nearest in, clear of the panel the pick opens; one the
  // user came to the Map for is framed instead.
  const sought = useRef<string | null>(null);
  const framedDone = useRef(0);
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (framed === null || framed.n === framedDone.current || framed.thread !== thread) return;
    if (core === null || element === null || drawn === null) return;
    framedDone.current = framed.n;
    sought.current = thread;
    const boxOf = (kind: (node: SceneNode) => boolean) =>
      drawn.nodes.flatMap((node) => {
        const drawnNode = core.getElementById(node.id);
        return kind(node) && drawnNode.nonempty() ? [drawnNode.boundingBox({ includeLabels: false })] : [];
      });
    const lit = boxOf((node) => node.lit);
    const open = openPart(element);
    // With the titles of the boxes they are in, when those fit too: else only the items, which matter more.
    const titled = headed(
      lit,
      boxOf((node) => node.kind === "box"),
      STRIP,
    );
    const view = frame(
      framesAll(titled, open) ? titled : lit,
      core.nodes().boundingBox({ includeLabels: false }),
      open,
      1,
    );
    if (view !== null) core.viewport(view);
  }, [framed, thread, drawn]);
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (thread === null) sought.current = null;
    if (thread === null || sought.current === thread || core === null || element === null || drawn === null) return;
    sought.current = thread;
    const lit = drawn.nodes.flatMap((node) => {
      const drawnNode = core.getElementById(node.id);
      return node.lit && drawnNode.nonempty() ? [drawnNode.renderedBoundingBox({ includeLabels: false })] : [];
    });
    const open = openPart(element);
    const target = nearest(lit, open);
    if (target !== null) core.panBy(reveal(target, open));
  }, [thread, drawn]);

  // The latest handlers, so the tap listener is added once.
  const handlers = useRef({ onSelect, onPick });
  handlers.current = { onSelect, onPick };
  /** Counts taps, so a tap's late reveal gives way to a newer tap's. */
  const taps = useRef(0);
  useEffect(() => {
    const core = cy.current;
    if (core === null) return;
    const tap = (event: cytoscape.EventObject) => {
      const { onSelect, onPick } = handlers.current;
      const now = ++taps.current;
      const target = event.target as cytoscape.NodeSingular | Core;
      const node = target !== core && "isNode" in target && target.isNode() ? target : null;
      if (onPick !== null) {
        onPick(node === null ? null : node.id());
        return;
      }
      if (node === null) {
        onSelect(null);
        return;
      }
      const id = node.id();
      const kind = node.data("kind") as string;
      if (onChevron(kind, node.boundingBox({ includeLabels: false }), event.position, core.zoom())) {
        const corner = node.renderedBoundingBox({ includeLabels: false });
        anchor.current = { id, x: corner.x1, y: corner.y1 };
        if (host.current !== null) host.current.dataset.state = "laying-out";
        setToggles((was) => new Map(was).set(id, kind === "box"));
        return;
      }
      onSelect(id);
      // The panel a tap opens may cover what was tapped: once it is drawn, the Map pans the least that shows it,
      // unless another tap came since, which the Map now answers to, or the Map has moved since, which leaves it
      // where it was moved to.
      const pan = core.pan();
      const zoom = core.zoom();
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          const still = cy.current?.zoom() === zoom && cy.current.pan().x === pan.x && cy.current.pan().y === pan.y;
          if (taps.current === now && still && cy.current !== null && host.current !== null)
            revealIn(cy.current, host.current, id);
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
      const at = event.renderedPosition;
      core.panBy({ x: at.x - from.x, y: at.y - from.y });
      from = { x: at.x, y: at.y };
    };
    const release = () => {
      from = null;
    };
    // A pointer over an item shows its links; touch has no hover, and gives none of these.
    const enter = (event: cytoscape.EventObject) => setHovered((event.target as cytoscape.NodeSingular).id());
    const leave = () => setHovered(null);
    core.on("tap", tap);
    core.on("tapstart", "node", press);
    core.on("tapdrag", drag);
    core.on("tapend", release);
    core.on("mouseover", HOVERABLE, enter);
    core.on("mouseout", HOVERABLE, leave);
    // Leaving the Map from over an item gives the renderer no move off it.
    const element = host.current;
    element?.addEventListener("mouseleave", leave);
    return () => {
      element?.removeEventListener("mouseleave", leave);
      core.off("mouseover", HOVERABLE, enter);
      core.off("mouseout", HOVERABLE, leave);
      core.off("tap", tap);
      core.off("tapstart", "node", press);
      core.off("tapdrag", drag);
      core.off("tapend", release);
    };
  }, []);

  return (
    <>
      <div ref={host} className="map" data-testid="map" data-state="laying-out" />
      {/* Until the plan is first drawn, which on a phone takes a second or two, a blank canvas would read as no plan. */}
      {drawnOnce || items.length === 0 ? null : (
        <p className="status map-status" role="status">
          Laying out the Map…
        </p>
      )}
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
function revealIn(core: Core, element: HTMLElement, id: string, centred = false, picked = false) {
  const node = core.getElementById(id);
  if (node.empty()) return;
  const open = openPart(element);
  const box = () => node.renderedBoundingBox({ includeLabels: false });
  const pan = reveal(box(), open, 8, centred);
  if ((picked || pan.x !== 0 || pan.y !== 0) && centred && core.zoom() < READABLE) {
    core.zoom({ level: READABLE, renderedPosition: node.renderedPosition() });
    core.panBy(reveal(box(), open, 8, true));
    return;
  }
  core.panBy(pan);
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
  return uncovered(map.width, map.height, coverOf(element));
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
