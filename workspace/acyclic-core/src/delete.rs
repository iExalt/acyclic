//! Deleting an item: a tombstone, so a replica that never saw the deletion can't bring the item back.
//!
//! Deleting writes `deleted: true` on the item. Every reader leaves a deleted item out, and every checked write
//! treats it as missing (`model::existing`), so a replica that has seen the tombstone refuses to link, move, claim,
//! finish, reopen, archive or place it. Adding the id again is refused too: a deleted id is retired, since another
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
//! way, not done and with no evidence, and no live claim on its thread but the deleter's own. In an approved thread
//! only the user deletes, as only the user may change its scope.

use std::fmt;

use serde::Serialize;

use crate::claim;
use crate::model::{self, By, LinkKind, Model, Thread, Value, WriteError};
use crate::schema::item as field;
use crate::scope::USER;
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
        /// The link's kind, as the command line names it.
        #[serde(serialize_with = "word")]
        kind: LinkKind,
    },
    /// It is done, or holds evidence of it.
    Finished,
    /// Someone else holds a live claim on its thread.
    Claimed {
        /// The holder.
        holder: String,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HasChildren { children } => {
                write!(f, "it has children ({})", crate::say::list(children, ", "))
            },
            Self::Linked { from, to, kind } => write!(f, "{from} {} {to}", kind.word()),
            Self::Finished => {
                write!(f, "it is done, or holds evidence of it, and finished work stays")
            },
            Self::Claimed { holder } => write!(f, "{holder} holds a claim on its thread"),
        }
    }
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde passes a reference")]
fn word<S: serde::Serializer>(kind: &LinkKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(kind.word())
}

/// A refused deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteError {
    /// The model layer refused, such as for an item that doesn't exist.
    Write(WriteError),
    /// Only the user deletes an item in an approved thread.
    NotAllowed {
        /// The actor.
        actor: String,
        /// The approved thread.
        thread: String,
    },
    /// The item is in use.
    InUse {
        /// The item.
        item: String,
        /// Why not.
        reasons: Vec<Reason>,
    },
}

impl fmt::Display for DeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(err) => err.fmt(f),
            Self::NotAllowed { actor, thread } => {
                write!(f, "only the user deletes an item in approved thread {thread}, not {actor}")
            },
            Self::InUse { item, reasons } => {
                write!(f, "{item} can't be deleted: {}", crate::say::list(reasons, "; "))
            },
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
            kind: link.kind,
        });
    }
    if let Some(item) = model.items.get(id)
        && (item.is_done() || item.evidence != Value::None)
    {
        found.push(Reason::Finished);
    }
    let alive = |holder: &str| model.run_alive(holder);
    for held in claim::covering(&replica.state, model, id, now, &alive)? {
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
/// Fails when the item doesn't exist, when it is in use, when it is in an approved thread and the actor isn't the
/// user, or when the state layer refuses.
pub fn delete_item(replica: &mut Replica, id: &str, by: By<'_>) -> Result<bool, DeleteError> {
    let key = model::item_key(id)?;
    if model::is_deleted(&replica.state, &key) {
        return Ok(false);
    }
    model::existing(&replica.state, id)?;
    let model = Model::read(&replica.state)?;
    if by.actor != USER
        && let Some(thread) = model
            .items
            .get(id)
            .into_iter()
            .flat_map(|item| item.thread.all())
            .find(|thread| model.threads.get(*thread).is_some_and(Thread::is_approved))
    {
        return Err(DeleteError::NotAllowed { actor: by.actor.to_owned(), thread: thread.clone() });
    }
    let reasons = reasons(replica, &model, id, by.actor, by.at)?;
    if !reasons.is_empty() {
        return Err(DeleteError::InUse { item: id.to_owned(), reasons });
    }
    model::put(replica, &key, field::DELETED, Some(&true), by)?;
    Ok(true)
}

#[cfg(test)]
mod tests;
