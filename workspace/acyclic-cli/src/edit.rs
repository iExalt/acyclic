//! The writes behind `add` and `edit`, which take the same fields: `edit` changes an existing entity and refuses an
//! unknown one, `add` creates one, and adding it again with what it already holds changes nothing while adding it with
//! other values is refused. Each runs inside a batch, so a refusal anywhere writes nothing at all.
//!
//! Membership and repositories of items and threads go through the scope rules, as every other write to them does: an
//! agent's change to approved scope waits as a proposal, which the result lists.

use acyclic_core::ids;
use acyclic_core::model::{self, By, ItemState, Level, Model, SetField, Size, Value};
use acyclic_core::place::{self, Inherited};
use acyclic_core::schema::{item as field, kind, repository as repo_field, thread as thread_field};
use acyclic_core::scope::{self, Outcome, Proposed, ScopeEdit};
use acyclic_core::state::{Key, Replica, State};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::commands::{element_if, existing, holds_only, placed, set_if};
use crate::error::{CliError, exit};

/// A single-valued field's change: `--field` sets it, `--no-field` clears it, and neither leaves it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Change<T> {
    /// Leave it as it is.
    #[default]
    Keep,
    /// Set it.
    Set(T),
    /// Clear it.
    Clear,
}

impl<T> Change<T> {
    /// The change a field's two flags ask for; the command line refuses both together.
    pub fn of(set: Option<T>, clear: bool) -> Self {
        match (set, clear) {
            (Some(value), _) => Self::Set(value),
            (None, true) => Self::Clear,
            (None, false) => Self::Keep,
        }
    }

    fn value(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            _ => None,
        }
    }
}

/// A set-valued field's change: what `--field` adds and `--no-field` removes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Delta {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

/// What an item's `add` or `edit` sets.
#[derive(Debug, Default)]
pub(crate) struct ItemFields {
    pub title: Option<String>,
    pub parent: Change<String>,
    pub thread: Change<String>,
    pub state: Option<ItemState>,
    pub notes: Change<String>,
    pub gate: Change<String>,
    pub size: Change<Size>,
    pub areas: Delta,
    pub resources: Delta,
    pub repositories: Delta,
}

/// What a thread's `add` or `edit` sets.
#[derive(Debug, Default)]
pub(crate) struct ThreadFields {
    pub title: Option<String>,
    pub brief: Change<String>,
    pub level: Change<Level>,
    pub repositories: Delta,
}

/// What a repository record's `add` or `edit` sets.
#[derive(Debug, Default)]
pub(crate) struct RepositoryFields {
    pub owner: Option<String>,
    pub name: Option<String>,
    pub default_branch: Option<String>,
    pub public: Option<bool>,
}

/// What a write did beyond writing: the proposals it left for the user, and the threads items inherited.
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub proposals: Vec<Proposed>,
    pub inherited: Vec<Inherited>,
}

impl Written {
    /// An item's data: the item, the proposals its scope changes left, and the threads items joined.
    pub fn item(self, id: &str) -> Json {
        json!({"item": id, "proposals": all_proposals(&self.proposals, &self.inherited),
               "placed": placed(&self.inherited)})
    }

    /// A thread's data: the thread and the proposals its scope changes left.
    pub fn thread(self, id: &str) -> Json {
        json!({"thread": id, "proposals": self.proposals})
    }
}

/// Every proposal a write left, each with the field it would change: those of its own edits and those that hold the
/// placements it made, each once.
pub(crate) fn all_proposals(direct: &[Proposed], placed: &[Inherited]) -> Vec<Proposed> {
    let held = placed
        .iter()
        .filter_map(|p| p.proposal.clone().map(|id| Proposed { id, field: "thread".to_owned() }));
    let mut all: Vec<Proposed> = direct.iter().cloned().chain(held).collect();
    all.sort();
    all.dedup();
    all
}

/// An add that names other values than the entity holds, which `edit` is for.
fn already_exists(what: &str, edit: &str, id: &str, differs: &[&str]) -> CliError {
    let fix: Vec<String> = differs
        .iter()
        .map(|flag| {
            // A pair of presence flags takes no value.
            if flag.contains('/') { flag.replace('/', "|") } else { format!("{flag} …") }
        })
        .collect();
    // The fields as the output names them: `--default-branch` is `default_branch`, a pair of flags is the first.
    let fields: Vec<String> = differs
        .iter()
        .map(|flag| {
            let first = flag.split('/').next().unwrap_or(flag);
            first.trim_start_matches('-').replace('-', "_")
        })
        .collect();
    CliError::refused(
        "already-exists",
        format!(
            "{what} {id} exists with other {}; change it with `acyclic {edit} {id} {}`",
            differs.join(", "),
            fix.join(" ")
        ),
    )
    .with(json!({"fields": fields}))
}

fn not_found(message: String) -> CliError {
    CliError::new("not-found", exit::NOT_FOUND, message)
}

fn write_change<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    change: &Change<T>,
    by: By<'_>,
) -> Result<(), CliError> {
    match change {
        Change::Keep => {},
        Change::Set(value) => set_if(replica, key, name, Some(value), by)?,
        Change::Clear => {
            if !holds_only(replica.state().field(key, name), None) {
                model::set::<T>(replica, key, name, None, by)?;
            }
        },
    }
    Ok(())
}

/// Whether a thread exists: it has fields of its own, or an item names it.
fn thread_exists(state: &State, model: &Model, id: &str) -> bool {
    // A deleted thread stays gone, though items another replica moved into it still name it.
    let deleted = Key::new(kind::THREAD, id).is_ok_and(|key| model::is_deleted(state, &key));
    !deleted
        && (model.threads.contains_key(id)
            || model.items.values().any(|item| item.thread.all().contains(&&id.to_owned())))
}

/// Refuses a thread or repository that isn't there, before anything is written.
fn check_targets(
    state: &State,
    thread: Option<&String>,
    repositories: &[String],
) -> Result<(), CliError> {
    if thread.is_none() && repositories.is_empty() {
        return Ok(());
    }
    let model = Model::read(state)?;
    if let Some(thread) = thread.filter(|thread| !thread_exists(state, &model, thread)) {
        return Err(not_found(format!(
            "no thread {thread}; add it with `acyclic thread add {thread} --title <TITLE>`"
        )));
    }
    match repositories.iter().find(|id| !model.repositories.contains_key(*id)) {
        Some(id) => Err(not_found(format!(
            "no repository record {id}; add it with `acyclic repo add {id}`"
        ))),
        None => Ok(()),
    }
}

/// The scope edits a set of item fields asks for.
fn item_scope(id: &str, fields: &ItemFields) -> Vec<ScopeEdit> {
    let member = |thread: Option<String>| ScopeEdit::Member { item: id.to_owned(), thread };
    let repository = |repository: &String, present| ScopeEdit::ItemRepository {
        item: id.to_owned(),
        repository: repository.clone(),
        present,
    };
    let mut edits = match &fields.thread {
        Change::Keep => Vec::new(),
        Change::Set(thread) => vec![member(Some(thread.clone()))],
        Change::Clear => vec![member(None)],
    };
    edits.extend(fields.repositories.add.iter().map(|r| repository(r, true)));
    edits.extend(fields.repositories.remove.iter().map(|r| repository(r, false)));
    edits
}

/// The scope edits a set of thread fields asks for.
fn thread_scope(id: &str, fields: &ThreadFields) -> Vec<ScopeEdit> {
    let repository = |repository: &String, present| ScopeEdit::ThreadRepository {
        thread: id.to_owned(),
        repository: repository.clone(),
        present,
    };
    let mut edits: Vec<ScopeEdit> =
        fields.repositories.add.iter().map(|r| repository(r, true)).collect();
    edits.extend(fields.repositories.remove.iter().map(|r| repository(r, false)));
    edits
}

/// Whether a scope edit would change nothing. The user putting an unapproved member in its thread again keeps it, so
/// that writes; an agent's repeat changes nothing.
fn scope_already(
    state: &State,
    model: &Model,
    actor: &str,
    change: &ScopeEdit,
) -> Result<bool, CliError> {
    Ok(match change {
        ScopeEdit::Member { item, thread } => {
            // Read the register itself: a conflict reads as no single thread, which a clear must still settle.
            let key = model::item_key(item)?;
            if !model.items.contains_key(item)
                || !holds_only(state.field(&key, field::THREAD), thread.as_ref().map(|t| json!(t)))
            {
                return Ok(false);
            }
            thread.is_none()
                || scope::classify(actor) != scope::Actor::User
                || !scope::unapproved(state, model)?.iter().any(|u| u.item == *item)
        },
        ScopeEdit::ItemRepository { item, repository, present } => {
            model.items.get(item).is_some_and(|i| i.repositories.contains(repository)) == *present
        },
        ScopeEdit::ThreadRepository { thread, repository, present } => {
            model.threads.get(thread).is_some_and(|t| t.repositories.contains(repository))
                == *present
        },
    })
}

/// The actor's own undecided proposal of the same edit, if any.
fn pending(state: &State, change: &ScopeEdit, actor: &str) -> Result<Option<String>, CliError> {
    Ok(scope::proposals(state)?
        .into_values()
        .find(|p| {
            p.decision == Value::None
                && p.edit.one() == Some(change)
                && p.actor.one().map(String::as_str) == Some(actor)
        })
        .map(|p| p.id))
}

/// Applies one scope edit unless it is so already, noting the proposal when it only proposes.
fn scope_step(
    replica: &mut Replica,
    change: &ScopeEdit,
    by: By<'_>,
    proposals: &mut Vec<Proposed>,
) -> Result<(), CliError> {
    let model = Model::read(replica.state())?;
    if scope_already(replica.state(), &model, by.actor, change)? {
        return Ok(());
    }
    if let Outcome::Proposed(id) = scope::edit(replica, change, by)? {
        proposals.push(Proposed { id, field: change.field().to_owned() });
    }
    Ok(())
}

/// Edits an item: plain fields and sets first, then its parent and state, then its scope, and last the thread it
/// inherits when it moved under a parent without being given one.
pub(crate) fn edit_item(
    replica: &mut Replica,
    id: &str,
    fields: &ItemFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    let key = existing(replica, id)?;
    check_targets(replica.state(), fields.thread.value(), &fields.repositories.add)?;
    set_if(replica, &key, field::TITLE, fields.title.as_ref(), by)?;
    write_change(replica, &key, field::NOTES, &fields.notes, by)?;
    write_change(replica, &key, field::GATE, &fields.gate, by)?;
    write_change(replica, &key, field::SIZE, &fields.size, by)?;
    for (set, delta) in [(SetField::Area, &fields.areas), (SetField::Resource, &fields.resources)] {
        for (names, present) in [(&delta.add, true), (&delta.remove, false)] {
            for name in names {
                element_if(replica, &key, set, name, present, by)?;
            }
        }
    }
    let parent = match &fields.parent {
        Change::Keep => None,
        Change::Set(parent) => Some(Some(parent.as_str())),
        Change::Clear => Some(None),
    };
    let mut moved = false;
    if let Some(parent) = parent
        && !holds_only(replica.state().field(&key, field::PARENT), parent.map(|p| json!(p)))
    {
        model::set_parent(replica, id, parent, by)?;
        moved = true;
    }
    if let Some(state) = fields.state {
        model::set_state(replica, id, state, by)?;
    }
    let mut written = Written::default();
    for change in item_scope(id, fields) {
        scope_step(replica, &change, by, &mut written.proposals)?;
    }
    // An explicit thread, or none, is the answer; otherwise a moved item joins the one its parent passes down.
    if moved && fields.thread == Change::Keep {
        written.inherited = place::place(replica, &[id.to_owned()], by)?;
    }
    Ok(written)
}

/// The id an add writes: the one given, unless it is new and the tracker's to assign, or else the next the tracker
/// assigns. An add without an id whose title is already a thread's, or an open item's, is refused naming that record,
/// so a retried add doesn't make a second one; give an id to add another with the same title. A done item's title is
/// free again, since titles recur once work is finished.
pub(crate) fn new_id(
    replica: &Replica,
    of: &str,
    given: Option<&str>,
    title: &str,
) -> Result<String, CliError> {
    if let Some(id) = given {
        let exists = if of == kind::THREAD {
            replica.state().entity(&thread_key(id)?).is_some()
                || thread_exists(replica.state(), &Model::read(replica.state())?, id)
        } else {
            replica.state().entity(&model::item_key(id)?).is_some()
        };
        if !exists && ids::reserved(of, id) {
            return Err(CliError::new(
                "reserved-id",
                exit::USAGE,
                format!(
                    "ids starting with {of}- are the tracker's to assign: leave out the id, or choose another"
                ),
            ));
        }
        return Ok(id.to_owned());
    }
    let model = Model::read(replica.state())?;
    let same = if of == kind::THREAD {
        model
            .threads
            .values()
            .find(|thread| thread.title.one().is_some_and(|t| t == title))
            .map(|t| &t.id)
    } else {
        model
            .items
            .values()
            .find(|item| {
                item.title.one().is_some_and(|t| t == title)
                    && !item.is_done()
                    && !model.is_archived(&item.id)
            })
            .map(|item| &item.id)
    };
    if let Some(id) = same {
        return Err(CliError::refused(
            "already-exists",
            format!("{of} {id} has this title; give an id to add another with the same title"),
        )
        .with(json!({"id": id})));
    }
    Ok(ids::next(replica.state(), of, replica.id()))
}

/// Adds an item, or accepts the same add again.
pub(crate) fn add_item(
    replica: &mut Replica,
    id: &str,
    fields: &ItemFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    let title = fields.title.as_deref().expect("an add has a title");
    let key = model::item_key(id)?;
    if model::is_deleted(replica.state(), &key) {
        return Err(model::WriteError::Deleted(id.to_owned()).into());
    }
    if replica.state().entity(&key).is_some() {
        return repeat_item(replica.state(), id, fields, by);
    }
    // Everything that can be refused is, before the item exists.
    check_targets(replica.state(), fields.thread.value(), &fields.repositories.add)?;
    model::add_item(replica, id, title, by)?;
    edit_item(replica, id, fields, by)
}

/// An add of an item that exists: it changes nothing when it names what the item holds, and is refused otherwise.
fn repeat_item(
    state: &State,
    id: &str,
    fields: &ItemFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    let model = Model::read(state)?;
    let item = model.items.get(id).ok_or_else(|| not_found(format!("no item {id}")))?;
    let mut differs: Vec<&str> = Vec::new();
    let mut check = |flag: &'static str, same: bool| {
        if !same {
            differs.push(flag);
        }
    };
    check("--title", fields.title.as_ref() == item.title.one());
    check("--parent", fields.parent.value().is_none_or(|p| item.parent.one() == Some(p)));
    check("--notes", fields.notes.value().is_none_or(|n| item.notes.one() == Some(n)));
    check("--gate", fields.gate.value().is_none_or(|g| item.gate.one() == Some(g)));
    check("--size", fields.size.value().is_none_or(|s| item.size.one() == Some(s)));
    check(
        "--state",
        fields.state.is_none_or(|s| item.state.one().copied().unwrap_or(ItemState::Backlog) == s),
    );
    check("--area", fields.areas.add.iter().all(|a| item.areas.contains(a)));
    check("--resource", fields.resources.add.iter().all(|r| item.resources.contains(r)));
    let changes = item_scope(id, fields);
    for (flag, wanted) in [("--thread", true), ("--repository", false)] {
        let satisfied = changes
            .iter()
            .filter(|change| matches!(change, ScopeEdit::Member { .. }) == wanted)
            .map(|change| {
                Ok(scope_already(state, &model, by.actor, change)?
                    || pending(state, change, by.actor)?.is_some())
            })
            .collect::<Result<Vec<bool>, CliError>>()?;
        check(flag, satisfied.into_iter().all(|same| same));
    }
    if !differs.is_empty() {
        return Err(already_exists("item", "item edit", id, &differs));
    }
    // The same proposal again is reported again, and nothing is written.
    let mut written = Written::default();
    for change in &changes {
        if let Some(id) = pending(state, change, by.actor)? {
            written.proposals.push(Proposed { id, field: change.field().to_owned() });
        }
    }
    Ok(written)
}

fn thread_key(id: &str) -> Result<Key, CliError> {
    Ok(Key::new(kind::THREAD, id)?)
}

fn write_thread(
    replica: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    check_targets(replica.state(), None, &fields.repositories.add)?;
    let key = thread_key(id)?;
    set_if(replica, &key, thread_field::TITLE, fields.title.as_ref(), by)?;
    write_change(replica, &key, thread_field::BRIEF, &fields.brief, by)?;
    write_change(replica, &key, thread_field::AUTONOMY, &fields.level, by)?;
    let mut written = Written::default();
    for change in thread_scope(id, fields) {
        scope_step(replica, &change, by, &mut written.proposals)?;
    }
    Ok(written)
}

/// Edits a thread, which exists when it has fields of its own or an item names it.
pub(crate) fn edit_thread(
    replica: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    if !thread_exists(replica.state(), &Model::read(replica.state())?, id) {
        return Err(not_found(format!("no thread {id}; `acyclic thread list` lists them")));
    }
    write_thread(replica, id, fields, by)
}

/// Adds a thread, or accepts the same add again.
pub(crate) fn add_thread(
    replica: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<Written, CliError> {
    let key = thread_key(id)?;
    if model::is_deleted(replica.state(), &key) {
        return Err(model::WriteError::Deleted(id.to_owned()).into());
    }
    if replica.state().entity(&key).is_none() {
        return write_thread(replica, id, fields, by);
    }
    let state = replica.state();
    let model = Model::read(state)?;
    let thread = model
        .threads
        .get(id)
        .ok_or_else(|| not_found(format!("no thread {id}; `acyclic thread list` lists them")))?;
    let mut differs: Vec<&str> = Vec::new();
    if fields.title.as_ref() != thread.title.one() {
        differs.push("--title");
    }
    if fields.brief.value().is_some_and(|b| thread.brief.one() != Some(b)) {
        differs.push("--brief");
    }
    if fields.level.value().is_some_and(|l| thread.autonomy.one() != Some(l)) {
        differs.push("--level");
    }
    let changes = thread_scope(id, fields);
    for change in &changes {
        if !scope_already(state, &model, by.actor, change)?
            && pending(state, change, by.actor)?.is_none()
            && !differs.contains(&"--repository")
        {
            differs.push("--repository");
        }
    }
    if !differs.is_empty() {
        return Err(already_exists("thread", "thread edit", id, &differs));
    }
    let mut written = Written::default();
    for change in &changes {
        if let Some(id) = pending(state, change, by.actor)? {
            written.proposals.push(Proposed { id, field: change.field().to_owned() });
        }
    }
    Ok(written)
}

/// Edits a repository record, which must exist; any subset of its fields.
pub(crate) fn edit_repository(
    replica: &mut Replica,
    id: &str,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<(), CliError> {
    let key = Key::new(kind::REPOSITORY, id)?;
    if replica.state().entity(&key).is_none() {
        return Err(not_found(format!(
            "no repository record {id}; `acyclic repo list` lists them"
        )));
    }
    write_repository(replica, &key, fields, by)
}

fn write_repository(
    replica: &mut Replica,
    key: &Key,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<(), CliError> {
    set_if(replica, key, repo_field::OWNER, fields.owner.as_ref(), by)?;
    set_if(replica, key, repo_field::NAME, fields.name.as_ref(), by)?;
    set_if(replica, key, repo_field::PUBLIC, fields.public.as_ref(), by)?;
    set_if(replica, key, repo_field::DEFAULT_BRANCH, fields.default_branch.as_ref(), by)
}

/// Adds a repository record, or accepts the same add again.
pub(crate) fn add_repository(
    replica: &mut Replica,
    id: &str,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<(), CliError> {
    let key = Key::new(kind::REPOSITORY, id)?;
    if replica.state().entity(&key).is_none() {
        return write_repository(replica, &key, fields, by);
    }
    let model = Model::read(replica.state())?;
    let record = model.repositories.get(id).ok_or_else(|| {
        not_found(format!("no repository record {id}; `acyclic repo list` lists them"))
    })?;
    let mut differs: Vec<&str> = Vec::new();
    for (flag, same) in [
        ("--owner", fields.owner.as_ref() == record.owner.one()),
        ("--name", fields.name.as_ref() == record.name.one()),
        ("--default-branch", fields.default_branch.as_ref() == record.default_branch.one()),
        ("--public/--private", fields.public.as_ref() == record.public.one()),
    ] {
        if !same {
            differs.push(flag);
        }
    }
    if differs.is_empty() {
        return Ok(());
    }
    Err(already_exists("repository record", "repo edit", id, &differs))
}

#[cfg(test)]
mod tests;
