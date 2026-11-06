//! Deleting an item: a tombstone, so a replica that never saw the deletion can't bring the item back.
//!
//! Deleting writes `deleted: true` on the item. Every reader leaves a deleted item out, and every checked write
//! treats it as missing (`model::existing`), so a replica that has seen the tombstone refuses to link, move, claim,
//! finish, reopen or place it. Adding the id again is refused too: a deleted id is retired, since another
//! replica may still be naming it.
//!
//! **Join rule: delete wins.** The tombstone is a field no write clears, so any join that holds it holds a deleted
//! item, whatever else the other replica wrote meanwhile. What another replica attached before it saw the deletion (a
//! child, a link, a thread membership, a claim) stays in the state and reads as dangling: `needs` lists a child's
//! parent or a link's end that names the deleted item, for the user to move, link elsewhere or delete too. Every
//! replica reaches the same state in any join order, since the tombstone and the stray writes are independent
//! fields.
//!
//! Only an item that nothing refers to is deleted, judged against the replica's own view: no children, no link either
//! way, not done and with no evidence, and no live claim on its thread but the deleter's own.
//!
//! A thread is deleted the same way, by the same field, once no item is in it and no one holds a live claim on it.
//! The exception is a loose task's thread of one, never deleted by hand: it is derived from its task, so a tombstone
//! on it would leave the task unclaimable alone. Readers ignore a tombstone an older version wrote on one.

use std::fmt;

use serde::Serialize;

use crate::claim;
use crate::model::{self, By, Model, Value, WriteError};
use crate::schema::item as field;
use crate::state::{Error, Replica};

/// Why an item isn't deleted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Reason {
    /// Items are under it.
    HasChildren {
        /// The children.
        children: Vec<String>,
    },
    /// A link joins it to another item, either way.
    Linked {
        /// The dependent item.
        from: String,
        /// The item it depends on.
        to: String,
        /// The link's kind as the command line names it: always `waits-for`.
        kind: &'static str,
    },
    /// It is done, or holds evidence of it.
    Finished,
    /// Someone else holds a live claim on its thread, or, for a thread, anyone holds one on it.
    Claimed {
        /// The holder.
        holder: String,
    },
    /// It is a loose task's thread of one, which stays: a tombstone on it would leave the task unclaimable alone.
    /// Its task leaves it by being deleted, or by being moved into another thread.
    TaskThread,
    /// Items are in the thread.
    HasMembers {
        /// The members.
        members: Vec<String>,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HasChildren { children } => {
                write!(f, "it has children ({})", crate::say::list(children, ", "))
            },
            Self::Linked { from, to, kind } => write!(f, "{from} {kind} {to}"),
            Self::Finished => {
                write!(f, "it is done, or holds evidence of it, and finished work stays")
            },
            Self::Claimed { holder } => write!(f, "{holder} holds a claim on its thread"),
            Self::TaskThread => {
                write!(f, "it is a task's own thread, which goes when the task does")
            },
            Self::HasMembers { members } => {
                write!(f, "items are in it ({})", crate::say::list(members, ", "))
            },
        }
    }
}

/// A refused deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteError {
    /// The model layer refused, such as for an item that doesn't exist.
    Write(WriteError),
    /// The item is in use.
    InUse {
        /// The item.
        item: String,
        /// Why not.
        reasons: Vec<Reason>,
    },
    /// No such thread, or it was deleted.
    NoThread(String),
    /// The thread is in use.
    ThreadInUse {
        /// The thread.
        thread: String,
        /// Why not.
        reasons: Vec<Reason>,
    },
}

impl fmt::Display for DeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(err) => err.fmt(f),
            Self::InUse { item: id, reasons } | Self::ThreadInUse { thread: id, reasons } => {
                write!(f, "{id} can't be deleted: {}", crate::say::list(reasons, "; "))
            },
            Self::NoThread(id) => write!(f, "no thread {id}"),
        }
    }
}

impl std::error::Error for DeleteError {}

impl From<WriteError> for DeleteError {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

impl From<Error> for DeleteError {
    fn from(err: Error) -> Self {
        Self::Write(WriteError::State(err))
    }
}

/// Why `actor` can't delete `id` in `model`; empty when it can. The actor's own claim on the item's thread is no
/// reason.
///
/// # Errors
///
/// Fails when a claim can't be read.
pub fn reasons(
    replica: &Replica,
    model: &Model,
    id: &str,
    actor: &str,
    now: crate::state::Millis,
) -> Result<Vec<Reason>, Error> {
    let mut found = Vec::new();
    let children: Vec<String> = model.children(id).into_iter().map(str::to_owned).collect();
    if !children.is_empty() {
        found.push(Reason::HasChildren { children });
    }
    for link in model.links.iter().filter(|link| link.from == id || link.to == id) {
        found.push(Reason::Linked {
            from: link.from.clone(),
            to: link.to.clone(),
            kind: "waits-for",
        });
    }
    if let Some(item) = model.items.get(id)
        && (item.is_done() || item.evidence != Value::None)
    {
        found.push(Reason::Finished);
    }
    for held in claim::covering(&replica.state, model, id, now)? {
        if held.holder != actor {
            found.push(Reason::Claimed { holder: held.holder });
        }
    }
    Ok(found)
}

/// Deletes an item, or does nothing when it is deleted already; returns whether it wrote.
///
/// # Errors
///
/// Fails when the item doesn't exist, when it is in use, or when the state layer refuses.
pub fn delete_item(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, DeleteError> {
    let key = model::item_key(id)?;
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    model::existing(&replica.state, id)?;
    let model = Model::read(&replica.state)?;
    let reasons = reasons(replica, &model, id, by.actor, by.at)?;
    if !reasons.is_empty() {
        return Err(DeleteError::InUse { item: id.to_owned(), reasons });
    }
    model::put(replica, &key, field::DELETED, Some(&true), by)?;
    // The actor's own claim on the task's thread of one goes with it, unless others share that thread: the thread
    // would stay held with nothing in it.
    let one = crate::ids::one(id);
    let alone = !model.items.values().any(|other| {
        other.id != id && model.thread_value(other).all().iter().any(|named| **named == one)
    });
    if alone {
        claim::release(replica, &claim::thread_key(&one)?, by.actor);
    }
    Ok(true)
}

/// Deletes a thread, or does nothing when it is deleted already; returns whether it wrote. A thread only an item names
/// has members, so only a thread with a record of its own can be deleted.
///
/// # Errors
///
/// Fails when the thread has no record, when an item is in it or anyone holds a live claim on it, or when the state
/// layer refuses.
pub fn delete_thread(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, DeleteError> {
    let key = claim::thread_key(id)?;
    if crate::ids::task_of_one(id).is_some() {
        return Err(DeleteError::ThreadInUse {
            thread: id.to_owned(),
            reasons: vec![Reason::TaskThread],
        });
    }
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    let model = Model::read(&replica.state)?;
    if !model.threads.contains_key(id) {
        return Err(DeleteError::NoThread(id.to_owned()));
    }
    let mut reasons = Vec::new();
    // Every item that names it, a conflicted one included, so none is left in a thread that is gone.
    let members: Vec<String> = model
        .items
        .values()
        .filter(|item| model.thread_value(item).all().iter().any(|thread| *thread == id))
        .map(|item| item.id.clone())
        .collect();
    if !members.is_empty() {
        reasons.push(Reason::HasMembers { members });
    }
    for held in claim::live(&replica.state, &key, by.at)? {
        reasons.push(Reason::Claimed { holder: held.holder });
    }
    if !reasons.is_empty() {
        return Err(DeleteError::ThreadInUse { thread: id.to_owned(), reasons });
    }
    model::put(replica, &key, field::DELETED, Some(&true), by)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
