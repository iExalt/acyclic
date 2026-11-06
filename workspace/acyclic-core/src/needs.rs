//! The needs-you list, computed on read: everything waiting on the user, as the state shows it (conflicts, cycles,
//! contests, done work waiting on open work, dangling references).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::claim;
use crate::model::{self, Model, Value};
use crate::ready::{ReadinessEvaluator, Stage};
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
        /// The record's kind.
        kind: String,
        /// The record's id.
        id: String,
        /// The field.
        field: String,
    },
    /// This clone pushes every write, yet holds writes the remote lacks: `acyclic sync` sends them.
    Unpushed {
        /// The remote.
        remote: String,
    },
    /// Waits-for links form a cycle.
    WaitsForCycle {
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
    /// Done work that waits for a leaf that isn't done, as when what it waits for was reopened or added later.
    DoneWaiting {
        /// The done item.
        item: String,
        /// The leaf it waits for.
        waits_for: String,
    },
    /// A reference to something that doesn't exist.
    Dangling {
        /// The referring record's kind.
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
}

impl Entry {
    /// A stable, readable key: its kind and what it is about, joined by `:`, so the same need on the next look is
    /// known as the same. Lists that change while the need stands, such as a contest's holders, stay out of it. Ids and field names hold no `:`.
    #[must_use]
    pub fn key(&self) -> String {
        let parts: Vec<&str> = match self {
            Self::Conflict { kind, id, field } => vec!["conflict", kind, id, field],
            Self::Unpushed { remote } => vec!["unpushed", remote],
            Self::WaitsForCycle { items } => {
                std::iter::once("waits-for-cycle").chain(items.iter().map(String::as_str)).collect()
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
            Self::DoneWaiting { item, waits_for } => vec!["done-waiting", item, waits_for],
            Self::Dangling { kind, id, field, target, fix: _ } => {
                vec!["dangling", kind, id, field, target]
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
            | Self::WaitsForCycle { .. }
            | Self::ParentCycle { .. }
            | Self::ThreadCycle { .. }
            | Self::Contest { .. }
            | Self::DoneWaiting { .. } => true,
            Self::Unpushed { .. }
            | Self::ClockAhead { .. }
            | Self::Unthreaded { .. }
            | Self::Dangling { .. } => false,
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
    for items in model.waits_for_cycles() {
        entries.insert(Entry::WaitsForCycle { items });
    }
    for items in model.parent_cycles() {
        entries.insert(Entry::ParentCycle { items });
    }
    for threads in readiness.thread_cycles()? {
        entries.insert(Entry::ThreadCycle { threads: threads.into_iter().collect() });
    }
    claims(state, model, now, &mut entries)?;
    done_waiting(model, readiness, &mut entries)?;
    dangling(model, &mut entries);
    for (id, item) in &model.items {
        if item.is_done() || !model.children(id).is_empty() {
            continue;
        }
        unthreaded(model, item, &mut entries);
    }
    Ok(entries.into_iter().collect())
}

/// Whether an item is an open, unparked leaf whose thread isn't in conflict: one `ready` lists only
/// while a thread with a record holds it. A conflicted thread is left to its conflict entry.
fn wants_thread(model: &Model, item: &model::Item) -> bool {
    !item.is_done()
        && model.children(&item.id).is_empty()
        && item.state != Value::One(model::ItemState::Parked)
        && !item.thread.is_conflict()
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

/// Each done item, leaf or parent, that waits directly for a leaf that isn't done: work finished before what it
/// assumed. Waits for a parent don't count, so new work under an ongoing part flags nothing that waited on it.
fn done_waiting(
    model: &Model,
    readiness: &ReadinessEvaluator<'_>,
    entries: &mut BTreeSet<Entry>,
) -> Result<(), Error> {
    for link in &model.links {
        let Some(target) = model.items.get(&link.to) else { continue };
        if target.is_done() || !model.children(&link.to).is_empty() {
            continue;
        }
        let stage = if model.children(&link.from).is_empty() {
            readiness.stage(&link.from)?
        } else {
            readiness.progress(&link.from)?.and_then(|rollup| rollup.stage)
        };
        if stage == Some(Stage::Done) {
            entries
                .insert(Entry::DoneWaiting { item: link.from.clone(), waits_for: link.to.clone() });
        }
    }
    Ok(())
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
