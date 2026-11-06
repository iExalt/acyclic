# Design

The app's agreed design: what later changes keep, and why. Values live in the code (`src/palette.ts`, `src/app.css`);
this names them and says where they apply. Update it in the same change as the code when a convention changes.

## Users

One person, the plan's owner, who hands work to agents. They check in on a phone between other things, and sit with
the plan on a desktop when they reshape it. Most visits are short: see what needs them, decide, leave.

## Primary tasks

1. Settle what needs them: resolve a conflict, untangle a cycle, look at done work still waiting on open work.
2. See where work stands: which threads are at work, which to start next, how each project is doing.
3. Reshape the plan: add, move, link and edit items on the Map.

## Product principles

- Task efficiency over novelty: familiar controls, few surprises.
- Every decision is the user's to make once, in one place, with one primary action.
- The plan's state reads as text; colour only ever repeats it.

## Accessibility & inclusion

WCAG 2.2 AA in both schemes: text at 4.5:1 on every surface it sits on, which `palette.test.ts` holds the palettes to.
Touch targets are 44 px on a coarse pointer, each a row's or a button's own box, never a reach drawn over its
neighbours; Needs you's item rows alone are 32 px, which clears the 24 px of WCAG 2.5.8, as the user asked for them
closer. With a fine pointer, `--hit` gives default buttons and fields 34 px, and compact rows and controls inside lists
go down to 28 px (a Needs you item row to 24 px), as Linear's do. Everything is reachable by keyboard; focus is always
visible. Reduced motion turns the sheets' rise off.

## Overview

A quiet tool (Operate mode throughout), in the manner of Linear: neutral surfaces, ink rather than a brand colour as
the accent, so colour is left to the threads' lines and the signal. The scheme follows the system's, light or dark,
unless the menu's Theme picks one on the device (System, Light or Dark; System by default).

## Colors

`PALETTES` in `src/palette.ts` is the source; `app.css` repeats it as custom properties under `/* palettes */`, each
`light-dark(light, dark)`, so a Theme picked on the root (`data-theme`) needs no second copy, and a unit test keeps the
two in step.

- **Surfaces:** `ground` (the page and the Map), `chrome` (sidebar and bars), `sheet` (panels), `tile` and `box` (the
  Map's items and boxes). A tile in progress takes a cooler tint, one stalled a warm one, one done, waiting, parked or
  canceled a greyer one; one in review keeps the tile's, its icon saying so, after a tint read as a selection.
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
  it can hold, so every line's badge takes a readable ink. Hues step round the wheel by a hash of the thread's id,
  a lap every 27 positions, each lap a quarter of the band lighter: positions 4 and 31 share a hue and look nearly
  alike (ΔE about 4), so every list names the thread beside its badge; the user's call: "Don't change the hue step."

## Typography

Overpass, variable, for the app and the Map's canvas. The Map measures and draws with one settled face: it waits for
Overpass to load, and uses `system-ui, sans-serif` if Overpass is unavailable, so a later font load cannot change its
label widths. 15 px base; wide, the sidebar a step smaller, at 14 px. Sentence case. Tabular figures where counts are
compared. Monospace only for ids.

A phone is set as compactly as Linear's and Slack's apps, in px on each element rather than a smaller base, which
would shrink every size set in rem with it: the bar's view name 17 px at 650, the tab bar's names 11 px, the Overview's
headings 14 px, its rows 14 px over a 13 px detail, and rows at the text's 500, so the titles over them lead. (At the
desktop's sizes the user found the phone "comically... big", and the Map had little room beside a panel.)

Text sets at 500, not the face's 400, and every heavier weight a step up with it. The Map's leaves' titles stay at 500,
as the text does, and its boxes' at 700: its renderer takes only hundreds, and at 600 a project's title barely led its
items. Overpass's regular strokes are thin on a screen, and the text strained to read
beside Linear's although its contrast was 14:1; at 500 it is as dark as Inter at 450. Inter, tried in its place, read
cold; Overpass, heavier, keeps the app's voice.

Panels set what they say in one size, 14 px, labels and values alike, and wide their fields too (a phone's stay at 16
px, so it doesn't zoom in on them); notes, the reading, at 15 px wide; the title 19 px at 650, not bold, 17
px on a phone; section headings 13 px at 600, muted, so the content leads. (Mixed sizes and weights read as awkward;
then 13 px read a smidge small beside Linear at the same window size, as Overpass sets smaller than Inter at a size.)

## Layout

- **Phone (under 900 px):** a 44 px bar on top (the view's name, Search, the outline, New item, the menu), the views,
  Needs you among them, in a tab bar along the bottom, its tabs 44 px with nothing around them, panels as sheets rising
  from the bottom. Each bar is its buttons' height, as Linear's and Slack's are. The outline's button and sheet are both
  called Outline; its projects carry no count, as opening one shows its threads.
- **Sheets, on a phone:** two heights. A panel's first is 144 px: its title, its status and its main action, so the Map
  keeps most of the screen beside it (the user: "This hurts especially when browsing half the map and half an item"),
  taller only where a long title or status pushes the main action, or an item's status, further down; while it loads,
  200 px, more than it will need, so what the Map frames above it stays clear. Either height ends on a row, within a
  row's height of it, so no row is cut through its text under the tab bar. The outline's, a list to pick from,
  half, at least 320 px. Full reaches the bar. The handle, a button in a strip at the top of the header, which stays in
  view as the sheet scrolls, switches them; it is 28 px tall, an exception to the 44 px targets, as a taller strip would
  push every panel down or lie over its buttons, and the title starts below it. A drag on it or on the header's free
  space follows the finger and lands by distance or by a flick, read from its last 100 ms, and down from the first
  height it closes the sheet as Escape does, so a form holding typing stays, saying how to leave it. A form (New item,
  an item's editor) takes the whole height, with no handle. A tap on the Map that opens a panel over what it tapped pans
  the least that shows it, measured where the panel comes to rest.
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
  px, as many as fit; on a phone, one column. (As a narrow pane on the right it left most of a
  desktop empty, and only the Overview made room for it.)
- **Up next** lists the threads that can start now: not at work, with an item ready, by id in natural order as
  `acyclic thread ready` lists them. The Open Threads link opens Threads, where waves and Any time show what waits for
  what.

## Shapes

One radius, 8 px (`--radius`), for controls, tiles and panels. Smaller things take smaller ones: chips, link rows, the
remove-link button and the command menu's rows 6 px, key caps and code 4 px; the ⋯ menu's popover 12 px; panels beside
the Map, wide, are square. Sheets on a phone round their top corners, by 14 px.

## Components

- **Buttons, by tier:** `.primary` filled with the accent; secondary outlined; `.quiet` text-only; `.danger-quiet`
  red text, last in its group.
- **Stage marks:** each stage's icon beside its word (`Stage.tsx`); counts always as text beside their icons, each count
  set apart by its icon rather than a comma, which only a screen reader reads; a Map box's line of counts, drawn as text
  on the canvas, which sets no icon inside a line, takes the commas. An item in no thread is a thread of one, so its
  stage reads like any other's (ready, or in progress once claimed); on the Map it carries no stripe, badge or ring, as
  a task alone has no thread to colour them, and its stage's icon ends the line. The editor says a state in words
  ("Open", "In progress"), Open with how it shows (ready or waiting, by its waits), and a finished item's state as the
  panel's stage, as text: finishing sets it, not the form.
- **A thread's panel** shows its counts under the title, then its Items, in the order their work runs (each after
  what it waits for in the thread), isolated items first, each with its stage's icon, the first eight and then how many
  more; "Starts with", its first items alone, meant nothing to the user. Then its notes.
- **A thread of one is a task alone.** Its row or tile, in Up next, In progress, the Threads view, the outline and the
  command menu, reads as a thread's does but takes no line's colour: its badge is an open ring in the quiet ink, the
  dot's size, and a tile's left edge is the plain rule. Opening it opens the task, as an item's row does, not a thread's
  panel, and the item's panel names no thread for it. A thread holding a title of its own, once a second item joined,
  is a thread like any other.
- **Thread badges:** a dot in its line's colour. A badge, and a stage's icon, stand on the middle of the
  capitals beside them, a tenth of an em over the line's, since Overpass sits high (the user saw them sit low).
- **Settings change at once.** A setting in the ⋯ menu shows its new value as it is picked. A setting leaves the
  menu open; only its buttons close it. (The menu closing on a checkbox read as bad UX.)
- **Notify this device:** in the menu, a setting of this device, read again each time the menu opens; not at the top
  of Needs you, where every visit passed it.
- **Key hints** (an item's ⋯ menu's letters, the sidebar's search chord, the command menu's keys) show only where a
  mouse is the main pointer, or once a key from a physical keyboard has been pressed: a phone's stylus is a fine
  pointer but brings no keys. The menu's switch for single-key shortcuts stays on every device.
- **Needs you:** the inbox icon and the count beside its name, in ink, as an inbox's; nothing when nothing waits. A
  need with no thread to badge it takes the inbox icon in its place. (An amber "!" read as a warning sign, out of
  place in the quiet look.)
- **Notices stay with what they are about** (`src/notices.ts`). News of what just happened (saved, undone, a link
  opened, with any offer it makes, such as Undo) goes once the user moves; a
  refusal or failure stays on the surface that raised it until that surface closes; a form's warning stays with the
  form. An offered Undo, and a plan that couldn't be read again, wait wherever the user goes, until the next write or
  Dismiss; the Undo stays on offer over a reload of the tab, shown once the plan read then says it can still be undone,
  and its guard checked again when pressed. App owns one notice, its lifetime, actions and announcement; it chooses
  where that notice appears. On desktop, and on a phone outside the Map when no panel is open, it uses the slot at the
  stage's foot in the page's flow. On the Map, a
  notice floats only when at least 44 px of clear space is available, within a bounded area. In other phone contexts,
  including a Map without that space or a non-Map panel, it stays closed behind a labeled Notice disclosure in the top
  bar, in place of the visible view name; the existing semantic h1 remains. A newly arrived notice does not open the
  disclosure, so one extra tap reveals it. Its bounded scroll area keeps the full text and actions available. The
  disclosure has explicit open, close and focus behavior. These placements leave forms, the header, Save and Cancel, the
  Map and its scrolling alone.
  Dismiss is the panels' quiet ✕. A notice of news or an Undo is edged with a rule; the signal's dash is for a refusal,
  a failure or a form's warning, as news needs nothing of the user. One shows at a time: the next replaces it, including
  one a link from a notification brings. (Notices that followed the user to unrelated panels, and floated over the Map's
  titles, read as noise.)
- **Item panel:** quiet, as Linear's. Properties as text beside muted labels: Thread and Parent are buttons that fill on
  hover, Status and Id text. Edit (a pencil), the ⋯ menu and Close beside the title, which keeps most of the header's
  width; Move, Reopen, Add inside, Waits for and Copy link in the ⋯ menu, each but Reopen and Copy link with
  its key. Links as rows cut to a line, the full title on hover, a section only when it holds one; a row's remove button
  shows on hover or with focus in the row, and always where there may be no hover. (Chosen by the user over one row of
  quiet buttons and over today's layout unboxed, as the calmest.) A project with no thread omits the Thread row; one
  with a thread keeps its chip. Other items with no thread say None. While it edits, the form's State and Thread stand
  in for the Status and Thread rows, which would only repeat them; Parent and Id stay. The form's thread picker names
  each thread by its title alone, as text names a thread without a badge.
- **A phone's bar** names the view alone, as a line under the name cost the bar its height.
- **The ⋯ menu:** the bar and sidebar footer share plain settings rows, each switch's name at the start and switch at
  the end, with the whole row as its target. Footer controls are at least 32 px with a fine pointer and at least 44 px
  on touch; the bar keeps 44 px targets. On a phone the menu is 260 px wide. Long notification labels wrap within
  their row. Theme stays a native select under a muted caption, and Refresh stays a button in its own ruled row.
- **Needs you's entries:** the badge in a column of its own; beside it what the entry asks, bold at 16 px (14 px at
  650 on a phone), then what
  it names as rows, muted, at 14 px (13 px on a phone), regular and 32 px apart on touch, so the head leads and they
  read as its parts (the user asked for "the thread title … more prominent" and the "line spacing between each item …
  smaller"); each underlines under the pointer, in ink, saying it opens (a stage's icon beside each read as a mark
  without a word), then its action, compact, after what it acts on. An entry the app can only show says what
  failed and what fixes it, in words, never a command to type, and names its item once, as its row. Merges unconfirmed
  for one cause in one repository are one entry, saying how many, its rows the first five, as the same why eight times
  read as noise; the count still counts each. A hairline between entries. Touch headings have equal padding above
  and below their text, whether they open a thread or are plain text. On a phone, terminal-only multi-thread cycles
  and grouped unconfirmed merges start with a two-line heading and Show details; one tap reveals their unchanged
  explanation and records. Entries with decisions and single-record entries stay open. Opening an entry from the
  Overview or returning to one of its records expands it before restoring focus and scroll.
- **Threads tiles** use 14 px titles at 500 and 13 px details on a phone, with less padding and space between tiles.
  Project names, stage counts and dependency information stay available, and each tile keeps its own 44 px target. A
  project label matching the thread name after trimming whitespace and ignoring case is omitted; distinct names remain.
- **Hover:** controls and rows fill with `--hover`, ink at 7 %, which shows on white sheets too; a filled button fades a
  little toward the ground instead. A view's row fills so too, lighter than the view shown, which keeps its fill under
  the pointer. Only where there is hover: a tap leaves no tint behind on a touch screen.
- **Notes,** an item's and a thread's: a small, safe part of Markdown, built as elements: code, strong (which may hold
  code) and emphasis. Nothing else is read, and never as markup.
- **Command menu:** a field over a list, the chosen row filled with a bar at its start, square where the bar runs; Close
  at the field's end. A thread's row is its badge and name. An item's or target's context names its parent and root
  project when they differ, and wraps in full; the title may still truncate. Context does not change result ranking or
  dispatch. A waiting move or link offers its targets with the projects in their own group, first before anything is
  typed, never the item itself, nor, for a move, where it is already or anything inside it.
- **Lists:** the Overview's rows a finger's height, ruled apart, each opening what it names, a chevron at its end. Rows
  inside an entry or a panel, such as Needs you's items and a thread's Items, are text that opens what it names, with
  no chevron.

## Interaction conventions

- **Views are places.** Overview, Map, Threads and Needs you are views; the one shown carries `aria-current="page"`
  and is filled, and a phone's bar names it.
- **Home is the Overview.** On a phone, Back from another view returns there before it leaves the app.
- **Layers stack in one order** (`src/nav.ts`), with each browser-history entry owning its snapshot and address.
  Back and Forward retrace items followed from the Inspector. Escape or ✕ closes to the underlying context, skipping
  intermediate item visits; Forward restores the stored place without transient menus, forms or sheets.
- **An item or a thread opens on the Map and closes back.** An item opened from the Overview, Threads or Needs you, or
  a thread from anywhere, shows on the Map (the user's call, so every thread opens the same way). Back may first
  retrace those item visits; Escape or ✕ closes to what was underneath. Needs you comes back scrolled as it was left,
  with focus on the picked row. Selecting on the Map makes Map the return context; going to
  another view closes the item, which it doesn't show, and going to the Overview or Needs you closes the thread too, as
  its panel would only stand over the list (the Threads view keeps it, its tile picked); what a link opened closes onto
  the Map.
- **A new item shows where it landed.** Added from another view, it opens on the Map once saved, selected and in view,
  at a zoom its title reads at, and closing it goes back (the user's ask); a refused add reopens its form where the
  user was.
- **A form never loses typing:** Escape and Back on a changed form say how to leave it instead.
- **Everything open has an address.** The address bar always says what's open (`?view=`, `?item=`, `?thread=`, and
  `?view=needs&needs=<entry>` for Needs you at an entry), so a reload, a copied link or a notification opens it again;
  an item implies the Map, and one left open over another view names that view too. An address from when Needs you
  opened over the views, `?needs` as notifications send it, opens its view. A link opens over home, one history entry
  per layer, so Back closes what it opened, then leaves.
  Older or malformed entry metadata falls back to the URL; when its ancestry is unknown, closure stays local rather
  than guessing browser history. A link to something gone says so. An item's panel has Copy link in its ⋯ menu,
  which says "Link copied"; a thread's panel carries it beside the title, ticking in place.
- **One field reaches anything.** Search (⌘K, Ctrl+K elsewhere; the sidebar's visible Search… with accessible name
  Search or jump to…; its optional keyboard hint does not change the row height; the phone bar's Search) opens the
  command menu over everything: views, Needs you among them,
  New item, Refresh,
  threads by title, and projects and items by title or id, grouped, the best first: exact titles, then prefixes, then
  contiguous title phrases, then the remaining matches. While a move or link waits for its target, it finds the target
  instead. Choosing a result replaces Search with its destination in history; the same destination just closes Search,
  without adding a duplicate visit. It leaves what's under it as it is, never closes a form holding typing, and gives
  the focus back where it was. A touch-only menu starts without a selected result; typing keeps it that way until
  arrow keys or a mouse choose a row. Enter on a nonempty query opens the first result when none is selected; Enter
  on an empty, unselected menu does nothing.
- **Keys, in Linear's habits.** G then O, M, T or N goes to the Overview, the Map, Threads or Needs you; C adds; / and
  ⌘K search; ? lists the keys in the command menu, each runnable, and under them as text the rest: an item's keys while
  none is open, J and K, search and undo, in the platform's own chords. With an item's panel shown, E edits, M moves, W
  picks what it waits for, A adds inside; a move or link from the keyboard finds its target in the menu. A move or link
  waiting for its target takes the focus in its pick bar, where typing finds the target (the key typed starts the search
  and Enter picks the first match) and Enter, / and ? open an empty search. J and K move through the Overview's rows and
  Needs you's entries, an entry being no control, so Enter on it does nothing; its focus ring is rounded to the radius
  and drawn outside the entry, clear of its badge. No single key settles a need or deletes. Keys go to fields, input
  methods and changed forms first, and the menu's Single-key shortcuts switch, per device, turns them off.
- **Undo, not confirm.** A move, an item's fields (but its thread) and a link added or removed offer Undo in their
  notice, and ⌘Z while it shows; nothing else does, as nothing else has an exact inverse (a conflict settled, a reopen,
  a thread changed: agents act on them). Undo reads the plan first, and without that read it undoes nothing; it takes
  back nothing changed since, nor a removed wait or a state once the work it released has started, saying so. It says
  "Undoing…" until done, then what it did (not "Undone": agents' work meanwhile stands), a link said as it reads ("Sow
  now waits for Dig", "Sow no longer waits for Dig"), a move "Moved X back", selecting X where it is again, fields "Set
  X back". Undo uses the same placement as other notices. Its actions remain available with its full text in the bounded
  disclosure or floating area; elsewhere its text wraps in the slot.
- **Settling keeps the keyboard in place.** An action in Needs you is held, not disabled, while its write is out, so
  focus stays on it; only the pressed one looks held, as greying every one said they were all at work; an item panel's
  remove buttons and Edit are held so too. Once what it settled goes, focus goes to the next entry's action, else to the
  view. What a one-press decision did is said once saved ("Used parked for state"), in its notice and to a screen reader
  politely.
- **A notification never reloads the app:** the open app is asked to show what it names (Needs you's view, for the
  server's), over the view shown if it names none, and waits, saying so, while a form holds what the user typed.
- **Opening from a row lands on its entry:** a Needs you row on the Overview goes to Needs you's view there, the entry
  focused and none of its actions.
- **Going to a thread frames it.** A thread opened from the Overview, the sidebar, an item's panel, or a Map link opens
  on the Map, picked with its panel. Its items fit beside the panel at up to 1:1, but no smaller than readable; when
  they cannot all fit, the frame starts at the first item. Include their boxes' titles only when the titles and items
  fit together. Otherwise keep the boxes and chevrons but hide their text until the user moves the Map or changes the
  selection. A project's title may remain off-screen when its items are far from it; showing both would require a zoom
  below readable. The picked thread's sidebar row unpicks it. A thread opened from an item's panel keeps that item
  underneath; closing the thread or pressing Back reopens the item unless it is gone.
- **The selection stays in view.** An unpinned item from Search, Needs you or a direct link, and a top-level add, comes
  to the middle of the open Map. Below readable zoom, its title comes in at a readable size. An explicit pin—the parent
  of an add or move, or the current item when following a link in the Inspector—holds that drawn box in place and
  reveals the selection only as much as needed. A write to the same selected item, and its Undo, keep the top-level
  box's screen corner and the chosen zoom; internal layout may change, and the selected item pans only as far as needed
  if hidden. A tapped item moves the least that shows it. A strip taken from the Map,
  such as the slot, or a window turned, shows the selected item again if it hid it; a phone's keyboard, which shortens
  the Map too, leaves it where it is while the user types. While a sheet changes height or covers the Map, the reveal
  waits until it settles and leaves room. Another selection, manual panning or zooming, or Fit takes precedence.
- **New item asks for little:** a title, a project (the one in context, else the top level) and a thread. Its id is
  a UUIDv7 the app makes, so a pending add is drawn under its real id and a retried send is the same add. Added, the
  item's panel takes the focus, and its Status says Saving… until the server has placed it, rather than its state
  first; its editor opens on Title. A picked thread, then the parent or selected item's thread, takes precedence;
  otherwise a project's sole visible thread supplies the default. Changing Project updates that default until the
  user chooses a thread, including Automatic; a reopened draft keeps its choice. Automatic follows the tracker's
  parent, sibling or ancestor placement policy, leaving no thread when none applies. On a narrow phone the existing
  actions use the form's spare height at its foot; the tablet keeps its content-height form.
- **One thing in focus.** Selecting an item outside the picked thread unpicks the thread, so the sidebar never marks
  a project and a thread the selection isn't in at once; an item in the thread keeps it, its panel over the thread's.
- **States:** loading, a failed load (what to check, then the error, then Try again), and an empty plan are each a page
  of their own. Until the Map is first drawn, which on a phone takes a second or two, it says it is laying out and marks
  the canvas busy, rather than show a blank canvas that reads as no plan.

## Voice & copy

Plain verbs on actions ("Use parked", "Add item"), and verbs any pointer fits: a move or link asks to "Choose" its
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
- **An item reads as a sidebar row does:** its thread's badge first, after the stripe, then its full title, then its
  stage icon and word. Stage words use the canonical vocabulary: Done, In progress, Ready, Stalled, Waiting, Parked
  and Canceled. Canceled is a circle struck through, in the quiet ink.
- **A project tells where it stands.** An open box's chevron and stage icon stay in the first 52 px strip; its full
  title, stage word, counts, hidden count and done-box note wrap below or beside them as needed. Its header grows to
  fit, and the child content starts below it. The stage is the work's whole stage, as core tells it (done once all of it
  is; else in progress once work under it has started and some of what is left isn't parked; else stalled, else ready,
  else waiting, else parked; canceled leaves are left out, and a box with only those reads
  canceled); one with no leaf to count, as on a cycle of parents, says its own state. Its title is muted once done or
  held up, as an item's title is.
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
- **Threads' waits stay named.** Every tile names the threads it waits on and any cycle peers. On wide screens, arrows
  appear only for a hovered or picked thread; phone layouts keep the names without arrows.
- **Packed tight, in order where it counts.** The top level is packed into a rectangle with as little room wasted as
  the boxes allow (the user's ask: no gaps between rows), each box after those it waits on: below one, or level with
  it or lower on its right. Placed largest first, the narrower boxes fill the room beside the wide ones. The sidebar's
  and the Overview's projects come in dependency order (`topOrder`), which the Map keeps wherever one box waits on
  another. Items never wait on themselves through their links, but boxes can: a box waits on another when an item in
  it waits on an item in the other, so two boxes linked both ways, by different items, wait on each other with no
  item in a cycle. Packing breaks such a cycle at its largest box, which then stands before what it waits on; moving
  an item, so that the links between two boxes all run one way, ends it.
- **Opening a box moves only what it covers.** Once packed, the top level keeps its boxes where they are: a box that
  grows, as one opening inside it makes it, stays at its corner, what it now covers makes way, right if it stood on its
  right and down if not, and the rest stays still, as does the screen, held at that top-level box's corner. Where a box
  stands depends on the source snapshot's packing and the boxes' sizes now, so what made way goes back when the box
  that moved it closes, whatever was opened in between. A source update that shrinks top-level boxes without growing
  any others packs them again; a chevron fold still holds their places. Undo restores the earlier source's packing.
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
- **Fit the whole plan.** A Fit button on the Map frames the whole drawn plan in the uncovered part of the Map,
  including beside a panel or above a phone sheet. Automatic views keep its top-left control strip clear, even if the
  Map first draws under another view. Fit changes only the viewport; selection and folds stay put. The button is a
  standard control, 44 px on touch, and is absent from other views.
- **Boxes fold.** A box's chevron folds or unfolds it, reaching 44 px for a finger; the box's corner stays where it was,
  and nothing pans to show what it opened. The Map opens with the top level's boxes open and every box below folded,
  and a fold survives the plan being read again.
- **Finished work folds away** (`src/map/fold.ts`), so what is left reads first (the user: "it's hard to tell what's
  done and what's not done"). In a box, a finished thread's items, two or more, are one done box named after the
  thread, its stripe in the thread's colour, its name on two lines over how many are done; finished work that no link
  ties, through other finished work, to anything left in the box goes into one box, Done, last in its columns ("37
  done in 5 threads"), when it holds two or more and something stands beside it. Done boxes open folded and fold as
  boxes do; unfolded, a finished thread's is a frame around its items with its name and chevron, the user's pick over a
  Fold button in the thread's panel or a Fold tile. A tap on a finished thread's box anywhere but its chevron picks the
  thread; a tap on Done folds or unfolds it, as it holds no one thing to pick. A move or a link never takes a done box
  as its target. Finding a folded item, or opening a link to it, unfolds every box above it. A thread is finished once
  every leaf is done or canceled, but canceled work isn't folded: it stays beside the Done box as the quiet tile it is.
- **Canceled work** reads as dropped, not delivered: a struck-through circle in the muted ink on a notch-greyer tint (a
  restrained default). The item form's State select sets it and Open undoes it, and Undo of a cancel sets the state
  back. A box counts its canceled leaves apart ("2 canceled").
- **The wheel zooms 12 % a notch** of a Mac's mouse, which scrolls 80 px, and in step with the pixels where a notch
  scrolls more (15 % for 100 px), with Ctrl held or not; a trackpad's scroll and pinch zoom smoothly, as Cytoscape's.
- **A pinch stops at the floor and keeps panning.** The furthest zoom out is the floor whichever way it is reached. Past
  it, a pinch holds the zoom there about the fingers' midpoint and still follows the fingers, where Cytoscape on its
  own drops each step that would pass it, pan and all, so the plan stuck short of the floor and then jumped.
- **Map labels fit their words.** Tiles are 200 px wide and at least 64 px high; their full titles wrap without a line
  cap or ellipsis, followed by the stage word when one applies. Box labels wrap the full title and any stage word,
  counts, hidden count or done-box note. An open box's header grows to fit those lines while its chevron and stage
  glyph stay in the first strip, keeping the fold target stable. Backticks, which set code in the panels, are dropped
  on the canvas. Tiles are drawn over every box, and a link across boxes between the two, so it never runs through a
  title.

## Decision log

- 2026-10-31: the quiet look over a familiar, header-and-tabs one; the user preferred it and wants more from Linear.
- 2026-10-31: the Overview as home over the Map; the user preferred it.
- 2026-10-31: Needs you open is marked lighter than the view shown, so only one place reads as current.
- 2026-10-31: the scheme follows the system's.
- 2026-11-03: The server checks Undo's field, link and started-work conditions under the batch lock, before lease
  renewal. A mismatch leaves the tracker unchanged and keeps “Changed since, so it can't be undone” in the notice.
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
- 2026-11-01: a denser phone, after the user found it "comically... big" beside Linear and Slack: of three mockups they
  picked B ("Let's go with B."), every row and button its own 44 px target, with C's gains that cost no target added
  (the 44 px bar and tab bar, a panel's 144 px first height, a thread's status and counts on one line, and Needs you's
  items 32 px apart); C's shorter rows, reaching 44 px over their neighbours, were left out. Needs you's items went
  muted with it, on every screen.
- 2026-11-03: finished work folds into done boxes, after the user found it "hard to tell what's done and what's not
  done" in a box of interleaved tiles. They asked for finished threads with links to fold too, as one box "named after
  the thread", rather than vanishing into Done, and chose a frame around an unfolded thread's items ("Approve, frame
  around items") over a Fold button in its panel or a Fold tile.
- 2026-11-03: opening a box no longer packs the top level afresh, after the user found that unfolding a thread's done
  box made "the entire map … shift over".
- 2026-11-04: source updates that only shrink top-level boxes repack them automatically; chevron folds still hold their
  places, and Undo restores the earlier source's packing.
