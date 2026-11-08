//! Applying a validated import to a replica: an upsert, judged as one change set by [`crate::write`]. Every field the
//! document sets is written when it differs, so importing the same document twice writes nothing. A single value the
//! document leaves out is kept; a list it gives (an item's `waits_for`) is the whole of it, so what the list leaves
//! out is removed. The judge refuses references to nothing, repository
//! names another record holds, cycles the import would close, and evidence on work that isn't done, and
//! brings what any write brings: done work set back is reopened, and new work with no thread joins the one its
//! siblings share, and the first child of a parent takes the parent's thread.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{Import, KeyKind, ValidationError, owner_name};
use crate::model::{By, Evidence, Model, Value, WriteError, shown_state};
use crate::place::Inherited;
use crate::schema::kind;
use crate::state::{Error, Replica};
use crate::write::InitiativeAssign;
use crate::write::{
    self, Assign, Change as Write, ChangeSet, FieldAddr, FieldName, ItemAssign, Outcome, Record,
    Repeat, RepositoryAssign, Rule, ThreadAssign, Violation, Written,
};

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Repository records created.
    pub repositories: usize,
    /// Initiatives created.
    pub initiatives: usize,
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
    /// The new work with no thread that joined the one its siblings, or its parent, hold.
    #[serde(skip)]
    pub placed: Vec<Inherited>,
}

/// What an import did to one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// The record's kind: `initiative`, `repository`, `thread`, `item` or `link`.
    pub record: &'static str,
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
    /// An initiative, repository, thread or item.
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
            Self::Internal(violations) => {
                let violations: Vec<String> = violations.iter().map(ToString::to_string).collect();
                write!(f, "internal error: import was refused for {}", violations.join("; "))
            },
            Self::Write(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ApplyError {}

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
/// Fails when the document is invalid, would close a cycle, or a write is refused.
pub fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let before = Model::read(replica.state())?;
    let doc = &named_commits(doc, &before)?;
    let outcome = judged(doc, &before, by)?;
    write::write(replica, &outcome, by)?;
    Ok(report(&before, &outcome))
}

/// What applying `doc` to `replica` would do, writing nothing.
///
/// # Errors
///
/// Fails as [`apply`] would.
pub fn preview(doc: &Import, replica: &Replica, by: By<'_>) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let before = Model::read(replica.state())?;
    let doc = &named_commits(doc, &before)?;
    let outcome = judged(doc, &before, by)?;
    Ok(report(&before, &outcome))
}

/// The document with each evidence commit named `<owner>/<name>@<sha>` given its repository's id instead: the
/// document's repository of that name, else the tracker's, in any case.
fn named_commits(doc: &Import, before: &Model) -> Result<Import, ApplyError> {
    let mut doc = doc.clone();
    let mut unknown = Vec::new();
    for item in &mut doc.items {
        let Some(commit) = item.evidence.as_mut().and_then(|e| e.commit.as_mut()) else {
            continue;
        };
        let Some((named, sha)) = commit.rsplit_once('@') else { continue };
        let Some((owner, name)) = owner_name(named) else { continue };
        let same = |o: &str, n: &str| o.eq_ignore_ascii_case(owner) && n.eq_ignore_ascii_case(name);
        let in_doc = doc_repository(&doc.repositories, &same);
        let in_tracker = || {
            before.repositories.values().find_map(|record| {
                let (Some(o), Some(n)) = (record.owner.one(), record.name.one()) else {
                    return None;
                };
                same(o, n).then(|| record.id.clone())
            })
        };
        match in_doc.or_else(in_tracker) {
            Some(id) => *commit = format!("{id}@{sha}"),
            None => unknown.push(ValidationError::UnknownReference {
                at: format!("item {}, evidence", item.id),
                kind: KeyKind::Repository,
                id: named.to_owned(),
            }),
        }
    }
    if unknown.is_empty() { Ok(doc) } else { Err(ApplyError::Invalid(unknown)) }
}

fn doc_repository(
    repositories: &[super::Repository],
    same: &impl Fn(&str, &str) -> bool,
) -> Option<String> {
    repositories.iter().find_map(|record| {
        let (o, n) = owner_name(&record.name)?;
        same(o, n).then(|| record.id.clone())
    })
}

/// Every new item names a parent, since each belongs to an initiative or to an item under one. An item the tracker has
/// keeps the parent it holds when the document leaves it out.
fn parents_named(doc: &Import, before: &Model) -> Result<(), ApplyError> {
    let missing: Vec<ValidationError> = doc
        .items
        .iter()
        .filter(|item| item.parent.is_none() && !before.items.contains_key(&item.id))
        .map(|item| ValidationError::ParentRequired(item.id.clone()))
        .collect();
    if missing.is_empty() { Ok(()) } else { Err(ApplyError::Invalid(missing)) }
}

fn judged(doc: &Import, before: &Model, by: By<'_>) -> Result<Outcome, ApplyError> {
    parents_named(doc, before)?;
    write::judge(before, &changes(doc, before), by, None)
        .map_err(|violations| refused(doc, before, &violations))
}

/// The document as one change set: each record's fields, then the lists it gives, removals before additions, then
/// the threads it puts items in, so a report lists them in that order.
pub(crate) fn changes(doc: &Import, before: &Model) -> ChangeSet {
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
    for record in &doc.initiatives {
        let id = &record.id;
        let mut with =
            vec![Assign::Initiative(id.clone(), InitiativeAssign::Title(record.title.clone()))];
        if let Some(notes) = &record.notes {
            with.push(Assign::Initiative(id.clone(), InitiativeAssign::Notes(Some(notes.clone()))));
        }
        changes.push(add(Record::Initiative(id.clone()), with));
    }
    for record in &doc.threads {
        let id = &record.id;
        let mut with = vec![Assign::Thread(id.clone(), ThreadAssign::Title(record.title.clone()))];
        if let Some(notes) = &record.notes {
            with.push(Assign::Thread(id.clone(), ThreadAssign::Notes(Some(notes.clone()))));
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
        }
    }
    for item in &doc.items {
        let Some(thread) = &item.thread else { continue };
        // `null` on a new item is a decision to be in no thread, which keeps it from joining its siblings'. On one the
        // tracker has it clears a real thread, and changes nothing when it has none or only its own thread of one.
        if thread.is_none() {
            let held = before.items.get(&item.id).map(|held| &held.thread);
            let own = crate::ids::one(&item.id);
            if matches!(held, Some(Value::None)) || held.is_some_and(|h| h.holds(Some(&own))) {
                continue;
            }
        }
        changes.push(Write::Set(Assign::Item(item.id.clone(), ItemAssign::Thread(thread.clone()))));
    }
    ChangeSet(changes)
}

/// An item's fields as the document gives them. One it leaves out keeps what it holds, finished work included; the
/// judge starts a new item open.
fn item_fields(item: &super::Item) -> Vec<Assign> {
    let id = &item.id;
    let field = |assign| Assign::Item(id.clone(), assign);
    let mut with = vec![field(ItemAssign::Title(item.title.clone()))];
    if let Some(notes) = &item.notes {
        with.push(field(ItemAssign::Notes(Some(notes.clone()))));
    }
    if let Some(state) = item.state {
        with.push(field(ItemAssign::State(state)));
    }
    if let Some(parent) = &item.parent {
        with.push(field(ItemAssign::Parent(parent.clone())));
    }
    if let Some(evidence) = &item.evidence {
        let commit = evidence.commit.as_ref().and_then(|c| c.rsplit_once('@'));
        with.push(field(ItemAssign::Evidence(Some(Evidence {
            repository: commit.map(|(repository, _)| repository.to_owned()),
            commit: commit.map(|(_, sha)| sha.to_owned()),
            text: evidence.text.clone(),
        }))));
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

/// An item's title as the import leaves it: the document's, else the tracker's.
fn item_title(doc: &Import, before: &Model, id: &str) -> Option<String> {
    doc.items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.title.clone())
        .or_else(|| before.items.get(id)?.title.one().cloned())
}

/// The title of the initiative an item is under once the document is applied, following parents the document sets
/// before the tracker's; none for an item in no initiative.
fn initiative_title(doc: &Import, before: &Model, id: &str) -> Option<String> {
    let mut at = id.to_owned();
    // A parent cycle is judged elsewhere; the walk is bounded by what is there.
    for _ in 0..=doc.items.len() + before.items.len() {
        if let Some(found) = doc.initiatives.iter().find(|initiative| initiative.id == at) {
            return Some(found.title.clone());
        }
        if let Some(found) = before.initiatives.get(&at) {
            return found.title.one().cloned();
        }
        let parent = match doc.items.iter().find(|item| item.id == at) {
            Some(item) if item.parent.is_some() => item.parent.clone(),
            _ => before.items.get(&at).and_then(|item| item.parent.one().cloned()),
        };
        at = parent?;
    }
    None
}

/// The judge's refusal in the import's own terms, one kind at a time as the import has always told them: a deleted
/// record, then references to nothing and names taken, then evidence on open work, then cycles.
#[expect(clippy::too_many_lines, reason = "one arm for each rule the import can break")]
fn refused(doc: &Import, before: &Model, violations: &[Violation]) -> ApplyError {
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
                    Record::Initiative(_) => KeyKind::Initiative,
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
            (Rule::TitleTaken(Record::Item(other)), Some(at)) => {
                let item = at.record.id();
                let title = item_title(doc, before, item).unwrap_or_default();
                // Each side of a clash is reported by the other: one error names the pair.
                let (first, second) = if item <= other.as_str() {
                    (item, other.as_str())
                } else {
                    (other.as_str(), item)
                };
                taken.push(ValidationError::TitleTaken {
                    item: first.to_owned(),
                    other: second.to_owned(),
                    title,
                    initiative: initiative_title(doc, before, item),
                });
            },
            (Rule::EvidenceNotDone, Some(at)) => {
                unfinished.push(ValidationError::EvidenceNotDone(at.record.id().to_owned()));
            },
            (Rule::ParentDone, Some(at)) => {
                unfinished.push(ValidationError::DoneParent(at.record.id().to_owned()));
            },
            (Rule::CancelDone, Some(at)) => {
                unfinished.push(ValidationError::CanceledDone(at.record.id().to_owned()));
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
            (Rule::IdTaken(other), _) => {
                unknown.push(ValidationError::SharedId(other.id().to_owned()));
            },
            (Rule::LineageLink { from, to }, _) => {
                unknown.push(ValidationError::LineageLink {
                    item: from.clone(),
                    waits_for: to.clone(),
                });
            },
            (Rule::ThreadUnclear { parent }, Some(at)) => {
                unfinished.push(ValidationError::ThreadUnclear {
                    item: at.record.id().to_owned(),
                    parent: parent.clone(),
                });
            },
            (Rule::SelfLink, Some(at)) => {
                unknown.push(ValidationError::SelfLink(at.record.id().to_owned()));
            },
            (Rule::ParentCycle(cycle) | Rule::WaitsForCycle(cycle), _) => {
                cycles.push(cycle.clone());
            },
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
                record: entity,
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
                    Record::Initiative(_) => report.initiatives += 1,
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
                    record: kind::LINK,
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
                let mut edit = Edit { from: held(before, &at), to: assign.value() };
                if at.field == FieldName::State {
                    // As reads show a state: open, not the `backlog` it is stored as.
                    edit = Edit { from: shown_state(&edit.from), to: shown_state(&edit.to) };
                }
                fields.insert(at.field.name().replace('-', "_"), edit);
            },
            Written::Place(_) => {},
        }
    }
    report
}

/// A field's value before the import: `null` when unset or new, and a conflict as `{"conflict": [values]}`.
fn held(before: &Model, at: &FieldAddr) -> Json {
    let value = match &at.record {
        Record::Initiative(id) => before.initiatives.get(id).map(|record| match at.field {
            FieldName::Title => json!(record.title),
            FieldName::Notes => json!(record.notes),
            _ => Json::Null,
        }),
        Record::Item(id) => before.items.get(id).map(|item| match at.field {
            FieldName::Title => json!(item.title),
            FieldName::Notes => json!(item.notes),
            FieldName::State => json!(item.state),
            FieldName::Parent => json!(item.parent),
            FieldName::Evidence => json!(item.evidence),
            FieldName::Thread => json!(item.thread),
            _ => Json::Null,
        }),
        Record::Thread(id) => before.threads.get(id).map(|thread| match at.field {
            FieldName::Title => json!(thread.title),
            FieldName::Notes => json!(thread.notes),
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

#[cfg(test)]
mod tests;
