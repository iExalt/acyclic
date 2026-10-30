//! Renumbering a thread: moving it to a fresh id the tracker assigns, when two replicas gave threads the same number.
//!
//! Two replicas that each add a thread before seeing the other's can end their ids in the same number, such as
//! `thread-mac-32` and `thread-pi-32`. The ids differ, so nothing breaks, but the app draws threads by number, and
//! `needs` lists the clash. The user resolves it by renumbering one of them.
//!
//! A thread's id is named in more places than its members: runs, questions, proposals, claims and the server's
//! repositories for its runs. So only a thread whose number clashes is renumbered, and only a fresh one: one that has
//! never had a run and has no question, live claim or open proposal naming it. Its own fields and its explicit
//! repositories are copied to the new id, its members move there, done ones included, and the old id is deleted: a
//! tombstone, as for items, so the id is never assigned again and a replica that still adds members under it sees
//! them dangle in `needs`.
//!
//! The user renumbering one thread on two replicas before they sync makes two new threads; each member's thread then
//! conflicts between them, and `needs` lists that.

use std::fmt;

use serde::Serialize;
use serde_json::Value as Json;

use crate::claim;
use crate::ids;
use crate::model::{self, By, Model, WriteError};
use crate::schema::{item, kind, question, set, thread};
use crate::scope::{self, ScopeEdit, USER};
use crate::state::{Error, Key, Millis, Replica, State};

/// The thread fields a renumber carries over; claims stay with the old id as history.
const CARRIED: [&str; 8] = [
    thread::TITLE,
    thread::BRIEF,
    thread::APPROVED,
    thread::APPROVED_MEMBERS,
    thread::PM_FLAG,
    thread::AUTONOMY,
    thread::LAUNCH,
    thread::PLACEMENT,
];

/// Why a thread isn't renumbered.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Reason {
    /// A run worked on it.
    HadRun {
        /// The run.
        run: String,
    },
    /// A question names it.
    Question {
        /// The question.
        question: String,
    },
    /// Someone holds a live claim on it.
    Claimed {
        /// The holder.
        holder: String,
    },
    /// An undecided proposal would change its scope.
    Proposal {
        /// The proposal.
        proposal: String,
    },
    /// No other thread's id ends in its number, so there is nothing to resolve, and its id stays.
    NoClash,
    /// A field it carries, or a member's thread, holds conflicting values.
    Conflict {
        /// The entity's kind.
        kind: String,
        /// The entity's id.
        id: String,
        /// The field.
        field: String,
    },
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HadRun { run } => write!(f, "run {run} worked on it"),
            Self::Question { question } => write!(f, "question {question} names it"),
            Self::Claimed { holder } => write!(f, "{holder} holds a claim on it"),
            Self::Proposal { proposal } => write!(f, "proposal {proposal} would change its scope"),
            Self::NoClash => write!(f, "no other thread's id ends in its number"),
            Self::Conflict { kind, id, field } => write!(f, "{kind} {id}'s {field} is in conflict"),
        }
    }
}

/// A refused renumber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenumberError {
    /// The model layer refused.
    Write(WriteError),
    /// Only the user renumbers a thread.
    NotTheUser(String),
    /// No such thread, or it was deleted.
    NoThread(String),
    /// The thread isn't fresh.
    InUse {
        /// The thread.
        thread: String,
        /// Why not.
        reasons: Vec<Reason>,
    },
}

impl fmt::Display for RenumberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(err) => err.fmt(f),
            Self::NotTheUser(actor) => write!(f, "only the user renumbers a thread, not {actor}"),
            Self::NoThread(id) => write!(f, "no thread {id}"),
            Self::InUse { thread, reasons } => {
                write!(f, "{thread} can't be renumbered: {}", crate::say::list(reasons, "; "))
            },
        }
    }
}

impl std::error::Error for RenumberError {}

impl From<WriteError> for RenumberError {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

impl From<Error> for RenumberError {
    fn from(err: Error) -> Self {
        Self::Write(err.into())
    }
}

/// A renumbered thread: its new id, its old one, and the members that moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Renumbered {
    /// The new id.
    pub thread: String,
    /// The old id, now deleted.
    pub from: String,
    /// The members moved.
    pub moved: Vec<String>,
}

/// The one value of a field, `None` when unset, or `Err` when it holds conflicting values.
fn single(state: &State, key: &Key, field: &str) -> Result<Option<Json>, ()> {
    let mut values: Vec<&Json> = state
        .field(key, field)
        .into_iter()
        .flat_map(|register| register.values().map(|stamped| &stamped.value))
        .collect();
    values.sort_by_key(ToString::to_string);
    values.dedup();
    match values[..] {
        [] => Ok(None),
        [value] => Ok((!value.is_null()).then(|| value.clone())),
        _ => Err(()),
    }
}

/// Whether an undecided proposal's edit would change `id`'s scope.
fn touches(edit: &ScopeEdit, model: &Model, id: &str) -> bool {
    let named = match edit {
        ScopeEdit::Member { thread, .. } => thread.as_deref() == Some(id),
        ScopeEdit::ThreadRepository { thread, .. } => thread == id,
        ScopeEdit::ItemRepository { .. } => false,
    };
    named || edit.approved_threads(model).iter().any(|thread| thread == id)
}

/// Why `id` isn't fresh, judged at `now`.
fn reasons(state: &State, model: &Model, id: &str, now: Millis) -> Result<Vec<Reason>, Error> {
    let mut found = Vec::new();
    let number = ids::number(kind::THREAD, id);
    let clashes = number.is_some()
        && model
            .threads
            .keys()
            .any(|other| other != id && ids::number(kind::THREAD, other) == number);
    if !clashes {
        found.push(Reason::NoClash);
    }
    for run in model.runs.values().filter(|run| run.thread.all().iter().any(|t| *t == id)) {
        found.push(Reason::HadRun { run: run.id.clone() });
    }
    for (key, _) in state.entities().filter(|(key, _)| key.kind == kind::QUESTION) {
        let asking: model::Value<String> = model::value(state, key, question::THREAD)?;
        if asking.all().iter().any(|t| *t == id) {
            found.push(Reason::Question { question: key.id.clone() });
        }
    }
    for held in claim::live(state, &claim::thread_key(id)?, now)? {
        found.push(Reason::Claimed { holder: held.holder });
    }
    for proposal in scope::proposals(state)?.into_values() {
        let open = proposal.decision == model::Value::None;
        if open && proposal.edit.all().iter().any(|edit| touches(edit, model, id)) {
            found.push(Reason::Proposal { proposal: proposal.id });
        }
    }
    let key = claim::thread_key(id)?;
    let names = state.entity(&key).into_iter().flatten().map(|(name, _)| name.as_str());
    for name in names.filter(|name| CARRIED.contains(name) || name.starts_with(set::REPOSITORY)) {
        if single(state, &key, name).is_err() {
            found.push(Reason::Conflict {
                kind: kind::THREAD.into(),
                id: id.into(),
                field: name.into(),
            });
        }
    }
    for member in model
        .items
        .values()
        .filter(|i| i.thread.is_conflict() && i.thread.all().contains(&&id.to_owned()))
    {
        found.push(Reason::Conflict {
            kind: kind::ITEM.into(),
            id: member.id.clone(),
            field: item::THREAD.into(),
        });
    }
    Ok(found)
}

/// Moves thread `id` to the next id this replica assigns.
///
/// # Errors
///
/// Fails when the actor isn't the user, when the thread has no record or was deleted, when it isn't fresh, or when
/// the state layer refuses.
pub fn renumber(replica: &mut Replica, id: &str, by: By<'_>) -> Result<Renumbered, RenumberError> {
    if by.actor != USER {
        return Err(RenumberError::NotTheUser(by.actor.to_owned()));
    }
    let model = Model::read(replica.state())?;
    if !model.threads.contains_key(id) {
        return Err(RenumberError::NoThread(id.to_owned()));
    }
    let reasons = reasons(replica.state(), &model, id, by.at)?;
    if !reasons.is_empty() {
        return Err(RenumberError::InUse { thread: id.to_owned(), reasons });
    }
    let old = claim::thread_key(id)?;
    let new_id = ids::next(replica.state(), kind::THREAD, replica.id());
    let new = claim::thread_key(&new_id)?;
    let names: Vec<String> = replica
        .state()
        .entity(&old)
        .into_iter()
        .flatten()
        .map(|(name, _)| name.clone())
        .filter(|name| CARRIED.contains(&name.as_str()) || name.starts_with(set::REPOSITORY))
        .collect();
    for name in names {
        if let Ok(Some(value)) = single(replica.state(), &old, &name) {
            replica.write(&new, &name, by.actor, by.at, Some(value))?;
        }
    }
    let moved: Vec<String> = model
        .items
        .values()
        .filter(|member| member.thread.one().is_some_and(|thread| thread == id))
        .map(|member| member.id.clone())
        .collect();
    for member in &moved {
        model::put(replica, &model::item_key(member)?, item::THREAD, Some(&new_id), by)?;
    }
    model::put(replica, &old, thread::DELETED, Some(&true), by)?;
    Ok(Renumbered { thread: new_id, from: id.to_owned(), moved })
}

/// Threads whose ids end in the same number, which the app would draw alike, with those that may be renumbered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Clash {
    /// The number.
    pub number: u64,
    /// The threads, in id order.
    pub threads: Vec<String>,
    /// Those fresh enough to renumber.
    pub fresh: Vec<String>,
}

/// Every number two or more threads' ids end in, in the tracker's id form, judged at `now`. Legacy ids such as
/// `thread-27` clash with assigned ones such as `thread-pi-27` too.
///
/// # Errors
///
/// Fails on an unreadable value.
pub fn clashes(state: &State, model: &Model, now: Millis) -> Result<Vec<Clash>, Error> {
    let mut by_number: std::collections::BTreeMap<u64, Vec<String>> =
        std::collections::BTreeMap::new();
    for id in model.threads.keys() {
        if let Some(number) = ids::number(kind::THREAD, id) {
            by_number.entry(number).or_default().push(id.clone());
        }
    }
    let mut found = Vec::new();
    for (number, threads) in by_number.into_iter().filter(|(_, threads)| threads.len() > 1) {
        let mut fresh = Vec::new();
        for thread in &threads {
            if reasons(state, model, thread, now)?.is_empty() {
                fresh.push(thread.clone());
            }
        }
        found.push(Clash { number, threads, fresh });
    }
    Ok(found)
}

#[cfg(test)]
mod tests;
