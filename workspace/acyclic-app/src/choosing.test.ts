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

const key = { kind: "key" } as const;
const change = (value: string) => ({ kind: "change", value }) as const;

describe("choose", () => {
  it("writes a pick from the open list at once", () => {
    expect(run("a", [{ kind: "pointer" }, change("b")])).toEqual({ state: UNCHOSEN, writes: ["b"] });
    expect(run("a", [change("a")]).writes).toEqual([]);
  });

  it("moves a draft on arrow keys, written once on Enter", () => {
    const stepped = run("a", [key, change("b"), key, change("c")]);
    expect(stepped).toEqual({ state: { draft: "c", keyed: false }, writes: [] });
    expect(run("a", [key, change("b"), key, change("c"), { kind: "enter" }])).toEqual({
      state: UNCHOSEN,
      writes: ["c"],
    });
  });

  it("drops a draft on Escape or leaving, and writes nothing for Enter without one or back where it was", () => {
    expect(run("a", [key, change("b"), { kind: "cancel" }])).toEqual({ state: UNCHOSEN, writes: [] });
    expect(run("a", [{ kind: "enter" }]).writes).toEqual([]);
    expect(run("a", [key, change("b"), key, change("a"), { kind: "enter" }]).writes).toEqual([]);
  });

  it("writes a pick at once after keys, once the pointer is back", () => {
    expect(run("a", [key, change("b"), { kind: "pointer" }, change("c")]).writes).toEqual(["c"]);
  });
});
