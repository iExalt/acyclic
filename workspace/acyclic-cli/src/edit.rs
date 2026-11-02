//! The writes behind `add` and `edit`, which take the same fields: `edit` changes an existing entity and refuses an
//! unknown one, `add` creates one, and adding it again with what it already holds changes nothing while adding it with
//! other values is refused. Each runs inside a batch, so a refusal anywhere writes nothing at all.
//!
//! Membership and repositories of items and threads are scope edits, as every other write to them is. What an item
//! waits for is a link entity, written by the model's checked link and unlink.
//!
//! The fields hold what the command line named until [`ItemFields::resolve`] or [`ThreadFields::resolve`] turns each
//! reference into the id of the record it names; the writes take ids.

use acyclic_core::archive;
use acyclic_core::ids;
use acyclic_core::import::owner_name;
use acyclic_core::model::{self, By, ItemState, Model, Size};
use acyclic_core::place::{self, Inherited};
use acyclic_core::schema::{
    item as field, kind, link, repository as repo_field, thread as thread_field,
};
use acyclic_core::scope::{self, ScopeEdit};
use acyclic_core::state::{Key, Replica, State};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::commands::{existing, holds_only, set_if};
use crate::error::{CliError, exit};
use crate::resolve::{self, Records};

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
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemFields {
    pub title: Option<String>,
    pub parent: Change<String>,
    pub thread: Change<String>,
    pub state: Option<ItemState>,
    pub notes: Change<String>,
    pub size: Change<Size>,
    pub repositories: Delta,
    pub waits_for: Delta,
}

/// What a thread's `add` or `edit` sets.
#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadFields {
    pub title: Option<String>,
    pub brief: Change<String>,
    pub repositories: Delta,
}

/// What a repository record's `add` or `edit` sets.
#[derive(Debug, Default)]
pub(crate) struct RepositoryFields {
    /// Its `owner/name`.
    pub name: Option<String>,
    pub default_branch: Option<String>,
}

impl Delta {
    /// Refuses a change that adds and removes the same record, however each flag spelled it: the check on the
    /// command line compares only what was typed.
    fn disjoint(self, flag: &str) -> Result<Self, CliError> {
        match self.add.iter().find(|id| self.remove.contains(id)) {
            Some(id) => Err(CliError::new(
                "usage",
                exit::USAGE,
                format!("--{flag} and --no-{flag} both name {id}"),
            )),
            None => Ok(self),
        }
    }
}

/// The repositories a removal names: each element `held` that it names by id or ending, else the record the resolver
/// finds.
fn removals(
    records: &Records<'_>,
    held: &[String],
    named: &[String],
) -> Result<Vec<String>, CliError> {
    named
        .iter()
        .map(|name| match resolve::held(held, name) {
            Some(id) => Ok(id),
            None => records.repository(name),
        })
        .collect()
}

impl ItemFields {
    /// The fields with every reference resolved to the id of the record it names. `item` is the item they are for,
    /// when it exists: a removal looks among what it holds first, so it finds an element whose record is gone.
    pub fn resolve(
        self,
        model: &Model,
        records: &Records<'_>,
        item: Option<&str>,
    ) -> Result<Self, CliError> {
        let parent = match self.parent {
            Change::Set(parent) => Change::Set(records.item(&parent)?),
            other => other,
        };
        let thread = match self.thread {
            Change::Set(thread) => Change::Set(records.thread(&thread)?),
            other => other,
        };
        let held_repositories: Vec<String> = item
            .and_then(|id| model.items.get(id))
            .map(|item| item.repositories.iter().cloned().collect())
            .unwrap_or_default();
        let held_waits: Vec<String> = model
            .links
            .iter()
            .filter(|link| Some(link.from.as_str()) == item)
            .map(|link| link.to.clone())
            .collect();
        let repositories = Delta {
            add: self
                .repositories
                .add
                .iter()
                .map(|r| records.repository(r))
                .collect::<Result<_, _>>()?,
            remove: removals(records, &held_repositories, &self.repositories.remove)?,
        }
        .disjoint("repository")?;
        let waits_for = Delta {
            add: self.waits_for.add.iter().map(|i| records.item(i)).collect::<Result<_, _>>()?,
            remove: self
                .waits_for
                .remove
                .iter()
                .map(|name| match resolve::held(&held_waits, name) {
                    Some(id) => Ok(id),
                    None => records.item(name),
                })
                .collect::<Result<_, _>>()?,
        }
        .disjoint("waits-for")?;
        Ok(Self { parent, thread, repositories, waits_for, ..self })
    }
}

impl ThreadFields {
    /// The fields with every repository resolved to the id of its record, a removal looking among what `thread`
    /// holds first.
    pub fn resolve(
        self,
        model: &Model,
        records: &Records<'_>,
        thread: Option<&str>,
    ) -> Result<Self, CliError> {
        let held: Vec<String> = thread
            .and_then(|id| model.threads.get(id))
            .map(|thread| thread.repositories.iter().cloned().collect())
            .unwrap_or_default();
        let repositories = Delta {
            add: self
                .repositories
                .add
                .iter()
                .map(|r| records.repository(r))
                .collect::<Result<_, _>>()?,
            remove: removals(records, &held, &self.repositories.remove)?,
        }
        .disjoint("repository")?;
        Ok(Self { repositories, ..self })
    }
}

/// What a write did beyond writing: the threads items inherited.
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub inherited: Vec<Inherited>,
}

impl Written {
    /// An item's data: the item, and the threads items joined as `{item, thread}`.
    pub fn item(self, id: &str) -> Json {
        json!({"item": id, "placed": self.inherited})
    }
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
            "no thread {thread}; `acyclic thread add --title <TITLE>` adds one"
        )));
    }
    match repositories.iter().find(|id| !model.repositories.contains_key(*id)) {
        Some(id) => Err(not_found(format!(
            "no repository {id}; `acyclic repository add --name <OWNER/NAME> --default-branch <BRANCH>` adds one"
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

/// Whether a scope edit would change nothing.
fn scope_already(state: &State, model: &Model, change: &ScopeEdit) -> Result<bool, CliError> {
    Ok(match change {
        ScopeEdit::Member { item, thread } => {
            // Read the register itself: a conflict reads as no single thread, which a clear must still settle.
            let key = model::item_key(item)?;
            model.items.contains_key(item)
                && holds_only(state.field(&key, field::THREAD), thread.as_ref().map(|t| json!(t)))
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

/// Applies one scope edit unless it is so already.
fn scope_step(replica: &mut Replica, change: &ScopeEdit, by: By<'_>) -> Result<(), CliError> {
    let model = Model::read(replica.state())?;
    if !scope_already(replica.state(), &model, change)? {
        scope::edit(replica, change, by)?;
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
    write_change(replica, &key, field::SIZE, &fields.size, by)?;
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
        // Work set back from done is reopened, as `item reopen` does: out of any archived subtree, and flagged so its
        // dependents are listed.
        let done = Model::read(replica.state())?.items.get(id).is_some_and(model::Item::is_done);
        if done && state != ItemState::Done {
            archive::reopen(replica, id, by)?;
        }
        model::set_state(replica, id, state, by)?;
    }
    // A removal names a link the item holds by its exact ids, so it lets go of one whose other end is gone.
    for to in &fields.waits_for.remove {
        model::unlink(replica, id, to, by)?;
    }
    for to in &fields.waits_for.add {
        if !waits_for(replica.state(), id, to)? {
            model::link(replica, id, to, by)?;
        }
    }
    let mut written = Written::default();
    for change in item_scope(id, fields) {
        scope_step(replica, &change, by)?;
    }
    // An explicit thread, or none, is the answer; otherwise a moved item joins the one its parent passes down.
    if moved && fields.thread == Change::Keep {
        written.inherited = place::place(replica, &[id.to_owned()], by)?;
    }
    Ok(written)
}

/// A given id as stored: a `UUIDv7` in any case, lowercased; any other is refused.
pub(crate) fn given_id(id: &str) -> Result<String, CliError> {
    ids::v7(id).ok_or_else(|| {
        CliError::new(
            "invalid-id",
            exit::USAGE,
            format!("{id:?} isn't a UUIDv7: leave --id out for the tracker to make one, or take one from `acyclic id`"),
        )
    })
}

/// The id an add writes: the one given, else a new one. An add of an id that exists is a repeat, for the add to
/// accept or refuse. Otherwise an add whose title is already a thread's, or an open item's, is refused naming that
/// record, so a retried add doesn't make a second one. A done item's title is free again, since titles recur once
/// work is finished.
pub(crate) fn new_id(
    replica: &Replica,
    model: &Model,
    of: &str,
    given: Option<&str>,
    title: &str,
) -> Result<String, CliError> {
    let given = given.map(given_id).transpose()?;
    if let Some(id) = &given {
        let exists = if of == kind::THREAD {
            replica.state().entity(&thread_key(id)?).is_some()
        } else {
            replica.state().entity(&model::item_key(id)?).is_some()
        };
        if exists {
            return Ok(id.clone());
        }
    }
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
            format!(
                "{of} {id} has this title, so the add would make a second; change that one with `acyclic {of} edit \
                 {id}`, or give this one another title"
            ),
        )
        .with(json!({"id": id})));
    }
    Ok(given.unwrap_or_else(ids::new))
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
        return repeat_item(replica.state(), id, fields);
    }
    // Everything that can be refused is, before the item exists.
    check_targets(replica.state(), fields.thread.value(), &fields.repositories.add)?;
    model::add_item(replica, id, title, by)?;
    edit_item(replica, id, fields, by)
}

/// An add of an item that exists: it changes nothing when it names what the item holds, and is refused otherwise.
fn repeat_item(state: &State, id: &str, fields: &ItemFields) -> Result<Written, CliError> {
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
    check("--size", fields.size.value().is_none_or(|s| item.size.one() == Some(s)));
    check(
        "--state",
        fields.state.is_none_or(|s| item.state.one().copied().unwrap_or(ItemState::Backlog) == s),
    );
    let mut waits = true;
    for to in &fields.waits_for.add {
        waits &= waits_for(state, id, to)?;
    }
    check("--waits-for", waits);
    let changes = item_scope(id, fields);
    for (flag, wanted) in [("--thread", true), ("--repository", false)] {
        let satisfied = changes
            .iter()
            .filter(|change| matches!(change, ScopeEdit::Member { .. }) == wanted)
            .map(|change| scope_already(state, &model, change))
            .collect::<Result<Vec<bool>, CliError>>()?;
        check(flag, satisfied.into_iter().all(|same| same));
    }
    if !differs.is_empty() {
        return Err(already_exists("item", "item edit", id, &differs));
    }
    Ok(Written::default())
}

/// Whether `from` waits for `to`: the link between them is present.
fn waits_for(state: &State, from: &str, to: &str) -> Result<bool, CliError> {
    Ok(state.field(&model::link_key(from, to)?, link::PRESENT).is_some())
}

fn thread_key(id: &str) -> Result<Key, CliError> {
    Ok(Key::new(kind::THREAD, id)?)
}

fn write_thread(
    replica: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<(), CliError> {
    check_targets(replica.state(), None, &fields.repositories.add)?;
    let key = thread_key(id)?;
    set_if(replica, &key, thread_field::TITLE, fields.title.as_ref(), by)?;
    write_change(replica, &key, thread_field::BRIEF, &fields.brief, by)?;
    for change in thread_scope(id, fields) {
        scope_step(replica, &change, by)?;
    }
    Ok(())
}

/// Edits a thread, which exists when it has fields of its own or an item names it.
pub(crate) fn edit_thread(
    replica: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<(), CliError> {
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
) -> Result<(), CliError> {
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
    for change in &thread_scope(id, fields) {
        if !scope_already(state, &model, change)? && !differs.contains(&"--repository") {
            differs.push("--repository");
        }
    }
    if !differs.is_empty() {
        return Err(already_exists("thread", "thread edit", id, &differs));
    }
    Ok(())
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
        return Err(not_found(format!("no repository {id}; `acyclic repository list` lists them")));
    }
    write_repository(replica, &key, fields, by)
}

/// Refuses an `owner/name` that isn't one, or that another record has, compared in any case.
fn check_name(model: &Model, id: &str, name: &str) -> Result<(String, String), CliError> {
    let Some((owner, short)) = owner_name(name) else {
        return Err(CliError::new(
            "usage",
            exit::USAGE,
            format!("--name takes the repository's owner/name, such as me/garden, not {name:?}"),
        ));
    };
    let taken = model.repositories.values().find(|record| {
        record.id != id
            && resolve::repository_name(record).is_some_and(|held| held.eq_ignore_ascii_case(name))
    });
    if let Some(record) = taken {
        return Err(CliError::refused(
            "already-exists",
            format!("repository {} is named {name} already", record.id),
        )
        .with(json!({"id": record.id})));
    }
    Ok((owner.to_owned(), short.to_owned()))
}

fn write_repository(
    replica: &mut Replica,
    key: &Key,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<(), CliError> {
    if let Some(name) = &fields.name {
        let (owner, short) = check_name(&Model::read(replica.state())?, &key.id, name)?;
        set_if(replica, key, repo_field::OWNER, Some(&owner), by)?;
        set_if(replica, key, repo_field::NAME, Some(&short), by)?;
    }
    set_if(replica, key, repo_field::DEFAULT_BRANCH, fields.default_branch.as_ref(), by)
}

/// The id a repository add writes: the one given, else a new one.
pub(crate) fn new_repository_id(given: Option<&str>) -> Result<String, CliError> {
    Ok(given.map(given_id).transpose()?.unwrap_or_else(ids::new))
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
        not_found(format!("no repository {id}; `acyclic repository list` lists them"))
    })?;
    let held = resolve::repository_name(record);
    let mut differs: Vec<&str> = Vec::new();
    for (flag, same) in [
        ("--name", fields.name.as_ref() == held.as_ref()),
        ("--default-branch", fields.default_branch.as_ref() == record.default_branch.one()),
    ] {
        if !same {
            differs.push(flag);
        }
    }
    if differs.is_empty() {
        return Ok(());
    }
    Err(already_exists("repository record", "repository edit", id, &differs))
}

#[cfg(test)]
mod tests;
