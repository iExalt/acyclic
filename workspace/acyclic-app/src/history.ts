// Cursor metadata for the browser history. A short parent list knows only the suffix of layer depths it covers.

import { openLayers, type Place, placeIn } from "./nav";

export interface Cursor {
  position: number;
  layers: number;
  parents: number[];
}

export interface ReadHistory {
  cursor: Cursor;
  place: Place | null;
}

export type Close = { kind: "go"; offset: number } | { kind: "replace"; cursor: Cursor };

const MAX_LAYERS = openLayers({
  command: true,
  mode: true,
  adding: true,
  projects: true,
  item: true,
  thread: true,
  under: true,
  away: true,
}).length;

const plainObject = (value: unknown): value is Record<string, unknown> => {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  try {
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
  } catch {
    return false;
  }
};

function cursorIn(value: unknown): Cursor | null {
  if (!plainObject(value)) return null;
  const { position, layers, parents } = value;
  if (
    !Number.isSafeInteger(position) ||
    (position as number) < 0 ||
    !Number.isSafeInteger(layers) ||
    (layers as number) < 0 ||
    (layers as number) > MAX_LAYERS ||
    !Array.isArray(parents)
  )
    return null;
  const copiedParents: unknown[] = [...parents];
  if (copiedParents.length > (layers as number)) return null;
  let previous = -1;
  for (const parent of copiedParents) {
    const parentPosition = parent as number;
    if (
      !Number.isSafeInteger(parentPosition) ||
      parentPosition < 0 ||
      parentPosition >= (position as number) ||
      parentPosition <= previous
    )
      return null;
    previous = parentPosition;
  }
  return { position: position as number, layers: layers as number, parents: copiedParents as number[] };
}

export function readHistory(raw: unknown): ReadHistory | null {
  const cursor = cursorIn(raw);
  if (cursor === null || !plainObject(raw)) return null;
  let place: Place | null = null;
  try {
    place = placeIn(raw);
  } catch {
    // A malformed place falls back to the address; it does not invalidate cursor metadata.
  }
  return { cursor, place };
}

export function boundary(layers: number): Cursor | null {
  if (!Number.isSafeInteger(layers) || layers < 0 || layers > MAX_LAYERS) return null;
  return { position: 0, layers, parents: [] };
}

export function visit(cursor: Cursor): Cursor | null {
  const valid = cursorIn(cursor);
  if (valid === null || !Number.isSafeInteger(valid.position + 1)) return null;
  return { ...valid, position: valid.position + 1 };
}

/** One cursor per newly opened layer, each recording the position of the layer beneath it. */
export function pushDepths(cursor: Cursor, toLayers: number): Cursor[] | null {
  let current = cursorIn(cursor);
  if (current === null || !Number.isSafeInteger(toLayers) || toLayers < current.layers || toLayers > MAX_LAYERS)
    return null;
  const pushed: Cursor[] = [];
  while (current.layers < toLayers) {
    const position: number = current.position + 1;
    if (!Number.isSafeInteger(position)) return null;
    current = { position, layers: current.layers + 1, parents: [...current.parents, current.position] };
    pushed.push(current);
  }
  return pushed;
}

/** Keeps this position while closing layers, retaining only the known parents that remain below it. */
export function replaceDepth(cursor: Cursor, toLayers: number): Cursor | null {
  const valid = cursorIn(cursor);
  if (valid === null || !Number.isSafeInteger(toLayers) || toLayers < 0 || toLayers > valid.layers) return null;
  const knownFrom = valid.layers - valid.parents.length;
  const keep = Math.max(0, Math.min(valid.parents.length, toLayers - knownFrom));
  return { position: valid.position, layers: toLayers, parents: valid.parents.slice(0, keep) };
}

/** Goes to a known parent only when its position is in this history; otherwise closes without waiting for popstate. */
export function close(cursor: Cursor, toLayers: number, historyLength: number): Close | null {
  const valid = cursorIn(cursor);
  if (
    valid === null ||
    !Number.isSafeInteger(toLayers) ||
    toLayers < 0 ||
    toLayers > valid.layers ||
    !Number.isSafeInteger(historyLength) ||
    historyLength < 0
  )
    return null;
  const knownFrom = valid.layers - valid.parents.length;
  const parent = toLayers >= knownFrom && toLayers < valid.layers ? valid.parents[toLayers - knownFrom] : undefined;
  const offset = parent === undefined ? 0 : parent - valid.position;
  if (parent !== undefined && Number.isSafeInteger(offset) && offset < 0 && -offset < historyLength)
    return { kind: "go", offset };
  const replaced = replaceDepth(valid, toLayers);
  return replaced === null ? null : { kind: "replace", cursor: replaced };
}
