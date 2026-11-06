//! Placing new work in threads. Grouping work into threads is the user's or their agents' judgement; acyclic only
//! follows the grouping they made: a new child joins the thread its siblings share, finished siblings included, and
//! never one its ancestors hold. A move never changes a thread.

use serde::Serialize;

use crate::model::{Model, Value};

/// Where a new child of a parent goes, as its siblings and parent say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// It joins this thread.
    Joins(String),
    /// Nothing says which thread: it is in none.
    Loose,
    /// The siblings disagree, or some are in a thread and some in none, so only the caller can say.
    Unclear,
}

/// The placement of a new child among `siblings`, the parent's other children as `model` read them, done and canceled
/// ones too. They place it only when they all hold one thread: all in none leaves it in none, and any mix or conflict
/// is [`Placement::Unclear`]. With no siblings it takes `parent_thread`, the thread its parent holds, whose first
/// child it is. No thread further up is ever looked at.
#[must_use]
pub fn placement(model: &Model, siblings: &[&str], parent_thread: Option<&str>) -> Placement {
    // A sibling in its own untitled thread of one, which a claim made, is still loose: that thread is no grouping.
    let none = Value::None;
    let mut threads = siblings.iter().filter_map(|id| model.items.get(*id)).map(|s| {
        let thread = model.thread_value(s);
        match thread.one() {
            Some(named) if model.shown_as_task(named) == Some(s.id.as_str()) => &none,
            _ => thread,
        }
    });
    let Some(first) = threads.next() else {
        return parent_thread
            .map_or(Placement::Loose, |thread| Placement::Joins(thread.to_owned()));
    };
    if first.is_conflict() || threads.any(|other| other != first) {
        return Placement::Unclear;
    }
    first.one().map_or(Placement::Loose, |thread| Placement::Joins(thread.clone()))
}

/// An item that joined the thread its siblings, or its parent, hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inherited {
    /// The item.
    pub item: String,
    /// The thread.
    pub thread: String,
}

#[cfg(test)]
mod tests;
