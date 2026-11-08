//! Deleting an item, thread or initiative: a tombstone, so a replica that never saw the deletion can't bring the item back.
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
//! Any existing item can be deleted. Known child-parent references and dependencies in either direction are removed.
//! Children survive, retaining any other concurrent parent values. Completion and evidence do not prevent deletion.
//!
//! A thread is deleted the same way, by the same field, once no item is in it and no one holds a live claim on it.
//! The exception is a loose task's thread of one, never deleted by hand: it is derived from its task, so a tombstone
//! on it would leave the task unclaimable alone. Readers ignore a tombstone an older version wrote on one.

use std::fmt;

use serde::Serialize;

use crate::claim;
use crate::model::{self, By, Model, WriteError};
use crate::schema::{initiative, item as field, kind};
use crate::state::{Key, Replica};

/// Why a thread or initiative cannot be deleted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Reason {
    /// Someone holds a live claim on the thread.
    Claimed {
        /// The holder.
        holder: String,
    },
    /// It is a loose task's thread of one, which stays: a tombstone on it would leave the task unclaimable alone.
    /// Its task leaves it by being deleted, or by being moved into another thread.
    TaskThread,
    /// Items are under the initiative.
    HoldsItems {
        /// The items directly under it.
        items: Vec<String>,
    },
    /// Items are in the thread.
    HasMembers {
        /// The members.
        members: Vec<String>,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claimed { holder } => write!(f, "{holder} holds a claim on its thread"),
            Self::TaskThread => {
                write!(f, "it is a task's own thread, which goes when the task does")
            },
            Self::HoldsItems { items } => {
                write!(f, "items are under it ({})", crate::say::list(items, ", "))
            },
            Self::HasMembers { members } => {
                write!(f, "items are in it ({})", crate::say::list(members, ", "))
            },
        }
    }
}

/// Deletes an item, or does nothing when it is deleted already; returns whether it wrote.
///
/// # Errors
///
/// Fails when the item doesn't exist or when the state layer refuses.
pub fn delete_item(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, WriteError> {
    let key = model::item_key(id)?;
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    model::existing(&replica.state, id)?;
    let model = Model::read(&replica.state)?;
    for child in model.children(id) {
        replica.retain(&model::item_key(child)?, field::PARENT, |stamped| {
            stamped.value.as_str() != Some(id)
        });
    }
    for link in model.links.iter().filter(|link| link.from == id || link.to == id) {
        replica.write(
            &model::link_key(&link.from, &link.to)?,
            crate::schema::link::PRESENT,
            by.actor,
            by.at,
            None,
        )?;
    }
    model::put(replica, &key, field::DELETED, Some(&true), by)?;
    // Release all observed claims on an emptied task thread, preserving claims on shared threads.
    let one = crate::ids::one(id);
    let alone = !model.items.values().any(|other| {
        other.id != id && model.thread_value(other).all().iter().any(|named| **named == one)
    });
    if alone {
        let thread = claim::thread_key(&one)?;
        replica.retain(&thread, claim::FIELD, |_| false);
    }
    Ok(true)
}

/// Deletes an initiative, or does nothing when it is deleted already; returns whether it wrote. An initiative that
/// holds items, an item directly under it however it came to, is refused: move or delete them first.
///
/// # Errors
///
/// Fails when the initiative doesn't exist, when an item is under it, or when the state layer refuses.
pub fn delete_initiative(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, WriteError> {
    let key = Key::new(kind::INITIATIVE, id).map_err(WriteError::from)?;
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    let model = Model::read(&replica.state)?;
    if !model.initiatives.contains_key(id) {
        return Err(WriteError::NoInitiative(id.to_owned()));
    }
    let items: Vec<String> = model.children(id).into_iter().map(str::to_owned).collect();
    if !items.is_empty() {
        return Err(WriteError::InitiativeInUse {
            initiative: id.to_owned(),
            reasons: vec![Reason::HoldsItems { items }],
        });
    }
    model::put(replica, &key, initiative::DELETED, Some(&true), by)?;
    Ok(true)
}

/// Deletes a thread, or does nothing when it is deleted already; returns whether it wrote. A thread only an item names
/// has members, so only a thread with a record of its own can be deleted.
///
/// # Errors
///
/// Fails when the thread has no record, when an item is in it or anyone holds a live claim on it, or when the state
/// layer refuses.
pub fn delete_thread(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, WriteError> {
    let key = claim::thread_key(id)?;
    if crate::ids::task_of_one(id).is_some() {
        return Err(WriteError::ThreadInUse {
            thread: id.to_owned(),
            reasons: vec![Reason::TaskThread],
        });
    }
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    let model = Model::read(&replica.state)?;
    if !model.threads.contains_key(id) {
        return Err(WriteError::NoThread(id.to_owned()));
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
        return Err(WriteError::ThreadInUse { thread: id.to_owned(), reasons });
    }
    model::put(replica, &key, field::DELETED, Some(&true), by)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
