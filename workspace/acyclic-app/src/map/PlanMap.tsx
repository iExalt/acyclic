// The Map: the plan drawn as nested boxes on a canvas, with Cytoscape.js placing nodes exactly where the scoped
// layout put them. Tapping a box's title collapses or expands it, and the box stays where it was on screen; tapping
// an item selects it, lighting up its thread and drawing its links across boxes. While an edit waits for a target, a
// tap picks one instead. Dragging pans; nothing moves items.

import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Item } from "../api";
import { chain, expandTo } from "../edits";
import { browserLayouts } from "./client";
import type { Placed } from "./layout";
import { buildModel, collapsedWith, type View, visible } from "./model";
import { reveal, uncovered } from "./reveal";
import { type Selection, scene } from "./scene";

/** How tall a box's title strip is, where a tap collapses or expands it. */
const TITLE = 36;

const STYLE: cytoscape.StylesheetJson = [
  {
    selector: "node",
    style: {
      shape: "round-rectangle",
      width: "data(width)",
      height: "data(height)",
      label: "data(label)",
      color: "#d8e4ee",
      "font-size": 13,
      "text-wrap": "ellipsis",
      "text-max-width": (node: cytoscape.NodeSingular) => `${node.data("textWidth")}px`,
      "text-valign": "center",
      "text-halign": "center",
      "background-color": "#24323f",
      "border-width": 1,
      "border-color": "#4a6378",
      "z-index": (node: cytoscape.NodeSingular) => node.data("layer") as number,
    },
  },
  {
    selector: "node.box",
    style: {
      "background-color": "#1b2530",
      "text-valign": "top",
      // The label's centre sits on the box's top edge; this brings it into the title strip.
      "text-margin-y": TITLE / 2 + 6,
    },
  },
  { selector: "node.collapsed", style: { "background-color": "#2b3a4a", "border-style": "dashed" } },
  { selector: "node.lit", style: { "border-color": "#7cc4ff", "border-width": 3 } },
  { selector: "node.dim", style: { opacity: 0.3 } },
  { selector: "node.selected", style: { "border-color": "#ffd166", "border-width": 3 } },
  {
    selector: "edge",
    style: {
      width: 1.5,
      "curve-style": "straight",
      "line-color": "#7cc4ff",
      "line-opacity": 0.6,
      "target-arrow-shape": "triangle",
      "target-arrow-color": "#7cc4ff",
      "z-index": 1000,
    },
  },
  {
    selector: "edge.crossing",
    style: { "line-style": "dashed", "line-color": "#ffd166", "target-arrow-color": "#ffd166" },
  },
];

export interface PlanMapProps {
  items: Item[];
  showArchived: boolean;
  /** The thread picked in the sidebar, if any. */
  thread: string | null;
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

export default function PlanMap({ items, showArchived, thread, selected, onSelect, focus, onPick }: PlanMapProps) {
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
    shown.current = focus.id;
    if (host.current !== null) host.current.dataset.state = "laying-out";
    setToggles((was) => expandTo(model, focus.id, was));
  }, [focus]);

  useEffect(() => {
    let live = true;
    const start = performance.now();
    layouts.layout(view).then((result) => {
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
      maxZoom: 2,
    });
    cy.current = core;
    // Browser journeys reach the renderer through its element, to find where a node is drawn on the canvas.
    Object.assign(host.current, { cy: core });
    return () => {
      core.destroy();
      cy.current = null;
    };
  }, []);

  const drawn = useMemo(() => {
    if (placed === null) return null;
    const selection: Selection = { item: selected, thread };
    return scene(view, placed, selection, (id) => model.items.get(id)?.thread ?? null);
  }, [view, placed, selected, thread, model]);

  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null || drawn === null || placed === null) return;
    const elements: ElementDefinition[] = [
      ...drawn.nodes.map((node) => ({
        group: "nodes" as const,
        data: { ...node, textWidth: Math.max(40, node.width - 16) },
        position: { x: node.x, y: node.y },
        classes: [node.kind, node.lit ? "lit" : "", node.dim ? "dim" : "", node.selected ? "selected" : ""].join(" "),
      })),
      ...drawn.edges.map((edge) => ({
        group: "edges" as const,
        data: { id: edge.id, source: edge.source, target: edge.target },
        classes: edge.crossing ? "crossing" : "",
      })),
    ];
    const first = core.elements().length === 0;
    core.batch(() => {
      core.elements().remove();
      core.add(elements);
    });
    // The whole plan must fit the screen at the furthest zoom out, and no further.
    const fit = Math.min(
      element.clientWidth / Math.max(1, placed.width),
      element.clientHeight / Math.max(1, placed.height),
    );
    core.minZoom(Math.min(1, fit * 0.9));
    if (first) {
      core.fit(undefined, 16);
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
      const node = core.getElementById(shown.current);
      if (node.nonempty()) {
        const map = element.getBoundingClientRect();
        const sheet = element.parentElement?.querySelector<HTMLElement>(".inspector, .sheet")?.getBoundingClientRect();
        const cover =
          sheet === undefined
            ? null
            : {
                x1: sheet.left - map.left,
                y1: sheet.top - map.top,
                x2: sheet.right - map.left,
                y2: sheet.bottom - map.top,
              };
        core.panBy(reveal(node.renderedBoundingBox({ includeLabels: false }), uncovered(map.width, map.height, cover)));
      }
      shown.current = null;
    }
    // What the renderer holds, for the journeys to compare with the plan.
    element.dataset.nodes = String(core.nodes().length);
    element.dataset.edges = String(core.edges().length);
    element.dataset.lit = String(drawn.nodes.filter((node) => node.lit).length);
    // Drawn once the browser has painted the frame after this one's changes.
    const drawing = performance.now();
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        element.dataset.drawMs = String(Math.round(performance.now() - drawing));
        element.dataset.state = "drawn";
      }),
    );
  }, [drawn, placed]);

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
