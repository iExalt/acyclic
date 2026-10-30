import { describe, expect, it } from "vitest";
import { Layouts } from "./client";
import { PORTRAIT } from "./elk";
import { item } from "./fixture";
import type { LayBox } from "./layout";
import { buildModel, visible } from "./model";

describe("Layouts", () => {
  it("answers a request a newer one overtook with null, and the newest with its layout", async () => {
    const answers: (() => void)[] = [];
    const layBox: LayBox = () =>
      new Promise((resolve) => answers.push(() => resolve({ width: 10, height: 10, at: {} })));
    const layouts = new Layouts(layBox);
    const model = buildModel([item("a")]);
    const old = layouts.layout(visible(model, { collapsed: new Set(), showArchived: false }), PORTRAIT);
    const fresh = layouts.layout(visible(model, { collapsed: new Set(), showArchived: true }), PORTRAIT);
    await new Promise((resolve) => setTimeout(resolve, 0));
    for (const answer of answers.splice(0)) answer();
    expect(await old).toBeNull();
    expect(await fresh).not.toBeNull();
  });
});
