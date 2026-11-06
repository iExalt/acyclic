# acyclic

acyclic is a Git-native issue tracker for one person and many agents, where work forms a graph and agents pick up
coherent pieces of it, plus the app and the agent runs that carry that work out.

## Work

**Item**:
One unit of tracked work, from a single task to a whole project; items nest to any depth.
_Avoid_: issue, task, ticket, battle

**Leaf**:
An item with no children. The Rust role is `Leaf`; JSON keeps the older role name `item`.

**Member**:
An item belonging to a thread, including an item with children.

**Group**:
An item with children under another item.
_Avoid_: epic, category, goal

**Project**:
A top-level item with children.
_Avoid_: theatre, repository, phase, milestone, campaign

**Evidence**:
What `item done` records to show an item was finished: the commit that completed it, or a line of text.
_Avoid_: note (that is the item's own notes field)

## Execution

**Readiness**:
Whether an item or thread can start now, given its waits-for links and claims. JSON keeps the older key `status`
for readiness values.
_Avoid_: status (for readiness alone), unblocked

**Blocker**:
Something that stops work going ahead, which the user must clear.
_Avoid_: blocking issue

**Placement**:
Choosing where a new item belongs: its parent, what it waits for and its thread.
_Avoid_: triage, filing

**Split**:
Dividing a set of items into threads, or one thread into several.
_Avoid_: partition, breakdown

**Thread**:
A bounded piece of work that one continuing agent conversation works through from start to finish. A thread exists
while it has a record; an item that names any other thread holds a dangling reference.
_Avoid_: workstream, track, lane, operation

**Wave**:
A step in the order threads can start: each thread with work left comes after every thread its work waits on, and a
thread that waits on none and that none waits on is in no wave and can start any time. Threads in a waits-for
cycle share a wave.
_Avoid_: round, tier

**Claim**:
An agent's time-limited hold on a thread it is working on. The holder and its subagents work every member.
_Avoid_: lock, assignment

**Archived**:
A finished subtree hidden from `thread ready` and the app by default; archiving hides, never deletes.
_Avoid_: closed, deleted

**Deleted**:
An item or thread removed for good: gone from every view and every reader, and its id never used again.
_Avoid_: archived (that only hides), closed

**Needs-you list**:
Everything waiting on the user: conflicts nobody resolved, cycles of items, parents or threads, reopened work,
claims two holders clash over, references to what doesn't exist, and items in no thread. The app calls it Needs you,
and shows it as a view of its own.
_Avoid_: inbox, notifications

## The app

**Overview**:
The app's home: lists of what needs the user, the threads in progress and up next, and the projects.
_Avoid_: dashboard, home page

**Map**:
The app's drawing of the plan: each item a tile in its project's boxes, with its links.
_Avoid_: graph view, canvas, board

**Threads view**:
The app's view of the threads in waves, each a tile with its items' stages.
_Avoid_: lanes

**Outline**:
The projects and their threads as a tree, in the sidebar or a phone's sheet.
_Avoid_: tree view, navigator

**Up next**:
The threads with work ready to start, by wave.
_Avoid_: queue, backlog

**Tile**:
How the Map draws an item with no children, and the Threads view draws a thread.
_Avoid_: card, node (in the app's words)

**Box**:
How the Map draws an item with children, holding their tiles and boxes; folded, it is drawn as a tile.
_Avoid_: group (for this), container, lane

**Done box**:
A box the Map makes of finished work, which holds no item of its own: a finished thread's items in a box, named after
the thread, or the box's other finished work that no link ties to anything left there, named Done, last in its
columns. A thread is finished when every leaf it has, archived ones aside, is done.
_Avoid_: pile, group, archive

**State**:
What is set by hand on an item: `backlog`, `parked` or `done`. An unset state reads as `backlog`. State is stored;
readiness and stage are computed on read. A parent's progress also derives from its leaves and may be in progress.
The stored value `backlog` is not the app's Up next list.

**Stage**:
Where an item's work stands, said one way everywhere with its icon: in progress, done, ready, blocked, parked, or
needs a thread. A parent's comes from its leaves: done once every leaf is; before that, in progress once any work below
has started and some of what is left isn't parked; otherwise ready, blocked, parked or needs a thread by its leaves.
The code word for needs a thread is `unthreaded`; the app shows a stage as an item's Status.
_Avoid_: phase, step

**Status**:
What a panel says of where something stands: an item's stage.

**Any time**:
The threads in no wave: they wait on no thread and none waits on them.

## Relationships between items

**Waits for**:
An item waits for another when it can't start until that one is done: `item edit --waits-for`. It is the only
relationship between items besides their parents.
_Avoid_: blocker link, depends_on, blocks (in prose), hard dependency, relates to

Stored data keeps the older kind name `blocks` for a waits-for link.

**Dangling reference**:
A parent, thread, repository or waits-for target that names a record that doesn't exist or was deleted. It lands on
the needs-you list, and an item whose thread dangles counts as in no thread.
_Avoid_: broken link, orphan

## Where the data lives

**Tracker**:
The repository that holds one trust domain's items and their changes; it is not where project knowledge is kept.
_Avoid_: board, project (for the repository)

**Record**:
A stored entry in the tracker, such as an item, thread or repository, named by its id.

**Session**:
The server token issued by `acyclic up` for access to the app and API. The stored actor kind `session/<id>` keeps
its older name; it identifies who acts, not a server token.

**Replica**:
One machine's copy of a tracker, shared by every agent on that machine.
_Avoid_: instance, node

**Conflict**:
A field holding two values that replicas wrote without seeing each other's, kept side by side until someone writes
the field again. Clearing a field counts as a value, so a clear and a set made apart conflict too: moving an item to
the top level while an agent moves it elsewhere asks which move stands. Sets of things, such as waits-for and
repositories, don't conflict: an add the remover never saw survives.
_Avoid_: merge conflict (nothing blocks a sync)

## Relationships

- A **Tracker** holds one **Trust domain**'s **Items**; each machine has one **Replica** of it
- An **Item** has at most one parent **Item**, and at most one **Thread**
- A **Thread** may hold **Items** from different branches of the outcome tree
- A **Thread** is the unit of work: one agent holds a **Claim** on it, and an item in no thread is never ready
- **Waits for** never forms a cycle

## Planned

These terms describe planned capabilities, not features implemented in the tracker, CLI, server or app.

**Spike**:
An item whose output is evidence for a decision rather than a deliverable.
_Avoid_: recon, research task

**Card**:
A one-shot task the intake chat suggests, which the user starts, keeps or dismisses with a tap.
_Avoid_: suggestion, ticket

**Run**:
One stretch of an agent working on a thread, from launch to stop; a thread's runs happen one at a time, each
continuing the same conversation.
_Avoid_: session, job, deployment

**Brief**:
What an agent receives when a thread is deployed: the work and its context.
_Avoid_: orders, prompt (for the whole brief)

**Dispatcher**:
The part of acyclic that starts runs of ready threads on its own, within limits the user sets.
_Avoid_: scheduler, project manager

**Intake**:
Adding work by describing it to an agent in a chat, which files it, places it and groups it into threads.
_Avoid_: triage, inbox

**Unfiled chat**:
An agent conversation that belongs to no thread yet; filing it makes it an item.
_Avoid_: loose chat, orphan

**Current-state document**:
A short Markdown document describing how things are now, rewritten in place rather than appended to.
_Avoid_: living doc, wiki page

**Trust domain**:
A set of projects whose items may share a tracker because the same people may see all of them.

**History document**:
A dated Markdown document of history (a decision, an experiment, an outcome), only ever added to.
_Avoid_: record (that is a stored tracker entry), log, ADR (for the general idea)

## Flagged ambiguities

- "campaign": the planning skills use it for a whole planned effort ending at a final gate. Resolved: in acyclic that
  is a **Project**; "campaign" stays the skills' word.
- "project": a top-level **Item** in acyclic, but a registered checkout in Paseo. Resolved: "project" means the
  acyclic sense here; say "Paseo project" for the other.
- "thread" and "workstream": earlier drafts said workstream. Resolved: **Thread**, the planning skills' word too.
- "thread" and a chat: in chat apps a thread is one conversation. Here a **Thread** is one continuing conversation
  across its **Runs**, so the two senses agree.
