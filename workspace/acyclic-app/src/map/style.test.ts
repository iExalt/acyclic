import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";
import { CLOSED, HATCH, OPEN, STYLE } from "./style";

/** A task's marks, as an image its data carries. */
const MARK = "data:image/svg+xml;utf8,%3Csvg%2F%3E";

describe("style", () => {
  it("draws each kind of box with its chevron, a task with its marks, crosses out what is archived, and dashes an open archived box", () => {
    const core = cytoscape({ headless: true, styleEnabled: true, style: STYLE });
    const node = (id: string, classes: string) =>
      core.add({
        group: "nodes",
        data: { id, label: id, width: 200, height: 64, textWidth: 150, layer: 0, mark: MARK },
        classes,
      });
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
    expect(drawn("leaf")).toEqual({ images: [MARK], border: "solid", align: "left-inside" });
    expect(drawn("leaf archived")).toEqual({ images: [HATCH, MARK], border: "solid", align: "left-inside" });
    expect(drawn("box")).toEqual({ images: [OPEN], border: "solid", align: "left-inside" });
    expect(drawn("box archived")).toEqual({ images: [OPEN], border: "dashed", align: "left-inside" });
    expect(drawn("collapsed")).toEqual({ images: [CLOSED], border: "solid", align: "left-inside" });
    expect(drawn("collapsed archived")).toEqual({ images: [HATCH, CLOSED], border: "solid", align: "left-inside" });
    // Lit or selected, an open archived box keeps its dash.
    expect(drawn("box archived lit selected").border).toBe("dashed");
  });
});
