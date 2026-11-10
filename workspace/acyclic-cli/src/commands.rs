//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use acyclic_core::claim;
use acyclic_core::delete;
use acyclic_core::ids;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, Model};
use acyclic_core::needs;
use acyclic_core::ready::ReadinessEvaluator;
use acyclic_core::schema::{item as field, kind};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use acyclic_core::write::{self, ChangeSet};
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{
    Class, Cli, Command, InitiativeCommand, ItemCommand, LEASE_MAX_MINUTES, RepositoryCommand,
    StateWord, ThreadCommand,
};
use crate::context;
use crate::edit::{
    self, Adding, Change, Delta, InitiativeFields, ItemFields, RepositoryFields, ThreadFields,
};
use crate::error::{CliError, exit};
use crate::resolve::Records;
use crate::view::{self, View};

/// What a command prints.
pub enum Output {
    /// The envelope's data.
    Json(Json),
    /// Plain text.
    Text(String),
}

const MINUTE: Millis = 60 * 1000;
/// The default time lease.
const LEASE: Millis = 60 * MINUTE;
/// The longest time lease.
const MAX_LEASE: Millis = LEASE_MAX_MINUTES as Millis * MINUTE;

/// What pretty output calls each record, by id.
pub type Names = BTreeMap<String, String>;

/// Runs a parsed command.
pub fn run(cli: &Cli, env: &Env) -> Result<Output, CliError> {
    run_with(cli, env, false, None).map(|(output, _)| output)
}

/// Runs one app write with conditions checked under the store lock before the batch changes anything.
pub fn run_expected(
    cli: &Cli,
    env: &Env,
    expected: Vec<acyclic_core::write::Expectation>,
) -> Result<Output, CliError> {
    run_with(cli, env, false, Some(expected)).map(|(output, _)| output)
}

/// Runs one app write and reports exact needs acknowledged by this batch.
pub fn run_observed(
    cli: &Cli,
    env: &Env,
    expected: Vec<acyclic_core::write::Expectation>,
    observed: Vec<String>,
) -> Result<(Output, Vec<String>), CliError> {
    run_with_observed(cli, env, false, Some(expected), Some(observed))
        .map(|(output, _, acknowledged)| (output, acknowledged))
}

/// Runs a parsed command, and says what pretty output calls each record the tracker holds once it is done: items and
/// threads by the shortest ending of their ids that names them, and repositories by `owner/name`. They come from the
/// state the command last read or wrote, so a record it added is named too, and from the tracker as it is after the
/// command when it neither read nor wrote through the model; none for a command with no tracker, or one that can't
/// be read.
pub fn run_named(cli: &Cli, env: &Env) -> Result<(Output, Names), CliError> {
    run_with(cli, env, true, None)
}

fn run_with(
    cli: &Cli,
    env: &Env,
    named: bool,
    expected: Option<Vec<acyclic_core::write::Expectation>>,
) -> Result<(Output, Names), CliError> {
    run_with_observed(cli, env, named, expected, None).map(|(output, names, _)| (output, names))
}

fn run_with_observed(
    cli: &Cli,
    env: &Env,
    named: bool,
    expected: Option<Vec<acyclic_core::write::Expectation>>,
    observed: Option<Vec<String>>,
) -> Result<(Output, Names, Vec<String>), CliError> {
    let text = |text: String| (Output::Text(text), Names::new(), Vec::new());
    match (cli.command.class(), &cli.command) {
        (Class::Standalone, Command::Init { path, replica, remote }) => {
            init(&env.cwd.join(path), replica, remote.as_deref())
                .map(|data| (Output::Json(data), Names::new(), Vec::new()))
        },
        (Class::Standalone, Command::Prime) => Ok(text(prime())),
        (Class::Standalone, Command::Id { count }) => {
            Ok(text((0..*count).map(|_| ids::new() + "\n").collect()))
        },
        (Class::Standalone, Command::Completions { shell }) => Ok(text(completions(*shell))),
        // `import --schema`, the only other command needing no tracker.
        (Class::Standalone, _) => Ok(text(import::SCHEMA.to_owned())),
        (Class::Server, _) => {
            Err(CliError::new("no-server", exit::CONFIG, "this build of acyclic has no server"))
        },
        (Class::Clone | Class::Read | Class::Write, command) => {
            if expected.is_some() && command.class() != Class::Write {
                return Err(CliError::new(
                    "usage",
                    exit::USAGE,
                    "write expectations require a write command",
                ));
            }
            let result =
                CommandContext::open(cli, env, named, expected, observed).and_then(|context| {
                    if matches!(command, Command::Export) {
                        return Ok((Output::Text(context.export()?), Names::new(), Vec::new()));
                    }
                    let data = context.dispatch(command)?;
                    let output_names = if named { context.names() } else { Names::new() };
                    let acknowledged = context.acknowledged.into_inner();
                    Ok((Output::Json(data), output_names, acknowledged))
                });
            result.map_err(|mut error| {
                // Doctor doesn't point at itself.
                if matches!(command, Command::Doctor)
                    && error.details == json!({"hint": crate::error::DOCTOR_HINT})
                {
                    error.details = Json::Null;
                }
                error
            })
        },
    }
}

/// What pretty output calls each record of `model`.
fn names_of(model: &Model) -> Names {
    let records = Records::of(model);
    let mut names = records.endings(true);
    names.extend(records.endings(false));
    for record in model.repositories.values() {
        if let Some(name) = crate::resolve::repository_name(record) {
            names.insert(record.id.clone(), name);
        }
    }
    names
}

struct CommandContext<'a> {
    tracker: Tracker,
    actor: String,
    now: Millis,
    env: &'a Env,
    /// The names of the records in the state the command last read or wrote through [`CommandContext::read`] or
    /// [`CommandContext::write`], kept only when pretty output asked for them.
    named: Option<RefCell<Option<Names>>>,
    expected: Option<Vec<write::Expectation>>,
    observed: Option<BTreeSet<String>>,
    acknowledged: RefCell<Vec<String>>,
    /// The learned clones this command has looked up, shared by every check of its commit evidence.
    learned: Rc<crate::learned::Learned>,
}

impl<'a> CommandContext<'a> {
    fn open(
        cli: &Cli,
        env: &'a Env,
        named: bool,
        expected: Option<Vec<write::Expectation>>,
        observed: Option<Vec<String>>,
    ) -> Result<Self, CliError> {
        let actor = context::actor(cli, env)?;
        let path = context::tracker(cli, env)?;
        let (tracker, renamed) = Tracker::open(&path)?;
        if let Some(renamed) = renamed {
            // A clone without its own state took a fresh id; say so once, but carry on.
            eprintln!("acyclic: this clone is now replica {renamed}");
        }
        Ok(Self {
            tracker,
            actor,
            now: context::now(env),
            env,
            named: named.then(|| RefCell::new(None)),
            expected,
            observed: observed.map(|ids| ids.into_iter().collect()),
            acknowledged: RefCell::new(Vec::new()),
            learned: Rc::default(),
        })
    }

    #[expect(clippy::too_many_lines, reason = "one short arm per command")]
    fn dispatch(&self, command: &Command) -> Result<Json, CliError> {
        match command {
            Command::Init { .. }
            | Command::Prime
            | Command::Id { .. }
            | Command::Import { file: None, .. }
            | Command::Export
            | Command::Serve(_)
            | Command::Up(_)
            | Command::Completions { .. } => {
                unreachable!("handled before dispatch")
            },
            Command::Thread(ThreadCommand::Ready { initiative }) => self.ready(initiative.as_deref()),
            Command::Initiative(InitiativeCommand::List) => self.initiatives(),
            Command::Initiative(InitiativeCommand::Show { initiatives }) => {
                self.show_initiatives(initiatives)
            },
            Command::Item(ItemCommand::List) => self.list(),
            Command::Item(ItemCommand::Show { items }) => self.show(items),
            Command::Needs => self.needs(),
            Command::Sync => self.sync(),
            Command::Compact => self.compact(),
            Command::Doctor => self.doctor(),
            Command::Item(ItemCommand::Add {
                id: given,
                title,
                parent,
                thread,
                no_thread,
                claim: claiming,
                state,
                notes,
                waits_for,
                allow_redundant,
            }) => {
                let fields = ItemFields {
                    title: Some(title.clone()),
                    parent: Change::of(parent.clone(), false),
                    thread: Change::of(thread.clone(), *no_thread),
                    state: state.map(StateWord::state),
                    force: false,
                    allow_redundant: *allow_redundant,
                    notes: Change::of(notes.clone(), false),
                    waits_for: Delta { add: waits_for.clone(), remove: Vec::new() },
                };
                self.write(|replica, by| {
                    let (id, adding) = match given {
                        Some(given) => (edit::given_id(given)?, Adding::Given),
                        None => (acyclic_core::ids::new(), Adding::Minted),
                    };
                    let outcome = edit::commit(replica, by, None, false, |model| {
                        let fields = fields.clone().resolve(model, &Records::of(model), None)?;
                        Ok(edit::item_changes(&id, &fields, adding))
                    })?;
                    self.remember(&outcome.after);
                    let mut data = json!({"item": id, "placed": outcome.placed(), "title": title});
                    if *claiming {
                        // In the same write: a refusal here leaves the add unwritten too.
                        data["claim"] = claim_thread_of(replica, by, &id, &outcome.after)?;
                    }
                    Ok(data)
                })
                .map_err(|error| {
                    self.misplaced(error, parent.as_deref(), || {
                        let id =
                            given.as_deref().map(|id| format!(" --id {id}")).unwrap_or_default();
                        format!(
                            "acyclic item add{id} --title <TITLE> --thread {}",
                            shell_word(parent.as_deref().unwrap_or_default())
                        )
                    })
                })
            },
            Command::Item(ItemCommand::Edit(edit)) => {
                let fields = ItemFields {
                    title: edit.title.clone(),
                    parent: Change::of(edit.parent.clone(), false),
                    thread: Change::of(edit.thread.clone(), edit.no_thread),
                    state: edit.state.map(StateWord::state),
                    force: edit.force,
                    allow_redundant: edit.allow_redundant,
                    notes: Change::of(edit.notes.clone(), edit.no_notes),
                    waits_for: Delta {
                        add: edit.waits_for.clone(),
                        remove: edit.no_waits_for.clone(),
                    },
                };
                let item = &edit.item;
                self.write(|replica, by| {
                    let mut id = String::new();
                    // Canceling is judged by the claims on the work, which readiness reads.
                    let canceling = fields.state == Some(ItemState::Canceled);
                    let outcome = edit::commit(replica, by, None, canceling, |model| {
                        let records = Records::of(model);
                        id = records.item(item)?;
                        let fields = fields.clone().resolve(model, &records, Some(&id))?;
                        Ok(edit::item_changes(&id, &fields, Adding::No))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"item": id, "placed": outcome.placed()}))
                })
                .map_err(|error| {
                    self.misplaced(error, edit.parent.as_deref(), || {
                        format!(
                            "acyclic item edit {} --thread {}",
                            shell_word(item),
                            shell_word(edit.parent.as_deref().unwrap_or_default())
                        )
                    })
                })
            },
            Command::Item(ItemCommand::Done { item, commit, evidence: text, force }) => {
                let commit = commit.as_deref().map(split_commit).transpose()?;
                let prepared = self.prepare_item_done(item, commit)?;
                self.write_prepared(&prepared, |replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, true, |model| {
                        let records = Records::of(model);
                        id = records.item(item)?;
                        let evidence = (commit.is_some() || text.is_some())
                            .then(|| -> Result<Evidence, CliError> {
                                Ok(Evidence {
                                    repository: commit
                                        .map(|(r, _)| records.repository(r))
                                        .transpose()?,
                                    commit: commit.map(|(_, sha)| sha.to_owned()),
                                    text: text.clone(),
                                })
                            })
                            .transpose()?;
                        Ok(ChangeSet(vec![write::Change::Finish {
                            item: id.clone(),
                            evidence,
                            force: *force,
                        }]))
                    })?;
                    self.remember(&outcome.after);
                    // The evidence it holds now, so a caller sees what was recorded.
                    let evidence =
                        view::evidence(&outcome.after, &outcome.after.items[&id].evidence);
                    Ok(json!({"item": id, "state": ItemState::Done.word(), "evidence": evidence}))
                })
            },
            Command::Thread(ThreadCommand::Release { thread }) => self.write(|replica, by| {
                let (id, key) = existing_thread(replica, thread)?;
                Ok(json!({"thread": id, "released": claim::release(replica, &key, by.actor)}))
            }),
            Command::Item(ItemCommand::Reopen { item }) => self.write(|replica, by| {
                let mut id = String::new();
                let outcome = edit::commit(replica, by, None, false, |model| {
                    id = Records::of(model).item(item)?;
                    if !model.children(&id).is_empty() {
                        return Err(CliError::refused(
                            "parent",
                            format!(
                                "{id} has children, so it reads done once they are: reopen one of them instead"
                            ),
                        )
                        .with(json!({"item": id})));
                    }
                    if !model.items[&id].is_done() {
                        return Err(CliError::refused(
                            "not-done",
                            format!("{id} isn't done, so there is nothing to reopen"),
                        ));
                    }
                    Ok(ChangeSet(vec![write::Change::Set(write::Assign::Item(
                        id.clone(),
                        write::ItemAssign::State(ItemState::Backlog),
                    ))]))
                })?;
                self.remember(&outcome.after);
                Ok(json!({"item": id}))
            }),
            Command::Thread(ThreadCommand::Claim { thread, lease, force }) => {
                self.claim(thread, *lease, *force)
            },
            Command::Thread(ThreadCommand::List) => self.threads(),
            Command::Thread(ThreadCommand::Show { threads }) => self.show_threads(threads),
            Command::Thread(ThreadCommand::Add { id: given, title, notes }) => {
                let fields = ThreadFields {
                    title: Some(title.clone()),
                    notes: Change::of(notes.clone(), false),
                };
                self.write(|replica, by| {
                    let (id, adding) = match given {
                        Some(given) => (edit::given_id(given)?, Adding::Given),
                        None => (acyclic_core::ids::new(), Adding::Minted),
                    };
                    let outcome = edit::commit(replica, by, None, false, |_| {
                        Ok(edit::thread_changes(&id, &fields, adding))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"thread": id, "title": title}))
                })
            },
            Command::Thread(ThreadCommand::Edit(edit)) => {
                let fields = ThreadFields {
                    title: edit.title.clone(),
                    notes: Change::of(edit.notes.clone(), edit.no_notes),
                };
                let thread = &edit.thread;
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, false, |model| {
                        id = Records::of(model).thread(thread)?;
                        Ok(edit::thread_changes(&id, &fields, Adding::No))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"thread": id}))
                })
            },
            Command::Repository(RepositoryCommand::List) => self.repositories(),
            Command::Repository(RepositoryCommand::Show { repositories }) => {
                self.show_repositories(repositories)
            },
            Command::Repository(RepositoryCommand::Add { id: given, name, default_branch }) => {
                let fields = RepositoryFields {
                    name: Some(name.clone()),
                    default_branch: Some(default_branch.clone()),
                };
                self.write(|replica, by| {
                    let id = edit::new_repository_id(given.as_deref())?;
                    let adding = if given.is_some() { Adding::Given } else { Adding::Minted };
                    let outcome = edit::commit(replica, by, Some(name), false, |_| {
                        edit::repository_changes(&id, &fields, adding)
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"repository": id}))
                })
            },
            Command::Repository(RepositoryCommand::Edit { repository, name, default_branch }) => {
                let fields =
                    RepositoryFields { name: name.clone(), default_branch: default_branch.clone() };
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, name.as_deref(), false, |model| {
                        id = Records::of(model).repository(repository)?;
                        edit::repository_changes(&id, &fields, Adding::No)
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"repository": id}))
                })
            },
            Command::Initiative(InitiativeCommand::Add { id: given, title, notes }) => {
                let fields = InitiativeFields {
                    title: Some(title.clone()),
                    notes: Change::of(notes.clone(), false),
                };
                self.write(|replica, by| {
                    let (id, adding) = match given {
                        Some(given) => (edit::given_id(given)?, Adding::Given),
                        None => (acyclic_core::ids::new(), Adding::Minted),
                    };
                    let outcome = edit::commit(replica, by, None, false, |_| {
                        Ok(edit::initiative_changes(&id, &fields, adding))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"initiative": id, "title": title}))
                })
            },
            Command::Initiative(InitiativeCommand::Edit(edit)) => {
                let fields = InitiativeFields {
                    title: edit.title.clone(),
                    notes: Change::of(edit.notes.clone(), edit.no_notes),
                };
                let initiative = &edit.initiative;
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, false, |model| {
                        id = Records::of(model).initiative(initiative)?;
                        Ok(edit::initiative_changes(&id, &fields, Adding::No))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"initiative": id}))
                })
            },
            Command::Initiative(InitiativeCommand::Delete { initiative }) => {
                self.write(|replica, by| {
                    let found = Records::of(&Model::read(replica.state())?).initiative(initiative);
                    let id = deletable(found, initiative, replica.state(), kind::INITIATIVE)?;
                    delete::delete_initiative(replica, &id, by)?;
                    Ok(json!({"initiative": id}))
                })
            },
            Command::Thread(ThreadCommand::Delete { thread }) => self.write(|replica, by| {
                let found = Records::of(&Model::read(replica.state())?).thread(thread);
                let id = deletable(found, thread, replica.state(), kind::THREAD)?;
                delete::delete_thread(replica, &id, by)?;
                Ok(json!({"thread": id}))
            }),
            Command::Import { file: Some(file), dry_run, .. } => self.import(file, *dry_run),
            Command::Item(ItemCommand::Delete { item }) => self.write(|replica, by| {
                let found = Records::of(&Model::read(replica.state())?).item(item);
                let id = deletable(found, item, replica.state(), kind::ITEM)?;
                delete::delete_item(replica, &id, by)?;
                Ok(json!({"item": id}))
            }),
        }
    }

    fn read(&self) -> Result<(State, Model), CliError> {
        let (_, state) = self.tracker.load()?;
        let model = Model::read(&state)?;
        acyclic_core::probe::mark("model read");
        self.remember(&model);
        Ok((state, model))
    }

    /// Keeps the names of `model`'s records, when pretty output asked for them.
    fn remember(&self, model: &Model) {
        if let Some(named) = &self.named {
            *named.borrow_mut() = Some(names_of(model));
        }
    }

    /// The names [`CommandContext::remember`] kept; else, for a command that neither read nor wrote through the model, such
    /// as `sync` or `doctor`, those of the tracker as it is now, read through the tracker this command opened.
    fn names(&self) -> Names {
        if let Some(names) = self.named.as_ref().and_then(|named| named.borrow_mut().take()) {
            return names;
        }
        let read = || -> Result<Names, CliError> {
            let (_, state) = self.tracker.load()?;
            Ok(names_of(&Model::read(&state)?))
        };
        read().unwrap_or_default()
    }

    /// Runs one batch as the actor. Lease renewal rides along after the command's own writes, so a batch can commit though the command changed
    /// nothing; `changed` says whether the command itself changed anything.
    fn write(
        &self,
        apply: impl FnOnce(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        self.write_prepared(&crate::evidence::Prepared::default(), apply)
    }

    fn write_prepared(
        &self,
        prepared: &crate::evidence::Prepared,
        apply: impl FnOnce(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        let result = RefCell::new((Json::Null, false));
        let unchecked = RefCell::new(Vec::new());
        let meta = Meta { actor: self.actor.clone() };
        let expected = self.expected.as_deref();
        self.tracker.batch(&meta, self.now, false, &initiative_of, |replica, at| {
            if let Some(expected) = expected {
                let model = Model::read(replica.state())?;
                let mismatches = write::check_expected(replica.state(), &model, expected, at)?;
                if !mismatches.is_empty() {
                    return Err(CliError::refused(
                        "changed-since",
                        "the state changed since the app read it",
                    )
                    .with(json!({"mismatches": mismatches})));
                }
            }
            let before_needs =
                self.observed.as_ref().and_then(|_| notifying_entries(replica.state(), at));
            let before = replica.changes();
            if let Some(named) = &self.named {
                *named.borrow_mut() = None;
            }
            let commits_before = crate::evidence::commits(&Model::read(replica.state())?);
            let data = apply(replica, By { actor: &self.actor, at })?;
            let changed = replica.changes() != before;
            // Renewal follows the batch's own writes, which count as activity; a failed batch returned above and
            // renews nothing.
            claim::renew_leases(replica, at, LEASE, MAX_LEASE)?;
            acyclic_core::probe::mark("batch: renew leases");
            if changed {
                *unchecked.borrow_mut() = crate::evidence::check(
                    self.env,
                    &self.learned,
                    &commits_before,
                    &Model::read(replica.state())?,
                    prepared,
                )?;
            }
            *result.borrow_mut() = (data, changed);
            if let (Some(observed), Some(before_needs)) = (&self.observed, before_needs)
                && let Some(after_needs) = notifying_entries(replica.state(), at)
            {
                *self.acknowledged.borrow_mut() =
                    acknowledged_entries(&before_needs, &after_needs, observed);
            }
            // The names of the state the batch commits, unless the command kept those of the model its writes left.
            // Naming never fails a write: a model that can't be read leaves the names to the tracker afterwards.
            if let Some(named) = &self.named
                && named.borrow().is_none()
            {
                *named.borrow_mut() = Model::read(replica.state()).ok().as_ref().map(names_of);
            }
            Ok(())
        })?;
        let (mut data, changed) = result.into_inner();
        if let Json::Object(map) = &mut data {
            map.insert("changed".to_owned(), Json::Bool(changed));
            let unchecked = unchecked.into_inner();
            if !unchecked.is_empty() {
                map.insert("unchecked_commits".to_owned(), Json::Array(unchecked));
            }
            // Asked to push every write: a failed push doesn't fail the write, which `acyclic sync` carries later.
            if changed
                && self.tracker.push_after_write()
                && let Some(remote) = self.tracker.remote()
            {
                let push = match self.tracker.push(remote) {
                    Ok(pushed) => json!({"ok": true, "pushed": pushed}),
                    Err(err) => json!({"ok": false, "reason": err.to_string()}),
                };
                map.insert("push".to_owned(), push);
            }
        }
        Ok(data)
    }

    /// A missing parent that names a thread, as when `--parent` was meant as `--thread`, says which command was meant.
    /// Only a failed command reads the model again, so the check costs a working one nothing.
    fn misplaced(
        &self,
        error: CliError,
        named: Option<&str>,
        meant: impl FnOnce() -> String,
    ) -> CliError {
        let Some(named) = named else { return error };
        if error.code != "not-found"
            || ![format!("no item {named}"), format!("no item or initiative {named}")]
                .contains(&error.message)
        {
            return error;
        }
        match self.read() {
            Ok((_, model)) if Records::of(&model).thread(named).is_ok() => {
                let meant = meant();
                CliError::new(
                    error.code,
                    error.exit,
                    format!("{}: {named} is a thread; did you mean `{meant}`?", error.message),
                )
                .with(json!({"meant": meant}))
            },
            _ => error,
        }
    }

    /// Reads the tracker and hands `read` a view of it.
    fn with_view<T>(
        &self,
        read: impl FnOnce(&View<'_>) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        let (state, model) = self.read()?;
        let readiness = ReadinessEvaluator::new(&state, &model, self.now);
        read(&View { state: &state, model: &model, readiness: &readiness, now: self.now })
    }

    /// The threads with ready work; with `initiative`, those with a task under it, so a thread shows under every
    /// initiative one of its tasks is in.
    fn ready(&self, initiative: Option<&str>) -> Result<Json, CliError> {
        self.with_view(|view| {
            let mut ready = view.readiness.ready(&self.actor)?;
            acyclic_core::probe::mark("readiness");
            if let Some(reference) = initiative {
                let wanted = Records::of(view.model).initiative(reference)?;
                ready.retain(|row| thread_in_initiative(view.model, &row.thread, &wanted));
            }
            Ok(json!({"ready": view.ready_rows(&ready)?}))
        })
    }

    /// Every initiative by id, with the items that are in none: the tops of the trees with no initiative above.
    fn initiatives(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let rows: Vec<Json> =
            model.initiatives.values().map(|record| view::initiative_row(&model, record)).collect();
        Ok(json!({"initiatives": rows, "no_initiative": view::no_initiative(&model)}))
    }

    fn show_initiatives(&self, ids: &[String]) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let records = Records::of(&model);
        let mut shown = Vec::with_capacity(ids.len());
        for reference in ids {
            let id = records.initiative(reference)?;
            shown.push(view::initiative(&model, &model.initiatives[&id]));
        }
        Ok(Json::Array(shown))
    }

    fn list(&self) -> Result<Json, CliError> {
        self.with_view(|view| {
            let items = view
                .model
                .items
                .keys()
                .map(|id| view.item_row(id))
                .collect::<Result<Vec<Json>, CliError>>()?;
            Ok(json!({"items": items}))
        })
    }

    fn show(&self, ids: &[String]) -> Result<Json, CliError> {
        self.with_view(|view| {
            let records = Records::of(view.model);
            let mut shown = Vec::with_capacity(ids.len());
            for reference in ids {
                let id = records.item(reference)?;
                shown.push(view.item(&id, &view.model.items[&id])?);
            }
            Ok(Json::Array(shown))
        })
    }

    /// Every thread's summary, with the threads it waits on, and the threads with work left in waves.
    fn threads(&self) -> Result<Json, CliError> {
        self.with_view(|view| {
            let waves = view.readiness.waves()?;
            let members = thread_members(view.model);
            let mut rows = view.thread_rows(&members)?;
            for row in &mut rows {
                let id = row["id"].as_str().unwrap_or_default().to_owned();
                row["waits_for"] = json!(waves.waits_for.get(&id).cloned().unwrap_or_default());
                row["progress"] =
                    view.thread_progress(members.get(&id).map_or(&[][..], Vec::as_slice))?;
            }
            Ok(json!({"threads": rows, "waves": waves.waves, "anytime": waves.anytime}))
        })
    }

    fn show_threads(&self, ids: &[String]) -> Result<Json, CliError> {
        self.with_view(|view| {
            let known = thread_members(view.model);
            let records = Records::of(view.model);
            let mut shown = Vec::with_capacity(ids.len());
            for reference in ids {
                let id = records.thread(reference)?;
                shown.push(view.thread(&id, &known[&id])?);
            }
            Ok(Json::Array(shown))
        })
    }

    fn repositories(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let records: Vec<Json> = model.repositories.values().map(view::repo_row).collect();
        Ok(json!({"repositories": records}))
    }

    fn show_repositories(&self, ids: &[String]) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let records = Records::of(&model);
        let mut shown = Vec::with_capacity(ids.len());
        for reference in ids {
            let id = records.repository(reference)?;
            shown.push(view::repo_row(&model.repositories[&id]));
        }
        Ok(Json::Array(shown))
    }

    fn needs(&self) -> Result<Json, CliError> {
        self.with_view(|view| {
            let mut entries = needs::needs_you(view.state, view.model, view.readiness, self.now)?;
            // Local to this clone, so derived here rather than kept in the shared state: asked to push every write,
            // it still holds writes the remote lacks.
            if let Some(remote) = self.tracker.remote()
                && self.tracker.push_after_write()
                && self.tracker.unpushed(remote)?
            {
                entries.push(needs::Entry::Unpushed { remote: remote.to_owned() });
            }
            Ok(json!({"needs": view.needs(&entries)?}))
        })
    }

    fn sync(&self) -> Result<Json, CliError> {
        let remote = self.remote()?;
        let synced = self.tracker.sync(remote)?;
        // A failed bounded local check must not turn a successful sync into a failure.
        let compact_suggested = self.tracker.compaction_due().ok();
        Ok(json!({
            "remote": remote,
            "fetched": synced.fetched,
            "joined": synced.joined,
            "pushed": synced.pushed,
            "changed": synced.joined,
            "compact_suggested": compact_suggested,
        }))
    }

    fn compact(&self) -> Result<Json, CliError> {
        let compacted = self.tracker.compact()?;
        Ok(json!({
            "changed": compacted.squashed.is_some(),
            "squashed": compacted.squashed,
        }))
    }

    /// The remote to sync with, or a configuration error naming how to add one.
    fn remote(&self) -> Result<&'static str, CliError> {
        self.tracker.remote().ok_or_else(|| {
            CliError::new(
                "no-remote",
                exit::CONFIG,
                format!(
                    "the tracker clone has no {}; set one with `acyclic init <clone> --replica <id> --remote <url>`",
                    store::REMOTE
                ),
            )
        })
    }

    fn doctor(&self) -> Result<Json, CliError> {
        // From Git, refusing non-canonical files, rather than from the cache.
        let (_, state) = self.tracker.load_strict()?;
        let (model, unreadable) = Model::read_lenient(&state);
        let unreadable: Vec<Json> = unreadable
            .into_iter()
            .map(|(key, err)| json!({"record": key.kind, "id": key.id, "error": err.to_string()}))
            .collect();
        Ok(json!({
            "replica": self.tracker.replica().as_str(),
            "counts": {
                "initiatives": model.initiatives.len(),
                "items": model.items.len(),
                "links": model.links.len(),
                "threads": model.threads.len(),
            },
            "unreadable": unreadable,
            // As each clone was last fetched: doctor reads, so it fetches nothing.
            "commits": crate::evidence::audit(self.env, &model),
        }))
    }

    /// The tracker as an import document, from one read of its state: nothing is written or synced.
    fn export(&self) -> Result<String, CliError> {
        let (_, state) = self.tracker.load()?;
        let model = Model::read(&state)?;
        let mut text = acyclic_core::export::export(&model)?.to_json();
        text.push('\n');
        Ok(text)
    }

    fn import(&self, file: &Path, dry_run: bool) -> Result<Json, CliError> {
        let path = self.env.cwd.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            CliError::new("not-found", exit::NOT_FOUND, format!("{}: {e}", file.display()))
        })?;
        let doc = Import::from_json(&text)?;
        let (_, state) = self.tracker.load()?;
        let mut probe = Replica::open(self.tracker.replica().clone(), state);
        let before_model = Model::read(probe.state())?;
        let before = crate::evidence::commits(&before_model);
        let at = probe.stamp(self.now, false)?;
        let by = By { actor: &self.actor, at };
        if dry_run {
            let report = import::preview(&doc, &probe, by)?;
            // Its commits are checked as a write would check them, on a copy that is never stored.
            import::apply(&doc, &mut probe, by)?;
            let after = Model::read(probe.state())?;
            let added = crate::evidence::commits(&after).difference(&before).cloned().collect();
            let prepared = crate::evidence::preflight(self.env, &self.learned, &added, &after);
            let unchecked =
                crate::evidence::check(self.env, &self.learned, &before, &after, &prepared)?;
            let mut shown = import_json(&report, true);
            if !unchecked.is_empty() {
                shown["unchecked_commits"] = Json::Array(unchecked);
            }
            return Ok(shown);
        }
        let prepared = if import::apply(&doc, &mut probe, by).is_ok() {
            let after = Model::read(probe.state())?;
            let added = crate::evidence::commits(&after).difference(&before).cloned().collect();
            crate::evidence::preflight(self.env, &self.learned, &added, &after)
        } else {
            crate::evidence::Prepared::default()
        };
        self.write_prepared(&prepared, |replica, by| {
            Ok(import_json(&import::apply(&doc, replica, by)?, false))
        })
    }

    fn prepare_item_done(
        &self,
        item: &str,
        commit: Option<(&str, &str)>,
    ) -> Result<crate::evidence::Prepared, CliError> {
        let Some((repository, sha)) = commit else {
            return Ok(crate::evidence::Prepared::default());
        };
        let (_, state) = self.tracker.load()?;
        let replica = Replica::open(self.tracker.replica().clone(), state);
        let model = Model::read(replica.state())?;
        let records = Records::of(&model);
        let (Ok(item), Ok(repository)) = (records.item(item), records.repository(repository))
        else {
            return Ok(crate::evidence::Prepared::default());
        };
        let candidate = (item, repository, sha.to_owned());
        if crate::evidence::commits(&model).contains(&candidate) {
            return Ok(crate::evidence::Prepared::default());
        }
        Ok(crate::evidence::preflight(
            self.env,
            &self.learned,
            &BTreeSet::from([candidate]),
            &model,
        ))
    }

    fn claim(&self, reference: &str, lease: Option<u32>, force: bool) -> Result<Json, CliError> {
        let lease = lease.map_or(LEASE, |minutes| Millis::from(minutes) * MINUTE);
        self.write(|replica, by| {
            let (id, key) = existing_thread(replica, reference)?;
            claim_thread(replica, by, &id, lease, !force)?;
            let held = claim::winner(replica.state(), &key, by.at)?;
            Ok(json!({"thread": id, "claim": held.as_ref().map(view::claim_json)}))
        })
    }
}

/// Claims the thread an item just added is in, as `item add --claim` does in the add's own write. An item in no
/// thread is its own thread of one, which the claim makes.
fn claim_thread_of(
    replica: &mut Replica,
    by: By<'_>,
    item: &str,
    after: &Model,
) -> Result<Json, CliError> {
    let Some(thread) = after.held_thread(item) else {
        return Err(CliError::refused(
            "no-thread",
            format!("item {item} is finished, so it has no thread to claim: leave out --claim"),
        )
        .with(json!({"item": item})));
    };
    claim_thread(replica, by, &thread, LEASE, true)?;
    let held = claim::winner(replica.state(), &claim::thread_key(&thread)?, by.at)?;
    Ok(json!({"thread": thread, "claim": held.as_ref().map(view::claim_json)}))
}

/// Claims `thread` for the actor, refusing as `acyclic thread claim` does.
fn claim_thread(
    replica: &mut Replica,
    by: By<'_>,
    thread: &str,
    lease: Millis,
    check_ready: bool,
) -> Result<(), CliError> {
    claim::claim(replica, thread, by.actor, by.at, (lease, MAX_LEASE), check_ready)
        .map(drop)
        .map_err(|error| match error {
            claim::ClaimError::NotReady(status) => CliError::refused(
                "not-ready",
                format!(
                    "thread {thread} isn't ready to claim: it has no ready work, or someone else holds it; \
                     `acyclic thread show {thread}` says why"
                ),
            )
            .with(json!({"readiness": status})),
            claim::ClaimError::State(error) => error.into(),
        })
}

/// The current notifying needs, keyed by the stable core key and carrying each exact compact identity.
fn notifying_entries(state: &State, now: Millis) -> Option<BTreeMap<String, String>> {
    let model = Model::read(state).ok()?;
    let readiness = ReadinessEvaluator::new(state, &model, now);
    let entries = needs::needs_you(state, &model, &readiness, now).ok()?;
    let mut current = BTreeMap::new();
    for entry in entries.into_iter().filter(needs::Entry::notifies) {
        let identity = match &entry {
            needs::Entry::Conflict { kind, id, field } => {
                let by = acyclic_core::model::By { actor: acyclic_core::scope::USER, at: now };
                let kept = needs::keeps(state, &model, (kind, id, field), by).ok()?;
                crate::keep::conflict_identity(&model, &entry, &kept)
            },
            _ => entry.identity(),
        };
        current.insert(entry.key(), identity);
    }
    Some(current)
}

/// Exact current identities this write may acknowledge: new stable keys and exact identities the client observed.
fn acknowledged_entries(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    observed: &BTreeSet<String>,
) -> Vec<String> {
    after
        .iter()
        .filter(|(key, identity)| !before.contains_key(*key) || observed.contains(*identity))
        .map(|(_, identity)| identity.clone())
        .collect()
}

#[cfg(test)]
mod acknowledged_entries_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use acyclic_core::model::{By, add_item};
    use acyclic_core::needs::Entry;
    use acyclic_core::schema::{item, kind, repository};
    use acyclic_core::scope::USER;
    use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
    use serde_json::{Value, json};

    use super::{Model, ReadinessEvaluator, acknowledged_entries, needs, notifying_entries};

    const NOW: Millis = 10_000;
    const ITEM_CONFLICT: &str = "conflict:item:x:title";
    const REPOSITORY_CONFLICT: &str = "conflict:repository:r:owner";

    fn title_conflict() -> (Replica, Replica) {
        let mut first = Replica::open(ReplicaId::new("first").unwrap(), State::new());
        add_item(&mut first, "x", "Base", By { actor: USER, at: 1 }).unwrap();
        let mut second = Replica::open(ReplicaId::new("second").unwrap(), State::new());
        second.join(first.state()).unwrap();
        let mut third = Replica::open(ReplicaId::new("third").unwrap(), State::new());
        third.join(first.state()).unwrap();

        first
            .write(&Key::new(kind::ITEM, "x").unwrap(), item::TITLE, USER, 2, Some(json!("one")))
            .unwrap();
        second
            .write(&Key::new(kind::ITEM, "x").unwrap(), item::TITLE, USER, 2, Some(json!("two")))
            .unwrap();
        first.join(second.state()).unwrap();
        (first, third)
    }

    fn repository_conflict() -> Replica {
        let mut first = Replica::open(ReplicaId::new("first").unwrap(), State::new());
        let key = Key::new(kind::REPOSITORY, "r").unwrap();
        for (field, value) in [
            (repository::OWNER, json!("base")),
            (repository::NAME, json!("base")),
            (repository::DEFAULT_BRANCH, json!("main")),
        ] {
            first.write(&key, field, USER, 1, Some(value)).unwrap();
        }
        let mut second = Replica::open(ReplicaId::new("second").unwrap(), State::new());
        second.join(first.state()).unwrap();

        first.write(&key, repository::OWNER, USER, 2, Some(json!("one"))).unwrap();
        first.write(&key, repository::NAME, USER, 2, Some(json!("app-one"))).unwrap();
        second.write(&key, repository::OWNER, USER, 2, Some(json!("two"))).unwrap();
        second.write(&key, repository::NAME, USER, 2, Some(json!("app-two"))).unwrap();
        first.join(second.state()).unwrap();
        first
    }

    fn identity(map: &BTreeMap<String, String>, key: &str) -> String {
        map.get(key).unwrap_or_else(|| panic!("missing {key}: {map:?}")).clone()
    }

    fn choice_values(identity: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(identity).unwrap()["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|choice| choice["value"].clone())
            .collect()
    }

    fn view_identity(state: &State, key: &str) -> String {
        let model = Model::read(state).unwrap();
        let readiness = ReadinessEvaluator::new(state, &model, NOW);
        let entries = needs::needs_you(state, &model, &readiness, NOW).unwrap();
        let view = crate::view::View { state, model: &model, readiness: &readiness, now: NOW };
        view.needs(&entries)
            .unwrap()
            .into_iter()
            .find(|entry| entry["key"] == key)
            .unwrap()["identity"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn a_new_need_with_a_large_core_identity_is_still_acknowledged() {
        let entry = Entry::Contest {
            thread: "thread".into(),
            holders: (0..1_000).map(|n| format!("session/holder-{n:04}")).collect(),
        };
        let identity = entry.identity();
        assert!(identity.len() > 16 * 1024);
        let before = BTreeMap::new();
        let after = BTreeMap::from([(entry.key(), identity.clone())]);

        assert_eq!(acknowledged_entries(&before, &after, &BTreeSet::new()), [identity]);
    }

    #[test]
    fn a_third_conflict_value_changes_the_shared_view_and_ack_identity() {
        let (mut replica, mut third) = title_conflict();
        assert_eq!(Model::read(replica.state()).unwrap().conflicts().count(), 1);
        let before = notifying_entries(replica.state(), NOW).unwrap();
        let old = identity(&before, ITEM_CONFLICT);
        assert_eq!(choice_values(&old), [json!("one"), json!("two")]);
        assert_eq!(view_identity(replica.state(), ITEM_CONFLICT), old);

        third
            .write(&Key::new(kind::ITEM, "x").unwrap(), item::TITLE, USER, 3, Some(json!("three")))
            .unwrap();
        replica.join(third.state()).unwrap();

        let after = notifying_entries(replica.state(), NOW).unwrap();
        let new = identity(&after, ITEM_CONFLICT);
        assert_eq!(choice_values(&new), [json!("one"), json!("two"), json!("three")]);
        assert_ne!(old, new);
        assert_eq!(view_identity(replica.state(), ITEM_CONFLICT), new);
        assert!(
            acknowledged_entries(&before, &after, &BTreeSet::from([old])).is_empty(),
            "an old observation must not acknowledge the changed conflict"
        );
    }

    #[test]
    fn unchanged_conflict_values_get_a_new_identity_when_resolution_becomes_available() {
        let mut replica = repository_conflict();
        let before = notifying_entries(replica.state(), NOW).unwrap();
        let old = identity(&before, REPOSITORY_CONFLICT);
        let old_payload: Value = serde_json::from_str(&old).unwrap();
        let old_values = choice_values(&old);
        assert!(
            old_payload["choices"]
                .as_array()
                .unwrap()
                .iter()
                .all(|choice| choice["command"].is_null())
        );

        replica
            .write(
                &Key::new(kind::REPOSITORY, "r").unwrap(),
                repository::NAME,
                USER,
                3,
                Some(json!("app-one")),
            )
            .unwrap();
        let after = notifying_entries(replica.state(), NOW).unwrap();
        let new = identity(&after, REPOSITORY_CONFLICT);
        let new_payload: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(choice_values(&new), old_values, "only resolution eligibility changed");
        assert_ne!(old, new);
        assert!(
            new_payload["choices"]
                .as_array()
                .unwrap()
                .iter()
                .all(|choice| choice["command"].is_object())
        );
        assert_eq!(view_identity(replica.state(), REPOSITORY_CONFLICT), new);
    }
}

#[cfg(test)]
mod partition_tests {
    use acyclic_core::model::{By, add_initiative, add_item, set_parent};
    use acyclic_core::scope::USER;
    use acyclic_core::state::{Key, Replica, ReplicaId, State};

    use super::initiative_of;

    #[test]
    fn audit_events_go_to_the_initiative_the_chain_of_parents_ends_in() {
        let by = By { actor: USER, at: 1 };
        let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        add_initiative(&mut r, "init", "Init", by).unwrap();
        for id in ["epic", "task", "bare", "loop"] {
            add_item(&mut r, id, id, by).unwrap();
        }
        set_parent(&mut r, "epic", Some("init"), by).unwrap();
        set_parent(&mut r, "task", Some("epic"), by).unwrap();
        // A parent that names nothing, written as a sync could leave it.
        r.write(&Key::new("item", "loop").unwrap(), "parent", USER, 2, Some("gone".into()))
            .unwrap();
        let of = |kind: &str, id: &str| initiative_of(r.state(), &Key::new(kind, id).unwrap());
        assert_eq!(of("initiative", "init").as_deref(), Some("init"), "an initiative's own events");
        assert_eq!(of("item", "epic").as_deref(), Some("init"));
        assert_eq!(of("item", "task").as_deref(), Some("init"), "however deep");
        assert_eq!(of("item", "bare"), None, "an item in no initiative shares one file");
        assert_eq!(of("item", "loop"), None, "and one whose parent names nothing");
        assert_eq!(of("thread", "t"), None);
    }
}

/// The record a delete names: the one the resolver `found`, or, named by its full id, one deleted already, which
/// deleting again changes nothing.
fn deletable(
    found: Result<String, CliError>,
    reference: &str,
    state: &State,
    of: &str,
) -> Result<String, CliError> {
    found.or_else(|error| match ids::v7(reference) {
        Some(id) if Key::new(of, &id).is_ok_and(|key| model::is_deleted(state, &key)) => Ok(id),
        _ => Err(error),
    })
}

/// Splits `--commit <REPOSITORY>@<SHA>` at its last `@`, refusing a commit with no repository.
fn split_commit(commit: &str) -> Result<(&str, &str), CliError> {
    match commit.rsplit_once('@') {
        Some((repository, sha)) if !repository.is_empty() => Ok((repository, sha)),
        _ => Err(CliError::new(
            "invalid-commit",
            exit::USAGE,
            format!(
                "--commit names the repository and the commit, as --commit <OWNER/NAME>@{commit}: a SHA alone \
                 doesn't say which repository holds it"
            ),
        )),
    }
}

/// The agent block, with this build's version.
pub(crate) fn prime() -> String {
    include_str!("prime.md").replace("{version}", acyclic_core::VERSION)
}

/// A completion script for `shell`. It completes from the plain command line, because the sectioned help hides the
/// subcommands from clap's own list, and a generator leaves hidden ones out.
pub(crate) fn completions(shell: clap_complete::Shell) -> String {
    use clap::CommandFactory as _;
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut crate::args::Cli::command(), "acyclic", &mut script);
    String::from_utf8(script).expect("a completion script is UTF-8")
}

/// An item as a thread lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Member<'a> {
    pub item: &'a str,
    /// Its thread is conflicted, so it is listed under each thread it was put in.
    pub conflict: bool,
}

/// Every thread by id with its members in the order its blocks links run them: ties by id, and members on a cycle
/// last. An item raced into two threads is in both; a thread with no record has no row, as the item's reference
/// dangles. A loose task is the one member of its thread of one.
pub(crate) fn thread_members(model: &Model) -> BTreeMap<String, Vec<Member<'_>>> {
    let mut threads: BTreeMap<String, Vec<Member<'_>>> =
        model.all_threads().into_iter().map(|id| (id, Vec::new())).collect();
    for item in model.items.values() {
        let named = model.thread_value(item).all();
        for thread in &named {
            if let Some(members) = threads.get_mut(thread.as_str()) {
                members.push(Member {
                    item: &item.id,
                    conflict: model.thread_value(item).is_conflict(),
                });
            }
        }
        // In none, or in a thread with no record: its own thread of one.
        if model.is_loose(&item.id)
            && let Some(members) = threads.get_mut(&acyclic_core::ids::one(&item.id))
            && members.iter().all(|member| member.item != item.id)
        {
            members.push(Member { item: &item.id, conflict: false });
        }
    }
    // Each thread's blocks links among its own members, gathered in one pass rather than once per thread.
    let mut links: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let threads_of = |id: &str| {
        model.items.get(id).map(|item| model.thread_value(item).all()).unwrap_or_default()
    };
    for link in &model.links {
        let to = threads_of(&link.to);
        for thread in threads_of(&link.from).into_iter().filter(|thread| to.contains(thread)) {
            links.entry(thread.as_str()).or_default().push((&link.from, &link.to));
        }
    }
    for (thread, members) in &mut threads {
        let among = links.get(thread.as_str()).map_or(&[][..], Vec::as_slice);
        let order = run_order(members.iter().map(|m| m.item).collect(), among);
        members.sort_by_key(|m| order[m.item]);
    }
    threads
}

/// Each item's place when `items` run along `links`, the blocks links among them as `(from, to)`.
fn run_order<'a>(
    items: BTreeSet<&'a str>,
    links: &[(&'a str, &'a str)],
) -> BTreeMap<&'a str, usize> {
    let mut waits: BTreeMap<&str, usize> = items.iter().map(|id| (*id, 0)).collect();
    let mut unblocks: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (from, to) in links {
        if let (Some(&from), Some(&to)) = (items.get(from), items.get(to)) {
            *waits.get_mut(from).expect("a member") += 1;
            unblocks.entry(to).or_default().push(from);
        }
    }
    let mut free: BTreeSet<&str> =
        waits.iter().filter(|(_, n)| **n == 0).map(|(id, _)| *id).collect();
    let mut order = BTreeMap::new();
    while let Some(id) = free.pop_first() {
        order.insert(id, order.len());
        for next in unblocks.get(id).into_iter().flatten() {
            let left = waits.get_mut(next).expect("a member");
            *left -= 1;
            if *left == 0 {
                free.insert(next);
            }
        }
    }
    // What a cycle holds never frees.
    for id in items {
        let at = order.len();
        order.entry(id).or_insert(at);
    }
    order
}

/// `word` as a shell reads it back: as it is when it holds only characters no shell treats specially, else in single
/// quotes, so a suggested command that repeats a title can be pasted as it is.
pub(crate) fn shell_word(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./@:,+=".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The thread a reference names, and its key, when the thread exists, which a loose task's thread of one does before it
/// has a record; naming an item in a thread says which thread was meant.
pub(crate) fn existing_thread(
    replica: &Replica,
    reference: &str,
) -> Result<(String, Key), CliError> {
    let model = Model::read(replica.state())?;
    let records = Records::of(&model);
    match records.thread(reference) {
        Ok(id) if model.thread_exists(&id) => {
            let key = claim::thread_key(&id)?;
            Ok((id, key))
        },
        Ok(id) => Err(acyclic_core::state::Error::NoThread(id).into()),
        Err(error) if error.code == "not-found" => {
            let meant = records
                .item(reference)
                .ok()
                .and_then(|item| model.thread_of(&item).map(str::to_owned));
            Err(match meant {
                Some(meant) => error.with(json!({"thread": meant})),
                None => error,
            })
        },
        Err(error) => Err(error),
    }
}

/// Whether a thread has a task under `initiative`: any member of it, or the task a thread of one is derived from.
fn thread_in_initiative(model: &Model, thread: &str, initiative: &str) -> bool {
    model
        .members(thread)
        .into_iter()
        .chain(model.task_of(thread))
        .any(|task| model.initiatives_of(task).contains(&initiative))
}

/// The initiative an entity's audit events go to: an initiative's own id, and an item's initiative, the top of its chain
/// of parents when that is an initiative record. None for anything else, and for an item in no initiative, whose events
/// share one file.
fn initiative_of(state: &State, key: &Key) -> Option<String> {
    match key.kind.as_str() {
        kind::INITIATIVE => return Some(key.id.clone()),
        kind::ITEM => {},
        _ => return None,
    }
    let mut seen = std::collections::BTreeSet::from([key.id.clone()]);
    let mut current = key.clone();
    loop {
        let parent = state
            .field(&current, field::PARENT)
            .and_then(|register| register.values().next())
            .and_then(|value| value.value.as_str().map(str::to_owned))?;
        if Key::new(kind::INITIATIVE, &parent).is_ok_and(|held| state.entity(&held).is_some()) {
            return Some(parent);
        }
        if !seen.insert(parent.clone()) {
            return None;
        }
        current = Key::new(kind::ITEM, &parent).ok()?;
    }
}

/// Creates the tracker repository, or configures an existing clone, as a new replica. Adds no files or hooks.
fn init(path: &Path, replica: &str, remote: Option<&str>) -> Result<Json, CliError> {
    let id = ReplicaId::new(replica)?;
    if nested_in_repository(path)? {
        return Err(CliError::refused(
            "nested-repository",
            "the tracker path is inside another Git repository",
        ));
    }
    let is_repo = store::setup::is_repository(path);
    let configured = if is_repo {
        store::setup::config_value(path, store::REPLICA_CONFIG).ok().flatten()
    } else {
        None
    };
    if let Some(existing) = &configured
        && existing != replica
    {
        return Err(CliError::refused(
            "already-exists",
            format!("this clone is already replica {existing}"),
        ));
    }
    let origin =
        if is_repo { store::setup::remote_url(path, store::REMOTE).ok().flatten() } else { None };
    let effective_remote = remote.or(origin.as_deref());
    let is_same_clone =
        configured.as_deref() == Some(replica) && effective_remote == origin.as_deref();
    if !is_same_clone
        && let Some(remote) = effective_remote
        && remote_has_replica(path, remote, &id)?
    {
        return Err(CliError::refused(
            "duplicate-replica",
            format!(
                "replica id {replica} already exists at the remote; choose a different --replica id"
            ),
        )
        .with(json!({"replica": replica})));
    }
    if is_repo {
        match configured {
            Some(existing) if existing == replica => {
                let changed = remote
                    .map(|url| store::setup::set_remote_url(path, store::REMOTE, url))
                    .transpose()?
                    .unwrap_or(false);
                return Ok(
                    json!({"path": path, "replica": replica, "created": false, "changed": changed}),
                );
            },
            Some(existing) => {
                return Err(CliError::refused(
                    "already-exists",
                    format!("this clone is already replica {existing}"),
                ));
            },
            None => {},
        }
    } else {
        std::fs::create_dir_all(path).map_err(|e| {
            CliError::new("config", exit::CONFIG, format!("can't create {}: {e}", path.display()))
        })?;
        store::setup::init_repository(path)?;
    }
    store::init_replica(path, &id)?;
    if let Some(url) = remote {
        store::setup::set_remote_url(path, store::REMOTE, url)?;
    }
    Ok(json!({"path": path, "replica": replica, "created": !is_repo, "changed": true}))
}

/// Whether the remote already has this replica's state ref, checked before `init` changes the destination.
fn remote_has_replica(path: &Path, remote: &str, replica: &ReplicaId) -> Result<bool, CliError> {
    let remote_path = Path::new(remote);
    let intended = canonical_init_path(path)?;
    let scp =
        remote.split_once(':').is_some_and(|(host, _)| !host.is_empty() && !host.contains('/'));
    let remote =
        if remote_path.is_absolute() || remote.contains("://") || remote.contains("::") || scp {
            remote.to_owned()
        } else {
            normalized_path(&intended.join(remote_path)).to_string_lossy().into_owned()
        };
    let reference = store::state_ref(replica);
    let root = intended
        .ancestors()
        .find(|ancestor| ancestor.is_dir())
        .ok_or_else(|| CliError::new("config", exit::CONFIG, "can't resolve tracker path"))?;
    let root = std::fs::canonicalize(root)
        .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?;
    store::setup::remote_has_ref(&root, &remote, &reference).map_err(|e| {
        CliError::new("git", exit::STORE, format!("can't check remote state refs: {e}"))
    })
}

/// The clone path's absolute location, resolving existing symlinks while allowing a not-yet-created suffix.
fn canonical_init_path(path: &Path) -> Result<PathBuf, CliError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?
            .join(path)
    };
    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| CliError::new("config", exit::CONFIG, "can't resolve tracker path"))?;
    }
    let suffix = absolute
        .strip_prefix(existing)
        .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?;
    let root = std::fs::canonicalize(existing)
        .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?
        .join(suffix);
    Ok(normalized_path(&root))
}

/// Resolves `.` and `..` lexically after existing symlinks have been resolved.
fn normalized_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                normalized.pop();
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            },
        }
    }
    normalized
}

/// Whether `path` is strictly inside an existing Git worktree or bare repository.
fn nested_in_repository(path: &Path) -> Result<bool, CliError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?
            .join(path)
    };
    let mut existing = absolute.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| CliError::new("config", exit::CONFIG, "can't resolve tracker path"))?;
    }
    let destination = std::fs::canonicalize(existing)
        .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?
        .join(
            absolute
                .strip_prefix(existing)
                .map_err(|e| CliError::new("config", exit::CONFIG, e.to_string()))?,
        );
    if let Some(root) = store::setup::enclosing_repository(existing)
        && let Ok(root) = std::fs::canonicalize(root)
    {
        return Ok(destination.starts_with(&root) && destination != root);
    }
    Ok(false)
}

/// An import's result: what it changed, in counts and field by field, and the threads items joined, as
/// `{item, thread}`. A dry run changes nothing.
pub(crate) fn import_json(report: &import::Report, dry_run: bool) -> Json {
    let mut data = json!({
        "dry_run": dry_run,
        "counts": {
            "initiatives": report.initiatives,
            "repositories": report.repositories,
            "items": report.items,
            "links": report.links,
            "threads": report.threads,
            "fields": report.fields,
        },
        "changes": report.changes,
        "placed": report.placed,
    });
    if dry_run {
        data["changed"] = json!(false);
    }
    data
}
