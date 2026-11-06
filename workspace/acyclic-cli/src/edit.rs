//! The change sets behind `add`, `edit` and `done`, and how the CLI tells their refusals. `add` and `edit` take the same
//! fields: `edit` changes an existing entity and refuses an unknown one, `add` creates one, and adding it again with
//! what it already holds changes nothing while adding it with other values is refused. The core's write module judges
//! each change set whole, so a refusal anywhere writes nothing at all.
//!
//! The fields hold what the command line named until [`ItemFields::resolve`] turns each reference into the id of the
//! record it names; the writes take ids.

use acyclic_core::ids;
use acyclic_core::import::owner_name;
use acyclic_core::model::{By, ItemState, Model, WriteError};
use acyclic_core::state::Replica;
use acyclic_core::write::{
    self, Assign, Change as WriteChange, ChangeSet, CommitError, FieldName, InitiativeAssign,
    ItemAssign, Outcome, Record, Repeat, RepositoryAssign, Rule, ThreadAssign, Violation,
};
use serde_json::json;

use crate::error::{CliError, DOCTOR_HINT, exit};
use crate::resolve::{self, Records};
use crate::view;

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
    /// Cancel it although someone else holds a claim on its work; only an edit's canceling asks.
    pub force: bool,
    pub notes: Change<String>,
    pub waits_for: Delta,
}

/// What an initiative's `add` or `edit` sets.
#[derive(Debug, Clone, Default)]
pub(crate) struct InitiativeFields {
    pub title: Option<String>,
    pub notes: Change<String>,
}

/// What a thread's `add` or `edit` sets.
#[derive(Debug, Clone, Default)]
pub(crate) struct ThreadFields {
    pub title: Option<String>,
    pub notes: Change<String>,
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

impl ItemFields {
    /// The fields with every reference resolved to the id of the record it names. `item` is the item they are for,
    /// when it exists: a removal looks among what it holds first, so it finds an element whose record is gone. An add
    /// (no `item`) that names no parent goes under the tracker's only initiative.
    pub fn resolve(
        self,
        model: &Model,
        records: &Records<'_>,
        item: Option<&str>,
    ) -> Result<Self, CliError> {
        let parent = match self.parent {
            Change::Set(parent) => Change::Set(records.parent(&parent)?),
            Change::Keep if item.is_none() => Change::Set(only_initiative(model)?),
            other => other,
        };
        let thread = match self.thread {
            Change::Set(thread) => Change::Set(records.thread(&thread)?),
            other => other,
        };
        let held_waits: Vec<String> = model
            .links
            .iter()
            .filter(|link| Some(link.from.as_str()) == item)
            .map(|link| link.to.clone())
            .collect();
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
        Ok(Self { parent, thread, waits_for, ..self })
    }
}

/// The initiative an add with no `--parent` goes under: the only one there is. With none or several, the add is refused,
/// naming the choices.
fn only_initiative(model: &Model) -> Result<String, CliError> {
    let mut found = model.initiatives.values();
    if let (Some(only), None) = (found.next(), found.next()) {
        return Ok(only.id.clone());
    }
    let listed: Vec<serde_json::Value> = model
        .initiatives
        .values()
        .map(|initiative| json!({"id": initiative.id, "title": initiative.title}))
        .collect();
    let message = if model.initiatives.is_empty() {
        "an item needs a parent and the tracker has no initiative: `acyclic initiative add --title <TITLE>` adds one, \
         or pass --parent <ITEM>"
            .to_owned()
    } else {
        let each: Vec<String> = model
            .initiatives
            .values()
            .map(|initiative| match initiative.title.all().first() {
                Some(title) => format!("{} ({title})", initiative.id),
                None => initiative.id.clone(),
            })
            .collect();
        format!(
            "an item needs a parent and the tracker has several initiatives: pass --parent <INITIATIVE or ITEM>, one of {}",
            each.join(", ")
        )
    };
    Err(CliError::refused("parent-required", message).with(json!({"initiatives": listed})))
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
/// does. `name` is the `owner/name` a repository write gave; `judge_finishing` judges finishing work by its readiness.
pub(crate) fn commit(
    replica: &mut Replica,
    by: By<'_>,
    name: Option<&str>,
    judge_finishing: bool,
    build: impl FnOnce(&Model) -> Result<ChangeSet, CliError>,
) -> Result<Outcome, CliError> {
    write::commit(replica, by, judge_finishing, build).map_err(|error| match error {
        CommitError::Build(error) => error,
        CommitError::Refused(violations, before) => refusal(&violations, name, &before),
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
    // No flag clears a parent, so a change to it is always a value.
    if let Some(Some(parent)) = fields.parent.given() {
        assigns.push(field(ItemAssign::Parent(parent.clone())));
    }
    let canceling = fields.state == Some(ItemState::Canceled) && adding == Adding::No;
    if let Some(state) = fields.state
        && !canceling
    {
        assigns.push(field(ItemAssign::State(state)));
    }
    if let Some(thread) = fields.thread.given() {
        assigns.push(field(ItemAssign::Thread(thread.cloned())));
    }
    for (list, present) in [(&fields.waits_for.remove, false), (&fields.waits_for.add, true)] {
        assigns.extend(list.iter().map(|to| field(ItemAssign::WaitsFor(to.clone(), present))));
    }
    let mut set = changes(Record::Item(id.to_owned()), assigns, adding);
    if canceling {
        set.0.push(WriteChange::Cancel { item: id.to_owned(), force: fields.force });
    }
    set
}

/// The change set an initiative's fields ask for.
pub(crate) fn initiative_changes(id: &str, fields: &InitiativeFields, adding: Adding) -> ChangeSet {
    let field = |assign| Assign::Initiative(id.to_owned(), assign);
    let mut assigns = Vec::new();
    if let Some(title) = &fields.title {
        assigns.push(field(InitiativeAssign::Title(title.clone())));
    }
    if let Some(notes) = fields.notes.given() {
        assigns.push(field(InitiativeAssign::Notes(notes.cloned())));
    }
    changes(Record::Initiative(id.to_owned()), assigns, adding)
}

/// The change set a thread's fields ask for.
pub(crate) fn thread_changes(id: &str, fields: &ThreadFields, adding: Adding) -> ChangeSet {
    let field = |assign| Assign::Thread(id.to_owned(), assign);
    let mut assigns = Vec::new();
    if let Some(title) = &fields.title {
        assigns.push(field(ThreadAssign::Title(title.clone())));
    }
    if let Some(notes) = fields.notes.given() {
        assigns.push(field(ThreadAssign::Notes(notes.cloned())));
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
const FLAGS: [(&str, &str); 8] = [
    ("title", "--title"),
    ("parent", "--parent"),
    ("notes", "--notes"),
    ("state", "--state"),
    ("waits-for", "--waits-for"),
    ("thread", "--thread"),
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

/// A refused cancel: the work is done, or someone else holds a claim on it.
fn cancel_refusal(rule: &Rule, record: Option<&Record>) -> CliError {
    let id = record.map_or("", Record::id);
    match rule {
        Rule::CancelHeld { holder } => CliError::refused(
            "claimed",
            format!("{holder} holds a claim on item {id}'s work; pass --force to cancel it anyway"),
        )
        .with(json!({"item": id, "holder": holder})),
        _ => CliError::refused(
            "done",
            format!("item {id} is done, so it can't be canceled: `acyclic item reopen {id}` first"),
        )
        .with(json!({"item": id})),
    }
}

/// A new item with no thread whose siblings don't share one.
fn unclear_refusal(id: &str, parent: &str) -> CliError {
    CliError::refused(
        "thread-unclear",
        format!(
            "item {id} joins no thread by itself: the other children of {parent} are in different threads, or some \
             in none; pass --thread <THREAD> or --no-thread"
        ),
    )
    .with(json!({"item": id, "parent": parent}))
}

/// One violation as the CLI has always told it: its code, exit status, message and details. `name` is the
/// `owner/name` a repository write gave, which a taken name repeats as given.
#[expect(clippy::too_many_lines, reason = "one arm for each rule the judge can refuse")]
fn violation_error(violation: &Violation, name: Option<&str>, before: &Model) -> CliError {
    let record = violation.at.as_ref().map(|at| &at.record);
    match &violation.rule {
        Rule::NoRecord(Record::Item(id)) => WriteError::NoItem(id.clone()).into(),
        Rule::NoRecord(Record::Initiative(id)) => not_found(format!(
            "no initiative {id}; `acyclic initiative add --title <TITLE>` adds one"
        )),
        Rule::NoRecord(Record::Thread(id)) => {
            not_found(format!("no thread {id}; `acyclic thread add --title <TITLE>` adds one"))
        },
        Rule::NoRecord(Record::Repository(id)) => not_found(format!(
            "no repository {id}; `acyclic repository add --name <OWNER/NAME> --default-branch <BRANCH>` adds one"
        )),
        Rule::Deleted(record) => WriteError::Deleted(record.id().to_owned()).into(),
        Rule::SelfLink => WriteError::SelfLink(record.map_or("", Record::id).to_owned()).into(),
        Rule::ParentCycle(_) => WriteError::ParentCycle(record.map_or("", Record::id).to_owned()).into(),
        Rule::WaitsForCycle(cycle) => WriteError::WaitsForCycle(cycle.clone()).into(),
        Rule::LineageLink { from, to } => CliError::refused("lineage-link", violation.to_string())
            .with(json!({"item": from, "waits_for": to})),
        Rule::IdTaken(other) => CliError::refused(
            "id-taken",
            format!(
                "{} {} has that id, and an id names one kind of record, since an item's parent names an item or an \
                 initiative by its id alone: leave --id out for the tracker to make one",
                other.kind(),
                other.id()
            ),
        )
        .with(json!({"id": other.id(), "kind": other.kind()})),
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
        Rule::EmptyRecord(_) => CliError::refused("refused", violation.to_string()),
        Rule::NameTaken { name: taken, other } => CliError::refused(
            "already-exists",
            format!("repository {other} is named {} already", name.unwrap_or(taken)),
        )
        .with(json!({"id": other})),
        Rule::NotReady(None) => {
            let id = record.map_or("", Record::id);
            CliError::new(
                "unreadable",
                exit::UNREADABLE,
                format!("can't read the claim needed to check item {id}'s readiness"),
            )
            .with(json!({"hint": DOCTOR_HINT}))
        },
        Rule::NotReady(status @ Some(_)) => {
            let id = record.map_or("", Record::id);
            CliError::refused(
                "not-ready",
                format!(
                    "item {id} isn't yours to finish: it isn't ready, or someone else holds it; `acyclic item show \
                     {id}` says which"
                ),
            )
            .with(json!({"readiness": status}))
        },
        Rule::ParentDone => {
            let id = record.map_or("", Record::id);
            CliError::refused(
                "parent",
                format!(
                    "item {id} has children, so it reads done once they are: finish its leaves instead (forcing \
                     doesn't change this)"
                ),
            )
            .with(json!({"item": id}))
        },
        Rule::ThreadUnclear { parent } => unclear_refusal(record.map_or("", Record::id), parent),
        Rule::CancelDone | Rule::CancelHeld { .. } => cancel_refusal(&violation.rule, record),
        Rule::EvidenceDiffers => {
            let id = record.map_or("", Record::id);
            let evidence = before.items.get(id).map(|item| view::evidence(before, &item.evidence));
            CliError::refused(
                "already-done",
                format!("item {id} is done with other evidence; pass --force to replace it"),
            )
            .with(json!({"evidence": evidence}))
        },
        Rule::InvalidCommit => CliError::new(
            "invalid-commit",
            exit::USAGE,
            "a commit is 7 to 64 hexadecimal characters, as git prints it",
        ),
        Rule::IncompleteRepository { repository, missing } => CliError::refused(
            "incomplete-repository",
            format!(
                "repository {repository} is missing {}; set it with `acyclic repository edit {repository}`",
                missing.join(", ")
            ),
        )
        .with(json!({"missing": missing.iter().map(|f| f.replace('-', "_")).collect::<Vec<_>>()})),
        Rule::EvidenceNotDone | Rule::RepeatDiffers | Rule::Contradicts | Rule::WrongRecord(_) => {
            CliError::refused("refused", violation.to_string())
        },
    }
}

/// A refused change set as one error. An add of a record that exists with other values is told once, naming every
/// flag that differs. Otherwise the first violation by exit status (a usage error, then one naming nothing, then a
/// refusal) and then by change speaks for all, its message and details as when it is alone, and with others the
/// details list every violation's message too.
pub(crate) fn refusal(violations: &[Violation], name: Option<&str>, before: &Model) -> CliError {
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
                    Record::Initiative(_) => ("initiative", "initiative edit"),
                    Record::Item(_) => ("item", "item edit"),
                    Record::Thread(_) => ("thread", "thread edit"),
                    Record::Repository(_) => ("repository record", "repository edit"),
                };
                errors.push(already_exists(what, edit, at.record.id(), &differs));
            },
            (Rule::RepeatDiffers, _) => {},
            _ => errors.push(violation_error(violation, name, before)),
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
