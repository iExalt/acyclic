// The Map's own wheel zoom. Cytoscape's zooms about 3% per notch of a mouse wheel, which crawls on a laptop; this one
// zooms by the distance scrolled, so a notched wheel and an accelerating one feel alike, and a trackpad's pinch,
// which browsers send as a wheel with ctrlKey held, has a gain of its own.

/** What zooming needs of a wheel event. */
export interface Wheel {
  deltaY: number;
  /** 0 for pixels, 1 for lines, 2 for pages, as WheelEvent has it. */
  deltaMode: number;
  /** Held for a trackpad's pinch. */
  ctrlKey: boolean;
}

/** A line of scrolling, in pixels, for wheels that count lines. */
const LINE = 16;
/** Zoom per pixel scrolled: 12% per 100 pixels, one notch of a mouse wheel in Chrome. */
const SCROLL_GAIN = Math.log(1.12) / 100;
/** Zoom per pixel of a pinch, whose deltas are a few pixels each. */
const PINCH_GAIN = 0.01;

/** The zoom after `wheel`: scrolling down or pinching in zooms out, within `min` and `max`. */
export function wheelZoom(zoom: number, wheel: Wheel, min: number, max: number, page: number): number {
  const pixels = wheel.deltaY * (wheel.deltaMode === 1 ? LINE : wheel.deltaMode === 2 ? page : 1);
  const next = zoom * Math.exp(-pixels * (wheel.ctrlKey ? PINCH_GAIN : SCROLL_GAIN));
  return Math.min(max, Math.max(min, next));
}
