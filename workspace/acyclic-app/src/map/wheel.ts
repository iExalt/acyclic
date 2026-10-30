// The Map's own wheel zoom. Cytoscape's zooms about 3% per notch of a mouse wheel, which crawls on a laptop; this one
// zooms 12% per notch. A trackpad's two-finger scroll and its pinch, which browsers send as a wheel with ctrlKey held,
// zoom as Cytoscape's always did.

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
/**
 * The least a mouse wheel's notch scrolls, in whole pixels: Chrome sends 80 a notch on a Mac and 100 or 120 elsewhere,
 * while a trackpad sends a stream of a few pixels at a time.
 */
const NOTCH = 40;
/** Zoom per pixel of a notched wheel: 12% for a Mac's notch of 80 pixels, a little more for the larger notches. */
const NOTCH_GAIN = Math.log(1.12) / 80;
/** Zoom per pixel of a trackpad's scroll or pinch: Cytoscape's, a tenfold zoom per 250 pixels. */
const SMOOTH_GAIN = Math.log(10) / 250;

/** The zoom after `wheel`: scrolling down or pinching in zooms out, within `min` and `max`. */
export function wheelZoom(zoom: number, wheel: Wheel, min: number, max: number, page: number): number {
  const pixels = wheel.deltaY * (wheel.deltaMode === 1 ? LINE : wheel.deltaMode === 2 ? page : 1);
  const notched = !wheel.ctrlKey && (wheel.deltaMode !== 0 || (Number.isInteger(pixels) && Math.abs(pixels) >= NOTCH));
  const next = zoom * Math.exp(-pixels * (notched ? NOTCH_GAIN : SMOOTH_GAIN));
  return Math.min(max, Math.max(min, next));
}
