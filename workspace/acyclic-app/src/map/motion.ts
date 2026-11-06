// What the Map is still doing after a write: node tweens running, links fading back in, and a redraw held back while
// the user pans or zooms. One counter, for whoever wants to know when the Map has come to rest.
//
// API for readers:
//   busy()          how many of those are going on now; 0 means the Map is at rest.
//   subscribe(cb)   calls cb after busy() changes, once per tick however many changes it saw; returns a function that
//                   stops the calls.
// `track` is for the Map's own code to count one piece of motion until it calls the function it returns.

let running = 0;
const listeners = new Set<() => void>();
let queued = false;

/** Tells the listeners once per tick, however many pieces of motion began or ended in it. */
function changed() {
  if (queued) return;
  queued = true;
  queueMicrotask(() => {
    queued = false;
    for (const listener of [...listeners]) listener();
  });
}

/** How many tweens, fades and held-back redraws are going on now; 0 when the Map is at rest. */
export function busy(): number {
  return running;
}

/** Calls `listener` after `busy()` changes, once per tick. Returns the function that removes it. */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** @internal Counts one piece of motion until the returned function is called; calling it again does nothing. */
export function track(): () => void {
  let live = true;
  running++;
  changed();
  return () => {
    if (!live) return;
    live = false;
    running--;
    changed();
  };
}
