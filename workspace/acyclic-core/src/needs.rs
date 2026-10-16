//! The needs-you list, computed on read: everything waiting on the user. What the state shows (conflicts, cycles,
//! contests, clashes, proposals, new blockers on approved work, reopened work, dangling references) always appears;
//! agents' questions are filtered by the thread's autonomy level (plan D61), and blockers always pass.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::claim;
use crate::model::{self, Level, Model, Value};
use crate::place;
use crate::ready::{Readiness, Status, Wait};
use crate::schema::{kind, link, question, set, thread};
use crate::scope;
use crate::state::{Error, Key, Millis, State};

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
    },
    /// This clone pushes every write, yet holds writes the remote lacks: `acyclic sync` sends them.
    Unpushed {
        /// The remote.
        remote: String,
    },
    /// Links of blocks and after kinds form a cycle.
    LinkCycle {
        /// The items on it.
        items: Vec<String>,
    },
    /// Parents form a cycle.
    ParentCycle {
        /// The items on it.
        items: Vec<String>,
    },
    /// Holders that didn't see each other claim one item.
    Contest {
        /// The item.
        item: String,
        /// The holders, winner first.
        holders: Vec<String>,
    },
    /// Claims on items sharing a named resource belong to different holders.
    ResourceClash {
        /// The resource.
        resource: String,
        /// Each item and its holder.
        holders: Vec<(String, String)>,
    },
    /// A claim's lease starts in the reader's future: the writer's clock ran ahead.
    ClockAhead {
        /// The item.
        item: String,
        /// The holder.
        holder: String,
    },
    /// A run's claim names a run with no record.
    UnknownRun {
        /// The item.
        item: String,
        /// The holder.
        holder: String,
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
        /// Its dependents, by hard or soft link.
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
    new_blockers(state, model, readiness, &mut entries)?;
    awaiting_approval(model, &mut entries);
    reopened_and_early(model, &mut entries);
    dangling(model, &mut entries);
    for (id, item) in &model.items {
        if item.is_done() || !model.children(id).is_empty() {
            continue;
        }
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

/// Unapproved threads with open work, each with the threads it would deadlock with. Finished or archived members
/// need no decision, since no run will take them.
fn awaiting_approval(model: &Model, entries: &mut BTreeSet<Entry>) {
    let mut members: BTreeMap<&str, usize> = BTreeMap::new();
    for item in model.items.values().filter(|item| !item.is_done() && !model.is_archived(&item.id))
    {
        if let Some(thread) = item.thread.one() {
            *members.entry(thread).or_default() += 1;
        }
    }
    let cycles = place::thread_cycles(model);
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
    for id in model.items.keys() {
        let key = Key::new(kind::ITEM, id)?;
        let live = claim::live_with(state, &key, now, &alive)?;
        let mut holders: Vec<String> = Vec::new();
        for held in &live {
            if !holders.contains(&held.holder) {
                holders.push(held.holder.clone());
            }
            if held.starts_in_future(now) {
                entries.insert(Entry::ClockAhead { item: id.clone(), holder: held.holder.clone() });
            }
            let unknown = held.lease_expires.is_none()
                && held
                    .holder
                    .strip_prefix("run/")
                    .is_some_and(|run| !model.runs.contains_key(run));
            if unknown {
                entries.insert(Entry::UnknownRun { item: id.clone(), holder: held.holder.clone() });
            }
        }
        if holders.len() > 1 {
            entries.insert(Entry::Contest { item: id.clone(), holders });
        }
    }
    Ok(())
}

/// Unmet blocks links from items in approved threads, added after the thread's approval. The two are compared by
/// their writers' stamps, so a blocker added within the clock guard's skew of an approval on another replica may be
/// misordered; re-approving a thread acknowledges the blockers before it.
fn new_blockers(
    state: &State,
    model: &Model,
    readiness: &Readiness<'_>,
    entries: &mut BTreeSet<Entry>,
) -> Result<(), Error> {
    let latest = |key: &Key, field: &str| {
        state.field(key, field).into_iter().flatten().map(|(_, v)| v.at).max()
    };
    for link in model.links.iter().filter(|l| l.kind == model::LinkKind::Blocks) {
        let Some(thread_id) = model.thread_of(&link.from) else { continue };
        if !model.threads.get(thread_id).is_some_and(model::Thread::is_approved) {
            continue;
        }
        let approved_at = latest(&Key::new(kind::THREAD, thread_id)?, thread::APPROVED);
        let link_key = model::link_key(&link.from, &link.to, link.kind)
            .map_err(|_| Error::InvalidName(link.from.clone()))?;
        let linked_at = latest(&link_key, link::PRESENT);
        let unmet = matches!(readiness.of(&link.from)?, Some(Status::Blocked { waiting }) if waiting.contains_key(&link.to));
        if unmet && linked_at > approved_at {
            entries.insert(Entry::NewBlocker {
                item: link.from.clone(),
                thread: thread_id.to_owned(),
                blocker: link.to.clone(),
            });
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
