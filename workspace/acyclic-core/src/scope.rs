//! Edits to approved scope, applied or proposed by actor (plan D17, D60, D71). The user's edits and a
//! project-management agent's apply at once; any other agent's edit to an approved thread's scope waits as a
//! proposal for the user to keep or drop. The replica that applies the write decides, against its own view then.
//!
//! An actor is the user only when it says so: anything undeclared or unknown is an agent.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{self, By, Model, Value, WriteError};
use crate::schema::{item as field, kind, proposal, set};
use crate::state::{Error, Key, Replica, State};

/// The user's actor id.
pub const USER: &str = "actor/user";

/// Who an edit comes from, as far as scope is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// The user.
    User,
    /// A run on a thread flagged as project management.
    ProjectManager,
    /// Any other run, session or undeclared actor.
    Agent,
}

/// Classifies an actor id against the model.
#[must_use]
pub fn classify(model: &Model, actor: &str) -> Actor {
    if actor == USER {
        return Actor::User;
    }
    let pm = actor
        .strip_prefix("run/")
        .and_then(|id| model.runs.get(id))
        .and_then(|run| run.thread.one())
        .and_then(|thread| model.threads.get(thread))
        .is_some_and(model::Thread::is_pm);
    if pm { Actor::ProjectManager } else { Actor::Agent }
}

/// An edit to a thread's scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "edit", deny_unknown_fields)]
pub enum ScopeEdit {
    /// Puts an item in a thread, moves it, or takes it out with `None`.
    Member {
        /// The item.
        item: String,
        /// The thread, or none.
        thread: Option<String>,
    },
    /// Adds or removes a repository an item touches.
    ItemRepository {
        /// The item.
        item: String,
        /// The repository record.
        repository: String,
        /// Whether it is added.
        present: bool,
    },
    /// Adds or removes a repository on a thread explicitly.
    ThreadRepository {
        /// The thread.
        thread: String,
        /// The repository record.
        repository: String,
        /// Whether it is added.
        present: bool,
    },
}

impl ScopeEdit {
    /// The approved threads whose scope the edit changes, in the model's view.
    #[must_use]
    pub fn approved_threads(&self, model: &Model) -> Vec<String> {
        let current = |item: &str| -> Vec<String> {
            model
                .items
                .get(item)
                .map(|i| i.thread.all().into_iter().cloned().collect())
                .unwrap_or_default()
        };
        let mut threads: Vec<String> = match self {
            Self::Member { item, thread } => {
                current(item).into_iter().chain(thread.clone()).collect()
            },
            Self::ItemRepository { item, .. } => current(item),
            Self::ThreadRepository { thread, .. } => vec![thread.clone()],
        };
        threads.retain(|t| model.threads.get(t).is_some_and(model::Thread::is_approved));
        threads.sort();
        threads.dedup();
        threads
    }
}

/// What an edit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Applied at once.
    Applied,
    /// Waiting as a proposal, by id.
    Proposed(String),
}

/// The user's decision on a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Applied.
    Keep,
    /// Not applied.
    Drop,
}

/// A proposal as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// The proposal's id.
    pub id: String,
    /// The proposed edit.
    pub edit: Value<ScopeEdit>,
    /// Who proposed it.
    pub actor: Value<String>,
    /// The user's decision, if any yet.
    pub decision: Value<Decision>,
}

/// A refused scope write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// The model layer refused.
    Write(WriteError),
    /// Only the user decides proposals.
    NotTheUser(String),
    /// No such proposal, or it holds no single edit.
    NoProposal(String),
    /// The proposal was already decided.
    Decided(String),
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Write(err) => err.fmt(f),
            Self::NotTheUser(actor) => write!(f, "only the user decides proposals, not {actor}"),
            Self::NoProposal(id) => write!(f, "no proposal {id} with a single edit"),
            Self::Decided(id) => write!(f, "proposal {id} is already decided"),
        }
    }
}

impl std::error::Error for ScopeError {}

impl From<WriteError> for ScopeError {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

impl From<Error> for ScopeError {
    fn from(err: Error) -> Self {
        Self::Write(WriteError::State(err))
    }
}

/// Applies a scope edit, or records it as a proposal when it changes an approved thread's scope and comes from
/// neither the user nor a project-management agent.
///
/// # Errors
///
/// Fails when the model layer refuses, such as for an unknown item or an invalid name.
pub fn edit(replica: &mut Replica, change: &ScopeEdit, by: By<'_>) -> Result<Outcome, ScopeError> {
    let model = Model::read(&replica.state)?;
    validate(&replica.state, change)?;
    let protected = !change.approved_threads(&model).is_empty();
    if !protected || classify(&model, by.actor) != Actor::Agent {
        apply(replica, change, by)?;
        return Ok(Outcome::Applied);
    }
    let next = replica.state.context.seen.get(replica.id()).copied().unwrap_or(0) + 1;
    let id = format!("{}-{next}", replica.id());
    let key = Key::new(kind::PROPOSAL, &id)?;
    model::put(replica, &key, proposal::EDIT, Some(change), by)?;
    model::put(replica, &key, proposal::ACTOR, Some(&by.actor), by)?;
    Ok(Outcome::Proposed(id))
}

/// Keeps a proposal: applies its edit and records the decision. The user's only.
///
/// # Errors
///
/// Fails when the actor isn't the user, the proposal is unknown or decided, or the edit is refused.
pub fn keep(replica: &mut Replica, id: &str, by: By<'_>) -> Result<(), ScopeError> {
    let change = undecided(replica, id, by)?;
    apply(replica, &change, by)?;
    decide(replica, id, Decision::Keep, by)
}

/// Drops a proposal, recording the decision. The user's only.
///
/// # Errors
///
/// Fails when the actor isn't the user or the proposal is unknown or decided.
pub fn drop(replica: &mut Replica, id: &str, by: By<'_>) -> Result<(), ScopeError> {
    undecided(replica, id, by)?;
    decide(replica, id, Decision::Drop, by)
}

/// Every proposal, by id.
///
/// # Errors
///
/// Fails on an unreadable value.
pub fn proposals(state: &State) -> Result<BTreeMap<String, Proposal>, Error> {
    let mut found = BTreeMap::new();
    for (key, _) in state.entities().filter(|(key, _)| key.kind == kind::PROPOSAL) {
        let read = Proposal {
            id: key.id.clone(),
            edit: model::value(state, key, proposal::EDIT)?,
            actor: model::value(state, key, proposal::ACTOR)?,
            decision: model::value(state, key, proposal::DECISION)?,
        };
        found.insert(key.id.clone(), read);
    }
    Ok(found)
}

fn undecided(replica: &Replica, id: &str, by: By<'_>) -> Result<ScopeEdit, ScopeError> {
    if by.actor != USER {
        return Err(ScopeError::NotTheUser(by.actor.to_owned()));
    }
    let all = proposals(&replica.state)?;
    let found = all.get(id).ok_or_else(|| ScopeError::NoProposal(id.to_owned()))?;
    if found.decision != Value::None {
        return Err(ScopeError::Decided(id.to_owned()));
    }
    found.edit.one().cloned().ok_or_else(|| ScopeError::NoProposal(id.to_owned()))
}

fn decide(
    replica: &mut Replica,
    id: &str,
    decision: Decision,
    by: By<'_>,
) -> Result<(), ScopeError> {
    let key = Key::new(kind::PROPOSAL, id)?;
    model::put(replica, &key, proposal::DECISION, Some(&decision), by)?;
    Ok(())
}

fn validate(state: &State, change: &ScopeEdit) -> Result<(), WriteError> {
    match change {
        ScopeEdit::Member { item, thread } => {
            model::existing(state, item)?;
            if let Some(thread) = thread {
                Key::new(kind::THREAD, thread)?;
            }
        },
        ScopeEdit::ItemRepository { item, .. } => {
            model::existing(state, item)?;
        },
        ScopeEdit::ThreadRepository { thread, .. } => {
            Key::new(kind::THREAD, thread)?;
        },
    }
    Ok(())
}

fn apply(replica: &mut Replica, change: &ScopeEdit, by: By<'_>) -> Result<(), WriteError> {
    match change {
        ScopeEdit::Member { item, thread } => {
            let key = model::item_key(item)?;
            model::put(replica, &key, field::THREAD, thread.as_ref(), by)
        },
        ScopeEdit::ItemRepository { item, repository, present } => {
            let key = model::item_key(item)?;
            model::put_element(replica, &key, set::REPOSITORY, repository, *present, by)
        },
        ScopeEdit::ThreadRepository { thread, repository, present } => {
            let key = Key::new(kind::THREAD, thread)?;
            model::put_element(replica, &key, set::REPOSITORY, repository, *present, by)
        },
    }
}

#[cfg(test)]
mod tests;
