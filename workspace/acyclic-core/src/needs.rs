//! The needs-you list, computed on read: everything waiting on the user, as the state shows it (conflicts, cycles,
//! contests, clashes, reopened work, dangling references).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::claim;
use crate::model::{self, Model, Value};
use crate::ready::{Readiness, Status, Wait};
use crate::renumber;
use crate::schema::{kind, set};
use crate::state::{Error, Millis, State};

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
    /// Threads wait on each other through `blocks` links between their open members, so not all of them can finish
    /// as they stand.
    ThreadCycle {
        /// The threads on it.
        threads: Vec<String>,
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
    /// An open leaf in no thread, or in one with no record, which is never ready until it joins one.
    Unthreaded {
        /// The item.
        item: String,
        /// The command that puts it in a thread.
        fix: String,
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
            | Self::LinkCycle { .. }
            | Self::ParentCycle { .. }
            | Self::ThreadCycle { .. }
            | Self::Contest { .. }
            | Self::ResourceClash { .. }
            | Self::Reopened { .. }
            | Self::DoneEarly { .. } => true,
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
    for threads in readiness.thread_cycles()? {
        entries.insert(Entry::ThreadCycle { threads: threads.into_iter().collect() });
    }
    claims(state, model, now, &mut entries)?;
    for clash in claim::clashes(state, model, now)? {
        entries.insert(Entry::ResourceClash { resource: clash.resource, holders: clash.holders });
    }
    reopened_and_early(model, &mut entries);
    dangling(model, &mut entries);
    for clash in renumber::clashes(state, model, now)? {
        if let Some(first) = clash.fresh.first() {
            let fix = format!("acyclic thread renumber {first}");
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
    for id in model.threads.keys() {
        let live = claim::live(state, &claim::thread_key(id)?, now)?;
        for held in &live {
            if held.starts_in_future(now) {
                entries
                    .insert(Entry::ClockAhead { thread: id.clone(), holder: held.holder.clone() });
            }
        }
        let holders = claim::holders(&live);
        if holders.len() > 1 {
            entries.insert(Entry::Contest { thread: id.clone(), holders });
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
    for link in &model.links {
        for end in [&link.from, &link.to].into_iter().filter(|end| !model.items.contains_key(*end))
        {
            add(kind::LINK, &format!("{}-{}", link.from, link.to), "end", end);
        }
    }
}

#[cfg(test)]
mod tests;
