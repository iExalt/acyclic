//! How each value of a conflict is kept: the change that keeps it, proven by the write judge to be accepted and to
//! settle the conflict, or why no change can.

use serde_json::Value as Json;

use crate::archive::{self, Reason};
use crate::model::{By, Evidence, Item, ItemState, Model, Reopened, Size, Value};
use crate::ready::Readiness;
use crate::schema::{item as field, kind, link, repository, thread};
use crate::state::{Error, Key, State};
use crate::write::{
    self, Assign, Change, ChangeSet, ItemAssign, RepositoryAssign, ThreadAssign, Violation,
};

/// One value a field was written apart, and how to keep it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The value, as written; a clear is `null`.
    pub value: Json,
    /// How to keep it.
    pub keep: Keep,
}

/// What keeps a value of a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keep {
    /// This change set, which the judge accepts and which settles the conflict.
    Write(ChangeSet),
    /// Reopening the item, which is done.
    Reopen(String),
    /// Archiving the item, which is eligible.
    Archive(String),
    /// Nothing can.
    Not(Unkept),
}

/// Why no change keeps a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unkept {
    /// Keeping done is finishing the work, which is left to a terminal so its evidence comes with it.
    NeedsEvidence,
    /// The field can't be cleared.
    NoClear,
    /// The value isn't one the field holds.
    Unreadable,
    /// No command writes the field.
    NoCommand,
    /// A repository's owner and name are written together, and the other one is in conflict too.
    NameAlsoConflicted,
    /// Only done work is reopened or keeps evidence, and the item isn't done.
    NotDone,
    /// The item can't be archived, for these reasons.
    Ineligible(Vec<Reason>),
    /// The judge refuses the change that would keep it.
    Refused(Vec<Violation>),
    /// The change would leave the field in conflict.
    Unsettled,
}

/// How to keep each value written apart in `field` of the `entity` record `id`, in the order the register holds them,
/// each value once. A change is judged as `by` would make it.
///
/// # Errors
///
/// Fails on an invalid key, or when a claim can't be read for an archive's eligibility.
pub fn keeps(
    state: &State,
    model: &Model,
    readiness: &Readiness<'_>,
    (entity, id, name): (&str, &str, &str),
    by: By<'_>,
) -> Result<Vec<Kept>, Error> {
    let key = Key::new(entity, id)?;
    let mut values: Vec<&Json> = Vec::new();
    for (_, stamped) in state.field(&key, name).into_iter().flatten() {
        if !values.contains(&&stamped.value) {
            values.push(&stamped.value);
        }
    }
    values
        .into_iter()
        .map(|value| {
            let keep = match keep(state, model, (entity, id, name), value) {
                Keep::Write(changes) => verify(model, (entity, id, name), changes, by),
                Keep::Archive(item) => {
                    let reasons = archive::eligibility(readiness, model, &item, by.at)?;
                    if reasons.is_empty() {
                        Keep::Archive(item)
                    } else {
                        Keep::Not(Unkept::Ineligible(reasons))
                    }
                },
                Keep::Reopen(item) if !model.items.get(&item).is_some_and(Item::is_done) => {
                    Keep::Not(Unkept::NotDone)
                },
                other => other,
            };
            Ok(Kept { value: value.clone(), keep })
        })
        .collect()
}

/// The change set kept only when the judge accepts it and it settles the conflict.
fn verify(
    model: &Model,
    (entity, id, name): (&str, &str, &str),
    changes: ChangeSet,
    by: By<'_>,
) -> Keep {
    match write::judge(model, &changes, by, None) {
        Err(violations) => Keep::Not(Unkept::Refused(violations)),
        Ok(outcome) if outcome.after.conflicts().any(|c| c == (entity, id, name)) => {
            Keep::Not(Unkept::Unsettled)
        },
        Ok(_) => Keep::Write(changes),
    }
}

/// The change that would keep `value`, before it is judged.
fn keep(
    state: &State,
    model: &Model,
    (entity, id, name): (&str, &str, &str),
    value: &Json,
) -> Keep {
    let parsed = |value: &Json| -> Option<Option<String>> {
        match value {
            Json::Null => Some(None),
            Json::String(text) => Some(Some(text.clone())),
            _ => None,
        }
    };
    let set = |assign: Assign| Keep::Write(ChangeSet(vec![Change::Set(assign)]));
    let item = |assign: ItemAssign| set(Assign::Item(id.to_owned(), assign));
    let required = |value: Option<Option<String>>, then: &dyn Fn(String) -> Keep| match value {
        None => Keep::Not(Unkept::Unreadable),
        Some(None) => Keep::Not(Unkept::NoClear),
        Some(Some(text)) => then(text),
    };
    match (entity, name) {
        (kind::ITEM, field::TITLE) => required(parsed(value), &|t| item(ItemAssign::Title(t))),
        (kind::ITEM, field::STATE) => match (value, typed::<ItemState>(value)) {
            (Json::Null, _) => Keep::Not(Unkept::NoClear),
            // Finishing could keep done without evidence, but a terminal asks for it: a choice, not a limit.
            (_, Some(ItemState::Done)) => Keep::Not(Unkept::NeedsEvidence),
            (_, Some(state)) => item(ItemAssign::State(state)),
            (_, None) => Keep::Not(Unkept::Unreadable),
        },
        (kind::ITEM, field::NOTES) => {
            parsed(value).map_or(Keep::Not(Unkept::Unreadable), |v| item(ItemAssign::Notes(v)))
        },
        (kind::ITEM, field::PARENT) => {
            parsed(value).map_or(Keep::Not(Unkept::Unreadable), |v| item(ItemAssign::Parent(v)))
        },
        (kind::ITEM, field::THREAD) => {
            parsed(value).map_or(Keep::Not(Unkept::Unreadable), |v| item(ItemAssign::Thread(v)))
        },
        (kind::ITEM, field::SIZE) => match value {
            Json::Null => item(ItemAssign::Size(None)),
            _ => typed(value)
                .map_or(Keep::Not(Unkept::Unreadable), |s: Size| item(ItemAssign::Size(Some(s)))),
        },
        (kind::ITEM, field::EVIDENCE) => match value {
            Json::Null => Keep::Not(Unkept::NoCommand),
            // Keeping evidence finishes the item again, which is more than keeping a value when it isn't done.
            _ if !model.items.get(id).is_some_and(Item::is_done) => Keep::Not(Unkept::NotDone),
            _ => typed(value).map_or(Keep::Not(Unkept::Unreadable), |evidence: Evidence| {
                let finish =
                    Change::Finish { item: id.to_owned(), evidence: Some(evidence), force: true };
                Keep::Write(ChangeSet(vec![finish]))
            }),
        },
        (kind::ITEM, field::ARCHIVED) => match value {
            Json::Bool(true) => Keep::Archive(id.to_owned()),
            Json::Null | Json::Bool(false) => Keep::Not(Unkept::NoCommand),
            _ => Keep::Not(Unkept::Unreadable),
        },
        (kind::ITEM, field::REOPENED) => match value {
            Json::Null => Keep::Not(Unkept::NoCommand),
            _ => typed::<Reopened>(value)
                .map_or(Keep::Not(Unkept::Unreadable), |_| Keep::Reopen(id.to_owned())),
        },
        (kind::THREAD, thread::TITLE) => {
            required(parsed(value), &|t| set(Assign::Thread(id.to_owned(), ThreadAssign::Title(t))))
        },
        (kind::THREAD, thread::BRIEF) => parsed(value).map_or(Keep::Not(Unkept::Unreadable), |v| {
            set(Assign::Thread(id.to_owned(), ThreadAssign::Brief(v)))
        }),
        (kind::REPOSITORY, repository::DEFAULT_BRANCH) => required(parsed(value), &|b| {
            set(Assign::Repository(id.to_owned(), RepositoryAssign::DefaultBranch(b)))
        }),
        (kind::REPOSITORY, repository::OWNER | repository::NAME) => {
            required(parsed(value), &|text| owner_and_name(model, id, name, text))
        },
        (kind::LINK, link::PRESENT) => {
            let end = |name: &str| {
                let key = Key::new(kind::LINK, id).ok()?;
                state.field(&key, name)?.values().next()?.value.as_str().map(str::to_owned)
            };
            match (value, end(link::FROM), end(link::TO)) {
                (Json::Bool(true), Some(from), Some(to)) => {
                    set(Assign::Item(from, ItemAssign::WaitsFor(to, true)))
                },
                _ => Keep::Not(Unkept::NoCommand),
            }
        },
        _ => Keep::Not(Unkept::NoCommand),
    }
}

/// A value read as the type its field holds.
fn typed<T: serde::de::DeserializeOwned>(value: &Json) -> Option<T> {
    serde_json::from_value(value.clone()).ok()
}

/// A repository's owner and name are given together, as `owner/name`: keeping one takes the other as it stands.
fn owner_and_name(model: &Model, id: &str, name: &str, text: String) -> Keep {
    let Some(record) = model.repositories.get(id) else { return Keep::Not(Unkept::NoCommand) };
    let other = if name == repository::OWNER { &record.name } else { &record.owner };
    let other = match other {
        Value::One(other) => other,
        Value::Conflict(_) => return Keep::Not(Unkept::NameAlsoConflicted),
        Value::None => return Keep::Not(Unkept::NoCommand),
    };
    let (owner, repo) =
        if name == repository::OWNER { (text, other.clone()) } else { (other.clone(), text) };
    let with = |assign| Assign::Repository(id.to_owned(), assign);
    Keep::Write(ChangeSet(vec![
        Change::Set(with(RepositoryAssign::Owner(owner))),
        Change::Set(with(RepositoryAssign::Name(repo))),
    ]))
}
