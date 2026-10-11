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
_Avoid_: theatre, repository

**Gate**:
The exit check an item must pass to be done.
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
_Avoid_: status (for this), unblocked

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

**Provisional thread**:
A thread acyclic or an agent proposed that the user has not yet approved.
_Avoid_: draft thread, suggested thread

**Proposed member**:
An item an agent added to an approved or running thread, waiting for the user's approval; the user's own changes and
a project-management agent's apply at once.

**Project-management agent**:
An agent the user deployed to change the graph's structure, whose edits apply like the user's.

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
An agent's time-limited hold on an item or thread it is working on.
_Avoid_: lock, assignment

**Dispatcher**:
The part of acyclic that starts runs of approved, ready threads on its own, within limits the user sets.
_Avoid_: scheduler, project manager

**Intake**:
Adding work by describing it to an agent in a chat, which files it, places it and splits it.
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
Everything waiting on the user: approvals, agents' questions and permission prompts, conflicts nobody resolved, and new
blockers on approved work.
_Avoid_: inbox, notifications

## Relationships between items

**Hard dependency**:
A dependency that blocks the dependent item until the prerequisite is met.
_Avoid_: blocker link, depends_on

**Soft dependency**:
A preferred order between two items that never blocks.
_Avoid_: after, weak dependency

**Related**:
A link that carries context only.
_Avoid_: relates_to, see also

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

## Relationships

- A **Tracker** holds one **Trust domain**'s **Items**; each machine has one **Replica** of it
- An **Item** has at most one parent **Item**, and at most one **Thread**
- A **Thread** may hold **Items** from different branches of the outcome tree
- A **Thread** has one **Run** at a time; each **Run** continues the thread's conversation
- A **Goal** is an **Item** whose **Gate** must pass before it is done
- **Hard dependencies** and **Soft dependencies** together never form a cycle

## Flagged ambiguities

- "campaign": the planning skills use it for a whole planned effort ending at a final gate. Resolved: in acyclic that
  is a **Goal**; "campaign" stays the skills' word.
- "project": a top-level **Item** in acyclic, but a registered checkout in Paseo. Resolved: "project" means the
  acyclic sense here; say "Paseo project" for the other.
- "thread" and "workstream": earlier drafts said workstream. Resolved: **Thread**, the planning skills' word too.
- "thread" and a chat: in chat apps a thread is one conversation. Here a **Thread** is one continuing conversation
  across its **Runs**, so the two senses agree.
