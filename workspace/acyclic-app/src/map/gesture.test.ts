import { describe, expect, it, vi } from "vitest";
import { createGesture, type Timers, WATCHDOG_MS, WHEEL_QUIET_MS } from "./gesture";

/** A clock the test moves by hand. */
function clock() {
  let now = 0;
  let next = 1;
  const due = new Map<number, { at: number; callback: () => void }>();
  const timers: Timers = {
    set: (callback, ms) => {
      due.set(next, { at: now + ms, callback });
      return next++;
    },
    clear: (handle) => {
      due.delete(handle as number);
    },
  };
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      const soonest = [...due.entries()].filter(([, one]) => one.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (soonest === undefined) break;
      due.delete(soonest[0]);
      now = soonest[1].at;
      soonest[1].callback();
    }
    now = end;
  };
  return { timers, advance };
}

function setup() {
  const time = clock();
  const pending = vi.fn();
  const gesture = createGesture({ timers: time.timers, onPending: pending });
  return { ...time, gesture, pending };
}

describe("gesture", () => {
  it("runs a job at once when nothing is going on", () => {
    const { gesture } = setup();
    const job = vi.fn();
    gesture.defer(job);
    expect(job).toHaveBeenCalledTimes(1);
    expect(gesture.pending()).toBe(false);
  });

  it("holds a job while a pointer is down, runs it when the pointer goes up, and the latest job wins", () => {
    const { gesture, pending } = setup();
    const first = vi.fn();
    const second = vi.fn();
    gesture.down(1);
    gesture.defer(first);
    gesture.defer(second);
    expect(first).not.toHaveBeenCalled();
    expect(second).not.toHaveBeenCalled();
    expect(pending.mock.calls).toEqual([[true]]);
    gesture.up(1);
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
    expect(pending.mock.calls).toEqual([[true], [false]]);
  });

  it("waits for every finger of a pinch", () => {
    const { gesture } = setup();
    const job = vi.fn();
    gesture.touches(2);
    gesture.defer(job);
    gesture.touches(1);
    expect(job).not.toHaveBeenCalled();
    gesture.touches(0);
    expect(job).toHaveBeenCalledTimes(1);
  });

  it("counts a wheel zoom until a pause of the quiet time, each step extending it", () => {
    const { gesture, advance } = setup();
    const job = vi.fn();
    gesture.wheel();
    gesture.defer(job);
    advance(WHEEL_QUIET_MS - 1);
    gesture.wheel();
    advance(WHEEL_QUIET_MS - 1);
    expect(job).not.toHaveBeenCalled();
    advance(1);
    expect(job).toHaveBeenCalledTimes(1);
    expect(gesture.active()).toBe(false);
  });

  it("ignores hover: moving with no pointer down is not a gesture", () => {
    const { gesture } = setup();
    const job = vi.fn();
    gesture.move();
    gesture.move();
    gesture.defer(job);
    expect(gesture.active()).toBe(false);
    expect(job).toHaveBeenCalledTimes(1);
  });

  it("clears a pointer whose end was never seen, once it has shown no sign of life for the watchdog's time", () => {
    const { gesture, advance } = setup();
    const job = vi.fn();
    gesture.down(7);
    gesture.defer(job);
    advance(WATCHDOG_MS - 1);
    gesture.move();
    advance(WATCHDOG_MS - 1);
    expect(job).not.toHaveBeenCalled();
    advance(1);
    expect(job).toHaveBeenCalledTimes(1);
    expect(gesture.active()).toBe(false);
  });

  it("forgets everything on a reset, as for a cancel, a blur or a hidden page", () => {
    const { gesture } = setup();
    const job = vi.fn();
    gesture.down(1);
    gesture.touches(2);
    gesture.wheel();
    gesture.defer(job);
    gesture.reset();
    expect(job).toHaveBeenCalledTimes(1);
    expect(gesture.active()).toBe(false);
  });

  it("drops a cancelled job", () => {
    const { gesture, pending } = setup();
    const job = vi.fn();
    gesture.down(1);
    gesture.defer(job);
    gesture.cancel();
    gesture.up(1);
    expect(job).not.toHaveBeenCalled();
    expect(pending.mock.calls).toEqual([[true], [false]]);
  });
});
