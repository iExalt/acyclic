# acyclic

acyclic is a Git-native issue tracker for one person and many agents, where work forms a graph and agents pick up
coherent pieces of it, plus the app and the agent runs that carry that work out.

## Work

**Item**:
One unit of tracked work, from a single task to a whole project; items nest to any depth.
_Avoid_: issue, task, ticket, battle

**Goal**:
An item with children and a gate.
_Avoid_: milestone, epic, campaign

**Group**:
An item with children and no gate, such as a side track that is never finished by design.
_Avoid_: epic, category

**Project**:
A top-level item with children; a top-level item is a project even when it also has a gate.
_Avoid_: theatre, repository, phase

**Gate**:
The exit check an item must pass to be done. Not enforced yet: `done` doesn't check it. Today it is free text that sets
the item's role and delays archiving a finished subtree.
_Avoid_: acceptance criteria (as a separate concept), exit criteria

**Area**:
A long-lived topic tag on items, independent of the outcome tree.
_Avoid_: track, front, sector, component

**Spike**:
An item whose output is evidence for a decision rather than a deliverable.
_Avoid_: recon, research task

## Execution

**Readiness**:
Whether an item or thread can start now, given its dependencies, claims and resources.
_Avoid_: status (for readiness alone), unblocked

**Blocker**:
Something that stops approved work going ahead, which the user must clear; at the most autonomous level, agents ask
about nothing else.
_Avoid_: blocking issue

**Placement**:
Choosing where a new item belongs: its parent, its links and its thread.
_Avoid_: triage, filing

**Split**:
Dividing a set of items into threads, or one thread into several.
_Avoid_: partition, breakdown

**Thread**:
A bounded piece of work that one continuing agent conversation works through from start to finish, approved as a
unit.
_Avoid_: workstream, track, lane, operation

**Wave**:
A step in the order threads can start: each thread with work left comes after every thread its work waits on, and a
thread that waits on none and that none waits on is in no wave and can start any time.
_Avoid_: round, tier

**Provisional thread**:
A thread an agent or an import created that the user has not yet approved.
_Avoid_: draft thread, suggested thread

**Proposal**:
An agent's change to an approved thread's scope, waiting for the user to keep or drop it; a proposed member is one.
_Avoid_: suggestion, pending change

**Proposed member**:
An item an agent added to an approved or running thread, waiting for the user's approval; the user's own changes
apply at once. An agent's add that crossed the approval between machines is listed in
the same way, kept by the user adding it to the thread again or, all at once, by approving the thread again.

**Card**:
A one-shot task the intake chat suggests, which the user starts, keeps or dismisses with a tap.
_Avoid_: suggestion, ticket

**Autonomy level**:
The rule for what reaches the user from runs, set globally and overridable per thread.
_Avoid_: trust level, mode (for this)

**Run**:
One stretch of an agent working on a thread, from launch to stop; a thread's runs happen one at a time, each
continuing the same conversation.
_Avoid_: session, job, deployment

**Brief**:
What an agent receives when a thread is deployed: the work, its context and its gates.
_Avoid_: orders, prompt (for the whole brief)

**Claim**:
An agent's time-limited hold on a thread it is working on. The holder and its subagents work every member, and the
claim holds the resources the thread's open members name.
_Avoid_: lock, assignment

**Dispatcher**:
The part of acyclic that starts runs of approved, ready threads on its own, within limits the user sets.
_Avoid_: scheduler, project manager

**Intake**:
Adding work by describing it to an agent in a chat, which files it, places it and groups it into threads.
_Avoid_: triage, inbox

**Unfiled chat**:
An agent conversation that belongs to no thread yet; filing it makes it an item.
_Avoid_: loose chat, orphan

**Resource**:
Something only one holder may use at a time, such as a file area, a GPU, a host or the user's attention.
_Avoid_: mutex, exclusion

**Archived**:
A finished subtree hidden from the graph, readiness and the dispatcher by default; archiving hides, never deletes.
_Avoid_: closed, deleted

**In review**:
The state of finished code work whose commit is not yet on its repository's default branch.

**Needs-you list**:
Everything waiting on the user: approvals, proposals and late members, agents' questions and permission prompts
(nothing records either yet), conflicts nobody resolved, new blockers on approved work, resources or numbers two
holders clash over, references to what doesn't exist, merges that can't be confirmed, and items in no thread. The app
calls it Needs you, and shows it as a view of its own.
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

**Stage**:
Where an item's work stands, said one way everywhere with its icon: in progress, in review, done, ready, blocked,
parked, or needs a thread; a parent's is its leaves'. The code's word: the app shows a stage as an item's Status.
_Avoid_: phase, step

**Status**:
What a panel says of where something stands: an item's stage, or a thread's approval.

**Any time**:
The threads in no wave: they wait on no thread and none waits on them.

## Relationships between items

**Hard dependency**:
A dependency that blocks the dependent item until the prerequisite is met; link kind `waits-for`.
_Avoid_: blocker link, depends_on, blocks (in prose)

**Related**:
A link that carries context only; link kind `relates-to`.
_Avoid_: relates_to, relates (in prose), see also

Stored data keeps the older kind names: `blocks` for `waits-for` and `relates` for `relates-to`.

## Where the data lives

**Tracker**:
The repository that holds one trust domain's items and their changes; it is not where project knowledge is kept.
_Avoid_: board, project (for the repository)

**Record**:
A dated Markdown document of history (a decision, an experiment, an outcome), only ever added to.
_Avoid_: log, ADR (for the general idea)

**Current-state document**:
A short Markdown document describing how things are now, rewritten in place rather than appended to.
_Avoid_: living doc, wiki page

**Trust domain**:
A set of projects whose items may share a tracker because the same people may see all of them.

**Replica**:
One machine's copy of a tracker, shared by every agent on that machine.
_Avoid_: instance, node

**Conflict**:
A field holding two values that replicas wrote without seeing each other's, kept side by side until someone writes
the field again. Clearing a field counts as a value, so a clear and a set made apart conflict too: moving an item to
the top level while an agent moves it elsewhere asks which move stands. Sets of things, such as links and areas, don't
conflict: an add the remover never saw survives.
_Avoid_: merge conflict (nothing blocks a sync)

## Relationships

- A **Tracker** holds one **Trust domain**'s **Items**; each machine has one **Replica** of it
- An **Item** has at most one parent **Item**, and at most one **Thread**
- A **Thread** may hold **Items** from different branches of the outcome tree
- A **Thread** has one **Run** at a time; each **Run** continues the thread's conversation
- A **Thread** is the unit of work: one agent holds a **Claim** on it, and an item in no thread is never ready
- A **Goal** is an **Item** whose **Gate** must pass before it is done
- **Hard dependencies** (`waits-for`) never form a cycle

## Flagged ambiguities

- "campaign": the planning skills use it for a whole planned effort ending at a final gate. Resolved: in acyclic that
  is a **Goal**; "campaign" stays the skills' word.
- "project": a top-level **Item** in acyclic, but a registered checkout in Paseo. Resolved: "project" means the
  acyclic sense here; say "Paseo project" for the other.
- "thread" and "workstream": earlier drafts said workstream. Resolved: **Thread**, the planning skills' word too.
- "thread" and a chat: in chat apps a thread is one conversation. Here a **Thread** is one continuing conversation
  across its **Runs**, so the two senses agree.
