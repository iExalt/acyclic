// How the Map draws its nodes and links: boxes are the zones of the map and items its stations, and a lit thread takes its line's colour.

import type cytoscape from "cytoscape";
import type { Stage } from "../api";
import type { Palette } from "../palette";
import { STRIP } from "./chevron";
import { STUB_TEXT_LEFT } from "./labelmetrics";
import { BOX_TEXT_LEFT, MARKS_LEFT, QUIET } from "./marks";

/** The face labels are drawn in. */
export const FONT = "Overpass, system-ui, sans-serif";
/** Its weights, a tile's title and a box's: the app's text weight, and two steps above it, so a box's title leads
 *  its tiles; in hundreds, as the renderer takes no other. */
export const WEIGHT = { regular: 500, bold: 700 } as const;
/** Where tiles start drawing, over every box at any depth: z-indexes below it are boxes and the links among them. */
export const TILES = 1000;

/** Each stage's tint, and a quieter title for work finished or set aside. */
const stagesOf = (p: Palette): cytoscape.StylesheetJson =>
  (Object.keys(p.tints) as Stage[]).map((stage) => ({
    selector: `node.leaf.stage-${stage}`,
    style: { "background-color": p.tints[stage], color: QUIET.has(stage) ? p.muted : p.ink },
  }));

const made = new WeakMap<Palette, Map<string, cytoscape.StylesheetJson>>();

/** The Map's stylesheet in a palette, made once for each. */
export function styleFor(p: Palette, family = FONT): cytoscape.StylesheetJson {
  let styles = made.get(p);
  if (styles === undefined) {
    styles = new Map();
    made.set(p, styles);
  }
  let style = styles.get(family);
  if (style === undefined) {
    style = sheetOf(p, family);
    styles.set(family, style);
  }
  return style;
}

/** The title band before a stub's reserved row. */
const lifted = (node: cytoscape.NodeSingular) => -Number(node.data("reserve") ?? 0) / 2;
const titleBand = (node: cytoscape.NodeSingular) =>
  Number(node.data("header") ?? STRIP) - Number(node.data("reserve") ?? 0);

/** Boxes are the zones of the map and items its stations; a lit item takes its thread's line colour. */
function sheetOf(p: Palette, family: string): cytoscape.StylesheetJson {
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
        "font-family": family,
        "font-size": 13,
        "font-weight": WEIGHT.regular,
        // Labels already wrap to fit, a line at a time; the slack keeps the renderer from wrapping a line again.
        "text-wrap": "wrap",
        "text-max-width": (node: cytoscape.NodeSingular) => `${node.data("textWidth") + 4}px`,
        "line-height": 1.25,
        "text-valign": "center",
        "text-halign": "center",
        "background-color": p.tile,
        "border-width": 1,
        "border-color": p.rule,
        // Drawn by z-index alone: by default Cytoscape draws every edge under every node, so a box hid the links in it.
        // Boxes take the even levels, and a link the odd one between the box it is in and the items it joins; tiles,
        // which never lie over one another, are drawn over every box, so a link across boxes can pass between.
        "z-index-compare": "manual",
        "z-index": (node: cytoscape.NodeSingular) =>
          (node.hasClass("box") ? 0 : TILES) + 2 * (node.data("layer") as number),
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
        "text-margin-y": (node: cytoscape.NodeSingular) => titleBand(node) - 8,
      },
    },
    // A box's marks, its chevron and the icon of where its work stands, drawn over its title strip; its title in ink
    // while its work moves or can, muted once done or held up, as a tile's.
    {
      selector: "node.box, node.collapsed",
      style: {
        "background-image": (node: cytoscape.NodeSingular) => [node.data("mark"), node.data("stageMark") ?? "none"],
        "background-fit": ["none", "none"],
        "background-clip": "node",
        "background-position-x": ["0px", "100%"],
        "background-offset-x": ["0px", "-12px"],
        "background-position-y": (node: cytoscape.NodeSingular) => [
          "0px",
          `${(node.hasClass("box") ? STRIP / 2 : (Number(node.data("height")) - Number(node.data("reserve") ?? 0)) / 2) - 8}px`,
        ],
      },
    },
    { selector: "node.box.quiet, node.collapsed.quiet", style: { color: p.muted } },
    // A box's title alone, with no counts under it, sits half a line lower: in the middle of the strip.
    {
      selector: "node.box.one-line",
      style: { "text-margin-y": (node: cytoscape.NodeSingular) => titleBand(node) / 2 + 8 },
    },
    // A tile's marks, its thread's stripe and its stage's icon, with its title between them.
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
        // The room at its foot a stub takes is not the title's: the title is centred in what is left.
        "text-margin-y": lifted,
      },
    },
    ...stagesOf(p),
    // A box's title starts at its left, after the chevron.
    {
      selector: "node.box, node.collapsed",
      style: { "text-halign": "left-inside", "text-margin-x": BOX_TEXT_LEFT },
    },
    {
      selector: "node.collapsed",
      style: {
        "border-color": p.muted,
        "font-weight": WEIGHT.bold,
        "text-margin-y": lifted,
      },
    },
    { selector: "node.lit", style: { "border-color": "data(line)", "border-width": 1.5 } },
    {
      selector: "node.preview",
      style: {
        "outline-color": p.ink,
        "outline-width": 1,
        "outline-offset": 3,
        "outline-opacity": 1,
      },
    },
    // The selection is told by its shape as well as its colour, since a lit border may be any hue, a dark one close to
    // ink: an ink edge twice a lit one's, and a faint halo around it that widens the node, as a focused tile's; a
    // stronger one read as a second ring.
    {
      selector: "node.selected",
      style: {
        "outline-width": 0,
        "border-color": p.ink,
        "border-width": 3,
        "underlay-color": p.ink,
        "underlay-opacity": 0.1,
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
    // A wait that leaves the initiative: a quiet dashed chip at a tile's foot, muted, so it draws less than a real
    // block does, with only its arrow in the signal's colour, as a link across boxes is; it takes a tap, which opens
    // what it names.
    {
      selector: "node.stub",
      style: {
        "background-opacity": 0,
        "border-width": 1,
        "border-style": "dashed",
        "border-opacity": 1,
        "border-color": p.muted,
        color: p.muted,
        "font-size": 12,
        "text-halign": "left-inside",
        "text-justification": "left",
        "text-valign": "center",
        "text-margin-x": STUB_TEXT_LEFT,
        "background-image": arrow(p.signal),
        "background-fit": "none",
        "background-clip": "node",
        "background-position-x": "8px",
        "background-position-y": "50%",
        "text-margin-y": 0,
        "z-index": (node: cytoscape.NodeSingular) => TILES + 2 * (node.data("layer") as number) + 1,
      },
    },
    {
      selector: "edge.crossing",
      style: { "line-style": "dashed", "line-color": p.signal, "target-arrow-color": p.signal },
    },
    // A lit thread's links are its route, in its line's colour; one across boxes keeps its dash.
    { selector: "edge.lit", style: { width: 2.25, "line-color": "data(line)", "target-arrow-color": "data(line)" } },
    // The hovered item's links, in ink over whatever else they are, as the Threads view lights a hovered tile's.
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
    // A box's line between its unlinked items and its graph: dashed, in the controls' edge colour; a pixel-wide node
    // filled with a dash repeated down it, as Cytoscape draws no border on a node with no width.
    {
      selector: "node.divider",
      style: {
        shape: "rectangle",
        width: 1,
        height: "data(height)",
        label: "",
        "background-opacity": 0,
        "background-image": dash(p.edge),
        "background-repeat": "repeat-y",
        "background-width": 1,
        "background-height": 8,
        "background-image-containment": "over",
        "border-width": 0,
        events: "no",
        "z-index-compare": "manual",
        "z-index": (node: cytoscape.NodeSingular) => 2 * (node.data("layer") as number) + 1,
      },
    },
    {
      selector: "node.junction-lit",
      style: { width: 7, height: 7, "background-color": "data(line)", "border-width": 0 },
    },
  ];
}

/** The stub's arrow, 12 px square, as an image. */
function arrow(colour: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12"><path d="M3 9L9 3M4.5 3H9v4.5" fill="none" stroke="${colour}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

/** A dash 4 px long in `colour` with a 4 px gap, as an image a pixel wide, repeated down a divider. */
function dash(colour: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1" height="8"><rect width="1" height="4" fill="${colour}"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
