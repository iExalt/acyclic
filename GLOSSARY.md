# acyclic

acyclic is a Git-native issue tracker for one person and many agents, where work forms a graph and agents pick up
coherent pieces of it.

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
Whether an item or workstream can start now, given its dependencies, claims and resources.
_Avoid_: status (for this), unblocked

**Placement**:
Choosing where a new item belongs: its parent, its links and its workstream.
_Avoid_: triage, filing

**Split**:
Dividing a set of items into workstreams, or one workstream into several.
_Avoid_: partition, breakdown

**Workstream**:
A bounded piece of work that one agent chat works through from start to finish, approved as a unit.
_Avoid_: thread (the planning skills' word for the same thing), track, lane, operation

**Provisional workstream**:
A workstream acyclic or an agent proposed that the user has not yet approved.
_Avoid_: draft workstream, suggested workstream

**Proposed member**:
An item added to an approved or running workstream that waits for the user's approval.

**Brief**:
What an agent receives when a workstream is deployed: the work, its context and its gates.
_Avoid_: orders, prompt (for the whole brief)

**Claim**:
An agent's time-limited hold on an item or workstream it is working on.
_Avoid_: lock, assignment

**Resource**:
Something only one holder may use at a time, such as a file area, a GPU, a host or the user's attention.
_Avoid_: mutex, exclusion

**In review**:
The state of finished code work whose commit is not yet on its repository's default branch.

**Needs-you list**:
Everything waiting on the user: approvals, conflicts nobody resolved, and new blockers on approved work.
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
The repository that holds one trust domain's items and history.
_Avoid_: board, project (for the repository)

**Trust domain**:
A set of projects whose items may share a tracker because the same people may see all of them.

**Replica**:
One machine's copy of a tracker, shared by every agent on that machine.
_Avoid_: instance, node

## Relationships

- A **Tracker** holds one **Trust domain**'s **Items**; each machine has one **Replica** of it
- An **Item** has at most one parent **Item**, and at most one **Workstream**
- A **Workstream** may hold **Items** from different branches of the outcome tree
- A **Goal** is an **Item** whose **Gate** must pass before it is done
- **Hard dependencies** and **Soft dependencies** together never form a cycle

## Flagged ambiguities

- "campaign": the planning skills use it for a whole planned effort ending at a final gate. Resolved: in acyclic that
  is a **Goal**; "campaign" stays the skills' word.
- "project": a top-level **Item** in acyclic, but a registered checkout in Paseo. Resolved: "project" means the
  acyclic sense here; say "Paseo project" for the other.
- "thread" and "workstream": the planning skills say thread. Resolved: **Workstream**.
