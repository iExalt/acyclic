//! The needs-you list, computed on read: everything waiting on the user, as the state shows it (conflicts, cycles,
//! contests, reopened work, dangling references).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::claim;
use crate::model::{self, Model, Value};
use crate::ready::{Readiness, ReadinessEvaluator, Wait};
use crate::schema::kind;
use crate::state::{Error, Millis, State};

mod keep;
pub use keep::{Keep, Kept, Unkept, keeps};

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
        /// The command that removes the reference, when one can.
        #[serde(skip_serializing_if = "Option::is_none")]
        fix: Option<String>,
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
    /// A stable, readable key: its kind and what it is about, joined by `:`, so the same need on the next look is
    /// known as the same. Lists that change while the need stands, such as a contest's holders or a reopened item's
    /// dependents, stay out of it. Ids and field names hold no `:`.
    #[must_use]
    pub fn key(&self) -> String {
        let parts: Vec<&str> = match self {
            Self::Conflict { kind, id, field } => vec!["conflict", kind, id, field],
            Self::Unpushed { remote } => vec!["unpushed", remote],
            Self::LinkCycle { items } => {
                std::iter::once("link-cycle").chain(items.iter().map(String::as_str)).collect()
            },
            Self::ParentCycle { items } => {
                std::iter::once("parent-cycle").chain(items.iter().map(String::as_str)).collect()
            },
            Self::ThreadCycle { threads } => {
                std::iter::once("thread-cycle").chain(threads.iter().map(String::as_str)).collect()
            },
            Self::Contest { thread, holders: _ } => vec!["contest", thread],
            Self::ClockAhead { thread, holder } => vec!["clock-ahead", thread, holder],
            Self::Unthreaded { item, fix: _ } => vec!["unthreaded", item],
            Self::Reopened { item, dependents: _ } => vec!["reopened", item],
            Self::DoneEarly { item, open: _ } => vec!["done-early", item],
            Self::Dangling { kind, id, field, target, fix: _ } => {
                vec!["dangling", kind, id, field, target]
            },
            Self::Unverified { item, repository, reason: _ } => {
                vec!["unverified", item, repository]
            },
        };
        parts.join(":")
    }

    /// A compact canonical identity for this exact core entry, before a CLI or app adds display details.
    ///
    /// # Panics
    ///
    /// Never: every entry field is a string, an optional string, or a vector of strings, and always serializes.
    #[must_use]
    pub fn identity(&self) -> String {
        serde_json::to_string(self).expect("need entries serialize")
    }

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
            | Self::Reopened { .. }
            | Self::DoneEarly { .. } => true,
            Self::Unpushed { .. }
            | Self::ClockAhead { .. }
            | Self::Unthreaded { .. }
            | Self::Dangling { .. }
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
    readiness: &ReadinessEvaluator<'_>,
    now: Millis,
) -> Result<Vec<Entry>, Error> {
    let mut entries = BTreeSet::new();
    for (kind, id, field) in model.conflicts() {
        entries.insert(Entry::Conflict {
            kind: kind.to_owned(),
            id: id.to_owned(),
            field: field.to_owned(),
        });
    }
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
    reopened_and_early(model, &mut entries);
    dangling(model, &mut entries);
    for (id, item) in &model.items {
        if item.is_done() || !model.children(id).is_empty() {
            continue;
        }
        unthreaded(model, item, &mut entries);
        if let Some(Readiness::Blocked { waiting }) = readiness.of(id)? {
            for (dependency, wait) in waiting {
                if let Wait::Unverified { repository, reason } = wait {
                    entries.insert(Entry::Unverified { item: dependency, repository, reason });
                }
            }
        }
    }
    Ok(entries.into_iter().collect())
}

/// Whether an item is an open, unarchived, unparked leaf whose thread isn't in conflict: one `ready` lists only
/// while a thread with a record holds it. A conflicted thread is left to its conflict entry.
fn wants_thread(model: &Model, item: &model::Item) -> bool {
    !item.is_done()
        && model.children(&item.id).is_empty()
        && item.state != Value::One(model::ItemState::Parked)
        && !item.thread.is_conflict()
        && !model.is_archived(&item.id)
}

/// A leaf that wants a thread and names none, whatever else holds it back: `ready` never lists it. One that names a
/// thread with no record is left to its dangling entry, which says so.
fn unthreaded(model: &Model, item: &model::Item, entries: &mut BTreeSet<Entry>) {
    if !wants_thread(model, item) || item.thread.one().is_some() {
        return;
    }
    entries.insert(Entry::Unthreaded {
        fix: format!("acyclic item edit {} --thread <THREAD>", item.id),
        item: item.id.clone(),
    });
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
                .filter(|l| l.to == *id && !model.is_archived(&l.from))
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
    let mut add = |kind: &str, id: &str, field: &str, target: &str, fix: Option<String>| {
        entries.insert(Entry::Dangling {
            kind: kind.to_owned(),
            id: id.to_owned(),
            field: field.to_owned(),
            target: target.to_owned(),
            fix,
        });
    };
    for (id, item) in &model.items {
        for parent in item.parent.all().into_iter().filter(|p| !model.items.contains_key(*p)) {
            add(kind::ITEM, id, "parent", parent, None);
        }
        for thread in item.thread.all().into_iter().filter(|t| !model.threads.contains_key(*t)) {
            let fix = wants_thread(model, item)
                .then(|| format!("acyclic item edit {id} --thread <THREAD>"));
            add(kind::ITEM, id, "thread", thread, fix);
        }
        for repository in item.repositories.iter().filter(|r| !model.repositories.contains_key(*r))
        {
            add(kind::ITEM, id, "repository", repository, None);
        }
    }
    for (id, thread) in &model.threads {
        for repository in
            thread.repositories.iter().filter(|r| !model.repositories.contains_key(*r))
        {
            add(kind::THREAD, id, "repository", repository, None);
        }
    }
    // A waiting item names what it waits for, and can let go of it; a link from an item that is gone is the link's.
    for link in &model.links {
        if !model.items.contains_key(&link.from) {
            add(kind::LINK, &format!("{}-{}", link.from, link.to), "end", &link.from, None);
        }
        if !model.items.contains_key(&link.to) {
            if model.items.contains_key(&link.from) {
                let fix = format!("acyclic item edit {} --no-waits-for {}", link.from, link.to);
                add(kind::ITEM, &link.from, "waits-for", &link.to, Some(fix));
            } else {
                add(kind::LINK, &format!("{}-{}", link.from, link.to), "end", &link.to, None);
            }
        }
    }
}

#[cfg(test)]
mod tests;
