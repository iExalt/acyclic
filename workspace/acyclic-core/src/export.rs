//! Export: the tracker's whole current graph as one import document, which `acyclic import` takes back into the same
//! tracker (changing nothing) or into a fresh one (making the same records).
//!
//! The document holds initiatives, repositories, threads, items with their parents, thread, waits-for, notes, state
//! and evidence. Claims, replica metadata, history and deleted records stay behind, and so does anything derived.
//! When the graph holds something a document can't say, nothing is exported: [`Refusal`] names each record in the way.
//! Whatever the document says is checked before it leaves: it must import cleanly into an empty tracker and change
//! nothing in the one it came from.

use std::collections::BTreeSet;
use std::fmt;

use crate::ids;
use crate::import::{self, Evidence, Import, Initiative, Item, Repository, Thread, owner_name};
use crate::model::{self, By, ItemState, Model, Value};
use crate::write::{self, Record, Rule, Violation, Written};

/// One record a document can't carry, and why.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Blocker {
    /// The record's kind: `initiative`, `repository`, `thread`, `item` or `link`.
    pub kind: String,
    /// Its id.
    pub id: String,
    /// What is wrong with it.
    pub reason: String,
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.kind, self.id, self.reason)
    }
}

/// Why the graph can't be exported: every record in the way, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub Vec<Blocker>);

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "an import document can't carry {}",
            crate::say::few(&self.0, "record", "records", "; ")
        )
    }
}

impl std::error::Error for Refusal {}

/// Who the self-check writes as. Nothing is written.
const BY: By<'static> = By { actor: "export", at: 0 };

/// The model as an import document.
///
/// # Errors
///
/// Fails with every record the document can't carry; see [`Refusal`].
pub fn export(model: &Model) -> Result<Import, Refusal> {
    let mut found = Blockers::default();
    for (kind, id, field) in model.conflicts() {
        found.add(kind, id, format!("its {field} holds different values written apart"));
    }
    let mut doc = Import::new();
    repositories(model, &mut doc, &mut found);
    initiatives(model, &mut doc, &mut found);
    threads(model, &mut doc, &mut found);
    items(model, &mut doc, &mut found);
    if !found.0.is_empty() {
        return Err(found.refusal());
    }
    self_check(model, &doc)?;
    Ok(doc)
}

#[derive(Default)]
struct Blockers(BTreeSet<Blocker>);

impl Blockers {
    fn add(&mut self, kind: &str, id: &str, reason: String) {
        self.0.insert(Blocker { kind: kind.to_owned(), id: id.to_owned(), reason });
    }

    /// The field's value, noting a blocker when it is unset. A conflict is noted by the sweep for them.
    fn need<'v, T>(
        &mut self,
        value: &'v Value<T>,
        kind: &str,
        id: &str,
        field: &str,
    ) -> Option<&'v T> {
        if matches!(value, Value::None) {
            self.add(kind, id, format!("it has no {field}"));
        }
        value.one()
    }

    fn id(&mut self, kind: &str, id: &str) {
        if ids::v7(id).as_deref() != Some(id) {
            self.add(kind, id, "its id isn't a lowercase UUIDv7".to_owned());
        }
    }

    fn refusal(self) -> Refusal {
        Refusal(self.0.into_iter().collect())
    }
}

fn repositories(model: &Model, doc: &mut Import, found: &mut Blockers) {
    for (id, record) in &model.repositories {
        found.id("repository", id);
        let owner = found.need(&record.owner, "repository", id, "owner");
        let name = found.need(&record.name, "repository", id, "name");
        let branch = found.need(&record.default_branch, "repository", id, "default branch");
        if let (Some(owner), Some(name), Some(branch)) = (owner, name, branch) {
            let joined = format!("{owner}/{name}");
            if owner_name(&joined).is_none() {
                found.add("repository", id, format!("{joined:?} isn't owner/name"));
            }
            doc.repositories.push(Repository {
                id: id.clone(),
                name: joined,
                default_branch: branch.clone(),
            });
        }
    }
}

fn initiatives(model: &Model, doc: &mut Import, found: &mut Blockers) {
    for (id, record) in &model.initiatives {
        found.id("initiative", id);
        if let Some(title) = found.need(&record.title, "initiative", id, "title") {
            doc.initiatives.push(Initiative {
                id: id.clone(),
                title: title.clone(),
                notes: record.notes.one().cloned(),
            });
        }
    }
}

/// Threads with a record. A thread of one (`<task>.one`) is no thread of the document's: an item in its own is in
/// none, and one no item names is dropped, but one that has a title or members from another item can't be said.
fn threads(model: &Model, doc: &mut Import, found: &mut Blockers) {
    for (id, record) in &model.threads {
        if ids::task_of_one(id).is_some() {
            let members = model.members(id);
            if !matches!(record.title, Value::None) && !members.is_empty() {
                found.add(
                    "thread",
                    id,
                    "it reads as a named thread, which an import document can't name: it holds items and its id isn't a UUIDv7"
                        .to_owned(),
                );
            }
            continue;
        }
        found.id("thread", id);
        if let Some(title) = found.need(&record.title, "thread", id, "title") {
            doc.threads.push(Thread {
                id: id.clone(),
                title: title.clone(),
                notes: record.notes.one().cloned(),
            });
        }
    }
}

fn items(model: &Model, doc: &mut Import, found: &mut Blockers) {
    for (id, item) in &model.items {
        found.id("item", id);
        let title = found.need(&item.title, "item", id, "title");
        let state = found.need(&item.state, "item", id, "state").copied();
        let parent = parent(model, item, found);
        let parental = !model.children(id).is_empty();
        if parental && state == Some(ItemState::Done) {
            found.add(
                "item",
                id,
                "it has children and is stored done: it reads done once they are".to_owned(),
            );
        }
        let thread = thread(model, item, parental, found);
        let evidence = evidence(model, item, state, found);
        let mut waits_for: Vec<String> =
            model.links.iter().filter(|l| l.from == *id).map(|l| l.to.clone()).collect();
        for to in &waits_for {
            if !model.items.contains_key(to) {
                found.add("item", id, format!("it waits for {to}, which no item is"));
            }
        }
        waits_for.sort();
        waits_for.dedup();
        if let Some(title) = title {
            doc.items.push(Item {
                id: id.clone(),
                title: title.clone(),
                notes: item.notes.one().cloned(),
                state,
                parent,
                thread: Some(thread),
                waits_for: Some(waits_for),
                evidence,
            });
        }
    }
}

fn parent(model: &Model, item: &model::Item, found: &mut Blockers) -> Option<String> {
    let id = &item.id;
    let parent = found.need(&item.parent, "item", id, "parent")?;
    if !model.items.contains_key(parent) && !model.initiatives.contains_key(parent) {
        found.add("item", id, format!("its parent {parent} is no item or initiative"));
    }
    Some(parent.clone())
}

/// The item's thread as the document says it. A conflicted thread (replicas set it apart) never gets here: the sweep
/// of `model.conflicts()` in [`export`] refuses it, and [`Value::one`] reads it as none.
///
/// The item's thread as the document says it: an id, or `None` for no thread, which is also what its own thread of
/// one is.
fn thread(
    model: &Model,
    item: &model::Item,
    parental: bool,
    found: &mut Blockers,
) -> Option<String> {
    let id = &item.id;
    let stored = item.thread.one()?;
    if parental {
        found.add("item", id, format!("it has children, so its stored thread {stored} is ignored"));
        return None;
    }
    if *stored == ids::one(id) {
        return None;
    }
    if ids::task_of_one(stored).is_some() {
        found.add("item", id, format!("it is in {stored}, another item's thread of one"));
    } else if !model.threads.contains_key(stored) {
        found.add("item", id, format!("its thread {stored} has no record"));
    }
    Some(stored.clone())
}

fn evidence(
    model: &Model,
    item: &model::Item,
    state: Option<ItemState>,
    found: &mut Blockers,
) -> Option<Evidence> {
    let id = &item.id;
    let stored = item.evidence.one()?;
    if state.is_some_and(|state| state != ItemState::Done) {
        found.add("item", id, "it has evidence but isn't done".to_owned());
    }
    let commit = match (&stored.repository, &stored.commit) {
        (Some(repository), Some(sha)) => {
            if !model.repositories.contains_key(repository) {
                found.add(
                    "item",
                    id,
                    format!("its evidence names repository {repository}, which no record is"),
                );
            }
            Some(format!("{repository}@{sha}"))
        },
        (None, None) => None,
        _ => {
            found.add(
                "item",
                id,
                "its evidence names a commit or a repository but not both".to_owned(),
            );
            None
        },
    };
    Some(Evidence { commit, text: stored.text.clone() })
}

/// The record a violation is about: the one it names, or the first it involves.
fn blocker(violation: &Violation) -> Blocker {
    let record = |kind: &str, id: &str| (kind.to_owned(), id.to_owned());
    let (kind, id) = match (&violation.at, &violation.rule) {
        (Some(at), _) => record(at.record.kind(), at.record.id()),
        (None, Rule::LineageLink { from, .. }) => record("item", from),
        (None, Rule::ParentCycle(path) | Rule::WaitsForCycle(path)) => {
            record("item", path.first().map_or("", String::as_str))
        },
        (
            None,
            Rule::NoRecord(r)
            | Rule::Deleted(r)
            | Rule::EmptyRecord(r)
            | Rule::IdTaken(r)
            | Rule::WrongRecord(r),
        ) => record(r.kind(), r.id()),
        (None, _) => record("document", ""),
    };
    Blocker { kind, id, reason: violation.to_string() }
}

/// Judges the document as an import would: against an empty tracker, where it must be accepted, and against the
/// tracker it came from, where it must change nothing.
fn self_check(model: &Model, doc: &Import) -> Result<(), Refusal> {
    let refuse = |violations: &[Violation]| Refusal(violations.iter().map(blocker).collect());
    let errors = doc.validate();
    if !errors.is_empty() {
        return Err(Refusal(
            errors
                .iter()
                .map(|e| Blocker {
                    kind: "document".to_owned(),
                    id: String::new(),
                    reason: e.to_string(),
                })
                .collect(),
        ));
    }
    write::judge(&Model::default(), &import::changes(doc, &Model::default()), BY, None)
        .map_err(|violations| refuse(&violations))?;
    match write::judge(model, &import::changes(doc, model), BY, None) {
        Ok(outcome) if !outcome.changes() => Ok(()),
        Ok(outcome) => {
            let records: BTreeSet<Record> = outcome
                .steps
                .iter()
                .map(|step| match &step.write {
                    Written::Create(record) => record.clone(),
                    Written::Assign(assign) => assign.at().record,
                    Written::Place(placed) => Record::Item(placed.item.clone()),
                })
                .collect();
            // A change that writes something names a record, so the list is never empty.
            Err(Refusal(
                records
                    .iter()
                    .map(|record| Blocker {
                        kind: record.kind().to_owned(),
                        id: record.id().to_owned(),
                        reason: "importing the document would change it".to_owned(),
                    })
                    .collect(),
            ))
        },
        Err(violations) => Err(refuse(&violations)),
    }
}

#[cfg(test)]
mod tests;
