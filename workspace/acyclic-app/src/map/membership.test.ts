import { describe, expect, it } from "vitest";
import { PORTRAIT } from "./elk";
import { item } from "./fixture";
import { type LayBox, LayoutCache, place } from "./layout";
import { buildModel, type View, visible } from "./model";

/** Children in rows of two, as far apart as packing puts them; a box grows with its children. */
const layBox: LayBox = async (input) => {
  const at: Record<string, { x: number; y: number }> = {};
  let width = 0;
  let y = 36;
  for (let row = 0; row < input.children.length; row += 2) {
    const pair = input.children.slice(row, row + 2);
    pair.forEach((child, i) => {
      at[child.id] = { x: 12 + i * (child.width + 24), y };
      width = Math.max(width, child.width);
    });
    y += Math.max(...pair.map((child) => child.height)) + 32;
  }
  return { width: width * 2 + 48, height: y + 12, at };
};

/** A small deterministic generator, so a failing plan can be replayed from its seed. */
function random(seed: number) {
  let state = seed;
  return (below: number) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state % below;
  };
}

type Plan = ReturnType<typeof item>[];

function startingPlan(pick: (below: number) => number): Plan {
  const plan: Plan = [item("r0"), item("r1")];
  for (let i = 0; i < 8; i++) plan.push(item(`n${i}`, plan[pick(plan.length)]?.id ?? null));
  return plan;
}

/** A write that moves nothing across boxes: adds or removes a leaf or a link, or retitles, inside one box. */
function write(plan: Plan, pick: (below: number) => number, step: number): Plan {
  const kind = pick(4);
  const leaves = plan.filter((one) => !plan.some((other) => other.parent === one.id));
  if (kind === 0) return [...plan, item(`add${step}`, plan[pick(plan.length)]?.id ?? null)];
  if (kind === 1) {
    const gone = leaves[pick(leaves.length)];
    return plan.filter((one) => one !== gone && one.links.waits_for.every((id) => id !== gone?.id));
  }
  if (kind === 2) {
    const chosen = plan[pick(plan.length)];
    const siblings = plan.filter((one) => one.parent === chosen?.parent);
    const before = siblings.slice(0, siblings.indexOf(chosen as Plan[number]));
    const waits = before[pick(before.length || 1)];
    return plan.map((one) =>
      one === chosen
        ? {
            ...one,
            links: {
              ...one.links,
              waits_for: waits === undefined ? [] : one.links.waits_for.includes(waits.id) ? [] : [waits.id],
            },
          }
        : one,
    );
  }
  const chosen = plan[pick(plan.length)];
  return plan.map((one) => (one === chosen ? { ...one, title: `${one.title} and a longer title` } : one));
}

const parents = (view: View) => new Map(view.nodes.map((node) => [node.id, node.parent]));

describe("a write that moves nothing across boxes", () => {
  it("keeps every surviving tile inside the box it was in, and inside that box's new frame", async () => {
    for (let seed = 1; seed <= 40; seed++) {
      const pick = random(seed);
      const cache = new LayoutCache();
      let plan = startingPlan(pick);
      let was = visible(buildModel(plan), { collapsed: new Set() });
      await place(was, cache, layBox, PORTRAIT);
      for (let step = 0; step < 6; step++) {
        plan = write(plan, pick, step);
        const view = visible(buildModel(plan), { collapsed: new Set() });
        const placed = await place(view, cache, layBox, PORTRAIT);
        const before = parents(was);
        const now = parents(view);
        for (const [id, parent] of now) {
          if (before.has(id)) expect(parent, `seed ${seed} step ${step}: ${id}`).toBe(before.get(id));
          const at = placed.nodes.get(id);
          expect(at, `seed ${seed} step ${step}: ${id} placed`).toBeDefined();
          if (parent === null || at === undefined) continue;
          const box = placed.nodes.get(parent);
          expect(box, `seed ${seed} step ${step}: ${parent} placed`).toBeDefined();
          if (box === undefined) continue;
          expect(at.x, `seed ${seed} step ${step}: ${id} in ${parent}`).toBeGreaterThanOrEqual(box.x);
          expect(at.y).toBeGreaterThanOrEqual(box.y);
          expect(at.x + at.width).toBeLessThanOrEqual(box.x + box.width);
          expect(at.y + at.height).toBeLessThanOrEqual(box.y + box.height);
        }
        was = view;
      }
    }
  });
});
