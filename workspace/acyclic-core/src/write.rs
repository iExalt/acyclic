//! The one judge of writes to records. A caller states what it means to change as a [`ChangeSet`]; [`judge`] adds what
//! every such change brings with it, whoever asks (new work joins the thread its siblings share), projects the result onto the model in memory, and
//! refuses it only for what it would break that wasn't broken before, naming every such violation. [`commit`] reads
//! the model once, judges, and writes only an accepted change.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::import::is_object_id;
use crate::model::{
    self, By, Evidence, Initiative, Item, ItemState, Model, Repository, Thread, Value, WriteError,
};
use crate::place::{Inherited, Placement, placement};
use crate::ready::{Readiness, ReadinessEvaluator, Stage};
use crate::schema::{initiative, item as field, kind, link, repository, thread};
use crate::state::{Error as StateError, Key, Millis, Replica, State};

/// A condition an app write requires to still hold in the tracker's current state.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Expectation {
    /// An item's field as read by the model; null means unset.
    ItemField {
        /// The item.
        item: String,
        /// A field an exact Undo can restore.
        field: ExpectedItemField,
        /// Its expected value, including the ordered values of a conflict.
        value: ExpectedValue,
    },
    /// A thread's title or notes as read by the model; null means unset.
    ThreadField {
        /// The thread.
        thread: String,
        /// A field an exact edit can restore.
        field: ExpectedThreadField,
        /// Its expected value, including the ordered values of a conflict.
        value: ExpectedValue,
    },
    /// Whether one item waits for one other item.
    WaitsFor {
        /// The waiting item.
        item: String,
        /// The item it waits for.
        target: String,
        /// Whether the link must be present.
        present: bool,
    },
    /// The number of started leaves under an item, or whether a leaf has started.
    StartedAtMost {
        /// The item.
        item: String,
        /// The greatest number of started leaves that is safe.
        count: usize,
    },
    /// Started counts for original waiters and every item that now waits on this item or a box above it.
    WaitersStartedAtMost {
        /// The item whose state is being undone.
        item: String,
        /// Counts from the plan before the original edit; current new waiters must still be unstarted.
        baselines: StartedBaselines,
    },
}

/// An item field an exact Undo can restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedItemField {
    /// The title.
    Title,
    /// The parent item.
    Parent,
    /// The state.
    State,
    /// The notes.
    Notes,
}

/// A thread field an exact edit can restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedThreadField {
    /// The title.
    Title,
    /// The notes; `brief`, as older clients say, reads as notes.
    #[serde(alias = "brief")]
    Notes,
}

/// A model value, serialized as null, one scalar, or a conflict with ordered sides.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ExpectedValue {
    /// Concurrent values, including clears, in write order.
    Conflict(ExpectedConflict),
    /// One value.
    One(String),
    /// No value, or one clear.
    Unset(()),
}

/// Ordered scalar values in a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectedConflict {
    /// The values in their model order.
    pub conflict: Vec<Option<String>>,
}

/// A captured waiter's started count.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartedBaseline {
    /// The waiting item.
    pub item: String,
    /// Its started count before the original edit.
    pub count: usize,
}

/// Original waiter counts for a state Undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StartedBaselines(Vec<StartedBaseline>);

impl<'de> Deserialize<'de> for StartedBaselines {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let baselines = Vec::<StartedBaseline>::deserialize(deserializer)?;
        let mut ids = BTreeSet::new();
        for baseline in &baselines {
            if !ids.insert(baseline.item.as_str()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate started baseline for {}",
                    baseline.item
                )));
            }
        }
        Ok(Self(baselines))
    }
}

impl<'de> Deserialize<'de> for ExpectedConflict {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            conflict: Vec<Option<String>>,
        }

        let Wire { conflict } = Wire::deserialize(deserializer)?;
        let distinct: BTreeSet<_> = conflict.iter().collect();
        if conflict.len() < 2 || distinct.len() < 2 {
            return Err(serde::de::Error::custom("a conflict needs at least two different values"));
        }
        Ok(Self { conflict })
    }
}

/// A failed expectation and the current value that did not match it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    /// The condition that did not hold.
    pub expectation: Expectation,
    /// The current value, or null when the named item is gone.
    pub actual: Json,
}

/// Checks the small set of app Undo conditions against one locked state snapshot.
///
/// Started counts treat every completed leaf as started.
///
/// # Errors
///
/// Fails when readiness cannot read a live claim.
///
/// # Panics
///
/// Never: the checked model fields are strings or enums and always serialize to JSON.
pub fn check_expected(
    state: &State,
    model: &Model,
    expected: &[Expectation],
    now: Millis,
) -> Result<Vec<Mismatch>, StateError> {
    let readiness = ReadinessEvaluator::new(state, model, now);
    let mut mismatches = Vec::new();
    for expectation in expected {
        if let Expectation::WaitersStartedAtMost { item, baselines } = expectation {
            let mut counts: BTreeMap<String, usize> = baselines
                .0
                .iter()
                .map(|baseline| (baseline.item.clone(), baseline.count))
                .collect();
            for waiter in current_waiters(model, item) {
                counts.entry(waiter).or_insert(0);
            }
            let mut started = Vec::new();
            for (waiter, at_most) in counts {
                let count = started_count(&readiness, model, &waiter)?.unwrap_or(0);
                if count > at_most {
                    started.push(
                        serde_json::json!({"item": waiter, "count": count, "at_most": at_most}),
                    );
                }
            }
            if !started.is_empty() {
                mismatches.push(Mismatch {
                    expectation: expectation.clone(),
                    actual: serde_json::json!({"started": started}),
                });
            }
            continue;
        }
        let actual = match expectation {
            Expectation::ItemField { item, field, .. } => {
                let Some(item) = model.items.get(item) else {
                    mismatches
                        .push(Mismatch { expectation: expectation.clone(), actual: Json::Null });
                    continue;
                };
                match field {
                    ExpectedItemField::Title => serde_json::to_value(&item.title),
                    ExpectedItemField::Parent => serde_json::to_value(&item.parent),
                    // As reads show it, so the app's `open` matches the stored `backlog`.
                    ExpectedItemField::State => {
                        serde_json::to_value(&item.state).map(|stored| model::shown_state(&stored))
                    },
                    ExpectedItemField::Notes => serde_json::to_value(&item.notes),
                }
                .expect("model values serialize")
            },
            Expectation::ThreadField { thread, field, .. } => {
                let Some(thread) = model.threads.get(thread) else {
                    mismatches
                        .push(Mismatch { expectation: expectation.clone(), actual: Json::Null });
                    continue;
                };
                match field {
                    ExpectedThreadField::Title => serde_json::to_value(&thread.title),
                    ExpectedThreadField::Notes => serde_json::to_value(&thread.notes),
                }
                .expect("model values serialize")
            },
            Expectation::WaitsFor { item, target, .. } => {
                if !model.items.contains_key(item) || !model.items.contains_key(target) {
                    let missing = [item, target]
                        .into_iter()
                        .filter(|id| !model.items.contains_key(*id))
                        .collect::<Vec<_>>();
                    mismatches.push(Mismatch {
                        expectation: expectation.clone(),
                        actual: serde_json::json!({"missing_items": missing}),
                    });
                    continue;
                }
                Json::Bool(model.links.iter().any(|link| link.from == *item && link.to == *target))
            },
            Expectation::StartedAtMost { item, .. } => {
                Json::from(started_count(&readiness, model, item)?)
            },
            Expectation::WaitersStartedAtMost { .. } => unreachable!("handled above"),
        };
        let matches = match expectation {
            // A state is compared as reads show it: `actual` already is, and an older app may still expect
            // `backlog`.
            Expectation::ItemField { field: ExpectedItemField::State, value, .. } => {
                actual
                    == model::shown_state(
                        &serde_json::to_value(value).expect("expected values serialize"),
                    )
            },
            Expectation::ItemField { value, .. } | Expectation::ThreadField { value, .. } => {
                actual == serde_json::to_value(value).expect("expected values serialize")
            },
            Expectation::WaitsFor { present, .. } => actual == Json::Bool(*present),
            Expectation::StartedAtMost { count, .. } => actual
                .as_u64()
                .and_then(|actual| usize::try_from(actual).ok())
                .is_some_and(|actual| actual <= *count),
            Expectation::WaitersStartedAtMost { .. } => unreachable!("handled above"),
        };
        if !matches {
            mismatches.push(Mismatch { expectation: expectation.clone(), actual });
        }
    }
    Ok(mismatches)
}

fn started_count(
    readiness: &ReadinessEvaluator<'_>,
    model: &Model,
    item: &str,
) -> Result<Option<usize>, StateError> {
    if !model.items.contains_key(item) {
        return Ok(None);
    }
    if model.children(item).is_empty() {
        Ok(Some(usize::from(matches!(
            readiness.stage(item)?,
            Some(Stage::InProgress | Stage::Done)
        ))))
    } else {
        Ok(Some(
            readiness
                .progress(item)?
                .map_or(0, |rollup| rollup.stages.in_progress + rollup.stages.done),
        ))
    }
}

fn current_waiters(model: &Model, item: &str) -> BTreeSet<String> {
    let targets = std::iter::once(item).chain(model.ancestors(item)).collect::<BTreeSet<_>>();
    model
        .links
        .iter()
        .filter(|link| targets.contains(link.to.as_str()) && model.items.contains_key(&link.from))
        .map(|link| link.from.clone())
        .collect()
}

/// A record a change is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum Record {
    /// An initiative.
    Initiative(String),
    /// An item.
    Item(String),
    /// A thread.
    Thread(String),
    /// A repository record.
    Repository(String),
}

impl Record {
    /// The record's id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Initiative(id) | Self::Item(id) | Self::Thread(id) | Self::Repository(id) => id,
        }
    }

    /// The record's kind as stored.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Initiative(_) => kind::INITIATIVE,
            Self::Item(_) => kind::ITEM,
            Self::Thread(_) => kind::THREAD,
            Self::Repository(_) => kind::REPOSITORY,
        }
    }

    fn key(&self) -> Result<Key, WriteError> {
        match self {
            Self::Item(id) => model::item_key(id),
            _ => Ok(Key::new(self.kind(), self.id())?),
        }
    }

    fn exists(&self, model: &Model) -> bool {
        match self {
            Self::Initiative(id) => model.initiatives.contains_key(id),
            Self::Item(id) => model.items.contains_key(id),
            Self::Thread(id) => model.threads.contains_key(id),
            Self::Repository(id) => model.repositories.contains_key(id),
        }
    }
}

/// A field of a record, without its value: where a change, a conflict or a violation is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldName {
    /// An initiative's, item's or thread's title.
    Title,
    /// An initiative's, item's or thread's notes.
    Notes,
    /// An item's state.
    State,
    /// An item's parent.
    Parent,
    /// An item's evidence.
    Evidence,
    /// An item's thread.
    Thread,
    /// A repository's owner.
    Owner,
    /// A repository's name.
    Name,
    /// A repository's default branch.
    DefaultBranch,
    /// One item an item waits for.
    WaitsFor(String),
}

impl FieldName {
    /// The field's name as reported, which is also how it is stored except for a thread's notes (stored as `brief`);
    /// a set's element names the set.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Title => field::TITLE,
            Self::Notes => field::NOTES,
            Self::State => field::STATE,
            Self::Parent => field::PARENT,
            Self::Evidence => field::EVIDENCE,
            Self::Thread => field::THREAD,
            Self::Owner => repository::OWNER,
            Self::Name => repository::NAME,
            Self::DefaultBranch => repository::DEFAULT_BRANCH,
            Self::WaitsFor(_) => "waits-for",
        }
    }
}

/// Where a change, a conflict or a violation is: a field of a record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct FieldAddr {
    /// The record.
    pub record: Record,
    /// Its field.
    pub field: FieldName,
}

/// A value for one of an item's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAssign {
    /// Its title.
    Title(String),
    /// Its notes, or none.
    Notes(Option<String>),
    /// Its state.
    State(ItemState),
    /// Its parent, an item or an initiative. No write clears it: an item always belongs to one.
    Parent(String),
    /// Its evidence, or none.
    Evidence(Option<Evidence>),
    /// Its thread, or none.
    Thread(Option<String>),
    /// Makes it wait for an item, or stop.
    WaitsFor(String, bool),
}

/// A value for one of an initiative's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitiativeAssign {
    /// Its title.
    Title(String),
    /// Its notes, or none.
    Notes(Option<String>),
}

/// A value for one of a thread's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadAssign {
    /// Its title.
    Title(String),
    /// Its notes, or none.
    Notes(Option<String>),
}

/// A value for one of a repository record's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryAssign {
    /// The owner, such as an organization.
    Owner(String),
    /// The repository's name.
    Name(String),
    /// The default branch.
    DefaultBranch(String),
}

/// One field of one record given one value: a field and its value always agree in type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assign {
    /// An initiative's field.
    Initiative(String, InitiativeAssign),
    /// An item's field.
    Item(String, ItemAssign),
    /// A thread's field.
    Thread(String, ThreadAssign),
    /// A repository record's field.
    Repository(String, RepositoryAssign),
}

impl Assign {
    /// The field it writes.
    #[must_use]
    pub fn at(&self) -> FieldAddr {
        let (record, field) = match self {
            Self::Initiative(id, assign) => (
                Record::Initiative(id.clone()),
                match assign {
                    InitiativeAssign::Title(_) => FieldName::Title,
                    InitiativeAssign::Notes(_) => FieldName::Notes,
                },
            ),
            Self::Item(id, assign) => (
                Record::Item(id.clone()),
                match assign {
                    ItemAssign::Title(_) => FieldName::Title,
                    ItemAssign::Notes(_) => FieldName::Notes,
                    ItemAssign::State(_) => FieldName::State,
                    ItemAssign::Parent(_) => FieldName::Parent,
                    ItemAssign::Evidence(_) => FieldName::Evidence,
                    ItemAssign::Thread(_) => FieldName::Thread,
                    ItemAssign::WaitsFor(to, _) => FieldName::WaitsFor(to.clone()),
                },
            ),
            Self::Thread(id, assign) => (
                Record::Thread(id.clone()),
                match assign {
                    ThreadAssign::Title(_) => FieldName::Title,
                    ThreadAssign::Notes(_) => FieldName::Notes,
                },
            ),
            Self::Repository(id, assign) => (
                Record::Repository(id.clone()),
                match assign {
                    RepositoryAssign::Owner(_) => FieldName::Owner,
                    RepositoryAssign::Name(_) => FieldName::Name,
                    RepositoryAssign::DefaultBranch(_) => FieldName::DefaultBranch,
                },
            ),
        };
        FieldAddr { record, field }
    }

    /// The value it writes, as stored: a clear is `null`, and a set's element is whether it is there.
    // The arms read alike but serialize values of different types.
    #[allow(clippy::match_same_arms)]
    #[must_use]
    pub fn value(&self) -> Json {
        match self {
            Self::Initiative(_, assign) => match assign {
                InitiativeAssign::Title(v) => to_json(v),
                InitiativeAssign::Notes(v) => to_json(v),
            },
            Self::Item(_, assign) => match assign {
                ItemAssign::Title(v) => to_json(v),
                ItemAssign::Notes(v) => to_json(v),
                ItemAssign::State(v) => to_json(v),
                ItemAssign::Parent(v) => to_json(v),
                ItemAssign::Evidence(v) => to_json(v),
                ItemAssign::Thread(v) => to_json(v),
                ItemAssign::WaitsFor(_, present) => Json::Bool(*present),
            },
            Self::Thread(_, assign) => match assign {
                ThreadAssign::Title(v) => to_json(v),
                ThreadAssign::Notes(v) => to_json(v),
            },
            Self::Repository(_, assign) => match assign {
                RepositoryAssign::Owner(v)
                | RepositoryAssign::Name(v)
                | RepositoryAssign::DefaultBranch(v) => to_json(v),
            },
        }
    }
}

fn to_json<T: Serialize>(value: &T) -> Json {
    serde_json::to_value(value).expect("model values always serialize")
}

/// What a repeated add of a record that exists does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// Refuses it when any field it gives differs, so retrying an add changes nothing and a different one is caught.
    Refuse,
    /// Updates the record to what it gives, as an import does.
    Update,
}

/// One change a caller means.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Adds a record with these fields, or, when it exists, does what `on_repeat` says.
    Add {
        /// The record.
        record: Record,
        /// Whether its id was made for this add rather than chosen by the caller: a minted thread may not take the
        /// title of another, since a retried add would otherwise make a twin. An item's title is judged however its id
        /// was made.
        minted: bool,
        /// What an add of a record that exists does.
        on_repeat: Repeat,
        /// Its fields. Each must be the record's own.
        with: Vec<Assign>,
    },
    /// Sets one field of a record that exists or that the change set adds.
    Set(Assign),
    /// Finishes an item: marks it done, with this evidence when given. Unless forced, the item must be ready, or held
    /// by the one finishing it, or a parent with no open work, or done already; and done work keeps the evidence it
    /// was finished with.
    Finish {
        /// The item.
        item: String,
        /// The evidence it is finished with.
        evidence: Option<Evidence>,
        /// Finish it although it isn't ready or someone else holds it, or replace its evidence.
        force: bool,
    },
    /// Cancels an item: dropped on purpose, so neither done nor open. Done work isn't canceled without being reopened
    /// first, nor is a parent that reads done. Unless forced, no one else may hold a live claim on the thread of an
    /// open leaf at or under it.
    Cancel {
        /// The item.
        item: String,
        /// Cancel it although someone else holds a claim on it.
        force: bool,
    },
}

/// What a caller means to change, judged and written as a whole. The order of its changes doesn't matter, except to
/// the order a report lists them in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet(pub Vec<Change>);

/// A write the judge accepted, in the order it is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// A record the change set adds.
    Create(Record),
    /// A field the change set sets, or one a consequence sets.
    Assign(Assign),
    /// New work joins the thread its siblings share, or its parent held.
    Place(Inherited),
}

/// A write and what caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The change that asked for it, or, for a consequence, the change it follows from.
    pub change: usize,
    /// Whether the change set asked for it, rather than it following from what it asked.
    pub asked: bool,
    /// The write.
    pub write: Written,
}

/// An accepted change set: the model it leaves and the writes that make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The model as it is once the writes are made.
    pub after: Model,
    /// The writes, in order; a change that would change nothing makes none.
    pub steps: Vec<Step>,
}

impl Outcome {
    /// Whether anything is written.
    #[must_use]
    pub fn changes(&self) -> bool {
        !self.steps.is_empty()
    }

    /// The work that joined a thread passed down to it.
    #[must_use]
    pub fn placed(&self) -> Vec<Inherited> {
        self.steps
            .iter()
            .filter_map(|step| match &step.write {
                Written::Place(placed) => Some(placed.clone()),
                _ => None,
            })
            .collect()
    }
}

/// What a change set would break.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// The record it changes, or a record a field names, doesn't exist.
    NoRecord(Record),
    /// A newly created record has no field write to persist it.
    EmptyRecord(Record),
    /// The record it adds or changes was deleted, and its id isn't used again.
    Deleted(Record),
    /// An item would wait for itself.
    SelfLink,
    /// Items would sit under each other in a cycle, through these.
    ParentCycle(Vec<String>),
    /// Items would wait for each other in a cycle, through these.
    WaitsForCycle(Vec<String>),
    /// An item would wait for one above or below it, which could then never be done: a parent is done only once its
    /// leaves are.
    LineageLink {
        /// The waiting item.
        from: String,
        /// The item it waits for.
        to: String,
    },
    /// A new initiative or item would take the id of this record of the other kind, or of one deleted: an item's
    /// `parent` names either by its id alone.
    IdTaken(Record),
    /// A record would take the title of this one: a minted thread another thread's, an item the title of one in its
    /// initiative that has not finished, or is a parent.
    TitleTaken(Record),
    /// A repository would take `owner/name` that this other record has, or could have once its conflict is settled.
    NameTaken {
        /// The name, as `owner/name`.
        name: String,
        /// The other repository record.
        other: String,
    },
    /// Evidence on an item that isn't done.
    EvidenceNotDone,
    /// An add of a record that exists gives this field another value.
    RepeatDiffers,
    /// Two changes give this field different values.
    Contradicts,
    /// An assignment in an add names a different record.
    WrongRecord(Record),
    /// The item isn't the finisher's to finish: not ready, or held by someone else. With no readiness to judge it by,
    /// no status.
    NotReady(Option<Readiness>),
    /// An item with children would be stored done: a parent reads done once its leaves are, so finish those.
    ParentDone,
    /// Done work would get other evidence than it was finished with.
    EvidenceDiffers,
    /// A new item's siblings are in different threads, or some in none, so no thread follows from them.
    ThreadUnclear {
        /// The parent whose children they are.
        parent: String,
    },
    /// The item is done, or a parent that reads done: reopen it before canceling.
    CancelDone,
    /// Someone else holds a live claim on the item's thread, or on one under it.
    CancelHeld {
        /// The holder.
        holder: String,
    },
    /// Evidence names a commit that isn't an object id.
    InvalidCommit,
    /// Evidence names a repository record missing these fields, so it could never be verified.
    IncompleteRepository {
        /// The repository record.
        repository: String,
        /// The fields it lacks, as the CLI names them.
        missing: Vec<&'static str>,
    },
}

/// One thing a change set would break, where.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    /// The change it comes from, when one change does.
    pub change: Option<usize>,
    /// The field, when one field does.
    pub at: Option<FieldAddr>,
    /// What it breaks.
    pub rule: Rule,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = self.at.as_ref().map_or_else(String::new, |at| {
            format!("{} {}'s {}: ", at.record.kind(), at.record.id(), at.field.name())
        });
        match &self.rule {
            Rule::NoRecord(record) => write!(f, "{at}no {} {}", record.kind(), record.id()),
            Rule::EmptyRecord(record) => {
                write!(f, "{} {} has no field to write", record.kind(), record.id())
            },
            Rule::Deleted(record) => {
                write!(
                    f,
                    "{at}{} {} was deleted, and its id isn't used again",
                    record.kind(),
                    record.id()
                )
            },
            Rule::SelfLink => write!(f, "{at}an item can't wait for itself"),
            Rule::ParentCycle(path) => {
                write!(f, "items would sit under each other in a cycle: {}", path.join(", "))
            },
            Rule::WaitsForCycle(path) => {
                write!(f, "items would wait for each other in a cycle: {}", path.join(", "))
            },
            Rule::LineageLink { from, to } => write!(
                f,
                "{at}{from} would wait for {to}, which is above or below it, so neither could ever be done"
            ),
            Rule::IdTaken(record) => {
                write!(
                    f,
                    "{at}{} {} has that id, and an id names one kind of record",
                    record.kind(),
                    record.id()
                )
            },
            Rule::TitleTaken(other) => {
                write!(f, "{at}{} {} already has that title", other.kind(), other.id())
            },
            Rule::NameTaken { name, other } => write!(f, "{at}repository {other} is {name} too"),
            Rule::EvidenceNotDone => write!(f, "{at}evidence on an item that isn't done"),
            Rule::RepeatDiffers => write!(f, "{at}it exists with another value"),
            Rule::Contradicts => write!(f, "{at}two changes give it different values"),
            Rule::WrongRecord(record) => {
                write!(
                    f,
                    "{at}the assignment must name the {} {} being added",
                    record.kind(),
                    record.id()
                )
            },
            Rule::NotReady(_) => write!(f, "{at}it isn't ready, or someone else holds it"),
            Rule::ParentDone => write!(f, "{at}it has children, so it reads done once they are"),
            Rule::EvidenceDiffers => write!(f, "{at}it is done with other evidence"),
            Rule::ThreadUnclear { parent } => write!(
                f,
                "{at}the other children of {parent} are in different threads, or some in none, so say which thread it joins"
            ),
            Rule::CancelDone => write!(f, "{at}it is done, so reopen it to cancel it"),
            Rule::CancelHeld { holder } => write!(f, "{at}{holder} holds a claim on its work"),
            Rule::InvalidCommit => {
                write!(f, "{at}a commit is 7 to 64 hexadecimal characters, as git prints it")
            },
            Rule::IncompleteRepository { repository, missing } => {
                write!(f, "{at}repository {repository} is missing {}", missing.join(", "))
            },
        }
    }
}

/// Why a [`commit`] wrote nothing.
#[derive(Debug)]
pub enum CommitError<E> {
    /// The caller couldn't say what to change.
    Build(E),
    /// The change set would break these, judged against this model.
    Refused(Vec<Violation>, Box<Model>),
    /// The tracker couldn't be read, or a write was refused below the judge.
    Write(WriteError),
}

impl<E> From<WriteError> for CommitError<E> {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

/// Reads the model once, asks `build` what to change in it, judges that, and writes it when accepted. Run it inside a
/// store batch, after anything else the batch writes, so the model judged is the one the writes land on. Finishing
/// work is judged by its readiness when `judge_finishing`; without it, only a forced finish passes.
///
/// # Errors
///
/// Fails when the model can't be read, when `build` fails, when the change set is refused, or when a write is.
pub fn commit<E>(
    replica: &mut Replica,
    by: By<'_>,
    judge_finishing: bool,
    build: impl FnOnce(&Model) -> Result<ChangeSet, E>,
) -> Result<Outcome, CommitError<E>> {
    let before = Model::read(&replica.state).map_err(WriteError::from)?;
    crate::probe::mark("write: model read");
    let changes = build(&before).map_err(CommitError::Build)?;
    let judged = {
        let readiness =
            judge_finishing.then(|| ReadinessEvaluator::new(&replica.state, &before, by.at));
        judge(&before, &changes, by, readiness.as_ref())
    };
    let outcome =
        judged.map_err(|violations| CommitError::Refused(violations, Box::new(before)))?;
    crate::probe::mark("write: judge");
    write(replica, &outcome, by)?;
    crate::probe::mark("write: write");
    Ok(outcome)
}

/// Judges a change set against the model it would change: what it brings with it, the model it leaves, and every
/// violation it would introduce. Nothing is read or written. `readiness`, over the same model, judges finishing work;
/// without it, only a forced finish passes.
///
/// # Errors
///
/// Fails with every violation the change set would introduce, in a fixed order.
pub fn judge(
    before: &Model,
    changes: &ChangeSet,
    by: By<'_>,
    readiness: Option<&ReadinessEvaluator<'_>>,
) -> Result<Outcome, Vec<Violation>> {
    let mut judging = Judging {
        before,
        after: before.clone(),
        at: by.at,
        actor: by.actor,
        readiness,
        steps: Vec::new(),
        violations: Vec::new(),
    };
    judging.explicit(changes);
    judging.consequences();
    judging.rules(changes);
    let Judging { after, steps, mut violations, .. } = judging;
    if violations.is_empty() {
        Ok(Outcome { after, steps })
    } else {
        violations.sort();
        violations.dedup();
        Err(violations)
    }
}

struct Judging<'a, 'r> {
    before: &'a Model,
    after: Model,
    at: Millis,
    actor: &'a str,
    readiness: Option<&'a ReadinessEvaluator<'r>>,
    steps: Vec<Step>,
    violations: Vec<Violation>,
}

impl Judging<'_, '_> {
    fn refuse(&mut self, change: Option<usize>, at: Option<FieldAddr>, rule: Rule) {
        self.violations.push(Violation { change, at, rule });
    }

    /// The changes the set asks for, less those that change nothing, projected in order.
    fn explicit(&mut self, changes: &ChangeSet) {
        let mut created: BTreeSet<Record> = BTreeSet::new();
        let mut given: BTreeMap<FieldAddr, (usize, Json)> = BTreeMap::new();
        for (n, change) in changes.0.iter().enumerate() {
            let finish;
            let cancel;
            let (record, assigns, repeat) = match change {
                Change::Add { record, on_repeat, with, .. } => {
                    (record.clone(), with.as_slice(), Some(*on_repeat))
                },
                Change::Set(assign) => (assign.at().record, std::slice::from_ref(assign), None),
                Change::Finish { item, evidence, .. } => {
                    finish = finishing(item, evidence.as_ref());
                    (Record::Item(item.clone()), finish.as_slice(), None)
                },
                Change::Cancel { item, .. } => {
                    cancel = [Assign::Item(item.clone(), ItemAssign::State(ItemState::Canceled))];
                    (Record::Item(item.clone()), cancel.as_slice(), None)
                },
            };
            if record.key().is_ok_and(|key| self.before.is_deleted(&key)) {
                self.refuse(Some(n), None, Rule::Deleted(record));
                continue;
            }
            let exists = record.exists(self.before);
            if let Change::Finish { item, evidence, force: false } = change
                && exists
            {
                self.finishable(n, item, evidence.as_ref());
            }
            if let Change::Cancel { item, force: false } = change
                && exists
            {
                self.cancelable(n, item);
            }
            match repeat {
                Some(_) if !exists && created.insert(record.clone()) => {
                    self.create(n, &record);
                },
                None if !exists && !created.contains(&record) => {
                    self.refuse(Some(n), Some(assign_at(assigns)), Rule::NoRecord(record));
                    continue;
                },
                _ => {},
            }
            for assign in assigns {
                let at = assign.at();
                if at.record != record {
                    self.refuse(Some(n), Some(at), Rule::WrongRecord(record.clone()));
                    continue;
                }
                let value = assign.value();
                match given.get(&at) {
                    Some((_, held)) if *held != value => {
                        self.refuse(Some(n), Some(at), Rule::Contradicts);
                        continue;
                    },
                    Some(_) => continue,
                    None => {
                        given.insert(at.clone(), (n, value));
                    },
                }
                if let Assign::Item(id, ItemAssign::WaitsFor(to, true)) = assign
                    && id == to
                {
                    self.refuse(Some(n), Some(at), Rule::SelfLink);
                    continue;
                }
                let holds = holds(self.before, assign);
                if exists && repeat == Some(Repeat::Refuse) && !holds {
                    self.refuse(Some(n), Some(at), Rule::RepeatDiffers);
                    continue;
                }
                if !holds || !exists {
                    self.apply(n, true, Written::Assign(assign.clone()));
                }
            }
        }
    }

    /// Refuses to finish an item the actor may not finish without forcing, or to give done work other evidence. Judged
    /// on the model before, as the actor saw it.
    fn finishable(&mut self, n: usize, id: &str, evidence: Option<&Evidence>) {
        let at =
            || Some(FieldAddr { record: Record::Item(id.to_owned()), field: FieldName::State });
        match self.readiness.map(|readiness| readiness.of(id)) {
            // An unreadable claim leaves readiness unknown, which only forcing passes.
            None | Some(Err(_)) => self.refuse(Some(n), at(), Rule::NotReady(None)),
            // A parent is refused by what it would store, forced or not.
            Some(Ok(Some(Readiness::Parent))) => {},
            Some(Ok(status)) => {
                if !status.as_ref().is_some_and(|status| status.finishable_by(self.actor)) {
                    self.refuse(Some(n), at(), Rule::NotReady(status));
                }
            },
        }
        let item = &self.before.items[id];
        if let Some(evidence) = evidence
            && item.is_done()
            && !item.evidence.all().is_empty()
            && item.evidence.one() != Some(evidence)
        {
            let at = FieldAddr { record: Record::Item(id.to_owned()), field: FieldName::Evidence };
            self.refuse(Some(n), Some(at), Rule::EvidenceDiffers);
        }
    }

    /// Refuses to cancel work someone else holds a live claim on, judged on the model before, as the actor saw it.
    /// Without readiness to judge by, there is no claim to see and nothing is refused.
    fn cancelable(&mut self, n: usize, id: &str) {
        let Some(readiness) = self.readiness else { return };
        let at =
            || Some(FieldAddr { record: Record::Item(id.to_owned()), field: FieldName::State });
        let mut holders = Vec::new();
        for leaf in self.before.subtree(id) {
            if !self.before.children(leaf).is_empty() || self.before.reads_finished(leaf) {
                continue;
            }
            // The claims on its thread, whatever else holds the leaf back: waiting work is held too.
            match readiness.holders(leaf) {
                Ok(held) => holders.extend(held),
                Err(_) => self.refuse(Some(n), at(), Rule::NotReady(None)),
            }
        }
        holders.sort();
        holders.dedup();
        if let Some(holder) = holders.into_iter().find(|holder| holder != self.actor) {
            self.refuse(Some(n), at(), Rule::CancelHeld { holder });
        }
    }

    fn create(&mut self, change: usize, record: &Record) {
        // A record exists once a field of it is written: an item always gets its state, a thread or repository needs
        // a field given.
        let id = record.id().to_owned();
        match record {
            Record::Initiative(_) => {
                self.after
                    .initiatives
                    .insert(id.clone(), Initiative { id, title: Value::None, notes: Value::None });
            },
            Record::Item(_) => {
                self.after.items.insert(id.clone(), empty_item(&id));
            },
            Record::Thread(_) => {
                self.after
                    .threads
                    .insert(id.clone(), Thread { id, title: Value::None, notes: Value::None });
            },
            Record::Repository(_) => {
                self.after.repositories.insert(
                    id.clone(),
                    Repository {
                        id,
                        owner: Value::None,
                        name: Value::None,
                        default_branch: Value::None,
                    },
                );
            },
        }
        self.steps.push(Step { change, asked: true, write: Written::Create(record.clone()) });
    }

    fn apply(&mut self, change: usize, asked: bool, write: Written) {
        project(&mut self.after, &write, self.at);
        self.steps.push(Step { change, asked, write });
    }

    /// What the asked-for writes bring with them, judged on the model they leave.
    fn consequences(&mut self) {
        let asked: Vec<(usize, Assign)> = self
            .steps
            .iter()
            .filter_map(|step| match &step.write {
                Written::Assign(assign) => Some((step.change, assign.clone())),
                _ => None,
            })
            .collect();
        self.start_open(&asked);
        self.place(&asked);
        self.graduate();
    }

    /// A loose task's thread of one that a second item joins takes the task's title, which it has none of until then:
    /// from then on it is a thread like any other. Written beside the join, so replicas that graduate it apart write
    /// the same title and agree.
    fn graduate(&mut self) {
        let joined: Vec<(usize, String)> = self
            .steps
            .iter()
            .filter_map(|step| match &step.write {
                Written::Place(placed) => Some((step.change, placed.thread.clone())),
                Written::Assign(Assign::Item(_, ItemAssign::Thread(Some(thread)))) => {
                    Some((step.change, thread.clone()))
                },
                _ => None,
            })
            .collect();
        let mut done = BTreeSet::new();
        for (n, thread) in joined {
            let Some(task) = crate::ids::task_of_one(&thread) else { continue };
            let after = &self.after;
            let untitled = after.threads.get(&thread).is_some_and(|t| t.title == Value::None);
            let joined = after.items.values().any(|item| {
                item.id != task && after.thread_value(item).one().is_some_and(|t| *t == thread)
            });
            let title = after.items.get(task).and_then(|item| item.title.one()).cloned();
            if untitled
                && joined
                && let Some(title) = title
                && done.insert(thread.clone())
            {
                self.apply(
                    n,
                    false,
                    Written::Assign(Assign::Thread(thread, ThreadAssign::Title(title))),
                );
            }
        }
    }

    fn start_open(&mut self, asked: &[(usize, Assign)]) {
        // A new item with no state given starts open.
        let stated: BTreeSet<&str> = asked
            .iter()
            .filter_map(|(_, assign)| match assign {
                Assign::Item(id, ItemAssign::State(_)) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let unstated: Vec<(usize, String)> = self
            .steps
            .iter()
            .filter_map(|step| match &step.write {
                Written::Create(Record::Item(id)) if !stated.contains(id.as_str()) => {
                    Some((step.change, id.clone()))
                },
                _ => None,
            })
            .collect();
        for (n, id) in unstated {
            self.apply(
                n,
                false,
                Written::Assign(Assign::Item(id, ItemAssign::State(ItemState::Backlog))),
            );
        }
    }

    /// New work joins the thread its siblings share, finished ones included, and refuses to guess when they differ;
    /// moves change no thread. An item that gains its first child gives its own thread up to it, or just drops it
    /// when no new child takes it on. All judged on the model before, so one placement doesn't steer the next.
    fn place(&mut self, asked: &[(usize, Assign)]) {
        let threaded: BTreeSet<&str> = asked
            .iter()
            .filter_map(|(_, assign)| match assign {
                Assign::Item(id, ItemAssign::Thread(_)) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let (before, after) = (self.before, &self.after);
        let mut created: Vec<(usize, &str)> = Vec::new();
        let mut entering: Vec<(usize, &str)> = Vec::new();
        for step in &self.steps {
            let id = match &step.write {
                Written::Create(Record::Item(id)) => {
                    created.push((step.change, id));
                    id
                },
                Written::Assign(Assign::Item(id, ItemAssign::Parent(_))) => id,
                _ => continue,
            };
            // Where it ends up, whichever write put it there.
            if let Some(parent) = after.items.get(id).and_then(|item| item.parent.one()) {
                // An item with children before has had its first already.
                if before.children(parent).is_empty() && entering.iter().all(|(_, p)| p != parent) {
                    entering.push((step.change, parent));
                }
            }
        }
        let mut placed = Vec::new();
        let mut unclear = Vec::new();
        for (n, id) in created {
            let Some(item) = after.items.get(id) else { continue };
            let Some(parent) = item.parent.one() else { continue };
            // An initiative's children are its epics and loose tasks, which share no thread by being siblings.
            if threaded.contains(id) || item.is_finished() || after.initiatives.contains_key(parent)
            {
                continue;
            }
            let siblings: Vec<&str> =
                before.children(parent).into_iter().filter(|sibling| *sibling != id).collect();
            let held = after.items.get(parent).and_then(|p| p.thread.one()).map(String::as_str);
            match placement(before, &siblings, held) {
                Placement::Joins(thread) => {
                    placed.push((n, Inherited { item: id.to_owned(), thread }));
                },
                Placement::Loose => {},
                Placement::Unclear => unclear.push((n, id.to_owned(), parent.to_owned())),
            }
        }
        let dropped: Vec<(usize, String)> = entering
            .into_iter()
            .filter(|(_, parent)| after.items.get(*parent).is_some_and(|p| p.thread != Value::None))
            .map(|(n, parent)| (n, parent.to_owned()))
            .collect();
        for (n, id, parent) in unclear {
            let at = FieldAddr { record: Record::Item(id), field: FieldName::Thread };
            self.refuse(Some(n), Some(at), Rule::ThreadUnclear { parent });
        }
        for (n, placed) in placed {
            self.apply(n, false, Written::Place(placed));
        }
        for (n, parent) in dropped {
            self.apply(n, false, Written::Assign(Assign::Item(parent, ItemAssign::Thread(None))));
        }
    }

    /// Every rule, judged on the model before and after, so only what the change set breaks is refused.
    fn rules(&mut self, changes: &ChangeSet) {
        let (after, steps) = (&self.after, &self.steps);
        let mut found = assign_rules(after, steps);
        found.extend(empty_creates(steps));
        found.extend(shared_ids(self.before, steps));
        // Cycles and lineage links are walks of the whole tracker, and only a write of a parent or a link can change
        // them, so a change set with none (a title, a state, notes) has nothing new to find.
        if steps.iter().any(|step| writes_graph(&step.write)) {
            found.extend(new_cycles(self.before, after, steps));
            found.extend(new_lineage_links(self.before, after, steps));
        }
        found.extend(taken_names(after, steps));
        found.extend(taken_titles(after, changes, steps));
        found.extend(taken_item_titles(self.before, after, steps));
        found.extend(cancel_rules(self.before, steps));
        self.violations.extend(found);
    }
}

/// Whether a write sets an item's parent or a waits-for link: the only writes that change what `after` holds of either.
fn writes_graph(write: &Written) -> bool {
    matches!(
        write,
        Written::Assign(Assign::Item(_, ItemAssign::Parent(_) | ItemAssign::WaitsFor(..)))
    )
}

/// New records need an emitted field write; a projected `Written::Create` alone has no effect when applied.
fn empty_creates(steps: &[Step]) -> Vec<Violation> {
    let writes: BTreeSet<Record> = steps
        .iter()
        .filter_map(|step| match &step.write {
            Written::Assign(assign) => Some(assign.at().record),
            Written::Place(placed) => Some(Record::Item(placed.item.clone())),
            Written::Create(_) => None,
        })
        .collect();
    steps
        .iter()
        .filter_map(|step| match &step.write {
            Written::Create(record) if !writes.contains(record) => Some(Violation {
                change: Some(step.change),
                at: None,
                rule: Rule::EmptyRecord(record.clone()),
            }),
            _ => None,
        })
        .collect()
}

/// The rules each asked-for field brings: what it names exists, an item never waits for itself, evidence is only on done
/// work, and only a leaf is stored done.
fn assign_rules(after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut found = Vec::new();
    for step in steps.iter().filter(|step| step.asked) {
        let Written::Assign(assign) = &step.write else { continue };
        let at = assign.at();
        let mut refuse = |rule: Rule| {
            found.push(Violation { change: Some(step.change), at: Some(at.clone()), rule });
        };
        match assign {
            Assign::Item(id, assign) => match assign {
                ItemAssign::Parent(parent)
                    if !after.items.contains_key(parent)
                        && !after.initiatives.contains_key(parent) =>
                {
                    refuse(Rule::NoRecord(Record::Item(parent.clone())));
                },
                ItemAssign::Thread(Some(thread)) if !after.threads.contains_key(thread) => {
                    refuse(Rule::NoRecord(Record::Thread(thread.clone())));
                },
                ItemAssign::WaitsFor(to, true) if !after.items.contains_key(to) => {
                    refuse(Rule::NoRecord(Record::Item(to.clone())));
                },
                ItemAssign::Evidence(Some(evidence)) => {
                    if evidence.commit.as_deref().is_some_and(|commit| !is_object_id(commit)) {
                        refuse(Rule::InvalidCommit);
                    }
                    match evidence.repository.as_ref().map(|r| (r, after.repositories.get(r))) {
                        Some((repository, None)) => {
                            refuse(Rule::NoRecord(Record::Repository(repository.clone())));
                        },
                        Some((repository, Some(record))) => {
                            let missing: Vec<&'static str> = [
                                ("owner", record.owner.one().is_some()),
                                ("name", record.name.one().is_some()),
                                ("default-branch", record.default_branch.one().is_some()),
                            ]
                            .into_iter()
                            .filter_map(|(field, set)| (!set).then_some(field))
                            .collect();
                            if !missing.is_empty() {
                                refuse(Rule::IncompleteRepository {
                                    repository: repository.clone(),
                                    missing,
                                });
                            }
                        },
                        None => {},
                    }
                    if !after.items.get(id).is_some_and(Item::is_done) {
                        refuse(Rule::EvidenceNotDone);
                    }
                },
                ItemAssign::State(ItemState::Done) if !after.children(id).is_empty() => {
                    refuse(Rule::ParentDone);
                },
                _ => {},
            },
            Assign::Initiative(..) | Assign::Thread(..) | Assign::Repository(..) => {},
        }
    }
    found
}

/// A new initiative or item whose id another kind has, live or deleted: an item's `parent` names an item or an
/// initiative by its id alone, so one id may not name both. Judged on the model before, as the id was free to take.
fn shared_ids(before: &Model, steps: &[Step]) -> Vec<Violation> {
    steps
        .iter()
        .filter_map(|step| {
            let Written::Create(record) = &step.write else { return None };
            let own = record.kind();
            if !matches!(record, Record::Initiative(_) | Record::Item(_))
                || !before.id_held_beyond(own, record.id())
            {
                return None;
            }
            let other = if own == kind::ITEM {
                Record::Initiative(record.id().to_owned())
            } else {
                Record::Item(record.id().to_owned())
            };
            Some(Violation { change: Some(step.change), at: None, rule: Rule::IdTaken(other) })
        })
        .collect()
}

/// Canceling done work, or a parent that reads done, is refused: reopen it first. Judged on the model before.
fn cancel_rules(before: &Model, steps: &[Step]) -> Vec<Violation> {
    steps
        .iter()
        .filter(|step| step.asked)
        .filter_map(|step| match &step.write {
            Written::Assign(assign @ Assign::Item(id, ItemAssign::State(ItemState::Canceled)))
                if before.reads_done(id) =>
            {
                Some(Violation {
                    change: Some(step.change),
                    at: Some(assign.at()),
                    rule: Rule::CancelDone,
                })
            },
            _ => None,
        })
        .collect()
}

/// Cycles of parents or waits-for the change set closes: one already there stays the user's to break.
fn new_cycles(before: &Model, after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut found = Vec::new();
    // Cycles are judged whole: one already there stays the user's to break, and only a new one is refused, or one a
    // write chooses an edge of, as settling a conflicted parent on the value that closes a cycle would.
    let closes = |parents: bool, cycle: &[String]| {
        steps.iter().any(|step| {
            let (id, to) = match &step.write {
                Written::Assign(Assign::Item(id, ItemAssign::Parent(to))) if parents => (id, to),
                Written::Assign(Assign::Item(id, ItemAssign::WaitsFor(to, true))) if !parents => {
                    (id, to)
                },
                _ => return false,
            };
            cycle.contains(id) && cycle.contains(to)
        })
    };
    let cycles = |model: &Model| -> BTreeSet<(bool, Vec<String>)> {
        model
            .parent_cycles()
            .into_iter()
            .map(|cycle| (true, cycle))
            .chain(model.waits_for_cycles().into_iter().map(|cycle| (false, cycle)))
            .collect()
    };
    let old = cycles(before);
    for (parents, cycle) in cycles(after).into_iter().filter(|(parents, cycle)| {
        !old.contains(&(*parents, cycle.clone())) || closes(*parents, cycle)
    }) {
        let change = steps.iter().find(|step| match &step.write {
            Written::Assign(Assign::Item(id, ItemAssign::Parent(_))) if parents => {
                cycle.contains(id)
            },
            Written::Assign(Assign::Item(id, ItemAssign::WaitsFor(_, true))) if !parents => {
                cycle.contains(id)
            },
            _ => false,
        });
        // Named at the write that closed it, so a refusal names the item moved or made to wait.
        let at = change.and_then(|step| match &step.write {
            Written::Assign(assign) => Some(assign.at()),
            _ => None,
        });
        found.push(Violation {
            change: change.map(|step| step.change),
            at,
            rule: if parents { Rule::ParentCycle(cycle) } else { Rule::WaitsForCycle(cycle) },
        });
    }
    found
}

/// Waits-for links between an item and one above or below it that the change set makes, by a link or by a move:
/// neither could ever be done, since a parent reads done only once its leaves are. One a sync brought is left be.
fn new_lineage_links(before: &Model, after: &Model, steps: &[Step]) -> Vec<Violation> {
    let lineage = |model: &Model, from: &str, to: &str| {
        model.ancestors(from).contains(&to) || model.ancestors(to).contains(&from)
    };
    // Items in a parent cycle sit above and below each other, and the cycle is refused on its own.
    let cyclic: BTreeSet<String> = after.parent_cycles().into_iter().flatten().collect();
    let mut found = Vec::new();
    for link in &after.links {
        let (from, to) = (link.from.as_str(), link.to.as_str());
        let held = before.links.iter().any(|l| l.from == from && l.to == to);
        if cyclic.contains(from)
            || cyclic.contains(to)
            || !lineage(after, from, to)
            || (held && lineage(before, from, to))
        {
            continue;
        }
        let involved: BTreeSet<&str> = [from, to]
            .into_iter()
            .chain(after.ancestors(from))
            .chain(after.ancestors(to))
            .collect();
        // Named at the link made, else at the move that brought the two into one line.
        let step = steps
            .iter()
            .find(|step| {
                matches!(&step.write, Written::Assign(Assign::Item(id, ItemAssign::WaitsFor(t, true)))
                    if id == from && t == to)
            })
            .or_else(|| {
                steps.iter().find(|step| {
                    matches!(&step.write, Written::Assign(Assign::Item(id, ItemAssign::Parent(_)))
                        if involved.contains(id.as_str()))
                })
            });
        let at = step.and_then(|step| match &step.write {
            Written::Assign(assign) => Some(assign.at()),
            _ => None,
        });
        found.push(Violation {
            change: step.map(|step| step.change),
            at,
            rule: Rule::LineageLink { from: from.to_owned(), to: to.to_owned() },
        });
    }
    found
}

/// The field a change's first assign names, or the record's title when it gives none.
fn assign_at(assigns: &[Assign]) -> FieldAddr {
    assigns.first().map_or_else(
        || FieldAddr { record: Record::Item(String::new()), field: FieldName::Title },
        Assign::at,
    )
}

/// Repositories added or renamed whose `owner/name`, in any case, another record has or could have: a record whose
/// owner or name is in conflict holds every name its values could make, so settling it never exposes a clash.
fn taken_names(after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut renamed: BTreeMap<&str, usize> = BTreeMap::new();
    for step in steps.iter().filter(|step| step.asked) {
        match &step.write {
            Written::Create(Record::Repository(id))
            | Written::Assign(Assign::Repository(
                id,
                RepositoryAssign::Owner(_) | RepositoryAssign::Name(_),
            )) => {
                renamed.entry(id).or_insert(step.change);
            },
            _ => {},
        }
    }
    let names = |record: &Repository| -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for owner in record.owner.all() {
            for name in record.name.all() {
                names.insert(format!("{owner}/{name}").to_lowercase());
            }
        }
        names
    };
    let mut found = Vec::new();
    for (id, n) in renamed {
        let Some(record) = after.repositories.get(id) else { continue };
        let mine = names(record);
        for other in after.repositories.values().filter(|other| other.id != id) {
            for name in mine.intersection(&names(other)) {
                found.push(Violation {
                    change: Some(n),
                    at: Some(FieldAddr {
                        record: Record::Repository(id.to_owned()),
                        field: FieldName::Name,
                    }),
                    rule: Rule::NameTaken { name: name.clone(), other: other.id.clone() },
                });
            }
        }
    }
    found
}

/// Minted threads whose title another thread already has: a retried add would otherwise make a twin. An item's title
/// has its own rule, [`taken_item_titles`].
fn taken_titles(after: &Model, changes: &ChangeSet, steps: &[Step]) -> Vec<Violation> {
    let created: BTreeSet<&Record> = steps
        .iter()
        .filter_map(|step| match &step.write {
            Written::Create(record) => Some(record),
            _ => None,
        })
        .collect();
    let mut found = Vec::new();
    for (n, change) in changes.0.iter().enumerate() {
        let Change::Add { record: record @ Record::Thread(id), minted: true, .. } = change else {
            continue;
        };
        if !created.contains(record) {
            continue;
        }
        let Some(title) = after.threads.get(id).and_then(|thread| thread.title.one()) else {
            continue;
        };
        let other = after.threads.values().find(|other| {
            other.id != *id
                && other.title.all().contains(&title)
                && !after.is_emptied_one(&other.id)
        });
        if let Some(other) = other {
            found.push(Violation {
                change: Some(n),
                at: Some(FieldAddr { record: record.clone(), field: FieldName::Title }),
                rule: Rule::TitleTaken(Record::Thread(other.id.clone())),
            });
        }
    }
    found
}

/// Items whose write brings them a title another item in their initiative holds: added, renamed, moved (with all they
/// hold, since the initiative they are in changes), reopened or un-canceled, or given a first child, which makes a
/// finished leaf a parent that keeps its name. A clash the tracker already had, as a sync can make, isn't refused again
/// while the pair stands: only what the change set makes is.
fn taken_item_titles(before: &Model, after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut touched: BTreeMap<&str, usize> = BTreeMap::new();
    for step in steps.iter().filter(|step| step.asked) {
        match &step.write {
            Written::Create(Record::Item(id))
            | Written::Assign(Assign::Item(id, ItemAssign::Title(_) | ItemAssign::State(_))) => {
                touched.entry(id).or_insert(step.change);
            },
            Written::Assign(Assign::Item(id, ItemAssign::Parent(parent))) => {
                for moved in after.subtree(id) {
                    touched.entry(moved).or_insert(step.change);
                }
                touched.entry(parent).or_insert(step.change);
            },
            _ => {},
        }
    }
    let clashing: BTreeSet<(&str, &str)> = after
        .duplicate_titles()
        .into_iter()
        .flat_map(|group| {
            group.iter().flat_map(|a| group.iter().map(move |b| (*a, *b))).collect::<Vec<_>>()
        })
        .filter(|(a, b)| a != b)
        .collect();
    let already: BTreeSet<(&str, &str)> = before
        .duplicate_titles()
        .into_iter()
        .flat_map(|group| {
            group.iter().flat_map(|a| group.iter().map(move |b| (*a, *b))).collect::<Vec<_>>()
        })
        .collect();
    let mut found = Vec::new();
    for (id, n) in touched {
        let other = clashing.iter().find(|(a, b)| *a == id && !already.contains(&(*a, *b)));
        if let Some((_, other)) = other {
            found.push(Violation {
                change: Some(n),
                at: Some(FieldAddr {
                    record: Record::Item(id.to_owned()),
                    field: FieldName::Title,
                }),
                rule: Rule::TitleTaken(Record::Item((*other).to_owned())),
            });
        }
    }
    found
}

/// The writes finishing an item asks for: its evidence, when given, and done.
fn finishing(id: &str, evidence: Option<&Evidence>) -> Vec<Assign> {
    evidence
        .map(|evidence| Assign::Item(id.to_owned(), ItemAssign::Evidence(Some(evidence.clone()))))
        .into_iter()
        .chain([Assign::Item(id.to_owned(), ItemAssign::State(ItemState::Done))])
        .collect()
}

/// Whether an assign would change nothing: the field holds exactly its value and isn't in conflict, or a set already
/// has or lacks its element. A raw conflict the typed value can't see still takes the write, which settles it.
fn holds(model: &Model, assign: &Assign) -> bool {
    let conflicted = |record: &str, id: &str, name: &str| {
        model.conflicts.contains(&(record.to_owned(), id.to_owned(), name.to_owned()))
    };
    match assign {
        Assign::Initiative(id, assign) => {
            let Some(record) = model.initiatives.get(id) else { return false };
            match assign {
                InitiativeAssign::Title(v) => {
                    !conflicted(kind::INITIATIVE, id, initiative::TITLE)
                        && record.title.holds(Some(v))
                },
                InitiativeAssign::Notes(v) => {
                    !conflicted(kind::INITIATIVE, id, initiative::NOTES)
                        && record.notes.holds(v.as_ref())
                },
            }
        },
        Assign::Item(id, assign) => {
            let Some(item) = model.items.get(id) else { return false };
            let name = assign_name(assign);
            if !matches!(assign, ItemAssign::WaitsFor(..)) && conflicted(kind::ITEM, id, name) {
                return false;
            }
            match assign {
                ItemAssign::Title(v) => item.title.holds(Some(v)),
                ItemAssign::Notes(v) => item.notes.holds(v.as_ref()),
                ItemAssign::State(v) => item.state.holds(Some(v)),
                ItemAssign::Parent(v) => item.parent.holds(Some(v)),
                ItemAssign::Evidence(v) => item.evidence.holds(v.as_ref()),
                ItemAssign::Thread(v) => item.thread.holds(v.as_ref()),
                ItemAssign::WaitsFor(to, present) => {
                    model.links.iter().any(|l| l.from == *id && l.to == *to) == *present
                },
            }
        },
        Assign::Thread(id, assign) => {
            let Some(record) = model.threads.get(id) else { return false };
            match assign {
                ThreadAssign::Title(v) => {
                    !conflicted(kind::THREAD, id, thread::TITLE) && record.title.holds(Some(v))
                },
                ThreadAssign::Notes(v) => {
                    !conflicted(kind::THREAD, id, thread::BRIEF) && record.notes.holds(v.as_ref())
                },
            }
        },
        Assign::Repository(id, assign) => {
            let Some(record) = model.repositories.get(id) else { return false };
            let (name, value) = match assign {
                RepositoryAssign::Owner(v) => (repository::OWNER, &record.owner.holds(Some(v))),
                RepositoryAssign::Name(v) => (repository::NAME, &record.name.holds(Some(v))),
                RepositoryAssign::DefaultBranch(v) => {
                    (repository::DEFAULT_BRANCH, &record.default_branch.holds(Some(v)))
                },
            };
            !conflicted(kind::REPOSITORY, id, name) && *value
        },
    }
}

const fn assign_name(assign: &ItemAssign) -> &'static str {
    match assign {
        ItemAssign::Title(_) => field::TITLE,
        ItemAssign::Notes(_) => field::NOTES,
        ItemAssign::State(_) => field::STATE,
        ItemAssign::Parent(_) => field::PARENT,
        ItemAssign::Evidence(_) => field::EVIDENCE,
        ItemAssign::Thread(_) => field::THREAD,
        ItemAssign::WaitsFor(..) => link::PRESENT,
    }
}

fn empty_item(id: &str) -> Item {
    Item {
        id: id.to_owned(),
        title: Value::None,
        notes: Value::None,
        state: Value::None,
        done_at: None,
        parent: Value::None,
        evidence: Value::None,
        thread: Value::None,
    }
}

fn one<T>(value: Option<T>) -> Value<T> {
    value.map_or(Value::None, Value::One)
}

/// Makes a write in the model in memory, as reading the state it leaves would give it.
fn project(model: &mut Model, write: &Written, at: Millis) {
    let settled = |model: &mut Model, record: &str, id: &str, name: &str| {
        model.conflicts.remove(&(record.to_owned(), id.to_owned(), name.to_owned()));
    };
    match write {
        Written::Create(_) => {},
        Written::Assign(Assign::Initiative(id, assign)) => {
            let Some(record) = model.initiatives.get_mut(id) else { return };
            let name = match assign {
                InitiativeAssign::Title(v) => {
                    record.title = Value::One(v.clone());
                    initiative::TITLE
                },
                InitiativeAssign::Notes(v) => {
                    record.notes = one(v.clone());
                    initiative::NOTES
                },
            };
            settled(model, kind::INITIATIVE, id, name);
        },
        Written::Assign(Assign::Item(id, assign)) => project_item(model, id, assign, at),
        Written::Assign(Assign::Thread(id, assign)) => {
            let Some(record) = model.threads.get_mut(id) else { return };
            let name = match assign {
                ThreadAssign::Title(v) => {
                    record.title = Value::One(v.clone());
                    thread::TITLE
                },
                ThreadAssign::Notes(v) => {
                    record.notes = one(v.clone());
                    thread::BRIEF
                },
            };
            settled(model, kind::THREAD, id, name);
        },
        Written::Assign(Assign::Repository(id, assign)) => {
            let Some(record) = model.repositories.get_mut(id) else { return };
            let name = match assign {
                RepositoryAssign::Owner(v) => {
                    record.owner = Value::One(v.clone());
                    repository::OWNER
                },
                RepositoryAssign::Name(v) => {
                    record.name = Value::One(v.clone());
                    repository::NAME
                },
                RepositoryAssign::DefaultBranch(v) => {
                    record.default_branch = Value::One(v.clone());
                    repository::DEFAULT_BRANCH
                },
            };
            settled(model, kind::REPOSITORY, id, name);
        },
        Written::Place(placed) => {
            if let Some(item) = model.items.get_mut(&placed.item) {
                item.thread = Value::One(placed.thread.clone());
            }
            settled(model, kind::ITEM, &placed.item, field::THREAD);
        },
    }
}

/// Makes a write to an item's field in the model in memory.
fn project_item(model: &mut Model, id: &str, assign: &ItemAssign, at: Millis) {
    if let ItemAssign::Parent(parent) = assign {
        let old: Vec<String> = model
            .items
            .get(id)
            .map(|item| item.parent.all().into_iter().cloned().collect())
            .unwrap_or_default();
        for old in old {
            if let Some(children) = model.children.get_mut(&old) {
                children.remove(id);
                if children.is_empty() {
                    model.children.remove(&old);
                }
            }
        }
        model.children.entry(parent.clone()).or_default().insert(id.to_owned());
    }
    if let ItemAssign::WaitsFor(to, present) = assign {
        let link = model::Link { from: id.to_owned(), to: to.clone() };
        let key = model::link_key(id, to).map(|key| key.id).unwrap_or_default();
        if *present {
            model.removed_links.remove(&key);
            if let Err(at) = model.links.binary_search(&link) {
                model.links.insert(at, link);
            }
        } else if let Ok(at) = model.links.binary_search(&link) {
            model.links.remove(at);
            model.removed_links.insert(key);
        }
        return;
    }
    let Some(item) = model.items.get_mut(id) else { return };
    let name = assign_name(assign);
    match assign {
        ItemAssign::Title(v) => item.title = Value::One(v.clone()),
        ItemAssign::Notes(v) => item.notes = one(v.clone()),
        ItemAssign::State(v) => {
            item.state = Value::One(*v);
            item.done_at = (*v == ItemState::Done).then_some(at);
        },
        ItemAssign::Parent(v) => item.parent = Value::One(v.clone()),
        ItemAssign::Evidence(v) => item.evidence = one(v.clone()),
        ItemAssign::Thread(v) => item.thread = one(v.clone()),
        ItemAssign::WaitsFor(..) => unreachable!("handled above"),
    }
    model.conflicts.remove(&(kind::ITEM.to_owned(), id.to_owned(), name.to_owned()));
}

/// How many fields a write sets in the tracker: a link made anew sets its ends and kind too.
#[must_use]
pub fn registers(before: &Model, write: &Written) -> usize {
    match write {
        Written::Create(_) => 0,
        Written::Assign(Assign::Item(from, ItemAssign::WaitsFor(to, true))) => {
            let key = model::link_key(from, to).map(|key| key.id).unwrap_or_default();
            if before.removed_links.contains(&key) { 1 } else { 4 }
        },
        _ => 1,
    }
}

/// Makes an accepted outcome's writes.
///
/// # Errors
///
/// Fails when the state layer refuses a write, such as for an invalid name.
pub fn write(replica: &mut Replica, outcome: &Outcome, by: By<'_>) -> Result<(), WriteError> {
    for step in &outcome.steps {
        match &step.write {
            Written::Create(_) => {},
            Written::Assign(assign) => emit(replica, assign, by)?,
            Written::Place(placed) => {
                model::put(
                    replica,
                    &model::item_key(&placed.item)?,
                    field::THREAD,
                    Some(&placed.thread),
                    by,
                )?;
            },
        }
    }
    Ok(())
}

fn emit(replica: &mut Replica, assign: &Assign, by: By<'_>) -> Result<(), WriteError> {
    let record = assign.at().record;
    let key = record.key()?;
    match assign {
        Assign::Item(from, ItemAssign::WaitsFor(to, present)) => {
            let key = model::link_key(from, to)?;
            if *present {
                if replica.state.field(&key, link::FROM).is_none() {
                    model::put(replica, &key, link::FROM, Some(from), by)?;
                    model::put(replica, &key, link::TO, Some(to), by)?;
                    model::put(replica, &key, link::KIND, Some(&link::BLOCKS), by)?;
                }
                model::put(replica, &key, link::PRESENT, Some(&true), by)
            } else {
                replica.write(&key, link::PRESENT, by.actor, by.at, None)?;
                Ok(())
            }
        },
        Assign::Item(_, item) => {
            let value = assign.value();
            replica.write(&key, assign_name(item), by.actor, by.at, Some(value))?;
            Ok(())
        },
        Assign::Initiative(_, InitiativeAssign::Title(_)) => {
            put_json(replica, &key, initiative::TITLE, assign, by)
        },
        Assign::Initiative(_, InitiativeAssign::Notes(_)) => {
            put_json(replica, &key, initiative::NOTES, assign, by)
        },
        Assign::Thread(_, ThreadAssign::Title(_)) => {
            put_json(replica, &key, thread::TITLE, assign, by)
        },
        Assign::Thread(_, ThreadAssign::Notes(_)) => {
            put_json(replica, &key, thread::BRIEF, assign, by)
        },
        Assign::Repository(_, RepositoryAssign::Owner(_)) => {
            put_json(replica, &key, repository::OWNER, assign, by)
        },
        Assign::Repository(_, RepositoryAssign::Name(_)) => {
            put_json(replica, &key, repository::NAME, assign, by)
        },
        Assign::Repository(_, RepositoryAssign::DefaultBranch(_)) => {
            put_json(replica, &key, repository::DEFAULT_BRANCH, assign, by)
        },
    }
}

fn put_json(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    assign: &Assign,
    by: By<'_>,
) -> Result<(), WriteError> {
    replica.write(key, name, by.actor, by.at, Some(assign.value()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
