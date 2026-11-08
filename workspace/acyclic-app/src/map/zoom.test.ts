import cytoscape from "cytoscape";
import { expect, it } from "vitest";
import type { Placed } from "./layout";
import { zoomOutTo } from "./PlanMap";

it("updates styles only when the zoom floor changes", () => {
  const core = cytoscape({ headless: true, styleEnabled: true });
  try {
    let updates = 0;
    const styled = core as cytoscape.Core & { updateStyle(): cytoscape.Core };
    const update = styled.updateStyle.bind(core);
    styled.updateStyle = () => {
      updates++;
      return update();
    };
    const host = { clientWidth: 1192, clientHeight: 800 } as HTMLElement;
    const placed = { width: 1600, height: 1000 } as Placed;

    zoomOutTo(core, host, placed);
    expect(updates).toBe(1);
    core.minZoom(0.01);
    zoomOutTo(core, host, placed);
    expect(updates).toBe(1);
    expect(core.minZoom()).toBeCloseTo(0.33525);

    Object.assign(host, { clientWidth: 832 });
    zoomOutTo(core, host, placed);
    expect(updates).toBe(2);
  } finally {
    core.destroy();
  }
});
