//! Applying a validated import to a replica: an upsert in one pass. Every field the document sets is written when it
//! differs; nothing the document leaves out is cleared or deleted, so importing the same document twice writes
//! nothing. The document is checked once as a whole, and the result once for cycles it would close with what is
//! already there; the writes themselves skip the per-write checks, which would read the whole model each time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{Import, ValidationError};
use crate::model::{self, By, Model, WriteError};
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
    /// Each record the import creates or changes, in the order they were met.
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
    /// The document breaks these rules.
    Invalid(Vec<ValidationError>),
    /// With what is already there, the document closes these cycles of parents or ordering links.
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
/// Fails when the document is invalid, when the result would hold a cycle the tracker didn't, or when a write is
/// refused.
pub fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    for item in &doc.items {
        if model::is_deleted(&replica.state, &model::item_key(&item.key)?) {
            return Err(WriteError::Deleted(item.key.clone()).into());
        }
    }
    let before = Model::read(&replica.state)?;
    let cycles_before: BTreeSet<Vec<String>> =
        before.parent_cycles().into_iter().chain(before.link_cycles()).collect();
    let mut writer =
        Writer { replica, by, before: &before, report: Report::default(), rows: BTreeMap::new() };
    writer.records(doc)?;
    // Scope last, against the view before the import, so each edit reports what it changed.
    writer.scope_edits(doc, &before)?;

    let after = Model::read(&writer.replica.state)?;
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
    /// Writes the records' own fields, sets and links.
    fn records(&mut self, doc: &Import) -> Result<(), ApplyError> {
        for record in &doc.repositories {
            let key = Key::new(kind::REPOSITORY, &record.key)?;
            self.created(&key, |r| &mut r.repositories);
            self.field(&key, repository::OWNER, Some(&record.owner))?;
            self.field(&key, repository::NAME, Some(&record.name))?;
            self.field(&key, repository::DEFAULT_BRANCH, Some(&record.default_branch))?;
        }
        for record in &doc.threads {
            let key = Key::new(kind::THREAD, &record.key)?;
            self.created(&key, |r| &mut r.threads);
            self.field(&key, thread::TITLE, Some(&record.title))?;
            self.field(&key, thread::BRIEF, record.brief.as_ref())?;
        }
        for item in &doc.items {
            let key = model::item_key(&item.key)?;
            self.created(&key, |r| &mut r.items);
            self.field(&key, field::TITLE, Some(&item.title))?;
            self.field(&key, field::NOTES, item.notes.as_ref())?;
            self.field(&key, field::STATE, Some(&item.state))?;
            self.field(&key, field::PARENT, item.parent.as_ref())?;
            self.field(&key, field::SIZE, item.size.as_ref())?;
            self.field(&key, field::EVIDENCE, item.evidence.as_ref())?;
            if item.archived {
                self.field(&key, field::ARCHIVED, Some(&true))?;
            }
        }
        for item in &doc.items {
            for to in &item.waits_for {
                self.link(&item.key, to)?;
            }
        }
        Ok(())
    }

    /// Makes `from` wait for `to`, as one link entity.
    fn link(&mut self, from: &str, to: &str) -> Result<(), ApplyError> {
        let key = model::link_key(from, to)?;
        if self.replica.state.field(&key, link::PRESENT).is_none() {
            self.report.links += 1;
            self.report.changes.push(Change {
                entity: kind::LINK,
                name: Name::Link { from: from.to_owned(), kind: "waits-for", to: to.to_owned() },
                action: Action::Create,
                fields: BTreeMap::new(),
            });
        }
        self.field(&key, link::FROM, Some(&from))?;
        self.field(&key, link::TO, Some(&to))?;
        self.field(&key, link::KIND, Some(&link::BLOCKS))?;
        self.field(&key, link::PRESENT, Some(&true))?;
        Ok(())
    }

    /// Adds repositories and members.
    fn scope_edits(&mut self, doc: &Import, before: &Model) -> Result<(), ApplyError> {
        for item in &doc.items {
            for name in &item.repositories {
                let present =
                    before.items.get(&item.key).is_some_and(|i| i.repositories.contains(name));
                if !present {
                    let change = ScopeEdit::ItemRepository {
                        item: item.key.clone(),
                        repository: name.clone(),
                        present: true,
                    };
                    self.scope(before, &change)?;
                }
            }
        }
        for record in &doc.threads {
            for name in &record.repositories {
                let present =
                    before.threads.get(&record.key).is_some_and(|t| t.repositories.contains(name));
                if !present {
                    let change = ScopeEdit::ThreadRepository {
                        thread: record.key.clone(),
                        repository: name.clone(),
                        present: true,
                    };
                    self.scope(before, &change)?;
                }
            }
            for member in &record.items {
                if before.thread_of(member) != Some(record.key.as_str()) {
                    let change = ScopeEdit::Member {
                        item: member.clone(),
                        thread: Some(record.key.clone()),
                    };
                    self.scope(before, &change)?;
                }
            }
        }
        Ok(())
    }
    fn created(&mut self, key: &Key, count: impl Fn(&mut Report) -> &mut usize) {
        if self.replica.state.entity(key).is_none() {
            *count(&mut self.report) += 1;
            self.row(entity(&key.kind), &key.id).action = Action::Create;
        }
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
        if current != [&json] {
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

    fn scope(&mut self, before: &Model, change: &ScopeEdit) -> Result<(), ApplyError> {
        scope::edit(self.replica, change, self.by)?;
        self.report.fields += 1;
        let (entity, id, element, present) = match change {
            ScopeEdit::Member { item, thread } => {
                let from = before.items.get(item).map_or(Json::Null, |i| json!(i.thread));
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

/// The kind of record a key names, as a change reports it.
fn entity(key_kind: &str) -> &'static str {
    [kind::ITEM, kind::THREAD, kind::REPOSITORY, kind::LINK]
        .into_iter()
        .find(|name| *name == key_kind)
        .unwrap_or(kind::ITEM)
}

#[cfg(test)]
mod tests;
