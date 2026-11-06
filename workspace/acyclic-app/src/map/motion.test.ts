import { describe, expect, it, vi } from "vitest";
import { busy, subscribe, track } from "./motion";

describe("motion", () => {
  it("counts what is tracked until it is released, and a release twice counts once", () => {
    const first = track();
    const second = track();
    expect(busy()).toBe(2);
    first();
    first();
    expect(busy()).toBe(1);
    second();
    expect(busy()).toBe(0);
  });

  it("tells a subscriber once per tick, however many changes the tick saw, and stops when it unsubscribes", async () => {
    const listener = vi.fn();
    const stop = subscribe(listener);
    const a = track();
    const b = track();
    a();
    b();
    expect(listener).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(listener).toHaveBeenCalledTimes(1);
    stop();
    track()();
    await Promise.resolve();
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
