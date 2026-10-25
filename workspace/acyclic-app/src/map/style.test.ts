import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import { CLOSED, HATCH, OPEN, STYLE } from "./style";

describe("style", () => {
  it("draws each kind of box with its chevron, crosses out what is archived, and dashes an open archived box", () => {
    const core = cytoscape({ headless: true, styleEnabled: true, style: STYLE });
    const node = (id: string, classes: string) =>
      core.add({ group: "nodes", data: { id, label: id, width: 200, height: 64, textWidth: 150, layer: 0 }, classes });
    const drawn = (classes: string) => {
      // The parsed style, which Cytoscape's types leave out.
      const one = node(classes.replaceAll(" ", "-"), classes) as unknown as {
        pstyle(name: string): { value: unknown };
      };
      return {
        images: one.pstyle("background-image").value,
        border: one.pstyle("border-style").value,
        align: one.pstyle("text-halign").value,
      };
    };
    expect(drawn("leaf")).toEqual({ images: ["none"], border: "solid", align: "center" });
    expect(drawn("leaf archived")).toEqual({ images: [HATCH], border: "solid", align: "center" });
    expect(drawn("box")).toEqual({ images: [OPEN], border: "solid", align: "left-inside" });
    expect(drawn("box archived")).toEqual({ images: [OPEN], border: "dashed", align: "left-inside" });
    expect(drawn("collapsed")).toEqual({ images: [CLOSED], border: "solid", align: "left-inside" });
    expect(drawn("collapsed archived")).toEqual({ images: [HATCH, CLOSED], border: "solid", align: "left-inside" });
    // Lit or selected, an open archived box keeps its dash.
    expect(drawn("box archived lit selected").border).toBe("dashed");
  });
});
