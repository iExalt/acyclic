// The app's colours in each scheme, light and dark, which follow the system's. app.css repeats them as custom
// properties for the page; the Map's canvas, which custom properties can't reach, draws from these, and redraws when
// the scheme changes.

import { useSyncExternalStore } from "react";
import type { Stage } from "./api";
import type { Look } from "./look";

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
  ink: string;
  muted: string;
  /**
   * What needs the user, conflicts, and links that cross boxes. Lines may share its hue, so it is never a cue by colour
   * alone: crossing links are dashed, conflicts have a dashed border, and the needs-you badge reads "!", which no
   * thread's badge does. So never draw it as a solid border or a badge with a number. Also set as text, so it reads at
   * 4.5:1 like any.
   */
  signal: string;
  /** The needs-you badge's fill, amber in either scheme, and the "!" on it, which carries its meaning. */
  signalFill: string;
  signalInk: string;
  danger: string;
  /** The primary action's fill, the current view and the focus ring; and the text on that fill. */
  accent: string;
  accentInk: string;
  /** Each stage's tint behind a task's title on the Map. */
  tints: Record<Stage, string>;
  /** How light threads' lines are, in OKLCH, darkest and lightest: each scheme's ground needs its own. */
  band: readonly [number, number];
  /** The two inks a line's badge may take, whichever reads better on the line. */
  badgeInks: readonly [string, string];
}

const DARK_GROUND = "#121417";

/** Each look's palettes: A quiet, with ink as its accent; B familiar, with a blue one. */
export const PALETTES: Record<Look, Record<Scheme, Palette>> = {
  a: {
    dark: {
      scheme: "dark",
      ground: DARK_GROUND,
      chrome: "#16191d",
      box: "#181b1f",
      tile: "#23282e",
      sheet: "#1c2025",
      rule: "#2b3138",
      edge: "#4a535e",
      ink: "#e8eaed",
      muted: "#a2aab4",
      signal: "#f2b33d",
      signalFill: "#f2b33d",
      signalInk: DARK_GROUND,
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
      ink: "#1a1c20",
      muted: "#5b626c",
      signal: "#8f5100",
      signalFill: "#f2b33d",
      signalInk: DARK_GROUND,
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
  },
  b: {
    dark: {
      scheme: "dark",
      ground: "#0f1318",
      chrome: "#161b22",
      box: "#13181e",
      tile: "#1c2229",
      sheet: "#161b22",
      rule: "#2d343d",
      edge: "#454e59",
      ink: "#e6eaee",
      muted: "#9aa3ad",
      signal: "#e3a72f",
      signalFill: "#e3a72f",
      signalInk: "#0f1318",
      danger: "#ff7b72",
      accent: "#4c94f0",
      accentInk: "#0f1318",
      tints: {
        done: "#161b21",
        "in-review": "#222b3d",
        "in-progress": "#24323f",
        ready: "#1c2229",
        blocked: "#12161b",
        parked: "#1f2125",
        unthreaded: "#1c2229",
      },
      band: [0.7, 0.86],
      badgeInks: ["#0f1318", "#ffffff"],
    },
    light: {
      scheme: "light",
      ground: "#f6f8fa",
      chrome: "#ffffff",
      box: "#ffffff",
      tile: "#ffffff",
      sheet: "#ffffff",
      rule: "#d1d8df",
      edge: "#b4bcc5",
      ink: "#1f2329",
      muted: "#57606a",
      signal: "#8c5a00",
      signalFill: "#e3a72f",
      signalInk: "#0f1318",
      danger: "#c62a2f",
      accent: "#0a66d8",
      accentInk: "#ffffff",
      tints: {
        done: "#f3f5f7",
        "in-review": "#ebf0fa",
        "in-progress": "#e0ebf5",
        ready: "#ffffff",
        blocked: "#f0f2f4",
        parked: "#eeeff1",
        unthreaded: "#ffffff",
      },
      band: [0.44, 0.56],
      badgeInks: ["#0f1318", "#ffffff"],
    },
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
