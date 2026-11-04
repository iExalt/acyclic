//! Applying a validated import to a replica: an upsert, judged as one change set by [`crate::write`]. Every field the
//! document sets is written when it differs, so importing the same document twice writes nothing. A single value the
//! document leaves out is kept; a list it gives (an item's `waits_for` and `repositories`, a thread's `repositories`)
//! is the whole of it, so what the list leaves out is removed. The judge refuses references to nothing, repository
//! names another record holds, cycles the import would close, and evidence and archiving on work that isn't done, and
//! brings what any write brings: done work set back is reopened, and new or moved work with no thread joins the one
//! passed down to it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{Import, KeyKind, ValidationError, owner_name};
use crate::archive::{self, ArchiveError};
use crate::model::{By, Evidence, ItemState, Model, Size, Value, WriteError};
use crate::place::Inherited;
use crate::ready::{Ancestry, Readiness};
use crate::schema::kind;
use crate::state::{Error, Replica};
use crate::write::{
    self, Assign, Change as Write, ChangeSet, FieldAddr, FieldName, ItemAssign, Outcome, Record,
    Repeat, RepositoryAssign, Rule, ThreadAssign, Violation, Written,
};

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Repository records created.
    pub repositories: usize,
    /// Items created.
    pub items: usize,
    /// Links created or restored.
    pub links: usize,
    /// Threads created.
    pub threads: usize,
    /// Fields written, the created records' included.
    pub fields: usize,
    /// Each record the import creates, changes or removes, in the order they were met.
    pub changes: Vec<Change>,
    /// The new or moved work with no thread that joined the one passed down to it.
    #[serde(skip)]
    pub placed: Vec<Inherited>,
}

/// What an import did to one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// The record's kind: `repository`, `thread`, `item` or `link`.
    pub entity: &'static str,
    /// Which record.
    #[serde(flatten)]
    pub name: Name,
    /// What happened to it.
    pub action: Action,
    /// The fields that changed, by name in snake case; none for a link.
    pub fields: BTreeMap<String, Edit>,
}

/// How a change names its record: by id, or, for a link, by the items and kind it joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Name {
    /// A repository, thread or item.
    Id {
        /// Its id.
        id: String,
    },
    /// A link.
    Link {
        /// The dependent item.
        from: String,
        /// The kind as the command line names it: always `waits-for`.
        kind: &'static str,
        /// The item it depends on.
        to: String,
    },
}

/// What an import did to a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// The record is new.
    Create,
    /// The record exists, and fields of it changed.
    Update,
    /// The link is gone: the item's `waits_for` left it out.
    Remove,
}

/// A field's value before an import and after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edit {
    /// The value before; `null` when unset, and the whole set for a set field.
    pub from: Json,
    /// The value after.
    pub to: Json,
}

/// Why an import was refused. The replica may then hold some of the writes: apply to a copy, or inside a store batch,
/// which discards them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The document breaks these rules, on its own or with what the tracker holds.
    Invalid(Vec<ValidationError>),
    /// On its own or with what is already there, the document closes these cycles of parents or waits-for.
    Cycles(Vec<Vec<String>>),
    /// Archiving an imported item was refused.
    Archive(ArchiveError),
    /// The judge refused a change the import mapper does not understand.
    Internal(Vec<Violation>),
    /// A write was refused.
    Write(WriteError),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(errors) => {
                write!(f, "the document breaks {}", crate::say::few(errors, "rule", "rules", "; "))
            },
            Self::Cycles(cycles) => {
                let cycles: Vec<String> = cycles.iter().map(|cycle| cycle.join(" -> ")).collect();
                write!(
                    f,
                    "the import would close {}",
                    crate::say::few(&cycles, "cycle", "cycles", "; ")
                )
            },
            Self::Archive(err) => err.fmt(f),
            Self::Internal(violations) => {
                let violations: Vec<String> = violations.iter().map(ToString::to_string).collect();
                write!(f, "internal error: import was refused for {}", violations.join("; "))
            },
            Self::Write(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<ArchiveError> for ApplyError {
    fn from(err: ArchiveError) -> Self {
        Self::Archive(err)
    }
}

impl From<WriteError> for ApplyError {
    fn from(err: WriteError) -> Self {
        Self::Write(err)
    }
}

impl From<Error> for ApplyError {
    fn from(err: Error) -> Self {
        Self::Write(WriteError::State(err))
    }
}

/// Applies `doc` to `replica` as `by`.
///
/// # Errors
///
/// Fails when the document is invalid, would close a cycle, archives an ineligible subtree, or a write is refused.
pub fn apply(
    doc: &Import,
    replica: &mut Replica,
    by: By<'_>,
    ancestry: &dyn Ancestry,
) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let before = Model::read(replica.state())?;
    let outcome = judged(doc, &before, by)?;
    archive_eligibility(&before, &outcome.after, replica.state(), by, ancestry)?;
    write::write(replica, &outcome, by)?;
    Ok(report(&before, &outcome))
}

/// What applying `doc` to `replica` would do, writing nothing.
///
/// # Errors
///
/// Fails as [`apply`] would.
pub fn preview(
    doc: &Import,
    replica: &Replica,
    by: By<'_>,
    ancestry: &dyn Ancestry,
) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let before = Model::read(replica.state())?;
    let outcome = judged(doc, &before, by)?;
    archive_eligibility(&before, &outcome.after, replica.state(), by, ancestry)?;
    Ok(report(&before, &outcome))
}

fn archive_eligibility(
    before: &Model,
    after: &Model,
    state: &crate::state::State,
    by: By<'_>,
    ancestry: &dyn Ancestry,
) -> Result<(), ApplyError> {
    let roots: Vec<&str> = after
        .items
        .iter()
        .filter(|(id, item)| {
            item.is_archived_root()
                && !before.items.get(*id).is_some_and(crate::model::Item::is_archived_root)
        })
        .map(|(id, _)| id.as_str())
        .collect();
    if roots.is_empty() {
        return Ok(());
    }
    // Clear them together so one candidate cannot hide another's merge or dependent checks.
    let mut candidate = after.clone();
    for root in &roots {
        candidate.items.get_mut(*root).expect("candidate root exists").archived = Value::None;
    }
    let readiness = Readiness::new(state, &candidate, by.at, ancestry);
    for root in roots {
        let reasons = archive::eligibility(&readiness, &candidate, root, by.at)?;
        if !reasons.is_empty() {
            return Err(ArchiveError::Ineligible { item: root.to_owned(), reasons }.into());
        }
    }
    Ok(())
}

fn judged(doc: &Import, before: &Model, by: By<'_>) -> Result<Outcome, ApplyError> {
    write::judge(before, &changes(doc, before), by, None)
        .map_err(|violations| refused(doc, &violations))
}

/// The document as one change set: each record's fields, then the lists it gives, removals before additions, then
/// the threads it puts items in, so a report lists them in that order.
fn changes(doc: &Import, before: &Model) -> ChangeSet {
    let add = |record: Record, with: Vec<Assign>| Write::Add {
        record,
        minted: false,
        on_repeat: Repeat::Update,
        with,
    };
    let mut changes = Vec::new();
    for record in &doc.repositories {
        let (owner, name) = owner_name(&record.name).expect("validated");
        let id = &record.id;
        let field = |assign| Assign::Repository(id.clone(), assign);
        changes.push(add(
            Record::Repository(id.clone()),
            vec![
                field(RepositoryAssign::Owner(owner.to_owned())),
                field(RepositoryAssign::Name(name.to_owned())),
                field(RepositoryAssign::DefaultBranch(record.default_branch.clone())),
            ],
        ));
    }
    for record in &doc.threads {
        let id = &record.id;
        let mut with = vec![Assign::Thread(id.clone(), ThreadAssign::Title(record.title.clone()))];
        if let Some(brief) = &record.brief {
            with.push(Assign::Thread(id.clone(), ThreadAssign::Brief(Some(brief.clone()))));
        }
        changes.push(add(Record::Thread(id.clone()), with));
    }
    for item in &doc.items {
        let with = item_fields(item);
        changes.push(add(Record::Item(item.id.clone()), with));
    }
    for adding in [false, true] {
        for item in &doc.items {
            let id = &item.id;
            if let Some(list) = &item.waits_for {
                let held: BTreeSet<&String> =
                    before.links.iter().filter(|l| l.from == *id).map(|l| &l.to).collect();
                for (to, present) in list_changes(&held, list, adding) {
                    changes.push(Write::Set(Assign::Item(
                        id.clone(),
                        ItemAssign::WaitsFor(to, present),
                    )));
                }
            }
            if let Some(list) = &item.repositories {
                let held = before
                    .items
                    .get(id)
                    .map(|i| i.repositories.iter().collect())
                    .unwrap_or_default();
                for (repository, present) in list_changes(&held, list, adding) {
                    changes.push(Write::Set(Assign::Item(
                        id.clone(),
                        ItemAssign::Repository(repository, present),
                    )));
                }
            }
        }
        for record in &doc.threads {
            if let Some(list) = &record.repositories {
                let held = before
                    .threads
                    .get(&record.id)
                    .map(|t| t.repositories.iter().collect())
                    .unwrap_or_default();
                for (repository, present) in list_changes(&held, list, adding) {
                    let assign = ThreadAssign::Repository(repository, present);
                    changes.push(Write::Set(Assign::Thread(record.id.clone(), assign)));
                }
            }
        }
    }
    for item in &doc.items {
        if let Some(thread) = &item.thread {
            changes.push(Write::Set(Assign::Item(
                item.id.clone(),
                ItemAssign::Thread(Some(thread.clone())),
            )));
        }
    }
    ChangeSet(changes)
}

/// An item's fields as the document gives them. One it leaves out keeps what it holds, finished work included; the
/// judge starts a new item in the backlog.
fn item_fields(item: &super::Item) -> Vec<Assign> {
    let id = &item.id;
    let field = |assign| Assign::Item(id.clone(), assign);
    let mut with = vec![field(ItemAssign::Title(item.title.clone()))];
    if let Some(notes) = &item.notes {
        with.push(field(ItemAssign::Notes(Some(notes.clone()))));
    }
    if let Some(state) = item.state {
        with.push(field(ItemAssign::State(state.into())));
    }
    if let Some(parent) = &item.parent {
        with.push(field(ItemAssign::Parent(Some(parent.clone()))));
    }
    if let Some(size) = item.size {
        with.push(field(ItemAssign::Size(Some(size.into()))));
    }
    if let Some(evidence) = &item.evidence {
        let commit = evidence.commit.as_ref().and_then(|c| c.rsplit_once('@'));
        with.push(field(ItemAssign::Evidence(Some(Evidence {
            repository: commit.map(|(repository, _)| repository.to_owned()),
            commit: commit.map(|(_, sha)| sha.to_owned()),
            text: evidence.text.clone(),
        }))));
    }
    if item.archived {
        with.push(field(ItemAssign::Archive));
    }
    with
}

/// The edits that take a list from `held` to `wanted`: when `adding`, each element of `wanted`, which the judge drops
/// where it is held already, and otherwise the removal of each held element `wanted` leaves out.
fn list_changes(held: &BTreeSet<&String>, wanted: &[String], adding: bool) -> Vec<(String, bool)> {
    let wanted: BTreeSet<&String> = wanted.iter().collect();
    if adding {
        wanted.difference(held).map(|r| ((*r).clone(), true)).collect()
    } else {
        held.difference(&wanted).map(|r| ((*r).clone(), false)).collect()
    }
}

/// The judge's refusal in the import's own terms, one kind at a time as the import has always told them: a deleted
/// record, then references to nothing and names taken, then evidence and archiving on open work, then cycles.
fn refused(doc: &Import, violations: &[Violation]) -> ApplyError {
    let mut deleted = None;
    let mut internal = false;
    let mut unknown = Vec::new();
    let mut taken = Vec::new();
    let mut unfinished = Vec::new();
    let mut cycles = Vec::new();
    for violation in violations {
        let at = violation.at.as_ref();
        match (&violation.rule, at) {
            (Rule::NoRecord(target), Some(at)) => unknown.push(ValidationError::UnknownReference {
                at: where_in(at),
                kind: match target {
                    Record::Item(_) => KeyKind::Item,
                    Record::Thread(_) => KeyKind::Thread,
                    Record::Repository(_) => KeyKind::Repository,
                },
                id: target.id().to_owned(),
            }),
            (Rule::NameTaken { other, .. }, Some(at)) => {
                let id = at.record.id();
                let name = doc
                    .repositories
                    .iter()
                    .find(|r| r.id == id)
                    .map_or_else(String::new, |r| r.name.clone());
                taken.push(ValidationError::RepositoryNameTaken {
                    repository: id.to_owned(),
                    name,
                    other: other.clone(),
                });
            },
            (Rule::EvidenceNotDone, Some(at)) => {
                unfinished.push(ValidationError::EvidenceNotDone(at.record.id().to_owned()));
            },
            (Rule::InvalidCommit, Some(at)) => {
                let item = at.record.id();
                let commit = doc
                    .items
                    .iter()
                    .find(|i| i.id == item)
                    .and_then(|i| i.evidence.as_ref()?.commit.clone())
                    .unwrap_or_default();
                unknown.push(ValidationError::InvalidCommit { item: item.to_owned(), commit });
            },
            (Rule::IncompleteRepository { repository, missing }, Some(at)) => {
                unfinished.push(ValidationError::IncompleteRepository {
                    item: at.record.id().to_owned(),
                    repository: repository.clone(),
                    missing: missing.iter().map(|&field| field.to_owned()).collect(),
                });
            },
            (Rule::ArchivedOpen(root), _) => {
                unfinished.push(ValidationError::ArchivedNotDone(root.clone()));
            },
            (Rule::SelfLink, Some(at)) => {
                unknown.push(ValidationError::SelfLink(at.record.id().to_owned()));
            },
            (Rule::ParentCycle(cycle) | Rule::LinkCycle(cycle), _) => cycles.push(cycle.clone()),
            (Rule::Deleted(record), _) => {
                deleted.get_or_insert(record);
            },
            // Import must report a judge refusal it cannot express, including alongside mapped violations.
            _ => internal = true,
        }
    }
    if internal {
        return ApplyError::Internal(violations.to_vec());
    }
    if let Some(record) = deleted {
        return WriteError::Deleted(record.id().to_owned()).into();
    }
    unknown.sort();
    unknown.dedup();
    taken.sort();
    taken.dedup();
    unknown.extend(taken);
    if !unknown.is_empty() {
        return ApplyError::Invalid(unknown);
    }
    unfinished.sort();
    unfinished.dedup();
    if !unfinished.is_empty() {
        return ApplyError::Invalid(unfinished);
    }
    ApplyError::Cycles(cycles)
}

/// Where in the document a field is, as a validation error says it.
fn where_in(at: &FieldAddr) -> String {
    let field = match &at.field {
        FieldName::Repository(_) => "repositories",
        FieldName::WaitsFor(_) => "waits_for",
        other => other.name(),
    };
    format!("{} {}, {field}", at.record.kind(), at.record.id())
}

/// What an accepted import does, record by record, in the order its writes are made.
fn report(before: &Model, outcome: &Outcome) -> Report {
    let mut report = Report { placed: outcome.placed(), ..Report::default() };
    let mut rows: BTreeMap<(&'static str, String), usize> = BTreeMap::new();
    let mut row = |report: &mut Report, entity: &'static str, id: &str| -> usize {
        *rows.entry((entity, id.to_owned())).or_insert_with(|| {
            report.changes.push(Change {
                entity,
                name: Name::Id { id: id.to_owned() },
                action: Action::Update,
                fields: BTreeMap::new(),
            });
            report.changes.len() - 1
        })
    };
    for step in &outcome.steps {
        if !matches!(step.write, Written::Place(_)) {
            report.fields += write::registers(before, &step.write);
        }
        match &step.write {
            Written::Create(record) => {
                let entity = record.kind();
                match record {
                    Record::Item(_) => report.items += 1,
                    Record::Thread(_) => report.threads += 1,
                    Record::Repository(_) => report.repositories += 1,
                }
                let at = row(&mut report, entity, record.id());
                report.changes[at].action = Action::Create;
            },
            Written::Assign(Assign::Item(from, ItemAssign::WaitsFor(to, present))) => {
                if *present {
                    report.links += 1;
                }
                report.changes.push(Change {
                    entity: kind::LINK,
                    name: Name::Link { from: from.clone(), kind: "waits-for", to: to.clone() },
                    action: if *present { Action::Create } else { Action::Remove },
                    fields: BTreeMap::new(),
                });
            },
            Written::Assign(assign) => {
                let at = assign.at();
                let entity = at.record.kind();
                let n = row(&mut report, entity, at.record.id());
                let fields = &mut report.changes[n].fields;
                match &at.field {
                    FieldName::Repository(element) => {
                        let held = held_set(before, &at.record);
                        let edit = fields
                            .entry("repositories".to_owned())
                            .or_insert_with(|| Edit { from: json!(held), to: json!(held) });
                        let mut to: BTreeSet<String> =
                            serde_json::from_value(edit.to.clone()).expect("a set of names");
                        if assign_present(assign) {
                            to.insert(element.clone());
                        } else {
                            to.remove(element);
                        }
                        edit.to = json!(to);
                    },
                    field => {
                        let edit = Edit { from: held(before, &at), to: assign.value() };
                        fields.insert(field.name().replace('-', "_"), edit);
                    },
                }
            },
            Written::Reopened(id, value) => {
                let n = row(&mut report, kind::ITEM, id);
                let from = before.items.get(id).map_or(Json::Null, |item| json!(item.reopened));
                report.changes[n]
                    .fields
                    .insert("reopened".to_owned(), Edit { from, to: json!(value) });
            },
            Written::Unarchive(id) => {
                let n = row(&mut report, kind::ITEM, id);
                let from = before.items.get(id).map_or(Json::Null, |item| json!(item.archived));
                report.changes[n]
                    .fields
                    .insert("archived".to_owned(), Edit { from, to: Json::Null });
            },
            Written::Place(_) => {},
        }
    }
    report
}

fn assign_present(assign: &Assign) -> bool {
    matches!(
        assign,
        Assign::Item(_, ItemAssign::Repository(_, true))
            | Assign::Thread(_, ThreadAssign::Repository(_, true))
    )
}

/// A record's repository set before the import.
fn held_set(before: &Model, record: &Record) -> BTreeSet<String> {
    match record {
        Record::Item(id) => before.items.get(id).map(|i| i.repositories.clone()),
        Record::Thread(id) => before.threads.get(id).map(|t| t.repositories.clone()),
        Record::Repository(_) => None,
    }
    .unwrap_or_default()
}

/// A field's value before the import: `null` when unset or new, and a conflict as `{"conflict": [values]}`.
fn held(before: &Model, at: &FieldAddr) -> Json {
    let value = match &at.record {
        Record::Item(id) => before.items.get(id).map(|item| match at.field {
            FieldName::Title => json!(item.title),
            FieldName::Notes => json!(item.notes),
            FieldName::State => json!(item.state),
            FieldName::Parent => json!(item.parent),
            FieldName::Size => json!(item.size),
            FieldName::Archived => json!(item.archived),
            FieldName::Evidence => json!(item.evidence),
            FieldName::Thread => json!(item.thread),
            FieldName::Reopened => json!(item.reopened),
            _ => Json::Null,
        }),
        Record::Thread(id) => before.threads.get(id).map(|thread| match at.field {
            FieldName::Title => json!(thread.title),
            FieldName::Brief => json!(thread.brief),
            _ => Json::Null,
        }),
        Record::Repository(id) => before.repositories.get(id).map(|record| match at.field {
            FieldName::Owner => json!(record.owner),
            FieldName::Name => json!(record.name),
            FieldName::DefaultBranch => json!(record.default_branch),
            _ => Json::Null,
        }),
    };
    value.unwrap_or(Json::Null)
}

/// The model's size for an imported one.
impl From<super::Size> for Size {
    fn from(size: super::Size) -> Self {
        match size {
            super::Size::Xs => Self::Xs,
            super::Size::S => Self::S,
            super::Size::M => Self::M,
            super::Size::L => Self::L,
            super::Size::Xl => Self::Xl,
        }
    }
}

/// The model's state for an imported one.
impl From<super::State> for ItemState {
    fn from(state: super::State) -> Self {
        match state {
            super::State::Backlog => Self::Backlog,
            super::State::Parked => Self::Parked,
            super::State::Done => Self::Done,
        }
    }
}

#[cfg(test)]
mod tests;
