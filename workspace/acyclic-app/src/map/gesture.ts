// Whether the user is in the middle of a gesture on the Map (a pan, a pinch, a wheel zoom), so a redraw that would move
// the tiles under their fingers waits until it ends. Pure but for the timers, which are injected. Hover is not a
// gesture. A gesture that never reports its end, as when the page loses the pointer, is cleared by a watchdog.

/** How long after the last wheel step a wheel zoom counts as going on. */
export const WHEEL_QUIET_MS = 300;
/**
 * How long a pointer or touch may be down without a sign of life (a pointer or touch move) before it is taken as stuck
 * and cleared. A finger held still sends nothing, so this is long enough for a pause to read the plan under it.
 */
export const WATCHDOG_MS = 5000;

export interface Timers {
  set(callback: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

const BROWSER: Timers = {
  set: (callback, ms) => setTimeout(callback, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface GestureOptions {
  timers?: Timers;
  /** Called with whether a job is waiting for the gesture to end. */
  onPending?: (pending: boolean) => void;
}

export interface Gesture {
  /** A pointer went down (a mouse button, a finger, a pen). */
  down(id: number): void;
  /** A pointer went up or was cancelled. */
  up(id: number): void;
  /** A pointer moved, or a touch did: a sign of life for the watchdog. Hover alone must not call this. */
  move(): void;
  /** The fingers on the screen now, from a touch event. */
  touches(count: number): void;
  /** A wheel step. */
  wheel(): void;
  /** Forgets everything: for a cancel, a blur, or the page being hidden. */
  reset(): void;
  active(): boolean;
  /** Runs `job` now, or, while a gesture is going on, once it ends; a later job replaces one still waiting. */
  defer(job: () => void): void;
  /** Drops a waiting job. */
  cancel(): void;
  pending(): boolean;
  dispose(): void;
}

export function createGesture({ timers = BROWSER, onPending }: GestureOptions = {}): Gesture {
  const pointers = new Set<number>();
  let fingers = 0;
  let wheeling = false;
  let wheelTimer: unknown = null;
  let dog: unknown = null;
  let job: (() => void) | null = null;

  const isActive = () => pointers.size > 0 || fingers > 0 || wheeling;
  const held = () => pointers.size > 0 || fingers > 0;

  const setPending = (next: (() => void) | null) => {
    const was = job !== null;
    job = next;
    if (was !== (job !== null)) onPending?.(job !== null);
  };
  const flush = () => {
    if (isActive() || job === null) return;
    const run = job;
    // Run first, so what it starts is counted before the wait stops being.
    try {
      run();
    } finally {
      if (job === run) setPending(null);
    }
  };
  const watch = () => {
    if (dog !== null) timers.clear(dog);
    dog = null;
    if (held())
      dog = timers.set(() => {
        dog = null;
        pointers.clear();
        fingers = 0;
        flush();
      }, WATCHDOG_MS);
  };
  const changed = () => {
    watch();
    flush();
  };

  return {
    down(id) {
      pointers.add(id);
      watch();
    },
    up(id) {
      pointers.delete(id);
      changed();
    },
    move() {
      if (held()) watch();
    },
    touches(count) {
      fingers = count;
      changed();
    },
    wheel() {
      wheeling = true;
      if (wheelTimer !== null) timers.clear(wheelTimer);
      wheelTimer = timers.set(() => {
        wheelTimer = null;
        wheeling = false;
        flush();
      }, WHEEL_QUIET_MS);
    },
    reset() {
      pointers.clear();
      fingers = 0;
      wheeling = false;
      if (wheelTimer !== null) timers.clear(wheelTimer);
      wheelTimer = null;
      changed();
    },
    active: isActive,
    defer(next) {
      if (isActive()) {
        setPending(next);
        return;
      }
      setPending(null);
      next();
    },
    cancel() {
      setPending(null);
    },
    pending: () => job !== null,
    dispose() {
      if (wheelTimer !== null) timers.clear(wheelTimer);
      if (dog !== null) timers.clear(dog);
      wheelTimer = dog = null;
      setPending(null);
    },
  };
}
