# Design

The app's agreed design: what later changes keep, and why. Values live in the code (`src/palette.ts`, `src/app.css`);
this names them and says where they apply. Update it in the same change as the code when a convention changes.

## Users

One person, the plan's owner, who hands work to agents. They check in on a phone between other things, and sit with
the plan on a desktop when they reshape it. Most visits are short: see what needs them, decide, leave.

## Primary tasks

1. Settle what needs them: approve a thread, keep or drop an agent's proposal, unblock an item.
2. See where work stands: which threads are at work, which to start next, how each phase is doing.
3. Reshape the plan: add, move, link and edit items on the Map.

## Product principles

- Task efficiency over novelty: familiar controls, few surprises.
- Every decision is the user's to make once, in one place, with one primary action.
- The plan's state reads as text; colour only ever repeats it.

## Accessibility & inclusion

WCAG 2.2 AA in both schemes: text at 4.5:1 on every surface it sits on, which `palette.test.ts` holds the palettes
to. Touch targets are 44 px on a coarse pointer and 34 px on a fine one (`--hit`). Everything is reachable by keyboard;
focus is always visible. Reduced motion turns the sheets' rise off.

## Overview

A quiet tool (Operate mode throughout), in the manner of Linear: neutral surfaces, ink rather than a brand colour as
the accent, so colour is left to the threads' lines and the signal. The scheme follows the system's, light or dark.

## Colors

`PALETTES` in `src/palette.ts` is the source; `app.css` repeats it as custom properties under `/* palette <scheme> */`,
and a unit test keeps the two in step.

- **Surfaces:** `ground` (the page and the Map), `chrome` (sidebar and bars), `sheet` (panels), `tile` and `box` (the
  Map's items and boxes).
- **Ink:** `ink` for text, `muted` for secondary text, `rule` between things, `edge` around controls.
- **Accent:** the same as `ink`. It fills the primary action and draws the focus ring.
- **Signal:** amber, as the "!" badge's fill; as text, a dark brown in the light scheme that reads at 4.5:1. What
  needs the user, conflicts, and links that cross boxes.
- **Link:** the Map's links, quieter than text so the items lead, at 3:1 on the ground and boxes. A dimmed link, outside
  a lit thread, falls below that on purpose: it is there to recede.
- **Danger:** red, as text only.
- **Lines:** each thread's colour, from an OKLCH lightness band per scheme, so every line's badge takes a readable ink.

## Typography

Overpass, variable, for the app and the Map's canvas. 15 px base, 14 px from 900 px wide. Sentence case. Tabular
figures where counts are compared. Monospace only for ids.

## Layout

- **Phone (under 900 px):** a bar on top (the view's name, the outline, New item, the menu), the views and Needs you
  in a tab bar along the bottom, panels as sheets rising from the bottom.
- **Wide:** a sidebar (the views, Needs you, New item, the outline of phases and threads, the menu at its foot); panels
  open on the right.

## Shapes

One radius, 8 px (`--radius`), for controls, cards and panels. Sheets on a phone round their top corners, by 14 px.

## Components

- **Buttons, by tier:** `.primary` filled with the accent; secondary outlined; `.quiet` text-only; `.danger-quiet`
  red text, last in its group.
- **Stage marks:** each stage's icon beside its word (`Stage.tsx`); counts always as text beside their icons.
- **Thread badges:** the thread's number on its line's colour.
- **Needs-you mark:** a "!" on the amber fill.
- **Notices:** a refusal, a failure or something to copy, with its own Dismiss; in the panel on top, under its heading,
  else over the Map.
- **Command menu:** a field over a list, the chosen row filled with a bar at its start; Close at the field's end.
- **Lists:** rows a finger's height, ruled apart, each opening what it names, a chevron at its end.

## Interaction conventions

- **Views are places, Needs you is a layer.** Overview, Map and Threads are views; the one shown carries
  `aria-current="page"` and is filled. Needs you opens over any view as a toggle (`aria-pressed`), marked by ink alone.
- **Home is the Overview.** On a phone, Back from another view returns there before it leaves the app.
- **Layers stack in one order** (`src/layers.ts`), with one history entry each, so Escape and Back close the top one.
- **A form never loses typing:** Escape and Back on a changed form say how to leave it instead.
- **Everything open has an address.** The address bar always says what's open (`?view=`, `?item=`, `?thread=`,
  `?needs=<entry>`), so a reload, a copied link or a notification opens it again. A link opens over home, one history
  entry per layer, so Back closes what it opened, then leaves. A link to something gone says so; one to an archived
  item shows archived items, with Hide archived. An item's and a thread's panels carry Copy link, which ticks in
  place. A device not yet paired loses the address through pairing.
- **One field reaches anything.** Search (⌘K, Ctrl+K elsewhere; the sidebar's Search or jump to…; the phone bar's
  Search) opens the command menu over everything: views, Needs you, New item, the archived switch, Refresh, and every
  thread, phase and item by title or id, grouped, the best first. While a move or link waits for its target, it finds
  the target instead. It leaves what's under it as it is, never closes a form holding typing, and gives the focus back
  where it was.
- **Keys, in Linear's habits.** G then O, M, T or N goes to the Overview, the Map, Threads or Needs you; C adds; / and
  ⌘K search; ? lists the keys in the command menu, each runnable. With an item's panel shown, E edits, M moves, W and R
  link, A adds inside; a move or link from the keyboard finds its target in the menu. J and K move through the
  Overview's rows and Needs you's entries, an entry being no control, so Enter on it does nothing. No single key
  approves, keeps, drops or deletes. Keys go to fields, input methods and changed forms first, and the menu's
  Single-key shortcuts switch, per device, turns them off.
- **A notification never reloads the app:** the open app is asked to show what it names, over the view shown, and
  waits, saying so, while a form holds what the user typed.
- **Opening from a row lands on its entry:** a Needs you row on the Overview opens the list there, the entry focused
  and none of its actions.
- **States:** loading, a failed load (what to check, then the error, then Try again), and an empty plan are each a
  page of their own.

## Voice & copy

Plain verbs on actions ("Approve 3 items", "Add item"). Names follow `GLOSSARY.md`: item, thread, phase, wave, Needs
you. Errors say what to check before what went wrong.

## Do's and Don'ts

- **The One Primary Rule.** One filled button per decision; everything else steps down a tier.
- **The Shape Rule.** Selection is drawn by shape (an ink border and outline, a fill, a pill), never by hue alone.
- **The Signal Rule.** The signal is text, dashes and the "!" fill. Never a solid border, and never a badge with a
  number, which a thread's badge could be mistaken for.
- **The Text Rule.** Stage counts are always written out; icons only accompany them.
- **The List Rule.** The Overview is lists to scan, never tiles or charts.

## The Map

- **Top to bottom.** Inside a box, items run down in layers by their links; a long chain stays one column, never wraps.
- **No link hidden without cause.** A link is left out only when a longer way through items in the same box already
  implies it; selecting an item, or lighting its thread, shows every link it has.
- **Out as one trunk, in one by one.** A node's outgoing links leave as one trunk that splits, marked by a dot; each
  arrives at its own point with its own arrowhead. Sharing both ends would let an outgoing trunk and an incoming one
  run as one line, and a link would seem to join items it doesn't. A dot marks only where a trunk splits; links that
  cross have none.
- **Opening view.** The whole plan when it fits at a readable zoom (`READABLE`); otherwise the first box's corner, its
  title and first item whole, at that zoom.

## Open decisions

None.

## Decision log

- 2026-10-31: the quiet look over a familiar, header-and-tabs one; the user preferred it and wants more from Linear.
- 2026-10-31: the Overview as home over the Map; the user preferred it.
- 2026-10-31: Needs you open is marked lighter than the view shown, so only one place reads as current.
- 2026-10-31: the scheme follows the system's.
- 2026-10-31: the Map's links bundled on the way out, in one column per chain; the user found it cleanest and liked it
  vertical, and showed that bundling both ends made links ambiguous. Wrapping long chains into columns is a non-goal.
