//! Edits to approved scope, applied or proposed by actor. The user's edits and a
//! project-management agent's apply at once; any other agent's edit to an approved thread's scope waits as a
//! proposal for the user to keep or drop. The replica that applies the write decides, against its own view then.
//!
//! An actor is the user only when it says so: anything undeclared or unknown is an agent.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{self, By, Model, Value, WriteError};
use crate::schema::{item as field, kind, link, proposal, set, thread as thread_field};
use crate::state::{Error, Key, Replica, State};

/// The user's actor id.
pub const USER: &str = "actor/user";

/// The prefix of an operator's actor id, such as `operator/mac`: the one other actor that starts and moves runs.
/// Anywhere else it counts as an agent.
pub const OPERATOR: &str = "operator/";

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

/// A proposal a write left for the user, with the field of the record that it would change, as `show` names it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Proposed {
    /// The proposal.
    pub id: String,
    /// The field: `thread` or `repositories`.
    pub field: String,
}

impl ScopeEdit {
    /// The field of its record the edit changes, as `show` names it: `thread` or `repositories`.
    #[must_use]
    pub fn field(&self) -> &'static str {
        match self {
            Self::Member { .. } => "thread",
            Self::ItemRepository { .. } | Self::ThreadRepository { .. } => "repositories",
        }
    }

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
            Self::NoProposal(id) => {
                write!(
                    f,
                    "no proposal {id} with a single edit; `acyclic needs` lists the open ones"
                )
            },
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
    edit_in(replica, &model, change, by)
}

/// Like [`edit`], deciding against `model`, a view the caller already read, so many edits share one read.
pub(crate) fn edit_in(
    replica: &mut Replica,
    model: &Model,
    change: &ScopeEdit,
    by: By<'_>,
) -> Result<Outcome, ScopeError> {
    validate(&replica.state, change)?;
    let protected = !change.approved_threads(model).is_empty();
    if !protected || classify(model, by.actor) != Actor::Agent {
        apply(replica, change, by)?;
        return Ok(Outcome::Applied);
    }
    // The same agent asking again gets its undecided proposal back, so a repeated command files nothing new.
    let pending = proposals(&replica.state)?.into_values().find(|p| {
        p.decision == Value::None
            && p.edit.one() == Some(change)
            && p.actor.one().map(String::as_str) == Some(by.actor)
    });
    if let Some(pending) = pending {
        return Ok(Outcome::Proposed(pending.id));
    }
    let next = replica.state.context.seen.get(replica.id()).copied().unwrap_or(0) + 1;
    let id = format!("{}-{next}", replica.id());
    let key = Key::new(kind::PROPOSAL, &id)?;
    model::put(replica, &key, proposal::EDIT, Some(change), by)?;
    model::put(replica, &key, proposal::ACTOR, Some(&by.actor), by)?;
    Ok(Outcome::Proposed(id))
}

/// Approves a thread's scope and records the members it sees, or revokes the approval. The user's only; writes
/// nothing already so, and returns whether it wrote.
///
/// # Errors
///
/// Fails when the actor isn't the user or the state layer refuses.
pub fn approve(
    replica: &mut Replica,
    thread: &str,
    approved: bool,
    by: By<'_>,
) -> Result<bool, ScopeError> {
    let key = Key::new(kind::THREAD, thread)?;
    let model = Model::read(&replica.state)?;
    let current = model.threads.get(thread);
    let mut changed = false;
    // Approving again acknowledges the blockers added since, so it writes even when nothing else changes.
    let acknowledges =
        approved && !blockers_since_approval(&replica.state, &model, thread)?.is_empty();
    if current.map(|t| &t.approved) != Some(&Value::One(approved)) || acknowledges {
        model::set(replica, &key, thread_field::APPROVED, Some(&approved), by)?;
        changed = true;
    }
    if approved {
        // Every member, done ones too, so one reopened later isn't taken for work that joined unseen.
        let members: Vec<String> = model
            .items
            .values()
            .filter(|item| item.thread.one().map(String::as_str) == Some(thread))
            .map(|item| item.id.clone())
            .collect();
        if current.map(|t| &t.approved_members) != Some(&Value::One(members.clone())) {
            model::set(replica, &key, thread_field::APPROVED_MEMBERS, Some(&members), by)?;
            changed = true;
        }
    }
    Ok(changed)
}

/// The blocks links from an approved thread's members added after its approval, as `(from, to)`. The two are
/// compared by their writers' stamps, so a blocker added within the clock guard's skew of an approval on another
/// replica may be misordered; approving the thread again acknowledges them.
///
/// # Errors
///
/// Fails on an invalid id.
pub fn blockers_since_approval<'m>(
    state: &State,
    model: &'m Model,
    thread: &str,
) -> Result<Vec<(&'m str, &'m str)>, Error> {
    if !model.threads.get(thread).is_some_and(model::Thread::is_approved) {
        return Ok(Vec::new());
    }
    let latest = |key: &Key, name: &str| {
        state.field(key, name).into_iter().flatten().map(|(_, v)| v.at).max()
    };
    let approved_at = latest(&Key::new(kind::THREAD, thread)?, thread_field::APPROVED);
    let mut found = Vec::new();
    for edge in &model.links {
        if edge.kind != model::LinkKind::Blocks || model.thread_of(&edge.from) != Some(thread) {
            continue;
        }
        let key = model::link_key(&edge.from, &edge.to, edge.kind)
            .map_err(|_| Error::InvalidName(edge.from.clone()))?;
        if latest(&key, link::PRESENT) > approved_at {
            found.push((edge.from.as_str(), edge.to.as_str()));
        }
    }
    Ok(found)
}

/// An open member of an approved thread that the approval didn't see, put there by an agent: an agent's edit made
/// before it saw the approval, which reached approved scope without a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unapproved {
    /// The item.
    pub item: String,
    /// Its thread.
    pub thread: String,
    /// Who put it there.
    pub actor: String,
}

/// Every [`Unapproved`] member, by item. The user keeps one by putting it in the thread again, which makes the user
/// its writer, or drops it by taking it out; approving the thread again keeps them all. A thread that recorded no members at its approval is left alone, as is a
/// member whose thread is in conflict, which the conflict covers.
///
/// # Errors
///
/// Fails on an invalid item id.
pub fn unapproved(state: &State, model: &Model) -> Result<Vec<Unapproved>, Error> {
    let mut found = Vec::new();
    for (id, item) in &model.items {
        let Value::One(thread) = &item.thread else { continue };
        if item.is_done() || model.is_archived(id) {
            continue;
        }
        let Some(record) = model.threads.get(thread) else { continue };
        if !record.is_approved() || record.approved_member(id) != Some(false) {
            continue;
        }
        let key = Key::new(kind::ITEM, id)?;
        let writers: Vec<&str> = state
            .field(&key, field::THREAD)
            .into_iter()
            .flatten()
            .map(|(_, v)| v.actor.as_str())
            .collect();
        if writers.iter().all(|actor| classify(model, actor) == Actor::Agent)
            && let Some(actor) = writers.first()
        {
            found.push(Unapproved {
                item: id.clone(),
                thread: thread.clone(),
                actor: (*actor).to_owned(),
            });
        }
    }
    Ok(found)
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
            model::put(replica, &key, field::THREAD, thread.as_ref(), by)?;
            match thread {
                Some(thread) => see_member(replica, thread, item, by),
                None => Ok(()),
            }
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

/// Extends an approved thread's approval to a member applied to it, so work the user kept, or the user or a
/// project manager added, counts as approved. The approval records members and nothing else, so a repository edit
/// has nothing to extend. Competing records of members, which approvals on two replicas leave, are united. A thread
/// that isn't approved, or whose approval recorded no members and so counts every member, is left alone.
fn see_member(
    replica: &mut Replica,
    thread: &str,
    item: &str,
    by: By<'_>,
) -> Result<(), WriteError> {
    let key = Key::new(kind::THREAD, thread)?;
    let approved: Value<bool> = model::value(&replica.state, &key, thread_field::APPROVED)?;
    if !approved.all().into_iter().any(|approved| *approved) {
        return Ok(());
    }
    let seen: Value<Vec<String>> =
        model::value(&replica.state, &key, thread_field::APPROVED_MEMBERS)?;
    let mut members: Vec<String> = seen.all().into_iter().flatten().cloned().collect();
    if seen == Value::None || (members.iter().any(|member| member == item) && !seen.is_conflict()) {
        return Ok(());
    }
    members.push(item.to_owned());
    members.sort();
    members.dedup();
    model::put(replica, &key, thread_field::APPROVED_MEMBERS, Some(&members), by)
}

#[cfg(test)]
mod tests;
