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
- **Ink:** `ink` for text, `muted` for secondary text, `rule` between things, `edge` around controls, and `field`
  around a field, which only its edge marks out, so at 3:1 on what it sits on.
- **Accent:** the same as `ink`. It fills the primary action and draws the focus ring.
- **Signal:** amber, as the "!" badge's fill; as text, a dark brown in the light scheme that reads at 4.5:1. What
  needs the user, conflicts, and links that cross boxes.
- **Link:** the Map's links, quieter than text so the items lead, at 3:1 on the ground and boxes. A dimmed link, outside
  a lit thread, falls below that on purpose: it is there to recede. A dimmed task fades its fill and marks but keeps its
  title muted at full strength, since it can still be picked; its badge and stage icon fade with it, below 3:1, on
  purpose, as the title carries it.
- **Danger:** red, as text only.
- **Lines:** each thread's colour, from an OKLCH lightness band per scheme, so every line's badge takes a readable ink.

## Typography

Overpass, variable, for the app and the Map's canvas. 15 px base; wide, the sidebar a step smaller, at 14 px.
Sentence case. Tabular figures where counts are compared. Monospace only for ids.

Text sets at 500, not the face's 400, and every heavier weight a step up with it. The Map's task titles stay at 500,
as the text does, and its boxes' at 700: its renderer takes only hundreds, and at 600 a phase's title barely led its
tasks. Overpass's regular strokes are thin on a screen, and the text strained to read
beside Linear's although its contrast was 14:1; at 500 it is as dark as Inter at 450. Inter, tried in its place, read
cold; Overpass, heavier, keeps the app's voice.

Panels, wide, set what they say in one size, 14 px, labels and values alike, their fields too; notes and briefs, the
reading, at 15 px; the title 19 px at 650, not bold; section headings 13 px at 600, muted, so the content leads.
(Mixed sizes and weights read as awkward; then 13 px read a smidge small beside Linear at the same window size, as
Overpass sets smaller than Inter at a size.)

## Layout

- **Phone (under 900 px):** a bar on top (the view's name over the autonomy level, the outline, New item, the menu),
  the views and Needs you in a tab bar along the bottom, panels as sheets rising from the bottom. The outline's
  button and sheet are both called Outline; its phases carry no count, as opening one shows its threads.
- **Sheets, on a phone:** two heights. Half, at least 320 px, so a small phone still shows a decision; full, up to the
  bar. The handle, a button in a strip at the top of the header, which stays in view as the sheet scrolls, switches
  them; it is 28 px tall, an exception to the 44 px targets, as a taller strip would push every panel down or lie over
  its buttons. A drag on it or on the header's free space follows the finger and lands by distance or by a flick,
  read from its last 100 ms, and down from half it closes the sheet as
  Escape does, so a form holding typing stays, saying how to leave it. Needs you opens full, a list of decisions;
  the other panels open at half. A tap on the Map that opens a panel over what it tapped pans the least that shows
  it, measured where the panel comes to rest. (At 45 % of a 360 × 640 phone, Needs you showed no decision at all.)
- **An upright tablet** keeps the phone's layout (the user's call over the wide one, which would leave the Map about
  210 px beside a panel), with In progress and Up next side by side from 700 px, and the phases in two columns.
- **Wide:** a sidebar (the views, Needs you, New item, the outline of phases and threads, the menu at its foot); panels
  open on the right.
- **The Overview, wide:** its lists side by side as the width allows (Needs you, In progress, Up next, each at least
  320 px), the phases across the page below them in columns of rows; an open panel keeps its width clear. (A single
  960 px column left most of a desktop empty.)
- **Up next** lists the threads that can start now: not at work, approved, with a task ready; Any time first, then by
  wave. One waiting for approval is in Needs you alone, and a wave all blocked has nothing to start. (The next wave,
  listed whole, showed only blocked threads while work elsewhere was ready; the user chose this.)

## Shapes

One radius, 8 px (`--radius`), for controls, cards and panels. Sheets on a phone round their top corners, by 14 px.

## Components

- **Buttons, by tier:** `.primary` filled with the accent; secondary outlined; `.quiet` text-only; `.danger-quiet`
  red text, last in its group.
- **Stage marks:** each stage's icon beside its word (`Stage.tsx`); counts always as text beside their icons. An item
  in no thread is never ready, so its stage reads "Needs a thread" ("3 need a thread"), its icon the dashed ring a task
  with no thread carries in its badge's place; on the Map the ring says it once, with no icon at the end. The editor
  says a state in words ("Backlog", "In progress"), Backlog with how it shows (ready or blocked, by its waits), and a
  finished item's state as the panel's stage, as text: finishing sets it, not the form.
- **A thread's status** follows Needs you: "Waiting for your approval" and a filled Approve only when Needs you asks
  for it. Unapproved with nothing open reads "Not approved, with nothing open", quiet, with an outlined Approve, as
  approval still covers what joins it later.
- **Thread badges:** the thread's number on its line's colour.
- **Notify this device:** in the menu, a setting of this device, read again each time the menu opens; not at the top
  of Needs you, where every visit passed it.
- **Key hints** (the ⋯ menu's letters, the sidebar's search chord, the command menu's keys) show only with a fine
  pointer or hover, where a keyboard is likely; the menu's switch for single-key shortcuts stays on every device.
- **Needs you:** the inbox icon and the count beside its name, in ink, as an inbox's; nothing when nothing waits. A
  need with no thread to badge it takes the inbox icon in its place. (An amber "!" read as a warning sign, out of
  place in the quiet look.)
- **Notices:** a refusal, a failure, an offered Undo or something to copy, with its own Dismiss; in the panel on top,
  under its heading, else over the Map. One shows at a time: the next replaces it, including one a link from a
  notification brings, as the user has moved on.
- **Item panel:** quiet, as Linear's. Properties as text beside muted labels, each a button that fills on hover.
  Edit (a pencil), the ⋯ menu, Copy link and Close beside the title; Move, Reopen, Add inside, Waits for and Relates
  to in the ⋯ menu, each with its key. Links as rows cut to a line, the full title on hover, a section only when it
  holds one; a row's remove button shows on hover or with focus in the row, and always where there may be no hover.
  (Chosen by the user over one row of quiet buttons and over today's layout unboxed, as the calmest.) While it edits,
  the form's State and Thread stand in for the Status and Thread rows, which would only repeat them; Parent and Id
  stay. The form's thread picker says each thread as "4. CLI features", as text names a thread without a badge.
- **Autonomy:** the level said the same way everywhere ("Autonomy: Blockers"), and under each list that sets it, what
  it lets reach the user, in core's words ("Agents ask only about blockers"), a thread without its own level adding
  "as set for every thread".
- **Needs you's entries:** the badge in a column of its own; beside it what the entry asks, then what it names as
  muted rows, then its decision, compact, after what it acts on. An approval names its thread alone, as its button
  and the list's filter say what it asks; the Overview, which shows no buttons, adds "4 items to approve".
  (Each row ending "waits for your approval" read as noise.) An approval lists the first five items it covers and
  says how many more, so its count and its list agree; an entry the app can only show says what failed and what fixes
  it. A hairline between entries; the filters are tabs, the one shown filled.
- **Hover:** quiet controls and rows fill with `--hover`, ink at 7 %, which shows on white sheets too. A view's row
  fills so too, lighter than the view shown, which keeps its fill under the pointer.
- **Notes and briefs:** a small, safe part of Markdown, built as elements: code, strong (which may hold code) and
  emphasis. Nothing else is read, and never as markup.
- **Command menu:** a field over a list, the chosen row filled with a bar at its start; Close at the field's end. A
  thread's row is its badge and name; the number is the badge's. A waiting move or link offers its targets with the
  phases in their own group, first before anything is typed, never the item itself, nor, for a move, where it is
  already or anything inside it.
- **Lists:** rows a finger's height, ruled apart, each opening what it names, a chevron at its end.

## Interaction conventions

- **Views are places, Needs you is a layer.** Overview, Map and Threads are views; the one shown carries
  `aria-current="page"` and is filled. Needs you opens over any view as a toggle (`aria-pressed`), marked by ink alone.
- **Home is the Overview.** On a phone, Back from another view returns there before it leaves the app.
- **Layers stack in one order** (`src/layers.ts`), with one history entry each, so Escape and Back close the top one.
  A thread the command menu opens while Needs you shows goes over it, where the user is looking.
- **An item or a thread opens on the Map and closes back.** One opened from the Overview, Threads or Needs you shows
  on the Map, and one Back, Escape or ✕ returns to the view it came from, Needs you still open if it was. Selecting on
  the Map or going to another view makes that where the user is; what a link opened closes onto the Map. While Needs
  you is open over the Overview, the Overview's own Needs you section steps aside rather than repeat it.
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
  ⌘K search; ? lists the keys in the command menu, each runnable, and under them as text the rest: an item's keys
  while none is open, J and K, search and undo, in the platform's own chords. With an item's panel shown, E edits, M
  moves, W and R link, A adds inside; a move or link from the keyboard finds its target in the menu. J and K move
  through the Overview's rows and Needs you's entries, an entry being no control, so Enter on it does nothing. No
  single key approves, keeps, drops or deletes. Keys go to fields, input methods and changed forms first, and the
  menu's Single-key shortcuts switch, per device, turns them off.
- **Undo, not confirm.** A move, an item's fields (but its thread), a link added or removed, and the global autonomy
  level offer Undo in their notice, and ⌘Z while it shows; nothing else does, as nothing else has an exact inverse
  (an approval, a proposal kept or dropped, a thread's own level or flag, a thread changed: agents act on them).
  Undo reads the plan first, and without that read it undoes nothing; it takes back nothing changed since, nor a
  removed wait or a state once the work it released has started, saying so. It says "Undoing…" until done, then
  what it did ("Autonomy set back to Blockers", not "Undone": agents' work meanwhile stands), a link said as it
  reads ("Sow now waits for Dig", "Sow no longer waits for Dig"), a move "Moved X back", fields "Set X back". The
  notice stays until dismissed or the next write.
- **Settling keeps the keyboard in place.** An action in Needs you or a thread's panel is held, not disabled, while
  its write is out, so focus stays on it; once what it settled goes, focus goes to the next entry's action, else to
  the panel. What a one-press decision did is said once saved ("Approved Codex: 2 items"), in its notice and to a
  screen reader politely.
- **A notification never reloads the app:** the open app is asked to show what it names, over the view shown, and
  waits, saying so, while a form holds what the user typed.
- **Opening from a row lands on its entry:** a Needs you row on the Overview opens the list there, the entry focused
  and none of its actions.
- **Going to a thread frames it.** A thread's row on the Overview or in the sidebar, or a link to a thread on the
  Map, goes to the Map with the thread picked and its panel open, its items fitted beside the panel (no closer than
  1:1, no further out than readable, else from its first item). The picked thread's row in the sidebar, on the Map,
  unpicks it. A card picked in the Threads view opens the thread's panel there.
- **One thing in focus.** Selecting an item outside the picked thread unpicks the thread, so the sidebar never marks
  a phase and a thread the selection isn't in at once; an item in the thread keeps it, its panel over the thread's.
- **States:** loading, a failed load (what to check, then the error, then Try again), and an empty plan are each a
  page of their own.

## Voice & copy

Plain verbs on actions ("Approve 3 items", "Add item"), and verbs any pointer fits: a move or link asks to "Choose"
its target, as a tap or a click both do. Names follow `GLOSSARY.md`: item, thread, phase, wave, Needs
you. Errors say what to check before what went wrong.

## Do's and Don'ts

- **The One Primary Rule.** One filled button per decision; everything else steps down a tier. In a list of
  decisions, with a pointer, an entry's main action is outlined until the entry is under the pointer or holds the
  focus: a column of identical filled buttons read as clutter. Without hover, they stay filled.
- **The Shape Rule.** Selection is drawn by shape (an ink edge with a soft halo, a fill, a pill), never by hue alone.
  One ring, not two: a double ring of ink read as heavy in both schemes.
- **The Signal Rule.** The signal is text and dashes. Never a solid border, a fill, or a badge with a number, which
  a thread's badge could be mistaken for.
- **The Text Rule.** Stage counts are always written out; icons only accompany them.
- **The List Rule.** The Overview is lists to scan, never tiles or charts.

## The Map

- **Top to bottom.** Inside a box, items run down in layers by their links; a long chain stays one column, never wraps.
- **A task reads as a sidebar row does:** its thread's badge first, after the stripe, then its title, then its
  stage's icon at the end.
- **A phase tells where it stands.** A box's title leads with the stage icon of its work as a whole (in progress while
  any of it moves, done once all of it is, else ready, else blocked), after its chevron, and is muted once done or
  held up, as a task's title is.
- **A join sits between what it waits on.** Inside a box, a task waiting on two sits centred under them, each pulling
  it equally (the balanced placement), not under whichever came first.
- **Hover shows a task's links**, as hovering a card on the Threads view does: its own, in ink, across boxes too, and
  the rest recede; the links a drawn way already implies stay out, since drawn straight they would cut across items.
  Touch has no hover; a tap selects, which shows them all.
- **Any time comes first.** Items with no link inside their box stand in columns on its left, before its graph, as
  the Threads view's Any time column stands before wave 1 (the user's call: what can start now reads first).
- **One reading order.** The top level is packed in rows, each box after those it waits on; the sidebar's and the
  Overview's phases come in that same order (`topOrder`), so a list and the Map never disagree.
- **A press leaves no mark.** Cytoscape's grey press disc and node shade are off: a tap selects, and the selection
  shows it.
- **No link hidden without cause.** A link is left out only when a longer way through items in the same box already
  implies it; selecting an item, or lighting its thread, shows every link it has.
- **Out as one trunk, in one by one.** A node's outgoing links leave as one trunk that splits, marked by a dot; each
  arrives at its own point with its own arrowhead. Sharing both ends would let an outgoing trunk and an incoming one
  run as one line, and a link would seem to join items it doesn't. A dot marks only where a trunk splits; links that
  cross have none.
- **Opening view.** The whole plan when it fits at a readable zoom (`READABLE`); otherwise the first box's corner, its
  title and first item whole, at that zoom.

## Open decisions

- Undo checks what a write left against a plan read just before it, in the app: a write landing between that read
  and the undo isn't caught. A compare-and-set on the server would close that window.

## Decision log

- 2026-10-31: the quiet look over a familiar, header-and-tabs one; the user preferred it and wants more from Linear.
- 2026-10-31: the Overview as home over the Map; the user preferred it.
- 2026-10-31: Needs you open is marked lighter than the view shown, so only one place reads as current.
- 2026-10-31: the scheme follows the system's.
- 2026-10-31: the Map's links bundled on the way out, in one column per chain; the user found it cleanest and liked it
  vertical, and showed that bundling both ends made links ambiguous. Wrapping long chains into columns is a non-goal.
- 2026-11-01: Up next lists the approved threads with a task ready, Any time first, then by wave, over the next wave
  whole; the user chose it.
- 2026-11-01: an item opened from another view shows on the Map, and Back returns to that view; the user chose it
  over opening items in place. A thread opened from another view does the same, by extension (proposed).
