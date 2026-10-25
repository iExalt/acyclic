import { describe, expect, it } from "vitest";
import { type ChoiceEvent, type ChoiceState, choose, UNCHOSEN } from "./choosing";

/** Runs events from no draft, returning the final state and every write. */
function run(applied: string, events: ChoiceEvent[]) {
  let state: ChoiceState = UNCHOSEN;
  const writes: string[] = [];
  for (const event of events) {
    const step = choose(state, applied, event);
    state = step.state;
    if (step.write !== null) writes.push(step.write);
  }
  return { state, writes };
}

const keyed = (value: string) => ({ kind: "change", value, keyed: true }) as const;
const picked = (value: string) => ({ kind: "change", value, keyed: false }) as const;

describe("choose", () => {
  it("writes a pick from the open list at once, whatever keys opened it", () => {
    expect(run("a", [picked("b")])).toEqual({ state: UNCHOSEN, writes: ["b"] });
    expect(run("a", [picked("a")]).writes).toEqual([]);
  });

  it("moves a draft on arrow keys, written once on Enter", () => {
    expect(run("a", [keyed("b"), keyed("c")])).toEqual({ state: { draft: "c" }, writes: [] });
    expect(run("a", [keyed("b"), keyed("c"), { kind: "enter" }])).toEqual({ state: UNCHOSEN, writes: ["c"] });
  });

  it("drops a draft on Escape or leaving, and writes nothing for Enter without one or back where it was", () => {
    expect(run("a", [keyed("b"), { kind: "cancel" }])).toEqual({ state: UNCHOSEN, writes: [] });
    expect(run("a", [{ kind: "enter" }]).writes).toEqual([]);
    expect(run("a", [keyed("b"), keyed("a"), { kind: "enter" }]).writes).toEqual([]);
  });

  it("writes a pick at once over a draft", () => {
    expect(run("a", [keyed("b"), picked("c")])).toEqual({ state: UNCHOSEN, writes: ["c"] });
  });
});
