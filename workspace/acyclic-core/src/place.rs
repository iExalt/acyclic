//! Placing new or moved work in threads. Grouping work into threads is the user's or their agents' judgement; acyclic
//! only follows the grouping they made: work under a parent joins the thread its parent, or else its siblings, hold,
//! through the scope rules, so an agent only proposes it into approved scope.

use serde::Serialize;

use crate::model::{By, Model, Value};
use crate::scope::{self, ScopeEdit, ScopeError};
use crate::state::Replica;

/// The thread new work under `id`'s parent joins, as [`passed_down`] says; `None` under a parent in conflict.
#[must_use]
pub fn inherited_thread<'m>(model: &'m Model, id: &str) -> Option<&'m str> {
    // A parent in conflict has no single line to inherit along.
    let parent = model.items.get(id).and_then(|item| item.parent.one())?;
    passed_down(model, parent)
}

/// The thread a new child of `parent` joins: `parent`'s own; else the one its threaded children share, so a group's
/// new work joins the thread its other work is in; else its nearest threaded ancestor's. `None` when the nearest
/// thread on that line is in conflict, or there is none.
#[must_use]
pub fn passed_down<'m>(model: &'m Model, parent: &str) -> Option<&'m str> {
    if model.items.get(parent).is_some_and(|item| item.thread == Value::None)
        && let Some(shared) = shared_by_children(model, parent)
    {
        return Some(shared);
    }
    std::iter::once(parent)
        .chain(model.ancestors(parent))
        .filter_map(|id| model.items.get(id))
        .map(|item| &item.thread)
        .find(|thread| **thread != Value::None)
        .and_then(Value::one)
        .map(String::as_str)
}

/// The one thread that `parent`'s open, unarchived children with a thread all hold; `None` when any of them is in
/// conflict or they differ. Finished work doesn't pull new work into its thread.
fn shared_by_children<'m>(model: &'m Model, parent: &str) -> Option<&'m str> {
    let mut shared = None;
    for child in model.children(parent) {
        let item = &model.items[child];
        if item.is_done() || model.is_archived(child) {
            continue;
        }
        match &item.thread {
            Value::None => {},
            Value::One(thread) if shared.is_none_or(|shared| shared == thread.as_str()) => {
                shared = Some(thread.as_str());
            },
            _ => return None,
        }
    }
    shared
}

/// An item that joined the thread passed down to it, or asked to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inherited {
    /// The item.
    pub item: String,
    /// The thread.
    pub thread: String,
    /// The proposal it waits as, when the thread's scope is approved and the actor may only propose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
}

/// Places `items` after an add, a move or an import: an item with no thread joins the one [`passed_down`] names,
/// through the scope rules, so an agent only proposes it into approved scope; an item that has a thread keeps it.
///
/// # Errors
///
/// Fails when the state layer refuses.
pub fn place(
    replica: &mut Replica,
    items: &[String],
    by: By<'_>,
) -> Result<Vec<Inherited>, ScopeError> {
    let model = Model::read(replica.state())?;
    let mut placed = Vec::new();
    for item in items {
        let Some(found) = model.items.get(item) else { continue };
        if found.thread != Value::None || found.is_done() {
            continue;
        }
        if let Some(thread) = inherited_thread(&model, item) {
            let change = ScopeEdit::Member { item: item.clone(), thread: Some(thread.to_owned()) };
            let proposal = match scope::edit_in(replica, &model, &change, by)? {
                scope::Outcome::Applied => None,
                scope::Outcome::Proposed(id) => Some(id),
            };
            placed.push(Inherited { item: item.clone(), thread: thread.to_owned(), proposal });
        }
    }
    Ok(placed)
}

#[cfg(test)]
mod tests;
