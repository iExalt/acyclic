//! Applying a validated import to a replica: an upsert in one pass. Every field the document sets is written when it
//! differs, so importing the same document twice writes nothing. A single value the document leaves out is kept; a
//! list it gives (an item's `waits_for` and `repositories`, a thread's `repositories`) is the whole of it, so what the
//! list leaves out is removed, and removals are written before additions. The document is checked once as a whole,
//! and the result once against what the tracker holds: references, cycles it would close, and evidence and archiving
//! on work that isn't done. The writes themselves skip the per-write checks, which would read the whole model each
//! time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{Import, KeyKind, ValidationError, owner_name};
use crate::model::{self, By, Evidence, ItemState, Model, WriteError};
use crate::schema::{item as field, kind, link, repository, thread};
use crate::scope::{self, ScopeEdit};
use crate::state::{Error, Key, Replica};

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
                    "the import would close {} with the tracker",
                    crate::say::few(&cycles, "cycle", "cycles", "; ")
                )
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
/// Fails when the document is invalid, on its own or with the tracker, when the result would hold a cycle the tracker
/// didn't, or when a write is refused.
pub fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    for item in &doc.items {
        if model::is_deleted(&replica.state, &model::item_key(&item.id)?) {
            return Err(WriteError::Deleted(item.id.clone()).into());
        }
    }
    for thread in &doc.threads {
        if model::is_deleted(&replica.state, &crate::claim::thread_key(&thread.id)?) {
            return Err(WriteError::Deleted(thread.id.clone()).into());
        }
    }
    let before = Model::read(&replica.state)?;
    let errors = unknown_references(doc, &before);
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let cycles_before: BTreeSet<Vec<String>> =
        before.parent_cycles().into_iter().chain(before.link_cycles()).collect();
    let mut writer =
        Writer { replica, by, before: &before, report: Report::default(), rows: BTreeMap::new() };
    writer.records(doc)?;
    // Removals first, so a document that turns a link around never holds both ways at once.
    writer.lists(doc, false)?;
    writer.lists(doc, true)?;
    writer.members(doc)?;

    let after = Model::read(&writer.replica.state)?;
    let errors = unfinished(doc, &after);
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    let new: Vec<Vec<String>> = after
        .parent_cycles()
        .into_iter()
        .chain(after.link_cycles())
        .filter(|cycle| !cycles_before.contains(cycle))
        .collect();
    if !new.is_empty() {
        return Err(ApplyError::Cycles(new));
    }
    Ok(writer.report)
}

/// The references that name no record of their kind, in the document or the tracker.
fn unknown_references(doc: &Import, before: &Model) -> Vec<ValidationError> {
    let mine = |ids: Vec<&String>| ids.into_iter().cloned().collect::<BTreeSet<String>>();
    let items = mine(doc.items.iter().map(|i| &i.id).collect());
    let threads = mine(doc.threads.iter().map(|t| &t.id).collect());
    let repositories = mine(doc.repositories.iter().map(|r| &r.id).collect());
    let known = |kind: KeyKind, id: &String| match kind {
        KeyKind::Item => items.contains(id) || before.items.contains_key(id),
        KeyKind::Thread => {
            threads.contains(id)
                || before.threads.contains_key(id)
                || before.items.values().any(|item| item.thread.all().contains(&id))
        },
        KeyKind::Repository => repositories.contains(id) || before.repositories.contains_key(id),
    };
    let mut errors = Vec::new();
    let mut check = |at: String, kind: KeyKind, id: &String| {
        if !known(kind, id) {
            errors.push(ValidationError::UnknownReference { at, kind, id: id.clone() });
        }
    };
    for record in &doc.threads {
        for id in record.repositories.iter().flatten() {
            check(format!("thread {}, repositories", record.id), KeyKind::Repository, id);
        }
    }
    for item in &doc.items {
        let at = |field: &str| format!("item {}, {field}", item.id);
        if let Some(parent) = &item.parent {
            check(at("parent"), KeyKind::Item, parent);
        }
        if let Some(thread) = &item.thread {
            check(at("thread"), KeyKind::Thread, thread);
        }
        for id in item.repositories.iter().flatten() {
            check(at("repositories"), KeyKind::Repository, id);
        }
        for id in item.waits_for.iter().flatten() {
            check(at("waits_for"), KeyKind::Item, id);
        }
        let commit = item.evidence.as_ref().and_then(|e| e.commit.as_ref());
        if let Some((repository, _)) = commit.and_then(|c| c.rsplit_once('@')) {
            check(at("evidence"), KeyKind::Repository, &repository.to_owned());
        }
    }
    errors.sort();
    errors.dedup();
    errors
}

/// Evidence on an item that isn't done, and archived subtrees with work that isn't, judged by the state each item has
/// once imported: its own where the document leaves its state out.
fn unfinished(doc: &Import, after: &Model) -> Vec<ValidationError> {
    let done = |id: &str| after.items.get(id).is_some_and(model::Item::is_done);
    let mut errors = Vec::new();
    for item in &doc.items {
        if item.evidence.is_some() && !done(&item.id) {
            errors.push(ValidationError::EvidenceNotDone(item.id.clone()));
        }
        if item.archived && !after.subtree(&item.id).into_iter().all(done) {
            errors.push(ValidationError::ArchivedNotDone(item.id.clone()));
        }
    }
    errors
}

struct Writer<'a, 'b> {
    replica: &'a mut Replica,
    by: By<'b>,
    /// The tracker as it was before the import.
    before: &'a Model,
    report: Report,
    /// Where each record's row sits in the report's changes.
    rows: BTreeMap<(&'static str, String), usize>,
}

impl Writer<'_, '_> {
    /// Writes the records' own fields.
    fn records(&mut self, doc: &Import) -> Result<(), ApplyError> {
        for record in &doc.repositories {
            let key = Key::new(kind::REPOSITORY, &record.id)?;
            self.created(&key, |r| &mut r.repositories);
            let (owner, name) = owner_name(&record.name).expect("validated");
            self.field(&key, repository::OWNER, Some(&owner))?;
            self.field(&key, repository::NAME, Some(&name))?;
            self.field(&key, repository::DEFAULT_BRANCH, Some(&record.default_branch))?;
        }
        for record in &doc.threads {
            let key = Key::new(kind::THREAD, &record.id)?;
            self.created(&key, |r| &mut r.threads);
            self.field(&key, thread::TITLE, Some(&record.title))?;
            self.field(&key, thread::BRIEF, record.brief.as_ref())?;
        }
        for item in &doc.items {
            let key = model::item_key(&item.id)?;
            let new = self.created(&key, |r| &mut r.items);
            self.field(&key, field::TITLE, Some(&item.title))?;
            self.field(&key, field::NOTES, item.notes.as_ref())?;
            // Backlog only for a new item: one left out keeps its state, finished work included.
            let state = item.state.or(new.then_some(super::State::Backlog));
            self.field(&key, field::STATE, state.as_ref())?;
            self.field(&key, field::PARENT, item.parent.as_ref())?;
            self.field(&key, field::SIZE, item.size.as_ref())?;
            let evidence = item.evidence.as_ref().map(|evidence| {
                let commit = evidence.commit.as_ref().and_then(|c| c.rsplit_once('@'));
                Evidence {
                    repository: commit.map(|(repository, _)| repository.to_owned()),
                    commit: commit.map(|(_, sha)| sha.to_owned()),
                    text: evidence.text.clone(),
                }
            });
            self.field(&key, field::EVIDENCE, evidence.as_ref())?;
            if item.archived {
                self.field(&key, field::ARCHIVED, Some(&true))?;
            }
        }
        Ok(())
    }

    /// Writes the lists the document gives: when `adding`, what each lacks, and otherwise the removal of what each
    /// holds and the document leaves out.
    fn lists(&mut self, doc: &Import, adding: bool) -> Result<(), ApplyError> {
        for item in &doc.items {
            if let Some(list) = &item.waits_for {
                let held: BTreeSet<&String> =
                    self.before.links.iter().filter(|l| l.from == item.id).map(|l| &l.to).collect();
                let wanted: BTreeSet<&String> = list.iter().collect();
                if adding {
                    for to in &wanted {
                        self.link(&item.id, to)?;
                    }
                } else {
                    for to in held.difference(&wanted) {
                        self.unlink(&item.id, to)?;
                    }
                }
            }
            if let Some(list) = &item.repositories {
                let held = self.before.items.get(&item.id).map(|i| i.repositories.clone());
                for (repository, present) in changes(&held.unwrap_or_default(), list, adding) {
                    let edit =
                        ScopeEdit::ItemRepository { item: item.id.clone(), repository, present };
                    self.scope(&edit)?;
                }
            }
        }
        for record in &doc.threads {
            if let Some(list) = &record.repositories {
                let held = self.before.threads.get(&record.id).map(|t| t.repositories.clone());
                for (repository, present) in changes(&held.unwrap_or_default(), list, adding) {
                    let edit = ScopeEdit::ThreadRepository {
                        thread: record.id.clone(),
                        repository,
                        present,
                    };
                    self.scope(&edit)?;
                }
            }
        }
        Ok(())
    }

    /// Puts each item the document gives a thread in it; an item it gives none stays where it is.
    fn members(&mut self, doc: &Import) -> Result<(), ApplyError> {
        for item in &doc.items {
            if let Some(thread) = &item.thread {
                let key = model::item_key(&item.id)?;
                let held = self.replica.state.field(&key, field::THREAD).into_iter().flatten();
                let values: Vec<&Json> = held.map(|(_, v)| &v.value).collect();
                if values != [&json!(thread)] {
                    let edit =
                        ScopeEdit::Member { item: item.id.clone(), thread: Some(thread.clone()) };
                    self.scope(&edit)?;
                }
            }
        }
        Ok(())
    }

    /// Makes `from` wait for `to`, as one link entity.
    fn link(&mut self, from: &str, to: &str) -> Result<(), ApplyError> {
        let key = model::link_key(from, to)?;
        if self.replica.state.field(&key, link::PRESENT).is_none() {
            self.report.links += 1;
            self.link_row(from, to, Action::Create);
        }
        self.field(&key, link::FROM, Some(&from))?;
        self.field(&key, link::TO, Some(&to))?;
        self.field(&key, link::KIND, Some(&link::BLOCKS))?;
        self.field(&key, link::PRESENT, Some(&true))?;
        Ok(())
    }

    /// Stops `from` waiting for `to`.
    fn unlink(&mut self, from: &str, to: &str) -> Result<(), ApplyError> {
        if model::unlink(self.replica, from, to, self.by)? {
            self.report.fields += 1;
            self.link_row(from, to, Action::Remove);
        }
        Ok(())
    }

    fn link_row(&mut self, from: &str, to: &str, action: Action) {
        self.report.changes.push(Change {
            entity: kind::LINK,
            name: Name::Link { from: from.to_owned(), kind: "waits-for", to: to.to_owned() },
            action,
            fields: BTreeMap::new(),
        });
    }

    /// Counts a record the import creates, and says whether it does.
    fn created(&mut self, key: &Key, count: impl Fn(&mut Report) -> &mut usize) -> bool {
        let new = self.replica.state.entity(key).is_none();
        if new {
            *count(&mut self.report) += 1;
            self.row(entity(&key.kind), &key.id).action = Action::Create;
        }
        new
    }

    /// The row for a record, added as an update when it is the first mention.
    fn row(&mut self, entity: &'static str, id: &str) -> &mut Change {
        let changes = &mut self.report.changes;
        let at = *self.rows.entry((entity, id.to_owned())).or_insert_with(|| {
            changes.push(Change {
                entity,
                name: Name::Id { id: id.to_owned() },
                action: Action::Update,
                fields: BTreeMap::new(),
            });
            changes.len() - 1
        });
        &mut self.report.changes[at]
    }

    /// Writes a field the document sets, unless it already holds exactly that value; `None` leaves it alone.
    fn field<T: Serialize>(
        &mut self,
        key: &Key,
        name: &str,
        value: Option<&T>,
    ) -> Result<(), ApplyError> {
        let Some(value) = value else { return Ok(()) };
        let json = serde_json::to_value(value).expect("import values serialize");
        let current: Vec<&Json> = self
            .replica
            .state
            .field(key, name)
            .into_iter()
            .flatten()
            .map(|(_, v)| &v.value)
            .collect();
        // Equal values written apart hold it too, so a done item is never marked done again.
        if current.is_empty() || current.iter().any(|held| **held != json) {
            let from = match current.as_slice() {
                [] => Json::Null,
                [one] => (*one).clone(),
                many => serde_json::json!({"conflict": many}),
            };
            model::put(self.replica, key, name, Some(&json), self.by)?;
            self.report.fields += 1;
            if key.kind != kind::LINK {
                let edit = Edit { from, to: json };
                self.row(entity(&key.kind), &key.id).fields.insert(name.replace('-', "_"), edit);
            }
        }
        Ok(())
    }

    /// Notes a repository added to or removed from a record's set on its row, with the whole set before and after.
    fn set_edit(&mut self, entity: &'static str, id: &str, element: &str, present: bool) {
        let current = self.before_set(entity, id);
        let row = self.row(entity, id);
        let edit = row
            .fields
            .entry("repositories".to_owned())
            .or_insert_with(|| Edit { from: json!(current), to: json!(current) });
        let mut to: BTreeSet<String> =
            serde_json::from_value(edit.to.clone()).expect("a set of names");
        if present {
            to.insert(element.to_owned());
        } else {
            to.remove(element);
        }
        edit.to = json!(to);
    }

    /// A repository set's members before the import.
    fn before_set(&self, entity: &str, id: &str) -> BTreeSet<String> {
        let before = match entity {
            kind::THREAD => self.before.threads.get(id).map(|t| t.repositories.clone()),
            _ => self.before.items.get(id).map(|i| i.repositories.clone()),
        };
        before.unwrap_or_default()
    }

    fn scope(&mut self, change: &ScopeEdit) -> Result<(), ApplyError> {
        scope::edit(self.replica, change, self.by)?;
        self.report.fields += 1;
        let (entity, id, element, present) = match change {
            ScopeEdit::Member { item, thread } => {
                let from = self.before.items.get(item).map_or(Json::Null, |i| json!(i.thread));
                let edit = Edit { from, to: json!(thread) };
                self.row(kind::ITEM, item).fields.insert("thread".to_owned(), edit);
                return Ok(());
            },
            ScopeEdit::ItemRepository { item, repository, present } => {
                (kind::ITEM, item, repository, *present)
            },
            ScopeEdit::ThreadRepository { thread, repository, present } => {
                (kind::THREAD, thread, repository, *present)
            },
        };
        self.set_edit(entity, id, element, present);
        Ok(())
    }
}

/// The edits that take a set from `held` to `wanted`: when `adding`, each element to add, and otherwise each to remove.
fn changes(held: &BTreeSet<String>, wanted: &[String], adding: bool) -> Vec<(String, bool)> {
    let wanted: BTreeSet<String> = wanted.iter().cloned().collect();
    if adding {
        wanted.difference(held).map(|r| (r.clone(), true)).collect()
    } else {
        held.difference(&wanted).map(|r| (r.clone(), false)).collect()
    }
}

/// The kind of record a key names, as a change reports it.
fn entity(key_kind: &str) -> &'static str {
    [kind::ITEM, kind::THREAD, kind::REPOSITORY, kind::LINK]
        .into_iter()
        .find(|name| *name == key_kind)
        .unwrap_or(kind::ITEM)
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
