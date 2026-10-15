//! Applying a validated import to a replica: an upsert in one pass. Every field the document sets is written when it
//! differs; nothing the document leaves out is cleared or deleted, so importing the same document twice writes
//! nothing. The document is checked once as a whole, and the result once for cycles it would close with what is
//! already there; the writes themselves skip the per-write checks, which would read the whole model each time.

use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;
use serde_json::Value as Json;

use super::{Import, LinkKind, ValidationError};
use crate::model::{self, By, Model, WriteError};
use crate::schema::{item as field, kind, link, repository, set, thread};
use crate::scope::{self, Outcome, ScopeEdit, ScopeError, USER};
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
    /// Proposals for edits to approved scope: made now, or the actor's still pending from before.
    pub proposals: Vec<String>,
}

/// Why an import was refused. The replica may then hold some of the writes: apply to a copy, or inside a store batch,
/// which discards them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The document breaks these rules.
    Invalid(Vec<ValidationError>),
    /// Only the user imports approved threads or archived items.
    UserOnly(String),
    /// With what is already there, the document closes these cycles of parents or ordering links.
    Cycles(Vec<Vec<String>>),
    /// A write was refused.
    Write(WriteError),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(errors) => write!(f, "the document breaks {} rule(s)", errors.len()),
            Self::UserOnly(what) => write!(f, "only the user imports {what}"),
            Self::Cycles(cycles) => {
                write!(f, "the import would close {} cycle(s) with the tracker", cycles.len())
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

impl From<ScopeError> for ApplyError {
    fn from(err: ScopeError) -> Self {
        match err {
            ScopeError::Write(err) => Self::Write(err),
            other => Self::Write(WriteError::State(Error::Format(other.to_string()))),
        }
    }
}

/// Applies `doc` to `replica` as `by`.
///
/// # Errors
///
/// Fails when the document is invalid, when anyone but the user imports approved threads or archived items, when
/// the result would hold a cycle the tracker didn't, or when a write is refused.
pub fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(ApplyError::Invalid(errors));
    }
    if by.actor != USER {
        if doc.threads.iter().any(|t| t.approved) {
            return Err(ApplyError::UserOnly("approved threads".to_owned()));
        }
        if doc.items.iter().any(|i| i.archived) {
            return Err(ApplyError::UserOnly("archived items".to_owned()));
        }
    }
    let before = Model::read(&replica.state)?;
    let cycles_before: BTreeSet<Vec<String>> =
        before.parent_cycles().into_iter().chain(before.link_cycles()).collect();
    let mut writer = Writer { replica, by, report: Report::default() };
    writer.records(doc)?;
    // Scope last, against the view before the import: membership and repositories go through apply-or-propose.
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
    report: Report,
}

impl Writer<'_, '_> {
    /// Writes the records' own fields, sets and links.
    fn records(&mut self, doc: &Import) -> Result<(), ApplyError> {
        for record in &doc.repositories {
            let key = Key::new(kind::REPOSITORY, &record.key)?;
            self.created(&key, |r| &mut r.repositories);
            self.field(&key, repository::OWNER, Some(&record.owner))?;
            self.field(&key, repository::NAME, Some(&record.name))?;
            self.field(&key, repository::PUBLIC, Some(&record.public))?;
            self.field(&key, repository::DEFAULT_BRANCH, Some(&record.default_branch))?;
        }
        for record in &doc.threads {
            let key = Key::new(kind::THREAD, &record.key)?;
            self.created(&key, |r| &mut r.threads);
            self.field(&key, thread::TITLE, Some(&record.title))?;
            self.field(&key, thread::BRIEF, record.brief.as_ref())?;
            if record.approved {
                self.field(&key, thread::APPROVED, Some(&true))?;
            }
        }
        for item in &doc.items {
            let key = model::item_key(&item.key)?;
            self.created(&key, |r| &mut r.items);
            self.field(&key, field::TITLE, Some(&item.title))?;
            self.field(&key, field::NOTES, item.notes.as_ref())?;
            self.field(&key, field::GATE, item.gate.as_ref())?;
            self.field(&key, field::STATE, Some(&item.state))?;
            self.field(&key, field::PARENT, item.parent.as_ref())?;
            self.field(&key, field::SIZE, item.size.as_ref())?;
            self.field(&key, field::EVIDENCE, item.evidence.as_ref())?;
            if item.archived {
                self.field(&key, field::ARCHIVED, Some(&true))?;
            }
            for (prefix, names) in [(set::AREA, &item.areas), (set::RESOURCE, &item.resources)] {
                for name in names {
                    self.element(&key, prefix, name)?;
                }
            }
        }
        for record in &doc.links {
            let kind = match record.kind {
                LinkKind::Blocks => model::LinkKind::Blocks,
                LinkKind::After => model::LinkKind::After,
                LinkKind::Relates => model::LinkKind::Relates,
            };
            let key = model::link_key(&record.from, &record.to, kind)?;
            if self.replica.state.field(&key, link::PRESENT).is_none() {
                self.report.links += 1;
            }
            self.field(&key, link::FROM, Some(&record.from))?;
            self.field(&key, link::TO, Some(&record.to))?;
            self.field(&key, link::KIND, Some(&kind))?;
            self.field(&key, link::PRESENT, Some(&true))?;
        }
        Ok(())
    }

    /// Adds repositories and members, applied or proposed as the actor's scope edits are.
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
        }
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
            model::put(self.replica, key, name, Some(&json), self.by)?;
            self.report.fields += 1;
        }
        Ok(())
    }

    fn element(&mut self, key: &Key, prefix: &str, name: &str) -> Result<(), ApplyError> {
        if self.replica.state.field(key, &format!("{prefix}{name}")).is_none() {
            model::put_element(self.replica, key, prefix, name, true, self.by)?;
            self.report.fields += 1;
        }
        Ok(())
    }

    fn scope(&mut self, before: &Model, change: &ScopeEdit) -> Result<(), ApplyError> {
        match scope::edit_in(self.replica, before, change, self.by)? {
            Outcome::Applied => self.report.fields += 1,
            Outcome::Proposed(id) => self.report.proposals.push(id),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
