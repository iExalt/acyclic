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

export type HistoryHint = "ordinary" | "visit" | "replaceTransient";

export type HistoryEffect = { kind: "replace" | "push"; cursor: Cursor; place: Place } | { kind: "go"; offset: number };

export interface HistoryPlan {
  cursor: Cursor;
  effects: HistoryEffect[];
}

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

function placesIn(places: readonly Place[], depth: number): Place[] | null {
  if (!Array.isArray(places) || places.length !== depth + 1) return null;
  const canonical: Place[] = [];
  for (const place of places) {
    try {
      const stored = placeIn({ place });
      if (stored === null) return null;
      canonical.push(stored);
    } catch {
      return null;
    }
  }
  return canonical;
}

function sourcePlacesIn(places: readonly Place[], layers: number): Place[] | null {
  if (!Array.isArray(places) || places.length === 0) return null;
  const sourceDepth = places.length - 1;
  if (sourceDepth > layers || boundary(sourceDepth) === null) return null;
  const canonical = placesIn(places, sourceDepth);
  const last = canonical?.at(-1);
  if (canonical === null || last === undefined) return null;
  while (canonical.length <= layers) canonical.push(last);
  return canonical;
}

function sameCameFrom(a: Place["cameFrom"], b: Place["cameFrom"]): boolean {
  return a === b || (a !== null && b !== null && a.view === b.view && a.by === b.by && a.item === b.item);
}

/** The context that must remain reachable; the list's scroll/focus snapshot is intentionally excluded. */
function sameContext(a: Place, b: Place): boolean {
  return (
    a.view === b.view &&
    a.item === b.item &&
    a.thread === b.thread &&
    sameCameFrom(a.cameFrom, b.cameFrom) &&
    (a.view !== "needs" || a.needsAt === b.needsAt)
  );
}

function firstContextMismatch(
  cursor: Cursor,
  source: readonly Place[],
  destination: readonly Place[],
  toLayers: number,
): number | null {
  const end = Math.min(cursor.layers, toLayers);
  for (let depth = cursor.layers - cursor.parents.length; depth < end; depth++) {
    const from = source[depth];
    const to = destination[depth];
    if (from !== undefined && to !== undefined && !sameContext(from, to)) return depth;
  }
  return null;
}

function writes(cursor: Cursor, toLayers: number, places: readonly Place[]): HistoryPlan | null {
  const pushed = pushDepths(cursor, toLayers);
  if (pushed === null) return null;
  const currentPlace = places[cursor.layers];
  if (currentPlace === undefined) return null;
  const effects: HistoryEffect[] = [{ kind: "replace", cursor, place: currentPlace }];
  for (const next of pushed) {
    const place = places[next.layers];
    if (place === undefined) return null;
    effects.push({ kind: "push", cursor: next, place });
  }
  return { cursor: pushed.at(-1) ?? cursor, effects };
}

/** Plans the minimal history writes for a landed place; browser effects stay with the caller. */
export function plan(
  cursor: Cursor,
  projectedPlaces: readonly Place[],
  hint: HistoryHint,
  historyLength: number,
  sourceProjectedPlaces?: readonly Place[],
): HistoryPlan | null {
  const current = cursorIn(cursor);
  if (current === null || !Number.isSafeInteger(historyLength) || historyLength < 0) return null;
  if (hint !== "ordinary" && hint !== "visit" && hint !== "replaceTransient") return null;
  const toLayers = projectedPlaces.length - 1;
  if (boundary(toLayers) === null) return null;
  const places = placesIn(projectedPlaces, toLayers);
  if (places === null) return null;
  const source =
    sourceProjectedPlaces === undefined ? undefined : sourcePlacesIn(sourceProjectedPlaces, current.layers);
  if (source === null) return null;

  if (hint !== "visit" && source !== undefined) {
    const mismatch = firstContextMismatch(current, source, places, toLayers);
    if (mismatch !== null) {
      const reset = close(current, Math.max(0, mismatch - 1), historyLength);
      if (reset === null) return null;
      return reset.kind === "go" ? { cursor: current, effects: [reset] } : writes(reset.cursor, toLayers, places);
    }
  }

  if (hint === "visit" && toLayers <= current.layers) {
    const replaced = replaceDepth(current, toLayers);
    const branched = replaced === null ? null : visit(replaced);
    const place = places[toLayers];
    return branched === null || place === undefined
      ? null
      : { cursor: branched, effects: [{ kind: "push", cursor: branched, place }] };
  }

  if (hint === "replaceTransient") {
    const base = replaceDepth(current, Math.min(current.layers, toLayers));
    return base === null ? null : writes(base, toLayers, places);
  }

  if (toLayers < current.layers) {
    const action = close(current, toLayers, historyLength);
    if (action === null) return null;
    if (action.kind === "go") return { cursor: current, effects: [action] };
    const place = places[toLayers];
    return place === undefined
      ? null
      : { cursor: action.cursor, effects: [{ kind: "replace", cursor: action.cursor, place }] };
  }

  return writes(current, toLayers, places);
}
