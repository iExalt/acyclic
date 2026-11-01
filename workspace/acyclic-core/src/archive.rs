//! Archiving done subtrees and reopening them. Archived is a field on a subtree's root that `ready` and
//! the graph filter on; links into archived items count as satisfied. A root is eligible once its whole subtree is
//! done and merged and no open item outside it links into it; a root without a gate also waits 30 days from its
//! last item's done. Reopening an archived item only flags its dependents; reopening any other done item blocks them
//! again and flags them.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::model::{self, By, ItemState, LinkKind, Model, Reopened, Value, WriteError};
use crate::ready::{Ancestry, Readiness, Status};
use crate::schema::item as field;
use crate::scope;
use crate::state::{Error, Millis, Replica};

/// How long a root without a gate waits after its last item is done.
pub const GATELESS_WAIT: Millis = 30 * 24 * 60 * 60 * 1000;

/// Why a root isn't eligible to archive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Reason {
    /// It, or an ancestor, is archived already.
    Archived,
    /// An item in the subtree isn't done.
    Open {
        /// The item.
        item: String,
    },
    /// An item's commit isn't merged, or can't be checked.
    Unmerged {
        /// The item.
        item: String,
    },
    /// An open item outside the subtree links into it.
    LiveDependent {
        /// The open item.
        from: String,
        /// The item in the subtree it depends on.
        to: String,
    },
    /// A root without a gate waits after its last item is done.
    Waiting {
        /// When it becomes eligible.
        #[serde(rename = "eligible_at")]
        until: Millis,
    },
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Archived => write!(f, "it is archived already"),
            Self::Open { item } => write!(f, "{item} isn't done"),
            Self::Unmerged { item } => {
                write!(f, "{item}'s commit isn't merged, or can't be checked")
            },
            Self::LiveDependent { from, to } => write!(f, "{from}, still open, depends on {to}"),
            Self::Waiting { .. } => {
                write!(f, "it has no gate, and its last item was done less than 30 days ago")
            },
        }
    }
}

/// Why `root` can't be archived at `now`; empty when it can.
///
/// # Errors
///
/// Fails when a claim can't be read.
pub fn eligibility(
    readiness: &Readiness<'_>,
    model: &Model,
    root: &str,
    now: Millis,
) -> Result<Vec<Reason>, Error> {
    let mut reasons = BTreeSet::new();
    if model.is_archived(root) {
        reasons.insert(Reason::Archived);
    }
    let subtree: BTreeSet<&str> = model.subtree(root).into_iter().collect();
    let mut last_done = Millis::MIN;
    for id in &subtree {
        let item = &model.items[*id];
        if !model.reads_done(id) {
            reasons.insert(Reason::Open { item: (*id).to_owned() });
            continue;
        }
        // A parent done by its leaves has no commit of its own: they are checked, and their times count.
        if !model.children(id).is_empty() && !item.is_done() {
            continue;
        }
        last_done = last_done.max(item.done_at.unwrap_or(Millis::MIN));
        if !matches!(readiness.of(id)?, Some(Status::Done | Status::Archived)) {
            reasons.insert(Reason::Unmerged { item: (*id).to_owned() });
        }
    }
    for link in model.links.iter().filter(|l| l.kind != LinkKind::Relates) {
        let open = model.items.contains_key(&link.from)
            && !model.reads_done(&link.from)
            && !model.is_archived(&link.from);
        if open && subtree.contains(link.to.as_str()) && !subtree.contains(link.from.as_str()) {
            reasons.insert(Reason::LiveDependent { from: link.from.clone(), to: link.to.clone() });
        }
    }
    let waits = model.items.get(root).is_some_and(|i| i.gate == Value::None);
    if waits && reasons.is_empty() && now < last_done.saturating_add(GATELESS_WAIT) {
        reasons.insert(Reason::Waiting { until: last_done.saturating_add(GATELESS_WAIT) });
    }
    Ok(reasons.into_iter().collect())
}

/// A refused archive or reopen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    /// The model layer refused.
    Write(WriteError),
    /// The root isn't eligible.
    Ineligible {
        /// The root.
        item: String,
        /// Why not.
        reasons: Vec<Reason>,
    },
    /// Reopening needs a done item.
    NotDone(String),
    /// Only the user or a project-management run archives.
    NotAllowed(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(err) => err.fmt(f),
            Self::Ineligible { item, reasons } => {
                // A wait is the only reason there is, and it ends on a date.
                let until = reasons.iter().find_map(|reason| match reason {
                    Reason::Waiting { until } => Some(crate::time::date(*until)),
                    _ => None,
                });
                match until {
                    Some(date) => write!(f, "{item} can't be archived until {date}: ")?,
                    None => write!(f, "{item} can't be archived: ")?,
                }
                write!(f, "{}", crate::say::list(reasons, "; "))
            },
            Self::NotAllowed(actor) => {
                write!(f, "{actor} may not archive; the user or a project manager may")
            },
            Self::NotDone(id) => write!(f, "{id} isn't done, so there is nothing to reopen"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<WriteError> for ArchiveError {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

impl From<Error> for ArchiveError {
    fn from(err: Error) -> Self {
        Self::Write(WriteError::State(err))
    }
}

/// Archives an eligible root, judged against the replica's own view at the write's time. Archiving takes work out
/// of `ready` and the graph, so only the user or a project-management run archives; any actor may reopen.
///
/// # Errors
///
/// Fails for any other actor, when it isn't eligible, a value can't be read, or the write is refused.
pub fn archive(
    replica: &mut Replica,
    ancestry: &dyn Ancestry,
    root: &str,
    by: By<'_>,
) -> Result<(), ArchiveError> {
    model::existing(&replica.state, root)?;
    let model = Model::read(&replica.state)?;
    if scope::classify(&model, by.actor) == scope::Actor::Agent {
        return Err(ArchiveError::NotAllowed(by.actor.to_owned()));
    }
    let reasons = {
        let readiness = Readiness::new(&replica.state, &model, by.at, ancestry);
        eligibility(&readiness, &model, root, by.at)?
    };
    if !reasons.is_empty() {
        return Err(ArchiveError::Ineligible { item: root.to_owned(), reasons });
    }
    let key = model::item_key(root)?;
    model::put(replica, &key, field::ARCHIVED, Some(&true), by)?;
    Ok(())
}

/// Reopens a done item: back in progress, and flagged so its dependents are listed. An item in an archived subtree
/// clears the archived root above it and leaves its dependents unblocked.
///
/// # Errors
///
/// Fails when the item isn't done or a write is refused.
pub fn reopen(replica: &mut Replica, id: &str, by: By<'_>) -> Result<(), ArchiveError> {
    model::existing(&replica.state, id)?;
    let model = Model::read(&replica.state)?;
    if !model.items.get(id).is_some_and(model::Item::is_done) {
        return Err(ArchiveError::NotDone(id.to_owned()));
    }
    let archived = model.is_archived(id);
    for root in std::iter::once(id).chain(model.ancestors(id)) {
        if model.items.get(root).is_some_and(|i| i.archived != Value::None) {
            model::put::<bool>(replica, &model::item_key(root)?, field::ARCHIVED, None, by)?;
        }
    }
    model::set_state(replica, id, ItemState::InProgress, by)?;
    model::put(replica, &model::item_key(id)?, field::REOPENED, Some(&Reopened { archived }), by)?;
    Ok(())
}

#[cfg(test)]
mod tests;
