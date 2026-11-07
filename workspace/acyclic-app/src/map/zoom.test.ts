import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import type { Placed } from "./layout";
import { zoomOutTo } from "./PlanMap";

describe("zoomOutTo", () => {
  it("skips an unchanged floor's global style update but still restores the minimum zoom", () => {
    const core = cytoscape({
      headless: true,
      styleEnabled: true,
      elements: [
        { data: { id: "a" }, position: { x: 0, y: 0 } },
        { data: { id: "b" }, position: { x: 100, y: 0 } },
        { data: { id: "a-b", source: "a", target: "b" } },
      ],
      style: [
        {
          selector: "node",
          style: {
            width: (node: cytoscape.NodeSingular) =>
              40 + 100 * ((node.cy().scratch("floor") as number | undefined) ?? 0),
          },
        },
      ],
    });
    try {
      let updateStyleCalls = 0;
      const styleCore = core as cytoscape.Core & { updateStyle(): cytoscape.Core };
      const updateStyle = styleCore.updateStyle.bind(core);
      styleCore.updateStyle = () => {
        updateStyleCalls++;
        return updateStyle();
      };
      const host = { clientWidth: 1192, clientHeight: 800 };
      const placed = { width: 1600, height: 1000 } as Placed;
      const nodes = [core.getElementById("a"), core.getElementById("b")];
      const viewport = { pan: { ...core.pan() }, zoom: core.zoom() };
      const widths = () => nodes.map((node) => node.width());

      expect(core.scratch("floor")).toBeUndefined();
      expect(widths()).toEqual([40, 40]);
      zoomOutTo(core, host as HTMLElement, placed);
      const first = {
        floor: core.scratch("floor"),
        minZoom: core.minZoom(),
        widths: widths(),
        calls: updateStyleCalls,
      };
      expect(first.floor).toBeCloseTo(0.6705);
      expect(first.minZoom).toBeCloseTo(0.33525);
      expect(first.widths[0]).toBeCloseTo(107.05);
      expect(first.widths[1]).toBeCloseTo(107.05);
      expect(first.calls).toBe(1);
      expect(core.pan()).toEqual(viewport.pan);
      expect(core.zoom()).toBe(viewport.zoom);

      core.minZoom(0.01);
      const beforeRepeat = updateStyleCalls;
      zoomOutTo(core, host as HTMLElement, placed);
      const repeated = {
        floor: core.scratch("floor"),
        minZoom: core.minZoom(),
        widths: widths(),
        calls: updateStyleCalls,
      };
      expect(repeated.floor).toBeCloseTo(0.6705);
      expect(repeated.minZoom).toBeCloseTo(0.33525);
      expect(repeated.widths[0]).toBeCloseTo(107.05);
      expect(repeated.widths[1]).toBeCloseTo(107.05);
      expect(repeated.calls).toBe(beforeRepeat);
      expect(core.pan()).toEqual(viewport.pan);
      expect(core.zoom()).toBe(viewport.zoom);

      host.clientWidth = 832;
      zoomOutTo(core, host as HTMLElement, placed);
      const narrower = {
        floor: core.scratch("floor"),
        minZoom: core.minZoom(),
        widths: widths(),
        calls: updateStyleCalls,
      };
      expect(narrower.floor).toBeCloseTo(0.468);
      expect(narrower.minZoom).toBeCloseTo(0.234);
      expect(narrower.widths[0]).toBeCloseTo(86.8);
      expect(narrower.widths[1]).toBeCloseTo(86.8);
      expect(narrower.calls).toBe(beforeRepeat + 1);
      expect(core.pan()).toEqual(viewport.pan);
      expect(core.zoom()).toBe(viewport.zoom);

      host.clientWidth = 1192;
      zoomOutTo(core, host as HTMLElement, placed);
      expect(core.scratch("floor")).toBeCloseTo(0.6705);
      expect(core.minZoom()).toBeCloseTo(0.33525);
      expect(widths()[0]).toBeCloseTo(107.05);
      expect(widths()[1]).toBeCloseTo(107.05);
      expect(updateStyleCalls).toBe(beforeRepeat + 2);
      expect(core.pan()).toEqual(viewport.pan);
      expect(core.zoom()).toBe(viewport.zoom);
    } finally {
      core.destroy();
    }
  });
});
