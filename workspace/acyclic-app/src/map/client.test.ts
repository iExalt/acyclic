import { describe, expect, it } from "vitest";
import { Layouts } from "./client";
import { PORTRAIT } from "./elk";
import { item } from "./fixture";
import { type BoxInput, type BoxLayout, type LayBox, ROOT } from "./layout";
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

  it("does not promote an overtaken root packing, even when it finishes last", async () => {
    let releaseOld: ((layout: BoxLayout) => void) | undefined;
    let waitingForOld = false;
    const layBox: LayBox = async (input: BoxInput) => {
      if (input.id === ROOT && input.children.some((child) => child.id === "c") && input.children[0]?.height === 200) {
        waitingForOld = true;
        return new Promise((resolve) => {
          releaseOld = resolve;
        });
      }
      if (input.id === ROOT)
        return {
          width: 300,
          height: 300,
          at: {
            a: { x: 12, y: 12 },
            b: { x: 12, y: input.children[0]?.height === 200 ? 236 : 100 },
            c: { x: 236, y: 12 },
            d: { x: 236, y: 12 },
          },
        };
      return {
        width: 200,
        height: input.children.length === 1 ? 200 : 300,
        at: Object.fromEntries(input.children.map((child, index) => [child.id, { x: 12, y: 12 + index * 100 }])),
      };
    };
    const layouts = new Layouts(layBox);
    const model = buildModel([item("a"), item("a1", "a"), item("b")]);
    const collapsed = visible(model, { collapsed: new Set(["a"]), showArchived: false });
    const packed = await layouts.layout(collapsed, PORTRAIT);
    expect(packed).not.toBeNull();

    const stale = layouts.layout(
      visible(buildModel([item("a"), item("a1", "a"), item("b"), item("c")]), {
        collapsed: new Set(),
        showArchived: false,
      }),
      PORTRAIT,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(waitingForOld).toBe(true);
    const newer = layouts.layout(
      visible(buildModel([item("a"), item("a1", "a"), item("b"), item("d")]), {
        collapsed: new Set(["a"]),
        showArchived: false,
      }),
      PORTRAIT,
    );
    expect(await newer).not.toBeNull();
    releaseOld?.({
      width: 300,
      height: 300,
      at: { a: { x: 12, y: 12 }, b: { x: 12, y: 236 } },
    });
    expect(await stale).toBeNull();

    // A third request revisits the discarded topology at a different size. It must freshly lay it out, not hold from
    // the stale expansion that was never displayed.
    const third = await layouts.layout(
      visible(buildModel([item("a"), item("a1", "a"), item("b"), item("c")]), {
        collapsed: new Set(["a"]),
        showArchived: false,
      }),
      PORTRAIT,
    );
    expect(third?.nodes.get("b")).toMatchObject({ y: packed?.nodes.get("b")?.y });
  });

  it("does not promote a submitted root packing after its effect becomes stale", async () => {
    let release: ((layout: BoxLayout) => void) | undefined;
    let live = true;
    let rootCalls = 0;
    const layBox: LayBox = (input) => {
      if (input.id !== ROOT) return Promise.resolve({ width: 200, height: 64, at: {} });
      rootCalls += 1;
      if (rootCalls === 1)
        return new Promise((resolve) => {
          release = resolve;
        });
      return Promise.resolve({ width: 300, height: 100, at: { a: { x: 12, y: 12 } } });
    };
    const layouts = new Layouts(layBox);
    const view = visible(buildModel([item("a")]), { collapsed: new Set(), showArchived: false });
    const old = layouts.layout(
      view,
      PORTRAIT,
      "same-source",
      new Map([["a", { width: 200, height: 80 }]]),
      new Map(),
      () => live,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    live = false;
    release?.({ width: 300, height: 100, at: { a: { x: 12, y: 12 } } });
    expect(await old).toBeNull();

    const retry = await layouts.layout(view, PORTRAIT, "same-source", new Map([["a", { width: 200, height: 64 }]]));
    expect(retry).not.toBeNull();
    expect(rootCalls).toBe(2);
  });
});
