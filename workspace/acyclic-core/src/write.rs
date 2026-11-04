//! The one judge of writes to records. A caller states what it means to change as a [`ChangeSet`]; [`judge`] adds what
//! every such change brings with it, whoever asks (done work set back is flagged as reopened and leaves its archive,
//! and new or moved work joins the thread passed down to it), projects the result onto the model in memory, and
//! refuses it only for what it would break that wasn't broken before, naming every such violation. [`commit`] reads
//! the model once, judges, and writes only an accepted change.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::import::is_object_id;
use crate::model::{
    self, By, Evidence, Item, ItemState, Model, Reopened, Repository, Size, Thread, Value,
    WriteError,
};
use crate::place::{Inherited, passed_down};
use crate::ready::{Ancestry, Readiness, Stage, Status};
use crate::schema::{item as field, kind, link, repository, set, thread};
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
    /// A thread's title or brief as read by the model; null means unset.
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
    /// The size.
    Size,
}

/// A thread field an exact edit can restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExpectedThreadField {
    /// The title.
    Title,
    /// The brief.
    Brief,
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
/// Started counts treat every completed leaf as started whether its evidence is merged or not; ancestry can change
/// `done` to `in-review`, but cannot change the count used by Undo.
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
    let ancestry = AssumeUnmerged;
    let readiness = Readiness::new(state, model, now, &ancestry);
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
                let count = started_count(&readiness, model, &waiter)?;
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
                    ExpectedItemField::State => serde_json::to_value(&item.state),
                    ExpectedItemField::Notes => serde_json::to_value(&item.notes),
                    ExpectedItemField::Size => serde_json::to_value(&item.size),
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
                    ExpectedThreadField::Brief => serde_json::to_value(&thread.brief),
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

struct AssumeUnmerged;

impl Ancestry for AssumeUnmerged {
    fn merged(&self, _: &Repository, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

fn started_count(
    readiness: &Readiness<'_>,
    model: &Model,
    item: &str,
) -> Result<usize, StateError> {
    if !model.items.contains_key(item) {
        return Ok(0);
    }
    if model.children(item).is_empty() {
        Ok(usize::from(matches!(
            readiness.stage(item)?,
            Some(Stage::InProgress | Stage::InReview | Stage::Done)
        )))
    } else {
        Ok(readiness.progress(item)?.map_or(0, |rollup| {
            rollup.stages.in_progress + rollup.stages.in_review + rollup.stages.done
        }))
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
            Self::Item(id) | Self::Thread(id) | Self::Repository(id) => id,
        }
    }

    /// The record's kind as stored.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
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
    /// An item's or thread's title.
    Title,
    /// An item's notes.
    Notes,
    /// An item's state.
    State,
    /// An item's parent.
    Parent,
    /// An item's size.
    Size,
    /// Whether an item is an archived subtree's root.
    Archived,
    /// An item's evidence.
    Evidence,
    /// An item's thread.
    Thread,
    /// What reopening an item recorded.
    Reopened,
    /// A thread's brief.
    Brief,
    /// A repository's owner.
    Owner,
    /// A repository's name.
    Name,
    /// A repository's default branch.
    DefaultBranch,
    /// One repository in an item's or thread's set.
    Repository(String),
    /// One item an item waits for.
    WaitsFor(String),
}

impl FieldName {
    /// The field's name as stored and reported; a set's element names the set.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Title => field::TITLE,
            Self::Notes => field::NOTES,
            Self::State => field::STATE,
            Self::Parent => field::PARENT,
            Self::Size => field::SIZE,
            Self::Archived => field::ARCHIVED,
            Self::Evidence => field::EVIDENCE,
            Self::Thread => field::THREAD,
            Self::Reopened => field::REOPENED,
            Self::Brief => thread::BRIEF,
            Self::Owner => repository::OWNER,
            Self::Name => repository::NAME,
            Self::DefaultBranch => repository::DEFAULT_BRANCH,
            Self::Repository(_) => "repositories",
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
    /// Its parent, or the top level.
    Parent(Option<String>),
    /// Its size, or none.
    Size(Option<Size>),
    /// Makes it an archived subtree's root. Only reopening work under it takes that back.
    Archive,
    /// Its evidence, or none.
    Evidence(Option<Evidence>),
    /// Its thread, or none.
    Thread(Option<String>),
    /// Adds a repository it touches, or removes one.
    Repository(String, bool),
    /// Makes it wait for an item, or stop.
    WaitsFor(String, bool),
}

/// A value for one of a thread's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadAssign {
    /// Its title.
    Title(String),
    /// Its brief, or none.
    Brief(Option<String>),
    /// Adds a repository to it explicitly, or removes one.
    Repository(String, bool),
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
            Self::Item(id, assign) => (
                Record::Item(id.clone()),
                match assign {
                    ItemAssign::Title(_) => FieldName::Title,
                    ItemAssign::Notes(_) => FieldName::Notes,
                    ItemAssign::State(_) => FieldName::State,
                    ItemAssign::Parent(_) => FieldName::Parent,
                    ItemAssign::Size(_) => FieldName::Size,
                    ItemAssign::Archive => FieldName::Archived,
                    ItemAssign::Evidence(_) => FieldName::Evidence,
                    ItemAssign::Thread(_) => FieldName::Thread,
                    ItemAssign::Repository(r, _) => FieldName::Repository(r.clone()),
                    ItemAssign::WaitsFor(to, _) => FieldName::WaitsFor(to.clone()),
                },
            ),
            Self::Thread(id, assign) => (
                Record::Thread(id.clone()),
                match assign {
                    ThreadAssign::Title(_) => FieldName::Title,
                    ThreadAssign::Brief(_) => FieldName::Brief,
                    ThreadAssign::Repository(r, _) => FieldName::Repository(r.clone()),
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
            Self::Item(_, assign) => match assign {
                ItemAssign::Title(v) => to_json(v),
                ItemAssign::Notes(v) => to_json(v),
                ItemAssign::State(v) => to_json(v),
                ItemAssign::Parent(v) => to_json(v),
                ItemAssign::Size(v) => to_json(v),
                ItemAssign::Archive => Json::Bool(true),
                ItemAssign::Evidence(v) => to_json(v),
                ItemAssign::Thread(v) => to_json(v),
                ItemAssign::Repository(_, present) | ItemAssign::WaitsFor(_, present) => {
                    Json::Bool(*present)
                },
            },
            Self::Thread(_, assign) => match assign {
                ThreadAssign::Title(v) => to_json(v),
                ThreadAssign::Brief(v) => to_json(v),
                ThreadAssign::Repository(_, present) => Json::Bool(*present),
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
        /// Whether its id was made for this add rather than chosen by the caller: a minted item or thread may not
        /// take the title of open work, since a retried add would otherwise make a twin.
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
    /// Done work set back records that it was reopened, and whether it was archived; work finished again clears it.
    Reopened(String, Option<Reopened>),
    /// Reopening work leaves the archive: each archived root on its line is unarchived.
    Unarchive(String),
    /// New or moved work joins the thread passed down to it.
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
    LinkCycle(Vec<String>),
    /// A minted record would take the title of this open one.
    TitleTaken(Record),
    /// A repository would take `owner/name` that this other record has, or could have once its conflict is settled.
    NameTaken {
        /// The name, as `owner/name`.
        name: String,
        /// The other repository record.
        other: String,
    },
    /// Work would be open under this archived root, or an archived root would have open work under it.
    ArchivedOpen(String),
    /// Evidence on an item that isn't done.
    EvidenceNotDone,
    /// An add of a record that exists gives this field another value.
    RepeatDiffers,
    /// Two changes give this field different values.
    Contradicts,
    /// The item isn't the finisher's to finish: not ready, or held by someone else. With no readiness to judge it by,
    /// no status.
    NotReady(Option<Status>),
    /// A parent with these open children, or held back as a leaf would be, can't be finished yet.
    OpenWork {
        /// Its children neither done nor archived.
        open: Vec<String>,
        /// What would hold it back as a leaf.
        held: Option<Status>,
    },
    /// Done work would get other evidence than it was finished with.
    EvidenceDiffers,
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
            Rule::LinkCycle(path) => {
                write!(f, "items would wait for each other in a cycle: {}", path.join(", "))
            },
            Rule::TitleTaken(other) => {
                write!(f, "{at}{} {} already has that title", other.kind(), other.id())
            },
            Rule::NameTaken { name, other } => write!(f, "{at}repository {other} is {name} too"),
            Rule::ArchivedOpen(root) => {
                write!(f, "{at}archived {root} would have work under it that isn't done")
            },
            Rule::EvidenceNotDone => write!(f, "{at}evidence on an item that isn't done"),
            Rule::RepeatDiffers => write!(f, "{at}it exists with another value"),
            Rule::Contradicts => write!(f, "{at}two changes give it different values"),
            Rule::NotReady(_) => write!(f, "{at}it isn't ready, or someone else holds it"),
            Rule::OpenWork { .. } => write!(f, "{at}it has open children or is held back"),
            Rule::EvidenceDiffers => write!(f, "{at}it is done with other evidence"),
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
/// work is judged by its readiness, which checks merges through `ancestry`; without it, only a forced finish passes.
///
/// # Errors
///
/// Fails when the model can't be read, when `build` fails, when the change set is refused, or when a write is.
pub fn commit<E>(
    replica: &mut Replica,
    by: By<'_>,
    ancestry: Option<&dyn Ancestry>,
    build: impl FnOnce(&Model) -> Result<ChangeSet, E>,
) -> Result<Outcome, CommitError<E>> {
    let before = Model::read(&replica.state).map_err(WriteError::from)?;
    crate::probe::mark("write: model read");
    let changes = build(&before).map_err(CommitError::Build)?;
    let judged = {
        let readiness =
            ancestry.map(|ancestry| Readiness::new(&replica.state, &before, by.at, ancestry));
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
    readiness: Option<&Readiness<'_>>,
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
    readiness: Option<&'a Readiness<'r>>,
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
            let (record, assigns, repeat) = match change {
                Change::Add { record, on_repeat, with, .. } => {
                    (record.clone(), with.as_slice(), Some(*on_repeat))
                },
                Change::Set(assign) => (assign.at().record, std::slice::from_ref(assign), None),
                Change::Finish { item, evidence, .. } => {
                    finish = finishing(item, evidence.as_ref());
                    (Record::Item(item.clone()), finish.as_slice(), None)
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
                    self.refuse(Some(n), Some(at), Rule::Contradicts);
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
                // Scope additions need a fresh dot so an unseen removal cannot erase this explicit intent.
                let reasserts_scope = repeat.is_none()
                    && matches!(
                        assign,
                        Assign::Item(_, ItemAssign::Repository(_, true))
                            | Assign::Thread(_, ThreadAssign::Repository(_, true))
                    );
                if !holds || !exists || reasserts_scope {
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
        match self.readiness.map(|readiness| (readiness, readiness.of(id))) {
            // An unreadable claim leaves readiness unknown, which only forcing passes.
            None | Some((_, Err(_))) => self.refuse(Some(n), at(), Rule::NotReady(None)),
            Some((readiness, Ok(status))) => {
                if status == Some(Status::Parent) {
                    let (open, held) = readiness.unfinished(id);
                    if !open.is_empty() || held.is_some() {
                        self.refuse(Some(n), at(), Rule::OpenWork { open, held });
                    }
                } else if !status.as_ref().is_some_and(|status| status.finishable_by(self.actor)) {
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

    fn create(&mut self, change: usize, record: &Record) {
        // A record exists once a field of it is written: an item always gets its state, a thread or repository needs
        // a field given.
        let id = record.id().to_owned();
        match record {
            Record::Item(_) => {
                self.after.items.insert(id.clone(), empty_item(&id));
            },
            Record::Thread(_) => {
                self.after.threads.insert(
                    id.clone(),
                    Thread {
                        id,
                        title: Value::None,
                        brief: Value::None,
                        repositories: BTreeSet::new(),
                    },
                );
            },
            Record::Repository(_) => {
                self.after.repositories.insert(
                    id.clone(),
                    Repository {
                        id,
                        owner: Value::None,
                        name: Value::None,
                        default_branch: Value::None,
                        installation: Value::None,
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
        self.start_in_backlog(&asked);
        self.reopen(&asked);
        self.place(&asked);
    }

    fn start_in_backlog(&mut self, asked: &[(usize, Assign)]) {
        // A new item with no state given starts in the backlog.
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

    fn reopen(&mut self, asked: &[(usize, Assign)]) {
        let archived_by_change: BTreeSet<String> = asked
            .iter()
            .filter_map(|(_, assign)| match assign {
                Assign::Item(id, ItemAssign::Archive) => Some(id.clone()),
                _ => None,
            })
            .collect();
        // Done work set back is reopened: flagged, and out of every archive on its line, unless the change set itself
        // archives that root, which the archive rule then judges. Each is judged on the model the asked-for writes
        // leave, before any archive is left, so the order of the changes doesn't matter.
        let mut reopens: Vec<(usize, String, bool, Vec<String>)> = Vec::new();
        let mut finished: Vec<(usize, String)> = Vec::new();
        for (n, assign) in asked {
            let Assign::Item(id, ItemAssign::State(state)) = assign else { continue };
            let was_done = self
                .before
                .items
                .get(id)
                .is_some_and(|item| item.state == Value::One(ItemState::Done));
            if was_done && *state != ItemState::Done {
                let line = std::iter::once(id.as_str())
                    .chain(self.after.ancestors(id))
                    .filter(|root| {
                        self.after.items.get(*root).is_some_and(|item| item.archived != Value::None)
                            && !archived_by_change.contains(*root)
                    })
                    .map(str::to_owned)
                    .collect();
                reopens.push((*n, id.clone(), self.after.is_archived(id), line));
            } else if *state == ItemState::Done
                && self.after.items.get(id).is_some_and(|item| item.reopened != Value::None)
            {
                finished.push((*n, id.clone()));
            }
        }
        let mut left: BTreeSet<String> = BTreeSet::new();
        for (n, id, archived, line) in reopens {
            for root in line {
                if left.insert(root.clone()) {
                    self.apply(n, false, Written::Unarchive(root));
                }
            }
            self.apply(n, false, Written::Reopened(id, Some(Reopened { archived })));
        }
        for (n, id) in finished {
            self.apply(n, false, Written::Reopened(id, None));
        }
    }

    fn place(&mut self, asked: &[(usize, Assign)]) {
        // New or moved work with no thread joins the one passed down to it, all judged on one snapshot so one
        // placement doesn't steer the next.
        let threaded: BTreeSet<&str> = asked
            .iter()
            .filter_map(|(_, assign)| match assign {
                Assign::Item(id, ItemAssign::Thread(_)) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let mut moved: Vec<(usize, &str)> = Vec::new();
        for step in &self.steps {
            match &step.write {
                Written::Create(Record::Item(id))
                | Written::Assign(Assign::Item(id, ItemAssign::Parent(_)))
                    if !moved.iter().any(|(_, m)| *m == id) =>
                {
                    moved.push((step.change, id));
                },
                _ => {},
            }
        }
        let snapshot = &self.after;
        let placed: Vec<(usize, Inherited)> = moved
            .into_iter()
            .filter(|(_, id)| !threaded.contains(id))
            .filter_map(|(n, id)| {
                let item = snapshot.items.get(id)?;
                if item.thread != Value::None || item.is_done() {
                    return None;
                }
                let thread = passed_down(snapshot, item.parent.one()?)?;
                Some((n, Inherited { item: id.to_owned(), thread: thread.to_owned() }))
            })
            .collect();
        for (n, placed) in placed {
            self.apply(n, false, Written::Place(placed));
        }
    }

    /// Every rule, judged on the model before and after, so only what the change set breaks is refused.
    fn rules(&mut self, changes: &ChangeSet) {
        let (after, steps) = (&self.after, &self.steps);
        let mut found = assign_rules(after, steps);
        found.extend(empty_creates(steps));
        found.extend(open_under_archive(after, steps));
        found.extend(new_cycles(self.before, after, steps));
        found.extend(taken_names(after, steps));
        found.extend(taken_titles(after, changes, steps));
        self.violations.extend(found);
    }
}

/// New records need an emitted field write; a projected `Written::Create` alone has no effect when applied.
fn empty_creates(steps: &[Step]) -> Vec<Violation> {
    let writes: BTreeSet<Record> = steps
        .iter()
        .filter_map(|step| match &step.write {
            Written::Assign(assign) => Some(assign.at().record),
            Written::Reopened(id, _) | Written::Unarchive(id) => Some(Record::Item(id.clone())),
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
/// work, and an archived root has no open work under it.
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
                ItemAssign::Parent(Some(parent)) if !after.items.contains_key(parent) => {
                    refuse(Rule::NoRecord(Record::Item(parent.clone())));
                },
                ItemAssign::Thread(Some(thread)) if !after.threads.contains_key(thread) => {
                    refuse(Rule::NoRecord(Record::Thread(thread.clone())));
                },
                ItemAssign::Repository(repository, true)
                    if !after.repositories.contains_key(repository) =>
                {
                    refuse(Rule::NoRecord(Record::Repository(repository.clone())));
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
                ItemAssign::Archive
                    if !after
                        .subtree(id)
                        .into_iter()
                        .all(|one| after.items.get(one).is_some_and(Item::is_done)) =>
                {
                    refuse(Rule::ArchivedOpen(id.clone()));
                },
                _ => {},
            },
            Assign::Thread(_, ThreadAssign::Repository(repository, true))
                if !after.repositories.contains_key(repository) =>
            {
                refuse(Rule::NoRecord(Record::Repository(repository.clone())));
            },
            _ => {},
        }
    }
    found
}

/// Work added, moved or set back open under an archived root, which would be hidden.
fn open_under_archive(after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut found = Vec::new();
    // Work added, moved or set back open under an archived root would be hidden.
    let mut opened: BTreeMap<&str, usize> = BTreeMap::new();
    for step in steps.iter().filter(|step| step.asked) {
        match &step.write {
            Written::Create(Record::Item(id))
            | Written::Assign(Assign::Item(id, ItemAssign::Parent(_) | ItemAssign::State(_))) => {
                opened.entry(id).or_insert(step.change);
            },
            _ => {},
        }
    }
    for (id, n) in opened {
        if after.items.get(id).is_some_and(Item::is_done) {
            continue;
        }
        for root in after.ancestors(id) {
            if after.items.get(root).is_some_and(Item::is_archived_root) {
                found.push(Violation {
                    change: Some(n),
                    at: Some(FieldAddr {
                        record: Record::Item(id.to_owned()),
                        field: FieldName::Archived,
                    }),
                    rule: Rule::ArchivedOpen(root.to_owned()),
                });
            }
        }
    }
    found
}

/// Cycles of parents or waits-for the change set closes: one already there stays the user's to break.
fn new_cycles(before: &Model, after: &Model, steps: &[Step]) -> Vec<Violation> {
    let mut found = Vec::new();
    // Cycles are judged whole: one already there stays the user's to break, and only a new one is refused, or one a
    // write chooses an edge of, as settling a conflicted parent on the value that closes a cycle would.
    let closes = |parents: bool, cycle: &[String]| {
        steps.iter().any(|step| {
            let (id, to) = match &step.write {
                Written::Assign(Assign::Item(id, ItemAssign::Parent(Some(to)))) if parents => {
                    (id, to)
                },
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
            .chain(model.link_cycles().into_iter().map(|cycle| (false, cycle)))
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
            rule: if parents { Rule::ParentCycle(cycle) } else { Rule::LinkCycle(cycle) },
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

/// Minted items and threads whose title open work already has: a retried add would otherwise make a twin.
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
        let Change::Add { record, minted: true, .. } = change else { continue };
        if !created.contains(record) {
            continue;
        }
        let other = match record {
            Record::Item(id) => {
                let Some(title) = after.items.get(id).and_then(|item| item.title.one()) else {
                    continue;
                };
                after
                    .items
                    .values()
                    .find(|other| {
                        other.id != *id
                            && other.title.all().contains(&title)
                            && !other.is_done()
                            && !after.is_archived(&other.id)
                    })
                    .map(|other| Record::Item(other.id.clone()))
            },
            Record::Thread(id) => {
                let Some(title) = after.threads.get(id).and_then(|thread| thread.title.one())
                else {
                    continue;
                };
                after
                    .threads
                    .values()
                    .find(|other| other.id != *id && other.title.all().contains(&title))
                    .map(|other| Record::Thread(other.id.clone()))
            },
            Record::Repository(_) => None,
        };
        if let Some(other) = other {
            found.push(Violation {
                change: Some(n),
                at: Some(FieldAddr { record: record.clone(), field: FieldName::Title }),
                rule: Rule::TitleTaken(other),
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
        Assign::Item(id, assign) => {
            let Some(item) = model.items.get(id) else { return false };
            let name = assign_name(assign);
            if !matches!(assign, ItemAssign::Repository(..) | ItemAssign::WaitsFor(..))
                && conflicted(kind::ITEM, id, name)
            {
                return false;
            }
            match assign {
                ItemAssign::Title(v) => item.title.holds(Some(v)),
                ItemAssign::Notes(v) => item.notes.holds(v.as_ref()),
                ItemAssign::State(v) => item.state.holds(Some(v)),
                ItemAssign::Parent(v) => item.parent.holds(v.as_ref()),
                ItemAssign::Size(v) => item.size.holds(v.as_ref()),
                ItemAssign::Archive => item.archived.holds(Some(&true)),
                ItemAssign::Evidence(v) => item.evidence.holds(v.as_ref()),
                ItemAssign::Thread(v) => item.thread.holds(v.as_ref()),
                ItemAssign::Repository(r, present) => item.repositories.contains(r) == *present,
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
                ThreadAssign::Brief(v) => {
                    !conflicted(kind::THREAD, id, thread::BRIEF) && record.brief.holds(v.as_ref())
                },
                ThreadAssign::Repository(r, present) => record.repositories.contains(r) == *present,
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
        ItemAssign::Size(_) => field::SIZE,
        ItemAssign::Archive => field::ARCHIVED,
        ItemAssign::Evidence(_) => field::EVIDENCE,
        ItemAssign::Thread(_) => field::THREAD,
        ItemAssign::Repository(..) => set::REPOSITORY,
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
        size: Value::None,
        archived: Value::None,
        evidence: Value::None,
        thread: Value::None,
        repositories: BTreeSet::new(),
        reopened: Value::None,
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
        Written::Assign(Assign::Item(id, assign)) => project_item(model, id, assign, at),
        Written::Assign(Assign::Thread(id, assign)) => {
            let Some(record) = model.threads.get_mut(id) else { return };
            let name = match assign {
                ThreadAssign::Title(v) => {
                    record.title = Value::One(v.clone());
                    thread::TITLE
                },
                ThreadAssign::Brief(v) => {
                    record.brief = one(v.clone());
                    thread::BRIEF
                },
                ThreadAssign::Repository(r, present) => {
                    if *present {
                        record.repositories.insert(r.clone());
                    } else {
                        record.repositories.remove(r);
                    }
                    return;
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
        Written::Reopened(id, value) => {
            if let Some(item) = model.items.get_mut(id) {
                item.reopened = one(*value);
            }
            settled(model, kind::ITEM, id, field::REOPENED);
        },
        Written::Unarchive(id) => {
            if let Some(item) = model.items.get_mut(id) {
                item.archived = Value::None;
            }
            settled(model, kind::ITEM, id, field::ARCHIVED);
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
        if let Some(parent) = parent {
            model.children.entry(parent.clone()).or_default().insert(id.to_owned());
        }
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
        ItemAssign::Parent(v) => item.parent = one(v.clone()),
        ItemAssign::Size(v) => item.size = one(*v),
        ItemAssign::Archive => item.archived = Value::One(true),
        ItemAssign::Evidence(v) => item.evidence = one(v.clone()),
        ItemAssign::Thread(v) => item.thread = one(v.clone()),
        ItemAssign::Repository(r, present) => {
            if *present {
                item.repositories.insert(r.clone());
            } else {
                item.repositories.remove(r);
            }
            return;
        },
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
            Written::Reopened(id, value) => {
                model::put(replica, &model::item_key(id)?, field::REOPENED, value.as_ref(), by)?;
            },
            Written::Unarchive(id) => {
                model::put::<bool>(replica, &model::item_key(id)?, field::ARCHIVED, None, by)?;
            },
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
        Assign::Item(_, ItemAssign::Repository(r, present))
        | Assign::Thread(_, ThreadAssign::Repository(r, present)) => {
            model::put_element(replica, &key, set::REPOSITORY, r, *present, by)
        },
        Assign::Item(_, item) => {
            let value = assign.value();
            replica.write(&key, assign_name(item), by.actor, by.at, Some(value))?;
            Ok(())
        },
        Assign::Thread(_, ThreadAssign::Title(_)) => {
            put_json(replica, &key, thread::TITLE, assign, by)
        },
        Assign::Thread(_, ThreadAssign::Brief(_)) => {
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
