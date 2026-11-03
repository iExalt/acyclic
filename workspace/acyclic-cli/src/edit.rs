//! The writes behind `add` and `edit`, which take the same fields: `edit` changes an existing entity and refuses an
//! unknown one, `add` creates one, and adding it again with what it already holds changes nothing while adding it with
//! other values is refused. Each runs inside a batch, so a refusal anywhere writes nothing at all.
//!
//! Membership and repositories of items and threads are scope edits, as every other write to them is. What an item
//! waits for is a link entity, written by the model's checked link and unlink.
//!
//! The fields hold what the command line named until [`ItemFields::resolve`] or [`ThreadFields::resolve`] turns each
//! reference into the id of the record it names; the writes take ids.

use acyclic_core::ids;
use acyclic_core::import::owner_name;
use acyclic_core::model::{By, ItemState, Model, Size, WriteError};
use acyclic_core::state::Replica;
use acyclic_core::write::{
    self, Assign, Change as WriteChange, ChangeSet, CommitError, FieldName, ItemAssign, Outcome,
    Record, Repeat, RepositoryAssign, Rule, ThreadAssign, Violation,
};
use serde_json::json;

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

    /// What the change gives the field: a value, or none for a clear; `None` to leave it.
    // The outer option is whether the field changes, the inner its value, as an assign takes it.
    #[allow(clippy::option_option)]
    fn given(&self) -> Option<Option<&T>> {
        match self {
            Self::Keep => None,
            Self::Set(value) => Some(Some(value)),
            Self::Clear => Some(None),
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

/// The id a repository add writes: the one given, else a new one.
pub(crate) fn new_repository_id(given: Option<&str>) -> Result<String, CliError> {
    Ok(given.map(given_id).transpose()?.unwrap_or_else(ids::new))
}

/// Judges and writes the change set `build` makes from the tracker's model, read once, telling a refusal as the CLI
/// does. `name` is the `owner/name` a repository write gave.
pub(crate) fn commit(
    replica: &mut Replica,
    by: By<'_>,
    name: Option<&str>,
    build: impl FnOnce(&Model) -> Result<ChangeSet, CliError>,
) -> Result<Outcome, CliError> {
    write::commit(replica, by, build).map_err(|error| match error {
        CommitError::Build(error) => error,
        CommitError::Refused(violations) => refusal(&violations, name),
        CommitError::Write(error) => error.into(),
    })
}

/// How an add treats the record: whether its id was made for it, which a title already on open work refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Adding {
    /// An edit of a record that exists.
    No,
    /// An add with the id given.
    Given,
    /// An add with an id made for it.
    Minted,
}

/// The change set an item's fields ask for: one add with every field given, or one change per field for an edit.
pub(crate) fn item_changes(id: &str, fields: &ItemFields, adding: Adding) -> ChangeSet {
    let field = |assign| Assign::Item(id.to_owned(), assign);
    let mut assigns = Vec::new();
    if let Some(title) = &fields.title {
        assigns.push(field(ItemAssign::Title(title.clone())));
    }
    if let Some(notes) = fields.notes.given() {
        assigns.push(field(ItemAssign::Notes(notes.cloned())));
    }
    if let Some(size) = fields.size.given() {
        assigns.push(field(ItemAssign::Size(size.copied())));
    }
    if let Some(parent) = fields.parent.given() {
        assigns.push(field(ItemAssign::Parent(parent.cloned())));
    }
    if let Some(state) = fields.state {
        assigns.push(field(ItemAssign::State(state)));
    }
    if let Some(thread) = fields.thread.given() {
        assigns.push(field(ItemAssign::Thread(thread.cloned())));
    }
    for (list, present) in [(&fields.waits_for.remove, false), (&fields.waits_for.add, true)] {
        assigns.extend(list.iter().map(|to| field(ItemAssign::WaitsFor(to.clone(), present))));
    }
    for (list, present) in [(&fields.repositories.add, true), (&fields.repositories.remove, false)]
    {
        assigns.extend(list.iter().map(|r| field(ItemAssign::Repository(r.clone(), present))));
    }
    changes(Record::Item(id.to_owned()), assigns, adding)
}

/// The change set a thread's fields ask for.
pub(crate) fn thread_changes(id: &str, fields: &ThreadFields, adding: Adding) -> ChangeSet {
    let field = |assign| Assign::Thread(id.to_owned(), assign);
    let mut assigns = Vec::new();
    if let Some(title) = &fields.title {
        assigns.push(field(ThreadAssign::Title(title.clone())));
    }
    if let Some(brief) = fields.brief.given() {
        assigns.push(field(ThreadAssign::Brief(brief.cloned())));
    }
    for (list, present) in [(&fields.repositories.add, true), (&fields.repositories.remove, false)]
    {
        assigns.extend(list.iter().map(|r| field(ThreadAssign::Repository(r.clone(), present))));
    }
    changes(Record::Thread(id.to_owned()), assigns, adding)
}

/// The change set a repository record's fields ask for; an `owner/name` that isn't one is a usage error.
pub(crate) fn repository_changes(
    id: &str,
    fields: &RepositoryFields,
    adding: Adding,
) -> Result<ChangeSet, CliError> {
    let field = |assign| Assign::Repository(id.to_owned(), assign);
    let mut assigns = Vec::new();
    if let Some(name) = &fields.name {
        let Some((owner, short)) = owner_name(name) else {
            return Err(CliError::new(
                "usage",
                exit::USAGE,
                format!(
                    "--name takes the repository's owner/name, such as me/garden, not {name:?}"
                ),
            ));
        };
        assigns.push(field(RepositoryAssign::Owner(owner.to_owned())));
        assigns.push(field(RepositoryAssign::Name(short.to_owned())));
    }
    if let Some(branch) = &fields.default_branch {
        assigns.push(field(RepositoryAssign::DefaultBranch(branch.clone())));
    }
    Ok(changes(Record::Repository(id.to_owned()), assigns, adding))
}

fn changes(record: Record, assigns: Vec<Assign>, adding: Adding) -> ChangeSet {
    ChangeSet(match adding {
        Adding::No => assigns.into_iter().map(WriteChange::Set).collect(),
        Adding::Given | Adding::Minted => vec![WriteChange::Add {
            record,
            minted: adding == Adding::Minted,
            on_repeat: Repeat::Refuse,
            with: assigns,
        }],
    })
}

#[cfg(test)]
mod tests;

/// The flag that sets a field, as an add's refusal names it, in the order the flags are checked.
const FLAGS: [(&str, &str); 11] = [
    ("title", "--title"),
    ("parent", "--parent"),
    ("notes", "--notes"),
    ("size", "--size"),
    ("state", "--state"),
    ("waits-for", "--waits-for"),
    ("thread", "--thread"),
    ("brief", "--brief"),
    ("repositories", "--repository"),
    ("name", "--name"),
    ("default-branch", "--default-branch"),
];

fn flag(field: &FieldName) -> &'static str {
    let name = match field {
        FieldName::Owner => "name",
        other => other.name(),
    };
    FLAGS.iter().find(|(of, _)| *of == name).map_or("--title", |(_, flag)| flag)
}

/// One violation as the CLI has always told it: its code, exit status, message and details. `name` is the
/// `owner/name` a repository write gave, which a taken name repeats as given.
fn violation_error(violation: &Violation, name: Option<&str>) -> CliError {
    let record = violation.at.as_ref().map(|at| &at.record);
    match &violation.rule {
        Rule::NoRecord(Record::Item(id)) => WriteError::NoItem(id.clone()).into(),
        Rule::NoRecord(Record::Thread(id)) => {
            not_found(format!("no thread {id}; `acyclic thread add --title <TITLE>` adds one"))
        },
        Rule::NoRecord(Record::Repository(id)) => not_found(format!(
            "no repository {id}; `acyclic repository add --name <OWNER/NAME> --default-branch <BRANCH>` adds one"
        )),
        Rule::Deleted(record) => WriteError::Deleted(record.id().to_owned()).into(),
        Rule::SelfLink => WriteError::SelfLink(record.map_or("", Record::id).to_owned()).into(),
        Rule::ParentCycle(_) => WriteError::ParentCycle(record.map_or("", Record::id).to_owned()).into(),
        Rule::LinkCycle(cycle) => WriteError::LinkCycle(cycle.clone()).into(),
        Rule::TitleTaken(other) => {
            let (of, id) = (other.kind(), other.id());
            CliError::refused(
                "already-exists",
                format!(
                    "{of} {id} has this title, so the add would make a second; change that one with `acyclic {of} \
                     edit {id}`, or give this one another title"
                ),
            )
            .with(json!({"id": id}))
        },
        Rule::NameTaken { name: taken, other } => CliError::refused(
            "already-exists",
            format!("repository {other} is named {} already", name.unwrap_or(taken)),
        )
        .with(json!({"id": other})),
        Rule::ArchivedOpen(root) => CliError::refused(
            "archived",
            format!(
                "item {root} is archived, and work under it that isn't done would be hidden; `acyclic item reopen` \
                 takes done work out of the archive"
            ),
        )
        .with(json!({"root": root})),
        Rule::EvidenceNotDone | Rule::RepeatDiffers | Rule::Contradicts => {
            CliError::refused("refused", violation.to_string())
        },
    }
}

/// A refused change set as one error. An add of a record that exists with other values is told once, naming every
/// flag that differs. Otherwise the first violation by exit status (a usage error, then one naming nothing, then a
/// refusal) and then by change speaks for all, its message and details as when it is alone, and with others the
/// details list every violation's message too.
pub(crate) fn refusal(violations: &[Violation], name: Option<&str>) -> CliError {
    // Every field an add gives another value is told once, where the first is.
    let mut errors: Vec<CliError> = Vec::new();
    let mut repeated = false;
    for violation in violations {
        match (&violation.rule, &violation.at) {
            (Rule::RepeatDiffers, Some(at)) if !repeated => {
                repeated = true;
                let mut differs: Vec<&str> = violations
                    .iter()
                    .filter(|v| v.rule == Rule::RepeatDiffers)
                    .filter_map(|v| v.at.as_ref())
                    .map(|at| flag(&at.field))
                    .collect();
                differs.sort_by_key(|flag| FLAGS.iter().position(|(_, f)| f == flag));
                differs.dedup();
                let (what, edit) = match at.record {
                    Record::Item(_) => ("item", "item edit"),
                    Record::Thread(_) => ("thread", "thread edit"),
                    Record::Repository(_) => ("repository record", "repository edit"),
                };
                errors.push(already_exists(what, edit, at.record.id(), &differs));
            },
            (Rule::RepeatDiffers, _) => {},
            _ => errors.push(violation_error(violation, name)),
        }
    }
    let rank = |error: &CliError| match error.exit {
        exit::USAGE => 0,
        exit::NOT_FOUND => 1,
        _ => 2,
    };
    debug_assert!(!errors.is_empty(), "a refusal names what it breaks");
    let first =
        errors.iter().enumerate().min_by_key(|(n, error)| (rank(error), *n)).map_or(0, |(n, _)| n);
    let mut chosen = errors[first].clone();
    if errors.len() > 1 {
        let all: Vec<&str> = errors.iter().map(|error| error.message.as_str()).collect();
        if !chosen.details.is_object() {
            chosen.details = json!({});
        }
        chosen.details["violations"] = json!(all);
    }
    chosen
}
