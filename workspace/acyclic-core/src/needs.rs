//! The needs-you list, computed on read: everything waiting on the user. What the state shows (conflicts, cycles,
//! contests, clashes, proposals, new blockers on approved work, reopened work, dangling references) always appears;
//! agents' questions are filtered by the thread's autonomy level (plan D61), and blockers always pass.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::claim;
use crate::model::{self, Level, Model, Value};
use crate::place;
use crate::ready::{Readiness, Status, Wait};
use crate::renumber;
use crate::schema::{kind, question, set};
use crate::scope;
use crate::state::{Error, Millis, State};

/// What kind of question an agent asks; the exact classes are settled with the app's mockups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    /// The agent can't continue without an answer.
    Blocker,
    /// About a change of scope.
    Scope,
    /// About an action that can't be undone.
    Irreversible,
    /// Anything else.
    Other,
}

/// Whether a question of `class` reaches the needs-you list at `level`.
#[must_use]
pub fn passes(level: Level, class: Class) -> bool {
    match level {
        Level::Everything => true,
        Level::ScopeAndIrreversible => class != Class::Other,
        Level::Blockers => class == Class::Blocker,
    }
}

/// One entry on the needs-you list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "entry")]
pub enum Entry {
    /// A field holds conflicting values.
    Conflict {
        /// The entity's kind.
        kind: String,
        /// The entity's id.
        id: String,
        /// The field.
        field: String,
    },
    /// A thread with members waits for the user to approve its scope.
    AwaitingApproval {
        /// The thread.
        thread: String,
        /// How many open, unarchived items it holds.
        members: usize,
        /// Threads it waits on in a cycle through `blocks` links, so not all of them can finish as they stand.
        waits_with: Vec<String>,
        /// Its heads: two or more say its order is partly unsaid, so approving it says they may run in any order.
        heads: Vec<String>,
    },
    /// This clone pushes every write, yet holds writes the remote lacks: `acyclic sync` sends them.
    Unpushed {
        /// The remote.
        remote: String,
    },
    /// Blocks links form a cycle.
    LinkCycle {
        /// The items on it.
        items: Vec<String>,
    },
    /// Parents form a cycle.
    ParentCycle {
        /// The items on it.
        items: Vec<String>,
    },
    /// Holders that didn't see each other claim one thread.
    Contest {
        /// The thread.
        thread: String,
        /// The holders, winner first.
        holders: Vec<String>,
    },
    /// Claims on threads whose open members name one resource belong to different holders.
    ResourceClash {
        /// The resource.
        resource: String,
        /// Each thread and its holder.
        holders: Vec<(String, String)>,
    },
    /// A claim's lease starts in the reader's future: the writer's clock ran ahead.
    ClockAhead {
        /// The thread.
        thread: String,
        /// The holder.
        holder: String,
    },
    /// A run's claim names a run with no record.
    UnknownRun {
        /// The thread.
        thread: String,
        /// The holder.
        holder: String,
    },
    /// An open leaf in no thread, or in one with no record, which is never ready until it joins one.
    Unthreaded {
        /// The item.
        item: String,
        /// The command that puts it in a thread.
        fix: String,
    },
    /// A thread has more than one active run.
    RunClash {
        /// The thread.
        thread: String,
        /// Its active runs.
        runs: Vec<String>,
    },
    /// An agent's edit to approved scope awaits keep or drop.
    Proposal {
        /// The proposal.
        id: String,
        /// Who proposed it.
        actor: String,
        /// The approved threads it changes.
        threads: Vec<String>,
    },
    /// An agent's member of an approved thread that the approval never saw: its edit raced the approval.
    Unapproved {
        /// The item.
        item: String,
        /// Its approved thread.
        thread: String,
        /// The agent that put it there.
        actor: String,
        /// The user's command that keeps it.
        keep: String,
        /// The user's command that drops it.
        drop: String,
    },
    /// A blocker added to approved work after its approval.
    NewBlocker {
        /// The blocked item.
        item: String,
        /// Its approved thread.
        thread: String,
        /// The blocking item.
        blocker: String,
    },
    /// A reopened item, with the items that depend on it.
    Reopened {
        /// The item.
        item: String,
        /// Its dependents, by waits-for link.
        dependents: Vec<String>,
    },
    /// A parent marked done while children are open.
    DoneEarly {
        /// The parent.
        item: String,
        /// Its open children.
        open: Vec<String>,
    },
    /// A reference to something that doesn't exist.
    Dangling {
        /// The referring entity's kind.
        kind: String,
        /// Its id.
        id: String,
        /// The referring field.
        field: String,
        /// The missing target.
        target: String,
    },
    /// Threads whose ids end in the same number, so the app draws them alike; renumbering a fresh one resolves it.
    /// Listed only while one is fresh, since only then is there something to do.
    NumberClash {
        /// The number.
        number: u64,
        /// The threads.
        threads: Vec<String>,
        /// Those that may be renumbered.
        fresh: Vec<String>,
        /// The command that resolves it.
        fix: String,
    },
    /// A dependency's merge can't be checked.
    Unverified {
        /// The dependency.
        item: String,
        /// Its repository record.
        repository: String,
        /// Why not.
        reason: String,
    },
    /// An agent's question that passes its thread's autonomy level.
    Question {
        /// The question.
        id: String,
        /// The thread asking.
        thread: Option<String>,
        /// Its class.
        class: Class,
        /// The question.
        text: String,
    },
}

impl Entry {
    /// Whether the user's devices are told when it first appears. Standing or housekeeping states (a clone with
    /// writes to push, a clock that ran ahead, a reference to nothing, a merge that can't be checked) and items in no
    /// thread, which agents leave routinely, stay on the list without a notification. With no catch-all arm, a new
    /// kind fails to compile until it is placed here.
    #[must_use]
    pub const fn notifies(&self) -> bool {
        match self {
            Self::Conflict { .. }
            | Self::AwaitingApproval { .. }
            | Self::LinkCycle { .. }
            | Self::ParentCycle { .. }
            | Self::Contest { .. }
            | Self::ResourceClash { .. }
            | Self::UnknownRun { .. }
            | Self::RunClash { .. }
            | Self::Proposal { .. }
            | Self::Unapproved { .. }
            | Self::NewBlocker { .. }
            | Self::Reopened { .. }
            | Self::DoneEarly { .. }
            | Self::Question { .. } => true,
            Self::Unpushed { .. }
            | Self::ClockAhead { .. }
            | Self::Unthreaded { .. }
            | Self::Dangling { .. }
            | Self::NumberClash { .. }
            | Self::Unverified { .. } => false,
        }
    }
}

/// The needs-you list at `now`, sorted.
///
/// # Errors
///
/// Fails on an unreadable value.
pub fn needs_you(
    state: &State,
    model: &Model,
    readiness: &Readiness<'_>,
    now: Millis,
) -> Result<Vec<Entry>, Error> {
    let mut entries = BTreeSet::new();
    conflicts(state, &mut entries);
    for items in model.link_cycles() {
        entries.insert(Entry::LinkCycle { items });
    }
    for items in model.parent_cycles() {
        entries.insert(Entry::ParentCycle { items });
    }
    claims(state, model, now, &mut entries)?;
    for clash in claim::clashes(state, model, now)? {
        entries.insert(Entry::ResourceClash { resource: clash.resource, holders: clash.holders });
    }
    for (thread, runs) in model.run_clashes() {
        entries.insert(Entry::RunClash { thread, runs });
    }
    for proposal in scope::proposals(state)?.into_values() {
        if proposal.decision == Value::None {
            let threads =
                proposal.edit.one().map(|e| e.approved_threads(model)).unwrap_or_default();
            let actor = proposal.actor.one().cloned().unwrap_or_default();
            entries.insert(Entry::Proposal { id: proposal.id, actor, threads });
        }
    }
    for found in scope::unapproved(state, model)? {
        entries.insert(Entry::Unapproved {
            keep: format!("acyclic item edit {} --thread {}", found.item, found.thread),
            drop: format!("acyclic item edit {} --no-thread", found.item),
            item: found.item,
            thread: found.thread,
            actor: found.actor,
        });
    }
    new_blockers(state, model, readiness, &mut entries)?;
    awaiting_approval(model, &mut entries);
    reopened_and_early(model, &mut entries);
    dangling(model, &mut entries);
    for clash in renumber::clashes(state, model, now)? {
        if let Some(first) = clash.fresh.first() {
            let fix = format!("acyclic thread renumber {first} --as-user");
            entries.insert(Entry::NumberClash {
                number: clash.number,
                threads: clash.threads,
                fresh: clash.fresh,
                fix,
            });
        }
    }
    for (id, item) in &model.items {
        if item.is_done() || !model.children(id).is_empty() {
            continue;
        }
        unthreaded(model, item, &mut entries);
        if let Some(Status::Blocked { waiting }) = readiness.of(id)? {
            for (dependency, wait) in waiting {
                if let Wait::Unverified { repository, reason } = wait {
                    entries.insert(Entry::Unverified { item: dependency, repository, reason });
                }
            }
        }
    }
    questions(state, model, &mut entries)?;
    Ok(entries.into_iter().collect())
}

/// An open, unarchived, unparked leaf that no thread with a record holds, whatever else holds it back: `ready` never
/// lists it. A conflicted thread is left to its conflict entry.
fn unthreaded(model: &Model, item: &model::Item, entries: &mut BTreeSet<Entry>) {
    let parked = item.state == Value::One(model::ItemState::Parked);
    if parked || item.thread.is_conflict() || model.is_archived(&item.id) {
        return;
    }
    if item.thread.one().is_some_and(|thread| model.threads.contains_key(thread)) {
        return;
    }
    entries.insert(Entry::Unthreaded {
        fix: format!("acyclic item edit {} --thread <THREAD>", item.id),
        item: item.id.clone(),
    });
}

/// Unapproved threads with open work, each with the threads it would deadlock with. Finished or archived members
/// need no decision, since no run will take them.
fn awaiting_approval(model: &Model, entries: &mut BTreeSet<Entry>) {
    let mut members: BTreeMap<&str, usize> = BTreeMap::new();
    for item in model
        .items
        .values()
        .filter(|item| !model.reads_done(&item.id) && !model.is_archived(&item.id))
    {
        if let Some(thread) = item.thread.one() {
            *members.entry(thread).or_default() += 1;
        }
    }
    let cycles = place::thread_cycles(model);
    let heads = model.heads_index();
    for (id, thread) in &model.threads {
        let Some(&count) = members.get(id.as_str()) else { continue };
        if thread.is_approved() {
            continue;
        }
        let waits_with: BTreeSet<&String> = cycles
            .iter()
            .filter(|cycle| cycle.contains(id))
            .flatten()
            .filter(|other| *other != id)
            .collect();
        entries.insert(Entry::AwaitingApproval {
            thread: id.clone(),
            members: count,
            waits_with: waits_with.into_iter().cloned().collect(),
            heads: heads.of(id).into_iter().map(str::to_owned).collect(),
        });
    }
}

/// Fields with differing values, except claims (contests) and set elements (always `true`).
fn conflicts(state: &State, entries: &mut BTreeSet<Entry>) {
    for (key, entity) in state.entities() {
        for (name, register) in entity {
            if name == claim::FIELD
                || [set::AREA, set::REPOSITORY, set::RESOURCE].iter().any(|p| name.starts_with(p))
            {
                continue;
            }
            let distinct: BTreeSet<String> =
                register.values().map(|v| v.value.to_string()).collect();
            if distinct.len() > 1 {
                entries.insert(Entry::Conflict {
                    kind: key.kind.clone(),
                    id: key.id.clone(),
                    field: name.clone(),
                });
            }
        }
    }
}

fn claims(
    state: &State,
    model: &Model,
    now: Millis,
    entries: &mut BTreeSet<Entry>,
) -> Result<(), Error> {
    let alive = |holder: &str| model.run_alive(holder);
    for id in model.threads.keys() {
        let live = claim::live_with(state, &claim::thread_key(id)?, now, &alive)?;
        let mut holders: Vec<String> = Vec::new();
        for held in &live {
            if !holders.contains(&held.holder) {
                holders.push(held.holder.clone());
            }
            if held.starts_in_future(now) {
                entries
                    .insert(Entry::ClockAhead { thread: id.clone(), holder: held.holder.clone() });
            }
            let unknown = held.lease_expires.is_none()
                && held
                    .holder
                    .strip_prefix("run/")
                    .is_some_and(|run| !model.runs.contains_key(run));
            if unknown {
                entries
                    .insert(Entry::UnknownRun { thread: id.clone(), holder: held.holder.clone() });
            }
        }
        if holders.len() > 1 {
            entries.insert(Entry::Contest { thread: id.clone(), holders });
        }
    }
    Ok(())
}

/// Unmet blocks links from items in approved threads, added after the thread's approval.
fn new_blockers(
    state: &State,
    model: &Model,
    readiness: &Readiness<'_>,
    entries: &mut BTreeSet<Entry>,
) -> Result<(), Error> {
    for thread in model.threads.keys() {
        for (item, blocker) in scope::blockers_since_approval(state, model, thread)? {
            let unmet = matches!(readiness.of(item)?, Some(Status::Blocked { waiting }) if waiting.contains_key(blocker));
            if unmet {
                entries.insert(Entry::NewBlocker {
                    item: item.to_owned(),
                    thread: thread.clone(),
                    blocker: blocker.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn reopened_and_early(model: &Model, entries: &mut BTreeSet<Entry>) {
    for (id, item) in &model.items {
        if item.reopened != Value::None {
            let dependents: Vec<String> = model
                .links
                .iter()
                .filter(|l| l.to == *id && l.kind.orders() && !model.is_archived(&l.from))
                .map(|l| l.from.clone())
                .collect();
            entries.insert(Entry::Reopened { item: id.clone(), dependents });
        }
        if item.is_done() {
            let open: Vec<String> = model
                .children(id)
                .into_iter()
                .filter(|child| !model.items[*child].is_done() && !model.is_archived(child))
                .map(str::to_owned)
                .collect();
            if !open.is_empty() {
                entries.insert(Entry::DoneEarly { item: id.clone(), open });
            }
        }
    }
}

fn dangling(model: &Model, entries: &mut BTreeSet<Entry>) {
    let mut add = |kind: &str, id: &str, field: &str, target: &str| {
        entries.insert(Entry::Dangling {
            kind: kind.to_owned(),
            id: id.to_owned(),
            field: field.to_owned(),
            target: target.to_owned(),
        });
    };
    for (id, item) in &model.items {
        for parent in item.parent.all().into_iter().filter(|p| !model.items.contains_key(*p)) {
            add(kind::ITEM, id, "parent", parent);
        }
        for thread in item.thread.all().into_iter().filter(|t| !model.threads.contains_key(*t)) {
            add(kind::ITEM, id, "thread", thread);
        }
        for repository in item.repositories.iter().filter(|r| !model.repositories.contains_key(*r))
        {
            add(kind::ITEM, id, "repository", repository);
        }
    }
    for (id, thread) in &model.threads {
        for repository in
            thread.repositories.iter().filter(|r| !model.repositories.contains_key(*r))
        {
            add(kind::THREAD, id, "repository", repository);
        }
    }
    for (id, run) in &model.runs {
        for thread in run.thread.all().into_iter().filter(|t| !model.threads.contains_key(*t)) {
            add(kind::RUN, id, "thread", thread);
        }
    }
    for link in &model.links {
        for end in [&link.from, &link.to].into_iter().filter(|end| !model.items.contains_key(*end))
        {
            add(kind::LINK, &format!("{}-{}", link.from, link.to), "end", end);
        }
    }
}

fn questions(state: &State, model: &Model, entries: &mut BTreeSet<Entry>) -> Result<(), Error> {
    for (key, _) in state.entities().filter(|(key, _)| key.kind == kind::QUESTION) {
        let open: Value<bool> = model::value(state, key, question::OPEN)?;
        if !open.all().into_iter().any(|open| *open) {
            continue;
        }
        let thread: Value<String> = model::value(state, key, question::THREAD)?;
        let class: Value<Class> = model::value(state, key, question::CLASS)?;
        // An unclear class is treated as a blocker, so it is never filtered away.
        let class = class.one().copied().unwrap_or(Class::Blocker);
        let thread = thread.one().cloned();
        if passes(model.level(thread.as_deref()), class) {
            let text: Value<String> = model::value(state, key, question::TEXT)?;
            let text = text.all().into_iter().cloned().collect::<Vec<_>>().join(" / ");
            entries.insert(Entry::Question { id: key.id.clone(), thread, class, text });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
