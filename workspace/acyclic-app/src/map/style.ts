// How the Map draws its nodes and links: boxes are the zones of the map and items its stations, crossed out when
// archived, and a lit thread takes its line's colour.

import type cytoscape from "cytoscape";
import type { Stage } from "../api";
import type { Palette } from "../palette";
import { STRIP } from "./chevron";
import { BOX_TEXT_LEFT, MARKS_LEFT, QUIET } from "./marks";

/** The face labels are drawn in. */
export const FONT = "Overpass, system-ui, sans-serif";
/** Its weights, a task's title and a box's: the app's text weight, and two steps above it, so a phase's title leads
 *  its tasks; in hundreds, as the renderer takes no other. */
export const WEIGHT = { regular: 500, bold: 700 } as const;

/** An inline SVG, as the renderer takes an image. */
const svg = (size: number, body: string) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${body}</svg>`)}`;
/** Lines across what is archived, crossing it out: a tile that repeats seamlessly, each line 8 px from the next. */
export const hatchOf = (p: Palette) =>
  svg(8, `<path d="M-2 2l4-4M0 8l8-8M6 10l4-4" stroke="${p.muted}" stroke-width="1"/>`);

/** Each stage's tint, and a quieter title for work finished or set aside. */
const stagesOf = (p: Palette): cytoscape.StylesheetJson =>
  (Object.keys(p.tints) as Stage[]).map((stage) => ({
    selector: `node.leaf.stage-${stage}`,
    style: { "background-color": p.tints[stage], color: QUIET.has(stage) ? p.muted : p.ink },
  }));

const made = new WeakMap<Palette, cytoscape.StylesheetJson>();

/** The Map's stylesheet in a palette, made once for each. */
export function styleFor(p: Palette): cytoscape.StylesheetJson {
  let style = made.get(p);
  if (style === undefined) {
    style = sheetOf(p);
    made.set(p, style);
  }
  return style;
}

/** Boxes are the zones of the map and items its stations; a lit item takes its thread's line colour. */
function sheetOf(p: Palette): cytoscape.StylesheetJson {
  const HATCH = hatchOf(p);
  return [
    // Cytoscape darkens where a press lands, a grey disc on the background and a shade over a node; a press here
    // selects, and the selection shows that.
    // Cytoscape's types ask for every core property, where the stylesheet takes any of them.
    { selector: "core", style: { "active-bg-opacity": 0 } as unknown as cytoscape.Css.Core },
    { selector: "node:active, edge:active", style: { "overlay-opacity": 0 } },
    {
      selector: "node",
      style: {
        shape: "round-rectangle",
        width: "data(width)",
        height: "data(height)",
        label: "data(label)",
        color: p.ink,
        "font-family": FONT,
        "font-size": 13,
        "font-weight": WEIGHT.regular,
        // Labels come cut to fit, a line at a time; the slack keeps the renderer from wrapping a line again.
        "text-wrap": "wrap",
        "text-max-width": (node: cytoscape.NodeSingular) => `${node.data("textWidth") + 4}px`,
        "line-height": 1.25,
        "text-valign": "center",
        "text-halign": "center",
        "background-color": p.tile,
        "border-width": 1,
        "border-color": p.rule,
        // Drawn by z-index alone: by default Cytoscape draws every edge under every node, so a box hid the links in it.
        // Nodes take the even levels, and a link the odd one between the box it is in and the items it joins.
        "z-index-compare": "manual",
        "z-index": (node: cytoscape.NodeSingular) => 2 * (node.data("layer") as number),
      },
    },
    {
      selector: "node.box",
      style: {
        "background-color": p.box,
        "border-color": p.rule,
        "font-weight": WEIGHT.bold,
        "text-valign": "top",
        // The label's foot sits on the box's top edge; this brings it down into the title strip.
        "text-margin-y": STRIP - 8,
      },
    },
    // A box's marks, its chevron and the icon of where its work stands, drawn over its title strip; its title in ink
    // while its work moves or can, muted once done or held up, as a task's.
    {
      selector: "node.box, node.collapsed",
      style: {
        "background-image": "data(mark)",
        "background-fit": "none",
        "background-clip": "node",
        "background-position-x": "0px",
        "background-position-y": "0px",
      },
    },
    { selector: "node.box.quiet, node.collapsed.quiet", style: { color: p.muted } },
    // A box's title alone, with no counts under it, sits half a line lower: in the middle of the strip.
    { selector: "node.box.one-line", style: { "text-margin-y": STRIP / 2 + 8 } },
    // A task's marks, its thread's stripe and badge and its stage's icon, with its title between them.
    {
      selector: "node.leaf",
      style: {
        "background-image": "data(mark)",
        "background-fit": "none",
        "background-clip": "node",
        "background-position-x": "0px",
        "background-position-y": "0px",
        "text-halign": "left-inside",
        "text-justification": "left",
        "text-margin-x": MARKS_LEFT,
      },
    },
    ...stagesOf(p),
    // A box's title starts at its left, after the chevron and the icon of where its work stands.
    {
      selector: "node.box, node.collapsed",
      style: { "text-halign": "left-inside", "text-margin-x": BOX_TEXT_LEFT },
    },
    {
      selector: "node.collapsed",
      style: {
        "border-color": p.muted,
        "font-weight": WEIGHT.bold,
      },
    },
    // Archived items are crossed out with lines; an open archived box, which holds them, has a dashed edge instead.
    {
      selector: "node.leaf.archived",
      style: {
        "background-image": (node: cytoscape.NodeSingular) => [HATCH, node.data("mark")],
        "background-repeat": ["repeat", "no-repeat"],
        "background-image-opacity": [0.3, 1],
        "background-position-x": ["0px", "0px"],
        "background-position-y": ["0px", "0px"],
      },
    },
    {
      selector: "node.collapsed.archived",
      style: {
        "background-image": (node: cytoscape.NodeSingular) => [HATCH, node.data("mark")],
        "background-repeat": ["repeat", "no-repeat"],
        "background-image-opacity": [0.3, 1],
        "background-position-x": ["0px", "0px"],
        "background-position-y": ["0px", "0px"],
      },
    },
    { selector: "node.box.archived", style: { "border-style": "dashed", "border-color": p.muted } },
    { selector: "node.lit", style: { "border-color": "data(line)", "border-width": 2 } },
    // Outside the thread lit, a task fades but its title stays legible, muted at full strength, since it can still be
    // picked. The selection is never dimmed.
    {
      selector: "node.dim",
      style: { "background-opacity": 0.35, "border-opacity": 0.35, "background-image-opacity": 0.35, color: p.muted },
    },
    { selector: "node.leaf.archived.dim", style: { "background-image-opacity": [0.1, 0.35] } },
    // The selection is told by its shape as well as its colour, since a lit border may be any hue: an ink edge, and a
    // soft halo around it that widens the node, as a focused card's.
    {
      selector: "node.selected",
      style: {
        "border-color": p.ink,
        "border-width": 2,
        "underlay-color": p.ink,
        "underlay-opacity": 0.14,
        "underlay-padding": 5,
        "underlay-shape": "round-rectangle",
      },
    },
    {
      selector: "edge",
      style: {
        width: 1.25,
        "curve-style": "straight",
        "line-color": p.link,
        "target-arrow-shape": "triangle",
        "target-arrow-color": p.link,
        "arrow-scale": 0.9,
        // Drawn over the nodes, so a tap must pass through to the node beneath.
        events: "no",
        "z-index-compare": "manual",
        "z-index": (edge: cytoscape.EdgeSingular) => edge.data("z") as number,
      },
    },
    // A link inside a box follows the way the layout routed around the box's other nodes, in right angles.
    {
      selector: "edge.routed",
      style: {
        "curve-style": "segments",
        // Bends are placed from the ends the layout gave; Cytoscape's types don't list this value, which it has had since
        // 3.25.
        "edge-distances": "endpoints" as "node-position",
        "source-endpoint": (edge: cytoscape.EdgeSingular) => edge.data("sourceEndpoint") as string,
        "target-endpoint": (edge: cytoscape.EdgeSingular) => edge.data("targetEndpoint") as string,
        "segment-weights": (edge: cytoscape.EdgeSingular) => edge.data("weights") as string,
        "segment-distances": (edge: cytoscape.EdgeSingular) => edge.data("distances") as string,
      },
    },
    {
      selector: "edge.crossing",
      style: { "line-style": "dashed", "line-color": p.signal, "target-arrow-color": p.signal },
    },
    // A lit thread's links are its route, in its line's colour; one across boxes keeps its dash.
    { selector: "edge.lit", style: { width: 2.25, "line-color": "data(line)", "target-arrow-color": "data(line)" } },
    { selector: "edge.dim", style: { opacity: 0.35 } },
    // The hovered item's links, in ink over whatever else they are, as the Threads view lights a hovered card's.
    { selector: "edge.hovered", style: { width: 2.25, "line-color": p.ink, "target-arrow-color": p.ink } },
    // Where a trunk splits: a dot in the links' colour, over them, told from two links crossing.
    {
      selector: "node.junction",
      style: {
        shape: "ellipse",
        width: 5,
        height: 5,
        label: "",
        "background-color": p.link,
        "background-image": "none",
        "border-width": 0,
        events: "no",
        "z-index-compare": "manual",
        "z-index": 999,
      },
    },
    {
      selector: "node.junction-lit",
      style: { width: 7, height: 7, "background-color": "data(line)", "border-width": 0 },
    },
  ];
}
