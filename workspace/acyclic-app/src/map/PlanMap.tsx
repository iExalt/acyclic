// The Map: the plan drawn as nested boxes on a canvas, with Cytoscape.js placing nodes exactly where the scoped
// layout put them. Tapping a box's title collapses or expands it, and the box stays where it was on screen; tapping
// an item selects it, lighting up its thread and drawing its links across boxes. While an edit waits for a target, a
// tap picks one instead. Dragging pans; nothing moves items.

import cytoscape, { type Core } from "cytoscape";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Item } from "../api";
import { chain, expandTo } from "../edits";
import { type Line, TOKENS } from "../lines";
import { browserLayouts } from "./client";
import { apply, type Drawn, elementsOf } from "./draw";
import type { Placed } from "./layout";
import { buildModel, collapsedWith, type Model, type View, visible } from "./model";
import { firstView, reveal, uncovered } from "./reveal";
import { type Selection, scene } from "./scene";

/** How tall a box's title strip is, where a tap collapses or expands it. */
const TITLE = 36;
/** The furthest the Map zooms in. */
const MAX_ZOOM = 2;

/** The face labels are drawn in; the canvas draws with whatever is loaded, so the first draw waits for it. */
const FONT = "Overpass, system-ui, sans-serif";
const fontLoaded: Promise<unknown> =
  typeof document === "undefined" || !("fonts" in document)
    ? Promise.resolve()
    : Promise.all([document.fonts.load("500 13px Overpass"), document.fonts.load("700 13px Overpass")]).catch(
        () => undefined,
      );

/** Boxes are the zones of the map and items its stations; a lit item takes its thread's line colour. */
const STYLE: cytoscape.StylesheetJson = [
  {
    selector: "node",
    style: {
      shape: "round-rectangle",
      width: "data(width)",
      height: "data(height)",
      label: "data(label)",
      color: TOKENS.ink,
      "font-family": FONT,
      "font-size": 13,
      "font-weight": 500,
      "text-wrap": "ellipsis",
      "text-max-width": (node: cytoscape.NodeSingular) => `${node.data("textWidth")}px`,
      "text-valign": "center",
      "text-halign": "center",
      "background-color": TOKENS.tile,
      "border-width": 1,
      "border-color": TOKENS.rule,
      // Drawn by z-index alone: by default Cytoscape draws every edge under every node, so a box hid the links in it.
      // Nodes take the even levels, and a link the odd one between the box it is in and the items it joins.
      "z-index-compare": "manual",
      "z-index": (node: cytoscape.NodeSingular) => 2 * (node.data("layer") as number),
    },
  },
  {
    selector: "node.box",
    style: {
      "background-color": TOKENS.box,
      "border-color": TOKENS.rule,
      color: TOKENS.muted,
      "font-weight": 700,
      "text-valign": "top",
      // The label's centre sits on the box's top edge; this brings it into the title strip.
      "text-margin-y": TITLE / 2 + 6,
    },
  },
  {
    selector: "node.collapsed",
    style: { "border-color": TOKENS.muted, "border-style": "dashed", "font-weight": 700 },
  },
  { selector: "node.lit", style: { "border-color": "data(line)", "border-width": 3 } },
  { selector: "node.dim", style: { opacity: 0.35 } },
  { selector: "node.selected", style: { "border-color": "#ffffff", "border-width": 3 } },
  {
    selector: "edge",
    style: {
      width: 1.5,
      "curve-style": "straight",
      "line-color": TOKENS.muted,
      "target-arrow-shape": "triangle",
      "target-arrow-color": TOKENS.muted,
      "arrow-scale": 0.9,
      // Drawn over the nodes, so a tap must pass through to the node beneath.
      events: "no",
      "z-index-compare": "manual",
      "z-index": (edge: cytoscape.EdgeSingular) => edge.data("z") as number,
    },
  },
  {
    selector: "edge.crossing",
    style: { "line-style": "dashed", "line-color": TOKENS.signal, "target-arrow-color": TOKENS.signal },
  },
  // A lit thread's links are its route, in its line's colour; one across boxes keeps its dash.
  { selector: "edge.lit", style: { width: 3, "line-color": "data(line)", "target-arrow-color": "data(line)" } },
  { selector: "edge.dim", style: { opacity: 0.35 } },
];

export interface PlanMapProps {
  items: Item[];
  showArchived: boolean;
  /** The thread picked in the sidebar, if any. */
  thread: string | null;
  /** Each thread's line, whose colour lights its items. */
  lines: ReadonlyMap<string, Line>;
  selected: string | null;
  onSelect: (item: string | null) => void;
  /**
   * An item to bring into view, expanding the boxes above it, while `pin` (or its nearest drawn ancestor) holds still
   * on screen. A new `n` asks again.
   */
  focus: Focus | null;
  /** Set while an edit waits for a target: a tap picks an item, or the background (null), and toggles nothing. */
  onPick: ((item: string | null) => void) | null;
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
  selected,
  onSelect,
  focus,
  onPick,
}: PlanMapProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const layouts = useMemo(browserLayouts, []);
  const model = useMemo(() => buildModel(items), [items]);
  /** The boxes the user has collapsed (true) or expanded (false); the rest follow the default view. */
  const [toggles, setToggles] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  /** The last layout, with the view it was computed for. */
  const [laid, setLaid] = useState<{ view: View; placed: Placed } | null>(null);
  /** The box last toggled, and where its top-left corner was on screen, to hold it there. */
  const anchor = useRef<{ id: string; x: number; y: number } | null>(null);
  /** An item brought into view: once drawn, it is panned out from under whatever covers the Map, if anything does. */
  const shown = useRef<string | null>(null);
  /** The elements the renderer holds, by id. */
  const held = useRef<ReadonlyMap<string, Drawn>>(new Map());
  /** The model last drawn, to tell whether a focus will be drawn afresh. */
  const drawnModel = useRef<Model | null>(null);
  const collapsed = useMemo(() => collapsedWith(model, toggles), [model, toggles]);
  const view = useMemo(() => visible(model, { collapsed, showArchived }), [model, collapsed, showArchived]);
  // Only the layout for the view on screen is drawn: one from before the last change would put boxes where they were.
  const placed = laid !== null && laid.view === view ? laid.placed : null;

  // Brought into view: the boxes above it open, and the pinned box held where it is now drawn.
  useEffect(() => {
    const core = cy.current;
    if (focus === null || core === null) return;
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
      if (host.current !== null) revealIn(core, host.current, focus.id);
      return;
    }
    shown.current = focus.id;
    if (host.current !== null) host.current.dataset.state = "laying-out";
    setToggles(next);
  }, [focus]);

  useEffect(() => {
    let live = true;
    const start = performance.now();
    Promise.all([layouts.layout(view), fontLoaded]).then(([result]) => {
      if (!live || result === null) return;
      // How long the layout took and how many boxes it laid out afresh, for timing runs.
      if (host.current !== null) {
        host.current.dataset.layoutMs = String(Math.round(performance.now() - start));
        host.current.dataset.layoutCalls = String(result.laidOut.length);
      }
      setLaid({ view, placed: result });
    });
    return () => {
      live = false;
    };
  }, [layouts, view]);

  useEffect(() => {
    if (host.current === null) return;
    const core = cytoscape({
      container: host.current,
      style: STYLE,
      autoungrabify: true,
      boxSelectionEnabled: false,
      maxZoom: MAX_ZOOM,
    });
    cy.current = core;
    // Browser journeys reach the renderer through its element, to find where a node is drawn on the canvas.
    Object.assign(host.current, { cy: core });
    return () => {
      core.destroy();
      cy.current = null;
      held.current = new Map();
    };
  }, []);

  // A phone's keyboard, or a window resized, changes the Map's size: the furthest zoom out follows.
  const placedNow = useRef<Placed | null>(null);
  placedNow.current = placed;
  useEffect(() => {
    const element = host.current;
    if (element === null) return;
    const observer = new ResizeObserver(() => {
      const core = cy.current;
      if (core === null || placedNow.current === null) return;
      core.resize();
      zoomOutTo(core, element, placedNow.current);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const drawn = useMemo(() => {
    if (placed === null) return null;
    const selection: Selection = { item: selected, thread };
    return scene(view, placed, selection, (id) => model.items.get(id)?.thread ?? null);
  }, [view, placed, selected, thread, model]);

  // Only the lit thread's colour, so a reload that leaves it alone doesn't redraw.
  const colour = drawn === null || drawn.thread === null ? undefined : lines.get(drawn.thread)?.colour;
  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null || drawn === null || placed === null) return;
    const first = core.elements().length === 0;
    // Only what changed: on a large plan, a write changes a few elements of thousands.
    held.current = apply(core, held.current, elementsOf(drawn, colour));
    zoomOutTo(core, element, placed);
    if (first) {
      // At a zoom its titles can be read at, which on a phone shows only the start of a large plan.
      const view = firstView(placed, { width: element.clientWidth, height: element.clientHeight }, 16, MAX_ZOOM);
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
      revealIn(core, element, shown.current);
      shown.current = null;
    }
    drawnModel.current = model;
    // What the renderer holds, for the journeys to compare with the plan.
    element.dataset.nodes = String(core.nodes().length);
    element.dataset.edges = String(core.edges().length);
    element.dataset.lit = String(drawn.nodes.filter((node) => node.lit).length);
    element.dataset.litEdges = String(drawn.edges.filter((edge) => edge.lit).length);
    element.dataset.font = String(document.fonts?.check("500 13px Overpass") ?? false);
    // Drawn once the browser has painted the frame after this one's changes.
    const drawing = performance.now();
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        element.dataset.drawMs = String(Math.round(performance.now() - drawing));
        element.dataset.state = "drawn";
      }),
    );
  }, [drawn, placed, colour]);

  // The latest handlers, so the tap listener is added once.
  const handlers = useRef({ onSelect, onPick });
  handlers.current = { onSelect, onPick };
  useEffect(() => {
    const core = cy.current;
    if (core === null) return;
    const tap = (event: cytoscape.EventObject) => {
      const { onSelect, onPick } = handlers.current;
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
      const box = node.boundingBox({ includeLabels: false });
      const inTitle = event.position.y - box.y1 <= TITLE;
      if (kind === "collapsed" || (kind === "box" && inTitle)) {
        const corner = node.renderedBoundingBox({ includeLabels: false });
        anchor.current = { id, x: corner.x1, y: corner.y1 };
        if (host.current !== null) host.current.dataset.state = "laying-out";
        setToggles((was) => new Map(was).set(id, kind === "box"));
        return;
      }
      onSelect(id);
    };
    core.on("tap", tap);
    return () => {
      core.off("tap", tap);
    };
  }, []);

  return <div ref={host} className="map" data-testid="map" data-state="laying-out" />;
}

/** Lets the Map zoom out until the whole plan fits it, and no further; a zoom already further out stays until moved. */
function zoomOutTo(core: Core, element: HTMLElement, placed: Placed) {
  const fit = Math.min(
    element.clientWidth / Math.max(1, placed.width),
    element.clientHeight / Math.max(1, placed.height),
  );
  core.minZoom(Math.min(1, fit * 0.9));
}

/** Pans the least that shows a drawn node clear of the sheet or pane over the Map, if one covers it. */
function revealIn(core: Core, element: HTMLElement, id: string) {
  const node = core.getElementById(id);
  if (node.empty()) return;
  const map = element.getBoundingClientRect();
  const sheet = element.parentElement?.querySelector<HTMLElement>(".inspector, .sheet")?.getBoundingClientRect();
  const cover =
    sheet === undefined
      ? null
      : { x1: sheet.left - map.left, y1: sheet.top - map.top, x2: sheet.right - map.left, y2: sheet.bottom - map.top };
  core.panBy(reveal(node.renderedBoundingBox({ includeLabels: false }), uncovered(map.width, map.height, cover)));
}
