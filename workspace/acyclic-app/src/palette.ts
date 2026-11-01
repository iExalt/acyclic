// The app's colours in each scheme, light and dark, which follow the system's. app.css repeats them as custom
// properties for the page; the Map's canvas, which custom properties can't reach, draws from these, and redraws when
// the scheme changes.

import { useSyncExternalStore } from "react";
import type { Stage } from "./api";

export type Scheme = "dark" | "light";

export interface Palette {
  scheme: Scheme;
  /** The Map's surface, and the page behind everything. */
  ground: string;
  /** The bar and the outline. */
  chrome: string;
  /** A box on the Map. */
  box: string;
  /** An item on the Map. */
  tile: string;
  /** A sheet or pane over the Map. */
  sheet: string;
  rule: string;
  /** A control's edge: stronger than a rule, so an outlined button reads as one. */
  edge: string;
  /** A field's edge, which alone marks it out: at 3:1 on the surfaces a field sits on and the ground it is filled with. */
  field: string;
  ink: string;
  muted: string;
  /** A link on the Map: quieter than text, so the items lead, but at 3:1 on the ground and boxes it is drawn over. */
  link: string;
  /**
   * Conflicts, links that cross boxes, and a thread waiting for approval. Lines may share its hue, so it is never a cue
   * by colour alone: crossing links are dashed, conflicts have a dashed border, a waiting thread says so. So never draw
   * it as a solid border or a badge with a number. Also set as text, so it reads at 4.5:1 like any.
   */
  signal: string;
  danger: string;
  /** The primary action's fill, the current view and the focus ring; and the text on that fill. */
  accent: string;
  accentInk: string;
  /** Each stage's tint behind an item's title on the Map. */
  tints: Record<Stage, string>;
  /** How light threads' lines are, in OKLCH, darkest and lightest: each scheme's ground needs its own. */
  band: readonly [number, number];
  /** The two inks a line's badge may take, whichever reads better on the line. */
  badgeInks: readonly [string, string];
}

const DARK_GROUND = "#121417";

/** The palettes, quiet: ink is the accent, so colour is left to the threads and the signal. */
export const PALETTES: Record<Scheme, Palette> = {
  dark: {
    scheme: "dark",
    ground: DARK_GROUND,
    chrome: "#16191d",
    box: "#181b1f",
    tile: "#23282e",
    sheet: "#1c2025",
    rule: "#2b3138",
    edge: "#4a535e",
    field: "#6c7580",
    ink: "#e8eaed",
    muted: "#a2aab4",
    link: "#6c7580",
    signal: "#f2b33d",
    danger: "#ff8a80",
    accent: "#e8eaed",
    accentInk: DARK_GROUND,
    tints: {
      done: "#1a1e22",
      "in-review": "#283042",
      "in-progress": "#2f3d48",
      ready: "#23282e",
      blocked: "#16191c",
      parked: "#25262a",
      unthreaded: "#23282e",
    },
    band: [0.7, 0.86],
    badgeInks: [DARK_GROUND, "#ffffff"],
  },
  light: {
    scheme: "light",
    ground: "#f3f4f6",
    chrome: "#eceef1",
    box: "#f9fafb",
    tile: "#ffffff",
    sheet: "#ffffff",
    rule: "#d0d5db",
    edge: "#aab0b9",
    field: "#80878f",
    ink: "#1a1c20",
    muted: "#5b626c",
    link: "#878e98",
    signal: "#8f5100",
    danger: "#b3261e",
    accent: "#1a1c20",
    accentInk: "#ffffff",
    tints: {
      done: "#f1f2f4",
      "in-review": "#e7ecf7",
      "in-progress": "#dbe5ee",
      ready: "#ffffff",
      blocked: "#eef0f2",
      parked: "#ececee",
      unthreaded: "#ffffff",
    },
    band: [0.44, 0.56],
    badgeInks: [DARK_GROUND, "#ffffff"],
  },
};

const QUERY = "(prefers-color-scheme: light)";

function subscribe(changed: () => void) {
  const query = window.matchMedia(QUERY);
  query.addEventListener("change", changed);
  return () => query.removeEventListener("change", changed);
}

/** The system's scheme, following it as it changes. */
export function useScheme(): Scheme {
  return useSyncExternalStore(
    subscribe,
    () => (window.matchMedia(QUERY).matches ? "light" : "dark"),
    () => "dark",
  );
}
