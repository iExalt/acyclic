// The Map: the plan drawn as nested boxes on a canvas, with Cytoscape.js placing nodes exactly where the scoped
// layout put them. Tapping a box's title collapses or expands it, and the box stays where it was on screen; tapping
// an item selects it, lighting up its thread and drawing its links across boxes. Dragging pans; nothing moves items.

import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Item } from "../api";
import { browserLayouts } from "./client";
import type { Placed } from "./layout";
import { buildModel, defaultView, visible } from "./model";
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
  onSelect: (item: string | null) => void;
}

export default function PlanMap({ items, showArchived, thread, onSelect }: PlanMapProps) {
  const host = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const layouts = useMemo(browserLayouts, []);
  const model = useMemo(() => buildModel(items), [items]);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => defaultView(model).collapsed);
  const [placed, setPlaced] = useState<Placed | null>(null);
  const [item, setItem] = useState<string | null>(null);
  /** The box last toggled, and where its top-left corner was on screen, to hold it there. */
  const anchor = useRef<{ id: string; x: number; y: number } | null>(null);
  const view = useMemo(() => visible(model, { collapsed, showArchived }), [model, collapsed, showArchived]);

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
      setPlaced(result);
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
    const selection: Selection = { item, thread };
    return scene(view, placed, selection, (id) => model.items.get(id)?.thread ?? null);
  }, [view, placed, item, thread, model]);

  useEffect(() => {
    const core = cy.current;
    const element = host.current;
    if (core === null || element === null || drawn === null || placed === null) return;
    // A layout from before the last toggle may still be drawn; draw only the one for the current view.
    if (drawn.nodes.length !== view.nodes.length) return;
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
        const held = box.renderedBoundingBox({ includeLabels: false });
        element.dataset.anchorDx = String(Math.round(held.x1 - anchor.current.x));
        element.dataset.anchorDy = String(Math.round(held.y1 - anchor.current.y));
      }
      anchor.current = null;
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
  }, [drawn, placed, view]);

  useEffect(() => {
    const core = cy.current;
    if (core === null) return;
    const tap = (event: cytoscape.EventObject) => {
      const target = event.target as cytoscape.NodeSingular | Core;
      if (target === core || !("isNode" in target) || !target.isNode()) {
        setItem(null);
        onSelect(null);
        return;
      }
      const id = target.id();
      const kind = target.data("kind") as string;
      const box = target.boundingBox({ includeLabels: false });
      const inTitle = event.position.y - box.y1 <= TITLE;
      if (kind === "collapsed" || (kind === "box" && inTitle)) {
        const corner = target.renderedBoundingBox({ includeLabels: false });
        anchor.current = { id, x: corner.x1, y: corner.y1 };
        if (host.current !== null) host.current.dataset.state = "laying-out";
        setCollapsed((was) => {
          const next = new Set(was);
          if (next.has(id)) next.delete(id);
          else next.add(id);
          return next;
        });
        return;
      }
      setItem(id);
      onSelect(id);
    };
    core.on("tap", tap);
    return () => {
      core.off("tap", tap);
    };
  }, [onSelect]);

  return <div ref={host} className="map" data-testid="map" data-state="laying-out" />;
}
