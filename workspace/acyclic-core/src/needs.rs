//! The needs-you list, computed on read: everything waiting on the user, as the state shows it (conflicts, cycles,
//! contests, done work waiting on open work, dangling references).

use std::collections::BTreeSet;

use serde::Serialize;

use crate::claim;
use crate::model::{self, Model, Value};
use crate::ready::{ReadinessEvaluator, Stage, ThreadReadiness};
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
    /// A thread holds only expired claims yet has work ready: its holder stopped without finishing or releasing it.
    Stalled {
        /// The thread.
        thread: String,
        /// The holder of the claim that ran out last.
        holder: String,
        /// When that lease ran out, as RFC 3339 in UTC.
        expired_at: String,
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
    /// Unfinished work that waits directly for canceled work, which neither unblocks it nor ever will: the user
    /// decides whether it still makes sense, as by dropping the wait or canceling it too.
    WaitsOnCanceled {
        /// The unfinished item.
        item: String,
        /// The canceled item, or the part that reads canceled, it waits for.
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
    /// A thread stored on an item that has children, as a sync or `item edit --thread` on a parent can leave one:
    /// reads ignore it, so the user decides whether to clear it.
    ThreadOnParent {
        /// The item with children.
        item: String,
        /// The thread stored on it.
        thread: String,
        /// The command that clears it.
        fix: String,
    },
    /// A thread no item is in, as when its last item left or was never added: the user decides whether to fill it or
    /// delete it.
    EmptyThread {
        /// The thread.
        thread: String,
        /// The command that deletes it.
        fix: String,
    },
}

impl Entry {
    /// A stable, readable key: its kind and what it is about, joined by `:`, so the same need on the next look is
    /// known as the same. Lists that change while the need stands, such as a contest's holders, stay out of it. Ids and field names hold no `:`.
    #[must_use]
    pub fn key(&self) -> String {
        let kind = self.kind();
        let parts: Vec<&str> = match self {
            Self::Conflict { kind: record, id, field } => vec![kind, record, id, field],
            Self::Unpushed { remote } => vec![kind, remote],
            Self::WaitsForCycle { items } | Self::ParentCycle { items } => {
                std::iter::once(kind).chain(items.iter().map(String::as_str)).collect()
            },
            Self::ThreadCycle { threads } => {
                std::iter::once(kind).chain(threads.iter().map(String::as_str)).collect()
            },
            Self::Contest { thread, holders: _ } | Self::EmptyThread { thread, fix: _ } => {
                vec![kind, thread]
            },
            Self::ClockAhead { thread, holder }
            | Self::Stalled { thread, holder, expired_at: _ } => {
                vec![kind, thread, holder]
            },
            Self::Unthreaded { item, fix: _ } => vec![kind, item],
            Self::DoneWaiting { item, waits_for } | Self::WaitsOnCanceled { item, waits_for } => {
                vec![kind, item, waits_for]
            },
            Self::Dangling { kind: record, id, field, target, fix: _ } => {
                vec![kind, record, id, field, target]
            },
            Self::ThreadOnParent { item, thread, fix: _ } => vec![kind, item, thread],
        };
        parts.join(":")
    }

    /// The kind's tag, as the entry serializes it and the CLI and app name it: the one place these strings are
    /// written.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Conflict { .. } => "conflict",
            Self::Unpushed { .. } => "unpushed",
            Self::WaitsForCycle { .. } => "waits-for-cycle",
            Self::ParentCycle { .. } => "parent-cycle",
            Self::ThreadCycle { .. } => "thread-cycle",
            Self::Contest { .. } => "contest",
            Self::ClockAhead { .. } => "clock-ahead",
            Self::Stalled { .. } => "stalled",
            Self::Unthreaded { .. } => "unthreaded",
            Self::DoneWaiting { .. } => "done-waiting",
            Self::WaitsOnCanceled { .. } => "waits-on-canceled",
            Self::Dangling { .. } => "dangling",
            Self::ThreadOnParent { .. } => "thread-on-parent",
            Self::EmptyThread { .. } => "empty-thread",
        }
    }

    /// One line saying what the need is, for a notification, a terminal and the app. `name` gives an item's or a
    /// thread's title by id, or the id when it has none. The words are the core's alone, so every surface says
    /// the same thing; with no catch-all arm, a new kind fails to compile until it has words. A line is display
    /// only: nothing may key on it, as the key and the identity do.
    #[must_use]
    pub fn summary(&self, name: &dyn Fn(&str) -> String) -> String {
        let names = |ids: &[String]| ids.iter().map(|id| name(id)).collect::<Vec<_>>().join(", ");
        match self {
            Self::Conflict { kind, id, field } => format!(
                "{}: its {} was written differently on two machines",
                name(id),
                crate::model::shown_field(kind, field)
            ),
            Self::Unpushed { remote } => format!("This clone holds writes {remote} lacks"),
            Self::WaitsForCycle { items } => {
                format!("Items wait for each other in a cycle: {}", names(items))
            },
            Self::ParentCycle { items } => format!("Parents form a cycle: {}", names(items)),
            Self::ThreadCycle { threads } => {
                format!("Threads wait for each other in a cycle: {}", names(threads))
            },
            Self::Contest { thread, holders: _ } => {
                format!("{} has a claim to sort out", name(thread))
            },
            Self::ClockAhead { thread, holder } => {
                format!("{} has a claim by {holder} that starts in the future", name(thread))
            },
            Self::Stalled { thread, holder, expired_at } => {
                format!("{} has stalled: {holder}'s claim ran out at {expired_at}", name(thread))
            },
            Self::Unthreaded { item, fix: _ } => format!("{} needs a thread", name(item)),
            Self::DoneWaiting { item, waits_for } => {
                format!("{} is done but waits for {}", name(item), name(waits_for))
            },
            Self::WaitsOnCanceled { item, waits_for } => {
                format!("{} waits for {}, which was canceled", name(item), name(waits_for))
            },
            Self::Dangling { kind, id, field, target, fix: _ } => {
                if (kind.as_str(), field.as_str()) == (crate::schema::kind::ITEM, "waits-for") {
                    format!("{} waits for {target}, which doesn't exist", name(id))
                } else {
                    format!("{}: its {field} names {target}, which doesn't exist", name(id))
                }
            },
            Self::ThreadOnParent { item, thread, fix: _ } => {
                format!("{} has children, so its thread {} is ignored", name(item), name(thread))
            },
            Self::EmptyThread { thread, fix: _ } => format!("{} has no items", name(thread)),
        }
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
    /// writes to push, a clock that ran ahead, a thread that stalled, which a busy agent can seem to, a reference to
    /// nothing, a merge that can't be checked), items in no thread, which agents leave routinely, and threads with no
    /// item, which they leave between writes, stay on the list without a notification. With no catch-all arm, a new kind fails to compile until it is placed here.
    #[must_use]
    pub const fn notifies(&self) -> bool {
        match self {
            Self::Conflict { .. }
            | Self::WaitsForCycle { .. }
            | Self::ParentCycle { .. }
            | Self::ThreadCycle { .. }
            | Self::Contest { .. }
            | Self::DoneWaiting { .. }
            | Self::WaitsOnCanceled { .. }
            | Self::ThreadOnParent { .. } => true,
            Self::Unpushed { .. }
            | Self::ClockAhead { .. }
            | Self::Stalled { .. }
            | Self::Unthreaded { .. }
            | Self::EmptyThread { .. }
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
    claims(state, model, readiness, now, &mut entries)?;
    done_waiting(model, readiness, &mut entries)?;
    waits_on_canceled(model, readiness, &mut entries)?;
    dangling(model, &mut entries);
    thread_membership(model, &mut entries);
    for (id, item) in &model.items {
        if item.is_finished() || !model.children(id).is_empty() {
            continue;
        }
        unthreaded(model, item, &mut entries);
    }
    Ok(entries.into_iter().collect())
}

/// The threads stored on parents, which reads ignore, and the threads with no item in them.
fn thread_membership(model: &Model, entries: &mut BTreeSet<Entry>) {
    for id in model.items.keys() {
        if let Some(thread) = model.ignored_thread(id) {
            entries.insert(Entry::ThreadOnParent {
                item: id.clone(),
                thread: thread.to_owned(),
                fix: format!("acyclic item edit {id} --no-thread"),
            });
        }
    }
    let occupied: BTreeSet<&str> = model
        .items
        .values()
        .flat_map(|item| model.thread_value(item).all())
        .map(String::as_str)
        .collect();
    for id in model.threads.keys().filter(|id| !occupied.contains(id.as_str())) {
        entries.insert(Entry::EmptyThread {
            thread: id.clone(),
            fix: format!("acyclic thread delete {id}"),
        });
    }
}

/// Whether an item is an open, unparked leaf whose thread isn't in conflict: one `ready` lists only
/// while a thread with a record holds it. A conflicted thread is left to its conflict entry.
fn wants_thread(model: &Model, item: &model::Item) -> bool {
    !item.is_finished()
        && model.children(&item.id).is_empty()
        && item.state != Value::One(model::ItemState::Parked)
        && !model.thread_value(item).is_conflict()
}

/// A leaf that wants a thread and names none, whatever else holds it back: `ready` never lists it. One that names a
/// thread with no record is left to its dangling entry, which says so.
fn unthreaded(model: &Model, item: &model::Item, entries: &mut BTreeSet<Entry>) {
    if !wants_thread(model, item) || model.thread_value(item).one().is_some() {
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
    readiness: &ReadinessEvaluator<'_>,
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
        // Only a thread no claim holds can have stalled, so the cheap test comes before reading its members.
        if live.is_empty()
            && claim::lapsed(state, &claim::thread_key(id)?, now)?.is_some()
            && let ThreadReadiness::Stalled { holder, expired_at, .. } = readiness.thread(id)?
        {
            entries.insert(Entry::Stalled { thread: id.clone(), holder, expired_at });
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
        // Waiting for canceled work is its own entry.
        if target.is_finished() || !model.children(&link.to).is_empty() {
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

/// Each unfinished item, leaf or parent, that waits directly for canceled work: a leaf canceled, or a part that reads
/// canceled. A part with some canceled leaves among others reads by those others, so it flags nothing.
fn waits_on_canceled(
    model: &Model,
    readiness: &ReadinessEvaluator<'_>,
    entries: &mut BTreeSet<Entry>,
) -> Result<(), Error> {
    for link in &model.links {
        if !model.items.contains_key(&link.from) || !readiness.canceled(&link.to) {
            continue;
        }
        let stage = if model.children(&link.from).is_empty() {
            readiness.stage(&link.from)?
        } else {
            readiness.progress(&link.from)?.and_then(|rollup| rollup.stage)
        };
        if !matches!(stage, Some(Stage::Done | Stage::Canceled)) {
            entries.insert(Entry::WaitsOnCanceled {
                item: link.from.clone(),
                waits_for: link.to.clone(),
            });
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
        for thread in
            model.thread_value(item).all().into_iter().filter(|t| !model.threads.contains_key(*t))
        {
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
