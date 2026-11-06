// When the Map has settled: its camera framed and no tile animation in flight, decided apart from the renderer so every case is tested without a canvas.

/** Consecutive animation frames the viewport must hold before the camera counts as framed. */
export const CALM_FRAMES = 2;

/** What the Map knows about its camera at one animation frame. */
export interface FrameSample {
  /** Whether a layout, a draw or a camera intent is still in flight (see `intentsPending`). */
  busy: boolean;
  /** Tile animations in flight, from the Map's one `animating` source (see PlanMap). */
  animating: number;
  /** The viewport, zoom and pan, as a string to compare. */
  viewport: string;
}

/** The viewport last seen and for how many frames it has held. */
export interface FrameWatch {
  viewport: string | null;
  calm: number;
}

export const FRESH_WATCH: FrameWatch = { viewport: null, calm: 0 };

/**
 * One animation frame's step. The Map has settled when nothing is in flight, no tile is animating, and the viewport
 * has been the same for `CALM_FRAMES` consecutive frames; anything in flight, or a moved viewport, starts again.
 */
export function stepFrame(watch: FrameWatch, now: FrameSample): FrameWatch & { settled: boolean } {
  const calm = !now.busy && now.animating === 0 && now.viewport === watch.viewport ? watch.calm + 1 : 0;
  return { viewport: now.viewport, calm, settled: calm >= CALM_FRAMES };
}

/** The camera intents the Map may hold. */
export interface Intents {
  /** An item to bring into view, or null. */
  shown: object | null;
  /**
   * The item last found unable to be brought into view: the part of the Map nothing covers is empty, or the item is not
   * drawn. It waits for the cover to lift, but a camera that can never move for it is as framed as it will be.
   */
  parked: object | null;
  /** A box to hold in place across a layout. */
  anchor: boolean;
  /** A thread to frame that this Map is showing, not one it will be asked for later. */
  toFrame: boolean;
  /** Taps whose late reveal has not yet run. */
  tapReveals: number;
}

/** Whether a camera intent can still move the camera: a parked reveal cannot, so a covered Map still frames. */
export function intentsPending(intents: Intents): boolean {
  return (
    (intents.shown !== null && intents.shown !== intents.parked) ||
    intents.anchor ||
    intents.toFrame ||
    intents.tapReveals > 0
  );
}
