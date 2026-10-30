// How the Map draws its nodes and links: boxes are the zones of the map and items its stations, crossed out when
// archived, and a lit thread takes its line's colour.

import type cytoscape from "cytoscape";
import type { Stage } from "../api";
import { TOKENS } from "../lines";
import { STRIP, TITLE } from "./chevron";
import { MARKS_LEFT, QUIET, TINTS } from "./marks";

/** The face labels are drawn in. */
export const FONT = "Overpass, system-ui, sans-serif";

/** An inline SVG, as the renderer takes an image. */
const svg = (size: number, body: string) =>
  `data:image/svg+xml;utf8,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${body}</svg>`)}`;
const stroke = `fill="none" stroke="${TOKENS.muted}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"`;
/** An open box's chevron points down, at what it holds; a closed one's points right. Drawn, not set in the font. */
export const OPEN = svg(12, `<path d="M2 4l4 4 4-4" ${stroke}/>`);
export const CLOSED = svg(12, `<path d="M4 2l4 4-4 4" ${stroke}/>`);
/** Lines across what is archived, crossing it out: a tile that repeats seamlessly, each line 8 px from the next. */
export const HATCH = svg(8, `<path d="M-2 2l4-4M0 8l8-8M6 10l4-4" stroke="${TOKENS.muted}" stroke-width="1"/>`);
/** Where a chevron is drawn: in the middle of the square at the left of the title strip. */
const CHEVRON_AT = `${(TITLE - 12) / 2}px`;
const CHEVRON_DOWN = `${(STRIP - 12) / 2}px`;

/** Each stage's tint, and a quieter title for work finished or set aside. */
const STAGES: cytoscape.StylesheetJson = (Object.keys(TINTS) as Stage[]).map((stage) => ({
  selector: `node.leaf.stage-${stage}`,
  style: { "background-color": TINTS[stage], color: QUIET.has(stage) ? TOKENS.muted : TOKENS.ink },
}));

/** Boxes are the zones of the map and items its stations; a lit item takes its thread's line colour. */
export const STYLE: cytoscape.StylesheetJson = [
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
      // Labels come cut to fit, a line at a time; the slack keeps the renderer from wrapping a line again.
      "text-wrap": "wrap",
      "text-max-width": (node: cytoscape.NodeSingular) => `${node.data("textWidth") + 4}px`,
      "line-height": 1.25,
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
      // The label's foot sits on the box's top edge; this brings it down into the title strip.
      "text-margin-y": STRIP - 8,
      "background-image": OPEN,
      "background-position-x": CHEVRON_AT,
      "background-position-y": CHEVRON_DOWN,
    },
  },
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
  ...STAGES,
  // A box's title starts at its left, after the chevron.
  {
    selector: "node.box, node.collapsed",
    style: { "text-halign": "left-inside", "text-margin-x": TITLE - 4 },
  },
  {
    selector: "node.collapsed",
    style: {
      "border-color": TOKENS.muted,
      "font-weight": 700,
      "background-image": CLOSED,
      "background-position-x": CHEVRON_AT,
      "background-position-y": "50%",
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
      "background-image": [HATCH, CLOSED],
      "background-repeat": ["repeat", "no-repeat"],
      "background-image-opacity": [0.3, 1],
      "background-position-x": ["0px", CHEVRON_AT],
      "background-position-y": ["0px", "50%"],
    },
  },
  { selector: "node.box.archived", style: { "border-style": "dashed", "border-color": TOKENS.muted } },
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
    style: { "line-style": "dashed", "line-color": TOKENS.signal, "target-arrow-color": TOKENS.signal },
  },
  // A lit thread's links are its route, in its line's colour; one across boxes keeps its dash.
  { selector: "edge.lit", style: { width: 3, "line-color": "data(line)", "target-arrow-color": "data(line)" } },
  { selector: "edge.dim", style: { opacity: 0.35 } },
];
