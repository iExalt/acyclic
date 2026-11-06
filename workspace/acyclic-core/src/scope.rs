//! Edits to a thread's scope: which items it holds. Every actor's edit applies at once; the actor is an audit label.

use crate::model::{self, By, WriteError};
use crate::schema::{item as field, kind};
use crate::state::{Key, Replica};

/// The user's actor id, and the actor of a write that names none.
pub const USER: &str = "actor/user";

/// An edit to a thread's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeEdit {
    /// Puts an item in a thread, moves it, or takes it out with `None`.
    Member {
        /// The item.
        item: String,
        /// The thread, or none.
        thread: Option<String>,
    },
}

fixture! {
    /// Applies a scope edit.
    ///
    /// # Errors
    ///
    /// Fails when the model layer refuses, such as for an unknown item or an invalid name.
    fn edit(replica: &mut Replica, change: &ScopeEdit, by: By<'_>) -> Result<(), WriteError> {
        match change {
            ScopeEdit::Member { item, thread } => {
                model::existing(&replica.state, item)?;
                if let Some(thread) = thread {
                    Key::new(kind::THREAD, thread)?;
                }
                let key = model::item_key(item)?;
                model::put(replica, &key, field::THREAD, thread.as_ref(), by)
            },
        }
    }
}

#[cfg(test)]
mod tests;
