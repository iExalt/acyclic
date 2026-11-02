# Design

The app's agreed design: what later changes keep, and why. Values live in the code (`src/palette.ts`, `src/app.css`);
this names them and says where they apply. Update it in the same change as the code when a convention changes.

## Users

One person, the plan's owner, who hands work to agents. They check in on a phone between other things, and sit with
the plan on a desktop when they reshape it. Most visits are short: see what needs them, decide, leave.

## Primary tasks

1. Settle what needs them: approve a thread, keep or drop an agent's proposal, resolve a conflict.
2. See where work stands: which threads are at work, which to start next, how each project is doing.
3. Reshape the plan: add, move, link and edit items on the Map.

## Product principles

- Task efficiency over novelty: familiar controls, few surprises.
- Every decision is the user's to make once, in one place, with one primary action.
- The plan's state reads as text; colour only ever repeats it.

## Accessibility & inclusion

WCAG 2.2 AA in both schemes: text at 4.5:1 on every surface it sits on, which `palette.test.ts` holds the palettes to.
Touch targets are 44 px on a coarse pointer. With a fine one, `--hit` gives default buttons and fields 34 px, and
compact rows and controls inside lists go down to 28 px (a Needs you item row to 24 px), as Linear's do. Everything is
reachable by keyboard; focus is always visible. Reduced motion turns the sheets' rise off.

## Overview

A quiet tool (Operate mode throughout), in the manner of Linear: neutral surfaces, ink rather than a brand colour as
the accent, so colour is left to the threads' lines and the signal. The scheme follows the system's, light or dark,
unless the menu's Theme picks one on the device (System, Light or Dark; System by default).

## Colors

`PALETTES` in `src/palette.ts` is the source; `app.css` repeats it as custom properties under `/* palettes */`, each
`light-dark(light, dark)`, so a Theme picked on the root (`data-theme`) needs no second copy, and a unit test keeps the
two in step.

- **Surfaces:** `ground` (the page and the Map), `chrome` (sidebar and bars), `sheet` (panels), `tile` and `box` (the
  Map's items and boxes). A tile in progress takes a cooler tint, one done, blocked or parked a greyer one; one in
  review keeps the tile's, its icon saying so, after a tint read as a selection.
- **Ink:** `ink` for text, `muted` for secondary text, `rule` between things, `edge` around controls, and `field`
  around a field, which only its edge marks out, so at 3:1 on what it sits on.
- **Accent:** the same as `ink`. It fills the primary action and draws the focus ring.
- **Signal:** amber in the dark scheme, a dark brown that reads at 4.5:1 in the light; text and dashes only. What
  needs the user, conflicts, threads that wait on each other, and links that cross boxes.
- **Link:** the Map's links, quieter than text so the items lead, at 3:1 on the ground and boxes. A dimmed link, outside
  a lit thread, falls below that on purpose: it is there to recede. A dimmed item fades its fill and marks but keeps its
  title muted at full strength, since it can still be picked; its badge and stage icon fade with it, below 3:1, on
  purpose, as the title carries it. Its fill stays opaque, the colour it would show at 35 % over its box, so a link
  across boxes passes beneath it.
- **Danger:** red, as text only.
- **Lines:** each thread's colour, from an Okhsl lightness band per scheme, every hue at the same share of the colour
  it can hold, so every line's badge takes a readable ink. Hues step round the wheel by number, a lap every 27
  threads, each lap a quarter of the band lighter: thread 4 and thread 31 share a hue and look nearly alike (ΔE about
  4), and the badge's number tells them apart; the user's call: "Don't change the hue step."

## Typography

Overpass, variable, for the app and the Map's canvas. 15 px base; wide, the sidebar a step smaller, at 14 px.
Sentence case. Tabular figures where counts are compared. Monospace only for ids.

Text sets at 500, not the face's 400, and every heavier weight a step up with it. The Map's leaves' titles stay at 500,
as the text does, and its boxes' at 700: its renderer takes only hundreds, and at 600 a project's title barely led its
items. Overpass's regular strokes are thin on a screen, and the text strained to read
beside Linear's although its contrast was 14:1; at 500 it is as dark as Inter at 450. Inter, tried in its place, read
cold; Overpass, heavier, keeps the app's voice.

Panels, wide, set what they say in one size, 14 px, labels and values alike, their fields too; notes and briefs, the
reading, at 15 px; the title 19 px at 650, not bold; section headings 13 px at 600, muted, so the content leads.
(Mixed sizes and weights read as awkward; then 13 px read a smidge small beside Linear at the same window size, as
Overpass sets smaller than Inter at a size.)

## Layout

- **Phone (under 900 px):** a bar on top (the view's name over the autonomy level, Search, the outline, New item, the
  menu), the views, Needs you among them, in a tab bar along the bottom, panels as sheets rising from the bottom. The
  outline's button and sheet are both called Outline; its projects carry no count, as opening one shows its threads.
- **Sheets, on a phone:** two heights. Half, at least 320 px, so a small phone still shows a decision; full, up to the
  bar. The handle, a button in a strip at the top of the header, which stays in view as the sheet scrolls, switches
  them; it is 28 px tall, an exception to the 44 px targets, as a taller strip would push every panel down or lie over
  its buttons. A drag on it or on the header's free space follows the finger and lands by distance or by a flick, read
  from its last 100 ms, and down from half it closes the sheet as Escape does, so a form holding typing stays, saying
  how to leave it. Panels open at half; a form (New item, an item's editor) takes the whole height, with no handle. A
  tap on the Map that opens a panel over what it tapped pans the least that shows it, measured where the panel comes to
  rest.
- **An upright tablet** keeps the phone's layout (the user's call over the wide one, which would leave the Map about 210
  px beside a panel), with the Overview's lists in two columns once it is 680 px wide (a 728 px screen), and the
  projects in two columns, as each wave's tiles in the Threads view are; New item opens from the top as tall as its
  fields, clear of the keyboard, the Map in view below it.
- **Wide:** a sidebar (the views, Needs you among them, New item, the outline of projects and threads, the menu at its
  foot); panels open on the right. All between the wordmark and the foot scrolls as one, and each outline row is a line,
  cut short with its full name on hover, as Linear's; a thread in more than one project adds a second line saying where
  else, which its hover says too. Its fold button, at its top, folds it to a 52 px rail of the controls' icons, Needs
  you's count on its icon, and unfolds it from the same place; each device remembers. (Rows wrapping to three lines, in
  a scroll area of their own, read as clutter.)
- **The Overview, wide:** Needs you beside In progress, with Up next under it, once the Overview is 680 px wide, a 976
  px window beside the sidebar (each at least 320 px, each up to eight rows; five for Needs you under 700 px, where they
  stack), the projects across the page below them in columns of rows; an open panel keeps its width clear. From 1,280 px
  with no panel open, Up next takes a third column of its own, the user's call: "allow three columns for the overview on
  wide screens"; an Up next at most half as long as the longest list beside it stays under In progress, and the projects
  take the third column instead, one row under another. (A single 960 px column left most of a desktop empty, and a
  one-row Up next column left a third of it empty, the projects below the first view.) Its Needs you section is a
  preview of the view: with more in Needs you than it shows, the link under its rows goes there ("19 more in Needs
  you"), and the header has none; the threads' say where theirs go too ("9 more in Threads").
- **Needs you's view** is a page over the Map, as the Overview is: wide, its entries stand in columns of at least 360
  px, as many as fit, the filters above them; on a phone, one column. (As a narrow pane on the right it left most of a
  desktop empty, and only the Overview made room for it.)
- **Up next** lists the threads that can start now: not at work, approved, with an item ready; Any time first, then by
  wave. One waiting for approval is in Needs you alone, and a wave all blocked has nothing to start. (The next wave,
  listed whole, showed only blocked threads while work elsewhere was ready; the user chose this.)

## Shapes

One radius, 8 px (`--radius`), for controls, tiles and panels. Smaller things take smaller ones: chips, link rows, the
remove-link button and the command menu's rows 6 px, key caps and code 4 px; the ⋯ menu's popover 12 px; panels beside
the Map, wide, are square. Sheets on a phone round their top corners, by 14 px.

## Components

- **Buttons, by tier:** `.primary` filled with the accent; secondary outlined; `.quiet` text-only; `.danger-quiet`
  red text, last in its group.
- **Stage marks:** each stage's icon beside its word (`Stage.tsx`); counts always as text beside their icons, each count
  set apart by its icon rather than a comma, which only a screen reader reads; a Map box's line of counts, drawn as text
  on the canvas, which sets no icon inside a line, takes the commas. An item in no thread is never ready, so its stage
  reads "Needs a thread" ("3 need a thread"), its icon the dashed ring an item with no thread carries in its badge's
  place; on the Map the ring says it once, with no icon at the end. The editor says a state in words ("Backlog", "In
  progress"), Backlog with how it shows (ready or blocked, by its waits), and a finished item's state as the panel's
  stage, as text: finishing sets it, not the form.
- **A thread's status** follows Needs you: "Waiting for your approval", beside the inbox icon Needs you has, and a
  filled Approve only when Needs you asks for it; until the thread is read again after a decision, it says what it said
  as it was read, not what the list says by then. Unapproved with nothing open reads "Not approved", quiet, with an
  outlined Approve, as approval still covers what joins it later. Under it, the thread's Items, in the order their work
  runs (each after what it waits for in the thread), each with its stage's icon, the first eight and then how many more;
  "Starts with", its first items alone, meant nothing to the user. Its settings, Autonomy and Scope edits, are rows as
  the item panel's properties are, label beside value, with what a value means on hover rather than a line under each.
- **Thread badges:** the thread's number on its line's colour. A badge, and a stage's icon, stand on the middle of the
  capitals beside them, a tenth of an em over the line's, since Overpass sits high (the user saw them sit low).
- **Settings change at once.** A setting, in the ⋯ menu or a thread's panel, shows its new value as it is picked and
  saves in the background, after any write already out, with no busy bar and nothing else held; a refusal puts the old
  value back and says why. A setting leaves the menu open; only its buttons close it. (A busy bar and greyed controls
  after each change read as "a brief loading screen - not good", and the menu closing on a checkbox as bad UX.)
- **Notify this device:** in the menu, a setting of this device, read again each time the menu opens; not at the top
  of Needs you, where every visit passed it.
- **Key hints** (the ⋯ menu's letters, the sidebar's search chord, the command menu's keys) show only where a mouse is
  the main pointer, or once a key from a physical keyboard has been pressed: a phone's stylus is a fine pointer but
  brings no keys. The menu's switch for single-key shortcuts stays on every device.
- **Needs you:** the inbox icon and the count beside its name, in ink, as an inbox's; nothing when nothing waits. A
  need with no thread to badge it takes the inbox icon in its place. (An amber "!" read as a warning sign, out of
  place in the quiet look.)
- **Notices stay with what they are about** (`src/notices.ts`). News of what just happened (saved, undone, a link
  opened, with any offer it makes, such as Hide archived, which the menu's switch also does) goes once the user moves; a
  refusal or failure stays on the surface that raised it until that surface closes; a form's warning stays with the
  form. An offered Undo, and a plan that couldn't be read again, wait wherever the user goes, until the next write or
  Dismiss; the Undo stays on offer over a reload of the tab, shown once the plan read then says it can still be undone,
  and its guard checked again when pressed. Every notice shows in the slot, a bar at the stage's foot in the page's
  flow, so it covers nothing and a sheet rises above it: one place to look, which doesn't move as panels open and close.
  Dismiss is the panels' quiet ✕. A notice of news or an Undo is edged with a rule; the signal's dash is for a refusal,
  a failure or a form's warning, as news needs nothing of the user. One shows at a time: the next replaces it, including
  one a link from a notification brings. (Notices that followed the user to unrelated panels, and floated over the Map's
  titles, read as noise.)
- **Item panel:** quiet, as Linear's. Properties as text beside muted labels: Thread and Parent are buttons that fill on
  hover, Status and Id text. Edit (a pencil), the ⋯ menu and Close beside the title, which keeps most of the header's
  width; Move, Reopen, Add inside, Waits for, Relates to and Copy link in the ⋯ menu, each but Reopen and Copy link with
  its key. Links as rows cut to a line, the full title on hover, a section only when it holds one; a row's remove button
  shows on hover or with focus in the row, and always where there may be no hover. (Chosen by the user over one row of
  quiet buttons and over today's layout unboxed, as the calmest.) While it edits, the form's State and Thread stand in
  for the Status and Thread rows, which would only repeat them; Parent and Id stay. The form's thread picker says each
  thread as "4. CLI features", as text names a thread without a badge.
- **Autonomy:** the level said the same way everywhere ("Autonomy: Blockers"); under the view's name on a phone and at
  the sidebar's foot, pressing it opens the menu at the setting, so the word is never left unexplained; on a touch
  screen the view's name above it is part of its target, as its own line is too short for a finger. Under each list that
  sets it, what it lets reach the user, in core's words ("Agents ask only about blockers"), a thread without its own
  level adding "as set for every thread".
- **Needs you's entries:** the badge in a column of its own; beside it what the entry asks, bold at 16 px, then what
  it names as rows in ink at 14 px, regular and 32 px apart on touch, so they read as the head's parts (the user's
  ask), and in ink, which says they open (muted, they read as inert notes; a stage's icon beside each read
  as a mark without a word), then its decision, compact, after what it acts on. An approval names its thread alone, as
  its button and the list's filter say what it asks; the Overview, which shows no buttons, adds "4 items to approve".
  (Each row ending "waits for your approval" read as noise.) An approval lists the first five items it covers and says
  how many more, so its count and its list agree; an entry the app can only show says what failed and what fixes it, in
  words, never a command to type, and names its item once, as its row. A hairline between entries; the filters are tabs,
  the one shown filled, named as nouns (Approvals, Questions, Problems), as "Approve 20" over the entries' Approve
  buttons read as one more of them.
- **Hover:** controls and rows fill with `--hover`, ink at 7 %, which shows on white sheets too; a filled button fades a
  little toward the ground instead. A view's row fills so too, lighter than the view shown, which keeps its fill under
  the pointer.
- **Notes and briefs:** a small, safe part of Markdown, built as elements: code, strong (which may hold code) and
  emphasis. Nothing else is read, and never as markup.
- **Command menu:** a field over a list, the chosen row filled with a bar at its start, square where the bar runs; Close
  at the field's end. A thread's row is its badge and name; the number is the badge's. An item's row says the project it
  is in, not its id, which finds it but would only be plumbing under its title. A waiting move or link offers its
  targets with the projects in their own group, first before anything is typed, never the item itself, nor, for a move,
  where it is already or anything inside it.
- **Lists:** the Overview's rows a finger's height, ruled apart, each opening what it names, a chevron at its end. Rows
  inside an entry or a panel, such as Needs you's items and a thread's Items, are text that opens what it names, with
  no chevron.

## Interaction conventions

- **Views are places.** Overview, Map, Threads and Needs you are views; the one shown carries `aria-current="page"`
  and is filled, and a phone's bar names it.
- **Home is the Overview.** On a phone, Back from another view returns there before it leaves the app.
- **Layers stack in one order** (`src/layers.ts`), with one history entry each, so Escape and Back close the top one.
- **An item or a thread opens on the Map and closes back.** One opened from the Overview, Threads or Needs you shows on
  the Map, and one Back, Escape or ✕ returns to the view it came from; Needs you comes back scrolled as it was left, the
  focus on the row picked. Selecting on the Map or going to another view makes that where the user is, and going to
  another view closes the item, which it doesn't show, and going to the Overview or Needs you closes the thread too, as
  its panel would only stand over the list (the Threads view keeps it, its tile picked); what a link opened closes onto
  the Map.
- **A new item shows where it landed.** Added from another view, it opens on the Map once saved, selected and in view,
  and closing it goes back (the user's ask); a refused add reopens its form where the user was.
- **A form never loses typing:** Escape and Back on a changed form say how to leave it instead.
- **Everything open has an address.** The address bar always says what's open (`?view=`, `?item=`, `?thread=`, and
  `?view=needs&needs=<entry>` for Needs you at an entry), so a reload, a copied link or a notification opens it again;
  an item implies the Map, and one left open over another view names that view too. An address from when Needs you
  opened over the views, `?needs` as notifications send it, opens its view. A link opens over home, one history entry
  per layer, so Back closes what it opened, then leaves. A link to something gone says so; one to an archived item shows
  archived items, with Hide archived. An item's panel has Copy link in its ⋯ menu, which says "Link copied"; a thread's
  panel carries it beside the title, ticking in place. A device not yet paired keeps the address through pairing, so the
  link it opened shows once it is.
- **One field reaches anything.** Search (⌘K, Ctrl+K elsewhere; the sidebar's Search or jump to…; the phone bar's
  Search) opens the command menu over everything: views, Needs you among them, New item, the archived switch, Refresh,
  and every thread, project and item by title or id, grouped, the best first. While a move or link waits for its target,
  it finds the target instead. It leaves what's under it as it is, never closes a form holding typing, and gives the
  focus back where it was.
- **Keys, in Linear's habits.** G then O, M, T or N goes to the Overview, the Map, Threads or Needs you; C adds; / and
  ⌘K search; ? lists the keys in the command menu, each runnable, and under them as text the rest: an item's keys
  while none is open, J and K, search and undo, in the platform's own chords. With an item's panel shown, E edits, M
  moves, W and R link, A adds inside; a move or link from the keyboard finds its target in the menu. J and K move
  through the Overview's rows and Needs you's entries, an entry being no control, so Enter on it does nothing. No
  single key approves, keeps, drops or deletes. Keys go to fields, input methods and changed forms first, and the
  menu's Single-key shortcuts switch, per device, turns them off.
- **Undo, not confirm.** A move, an item's fields (but its thread), a link added or removed, the global autonomy level,
  and a thread's first approval offer Undo in their notice, and ⌘Z while it shows; nothing else does, as nothing else
  has an exact inverse (an approval again, a revoking, a proposal kept or dropped, a thread's own level or flag, a
  thread changed: agents act on them). An approval is taken back, by a revoke, only while no agent has started on the
  work it approved, as a claim shows on any of its items that could start. Undo reads the plan first, and without that
  read it undoes nothing; it takes back nothing changed since, nor a removed wait or a state once the work it released
  has started, saying so. It says "Undoing…" until done, then what it did ("Autonomy set back to Blockers", not
  "Undone": agents' work meanwhile stands), a link said as it reads ("Sow now waits for Dig", "Sow no longer waits for
  Dig"), a move "Moved X back", selecting X where it is again, fields "Set X back". Undo sits in the slot, its text two
  lines at most there, so it says what was done, as its button says what it offers; anything else in the slot wraps, as
  it is the only place its reason shows.
- **Settling keeps the keyboard in place.** An action in Needs you or a thread's panel is held, not disabled, while its
  write is out, so focus stays on it; only the pressed one looks held, as greying every one said they were all at work.
  Once what it settled goes, focus goes to the next entry's action, else to the view or the panel. What a one-press
  decision did is said once saved ("Approved Codex: 2 items"), in its notice and to a screen reader politely.
- **A notification never reloads the app:** the open app is asked to show what it names (Needs you's view, for the
  server's), over the view shown if it names none, and waits, saying so, while a form holds what the user typed.
- **Opening from a row lands on its entry:** a Needs you row on the Overview goes to Needs you's view there, the entry
  focused and none of its actions.
- **Going to a thread frames it.** A thread's row on the Overview or in the sidebar, or a link to a thread on the Map,
  goes to the Map with the thread picked and its panel open, its items fitted beside the panel (no closer than 1:1, no
  further out than readable, else from its first item), with the titles of the boxes they are in when those fit too. A
  known limit: a thread whose items sit far from its project's title, which is at the box's left edge, frames the items,
  and the title is off-screen, as both together would need a zoom below readable. The picked thread's row in the
  sidebar, on the Map, unpicks it. A tile picked in the Threads view opens the thread's panel there.
- **The selection stays in view.** An item brought in from elsewhere (Search, Needs you, a link, a write that moved it)
  comes to the middle of the open Map if it was out of view, rather than to an edge with its links beyond it. On a Map
  zoomed out past reading, one the user picked comes in at a zoom its title reads at, wherever it was, and one a write
  moved does only if it must move, so a zoom the user chose survives their edits; a tapped one moves the least that
  shows it. A strip taken from the Map, such as the slot, or a window turned, shows the selected item again if it hid
  it; a phone's keyboard, which shortens the Map too, leaves the Map where it is while the user types.
- **New item asks for little:** a title, a project (the one in context, else the top level) and a thread. Its id is made
  from the title and asked for only once the server refuses it. Added, the item's panel takes the focus, and its Status
  says Saving… until the server has placed it, rather than its state first; its editor opens on Title.
- **One thing in focus.** Selecting an item outside the picked thread unpicks the thread, so the sidebar never marks
  a project and a thread the selection isn't in at once; an item in the thread keeps it, its panel over the thread's.
- **States:** loading, a failed load (what to check, then the error, then Try again), and an empty plan are each a page
  of their own. Until the Map is first drawn, which on a phone takes a second or two, it says it is laying out, rather
  than show a blank canvas that reads as no plan. A browser the server no longer knows, after `acyclic up --rotate`, is
  told how to pair again, with the server's port, and offered no Try again or Refresh: coming back to the app once
  paired reads the plan again.

## Voice & copy

Plain verbs on actions ("Approve 3 items", "Add item"), and verbs any pointer fits: a move or link asks to "Choose" its
target, as a tap or a click both do. Names follow `GLOSSARY.md`, the source of truth for every word the app shows:
item, project, thread, wave, the needs-you list (written Needs you), and its other words. Where the app needs a word
the glossary lacks, the glossary gains it first; the app never has words of its own for the tracker's ideas. Threads
that wait on nothing, and that nothing waits on, are Any time. One word is the code's alone and never shown: an
item's stage is where it stands (its readiness, or its state once it is done). A thread on the Threads view is a tile,
`ThreadTile` in the code, as an item on the Map is; the glossary's Card is something else. Errors say what to check
before what went wrong.

## Do's and Don'ts

- **The One Primary Rule.** One filled button per decision; everything else steps down a tier. In a list of
  decisions, with a pointer, an entry's main action is outlined until the entry is under the pointer or holds the
  focus: a column of identical filled buttons read as clutter. Without hover, they stay filled.
- **The Shape Rule.** Selection is drawn by shape (an ink edge with a soft halo, a fill, a pill), never by hue alone.
  One ring, not two: a double ring of ink read as heavy in both schemes. A selected tile's edge is twice a lit one's, as
  a dark thread's line came close to ink, and its halo faint: a stronger one read as a second ring. A field's focus ring
  lies over its border, for the same reason.
- **The Signal Rule.** The signal is text and dashes. Never a solid border, a fill, or a badge with a number, which
  a thread's badge could be mistaken for.
- **The Text Rule.** Stage counts are always written out; icons only accompany them.
- **The List Rule.** The Overview is lists to scan, never tiles or charts.

## The Map

- **Top to bottom.** Inside a box, items run down in layers by their links; a long chain stays one column, never wraps.
- **An item reads as a sidebar row does:** its thread's badge first, after the stripe, then its title, then its
  stage's icon at the end.
- **A project tells where it stands.** A box's title leads with the stage icon of its work as a whole (in progress while
  any of it moves, else in review, done once all of it is, else ready, else blocked, else parked, else needs a thread),
  after its chevron, and is muted once done or held up, as an item's title is.
- **A join sits between what it waits on.** Inside a box, an item waiting on two sits centred under them, each pulling
  it equally (the balanced placement), not under whichever came first.
- **Hover shows an item's links**, as hovering a tile on the Threads view does: its own, in ink, across boxes too, and
  the other links recede, the tiles staying as they are; a folded box shows its items' links; the links a drawn way
  already implies stay out, since drawn straight they would cut across items. Touch has no hover; a tap selects, which
  shows them all.
- **Any time comes first.** Items with no link inside their box stand in columns on its left, before its graph, as
  the Threads view's Any time column stands before wave 1 (the user's call: what can start now reads first). A dashed
  line, in the links' colour, runs down between the columns and the graph, so where one ends and the other starts reads
  at a glance (the user's ask).
- **Packed tight, in order where it counts.** The top level is packed into a rectangle with as little room wasted as
  the boxes allow (the user's ask: no gaps between rows), each box after those it waits on: below one, or level with
  it or lower on its right. Placed largest first, the narrower boxes fill the room beside the wide ones. The sidebar's
  and the Overview's projects come in dependency order (`topOrder`), which the Map keeps wherever one box waits on
  another.
- **A press leaves no mark.** Cytoscape's grey press disc and node shade are off: a tap selects, and the selection
  shows it.
- **No link hidden without cause.** Inside a box, a link is left out only when a longer way through items in the same
  box already implies it. A link across boxes shows only for the item selected or hovered, or a lit thread's, as all of
  them at once crossed the Map; selecting an item, or lighting its thread, shows every link it has.
- **Out as one trunk, in one by one.** A node's outgoing links leave as one trunk that splits, marked by a dot; each
  arrives at its own point with its own arrowhead. Sharing both ends would let an outgoing trunk and an incoming one
  run as one line, and a link would seem to join items it doesn't. A dot marks only where a trunk splits; links that
  cross have none.
- **Opening view.** The whole plan when it fits at 1:1 or closer, fitted up to 2×, on every screen, a phone's too (at
  0.85, a phone's titles read at 11 px); otherwise the first box's corner, its title and first item whole, at 1:1.
- **Boxes fold.** A box's chevron folds or unfolds it, reaching 44 px for a finger; the box's corner stays where it was,
  and nothing pans to show what it opened. The Map opens with the top level's boxes open and every box below folded,
  and a fold survives the plan being read again.
- **The wheel zooms 12 % a notch** of a Mac's mouse, which scrolls 80 px, and in step with the pixels where a notch
  scrolls more (15 % for 100 px), with Ctrl held or not; a trackpad's scroll and pinch zoom smoothly, as Cytoscape's.
- **A pinch stops at the floor and keeps panning.** The furthest zoom out is the floor whichever way it is reached. Past
  it, a pinch holds the zoom there about the fingers' midpoint and still follows the fingers, where Cytoscape on its
  own drops each step that would pass it, pan and all, so the plan stuck short of the floor and then jumped.
- **Titles read whole.** An item's title takes up to three lines on its tile, cut short only past that; backticks,
  which set code in the panels, are dropped on the canvas. Tiles are drawn over every box, and a link across boxes
  between the two, so it never runs through a title.

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
- 2026-11-01: Up next lists the approved threads with an item ready, Any time first, then by wave, over the next wave
  whole; the user chose it.
- 2026-11-01: tiles three lines tall over wider ones or today's two, and a first view at 1:1 on a wide screen; the
  user chose them. Later the same day, 1:1 on a phone too, 13 px text as on desktop; the user chose it.
- 2026-11-01: an item opened from another view shows on the Map, and Back returns to that view; the user chose it
  over opening items in place. A thread opened from another view does the same, by extension (proposed).
- 2026-11-01: threads' lines in Okhsl, at saturation 0.9, with neighbouring numbers still alike; every notice in the
  slot, over notices under the panel on top; Undo for a first approval while no agent has started its work; the user
  chose them.
- 2026-11-01: the phone's view tabs close Needs you, and the Overview stands in three columns on a wide screen; the
  user chose them. Needs you open is marked by ink alone, after the lighter fill read as a second current place.
- 2026-11-01: a thread picked from Needs you goes to the Map, as every thread does; a short Up next stacks under In
  progress, the projects taking the third column; the thread colours' hue step stays, and project names show as
  written; the user chose them.
- 2026-11-01: a Theme in the menu, System, Light or Dark, System by default, after the user asked for a switch on the
  phone; settings change at once and leave the menu open; a thread's panel lists its Items in place of Starts with. The
  user chose them.
- 2026-11-01: Needs you is a view of its own, a peer of the Overview, the Map and Threads, over a pane on the right that
  opened over them, which marks, toggling, the phone tabs closing it and the Overview stepping aside existed to explain.
  The user: "The overview shrinks so it doesn't hide behind 'Needs you', but neither the map nor the threads view do
  that. Honestly, I don't know if I like `Needs you` being a floating pane on the right at all, it feels like it should
  probably get its own view like overview/map/threads?" Asked to confirm, they chose the view.
