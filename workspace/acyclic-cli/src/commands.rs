//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use acyclic_core::archive;
use acyclic_core::claim;
use acyclic_core::delete;
use acyclic_core::ids;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, Model};
use acyclic_core::needs;
use acyclic_core::ready::{Ancestry, GitAncestry, Readiness};
use acyclic_core::schema::{item as field, kind};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use acyclic_core::write::{self, ChangeSet};
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{
    Class, Cli, Command, ItemCommand, LEASE_MAX_MINUTES, RepositoryCommand, StateWord,
    ThreadCommand,
};
use crate::context;
use crate::edit::{self, Adding, Change, Delta, ItemFields, RepositoryFields, ThreadFields};
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
    run_with(cli, env, false).map(|(output, _)| output)
}

/// Runs a parsed command, and says what pretty output calls each record the tracker holds once it is done: items and
/// threads by the shortest ending of their ids that names them, and repositories by `owner/name`. They come from the
/// state the command last read or wrote, so a record it added is named too, and from the tracker as it is after the
/// command when it neither read nor wrote through the model; none for a command with no tracker, or one that can't
/// be read.
pub fn run_named(cli: &Cli, env: &Env) -> Result<(Output, Names), CliError> {
    run_with(cli, env, true)
}

fn run_with(cli: &Cli, env: &Env, named: bool) -> Result<(Output, Names), CliError> {
    let text = |text: String| (Output::Text(text), Names::new());
    match (cli.command.class(), &cli.command) {
        (Class::Standalone, Command::Init { path, replica, remote }) => {
            init(&env.cwd.join(path), replica, remote.as_deref())
                .map(|data| (Output::Json(data), Names::new()))
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
            let result = Session::open(cli, env, named).and_then(|session| {
                let data = session.dispatch(command)?;
                Ok((Output::Json(data), if named { session.names() } else { Names::new() }))
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

struct Session<'a> {
    tracker: Tracker,
    actor: String,
    now: Millis,
    env: &'a Env,
    /// The names of the records in the state the command last read or wrote through [`Session::read`] or
    /// [`Session::write`], kept only when pretty output asked for them.
    named: Option<RefCell<Option<Names>>>,
}

impl<'a> Session<'a> {
    fn open(cli: &Cli, env: &'a Env, named: bool) -> Result<Self, CliError> {
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
        })
    }

    #[expect(clippy::too_many_lines, reason = "one short arm per command")]
    fn dispatch(&self, command: &Command) -> Result<Json, CliError> {
        match command {
            Command::Init { .. }
            | Command::Prime
            | Command::Id { .. }
            | Command::Import { file: None, .. }
            | Command::Serve(_)
            | Command::Up(_)
            | Command::Completions { .. } => {
                unreachable!("handled before dispatch")
            },
            Command::Thread(ThreadCommand::Ready { include_archived }) => {
                self.ready(*include_archived)
            },
            Command::Item(ItemCommand::List) => self.list(),
            Command::Item(ItemCommand::Show { items }) => self.show(items),
            Command::Needs => self.needs(),
            Command::Sync => self.sync(),
            Command::Doctor => self.doctor(),
            Command::Item(ItemCommand::Add {
                id: given,
                title,
                parent,
                thread,
                state,
                notes,
                size,
                repository,
                waits_for,
            }) => {
                let fields = ItemFields {
                    title: Some(title.clone()),
                    parent: Change::of(parent.clone(), false),
                    thread: Change::of(thread.clone(), false),
                    state: state.map(StateWord::state),
                    notes: Change::of(notes.clone(), false),
                    size: Change::of(*size, false),
                    repositories: Delta { add: repository.clone(), remove: Vec::new() },
                    waits_for: Delta { add: waits_for.clone(), remove: Vec::new() },
                };
                self.write(|replica, by| {
                    let (id, adding) = match given {
                        Some(given) => (edit::given_id(given)?, Adding::Given),
                        None => (acyclic_core::ids::new(), Adding::Minted),
                    };
                    let outcome = edit::commit(replica, by, None, None, |model| {
                        let fields = fields.clone().resolve(model, &Records::of(model), None)?;
                        Ok(edit::item_changes(&id, &fields, adding))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"item": id, "placed": outcome.placed(), "title": title}))
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
                    parent: Change::of(edit.parent.clone(), edit.no_parent),
                    thread: Change::of(edit.thread.clone(), edit.no_thread),
                    state: edit.state.map(StateWord::state),
                    notes: Change::of(edit.notes.clone(), edit.no_notes),
                    size: Change::of(edit.size, edit.no_size),
                    repositories: Delta {
                        add: edit.repository.clone(),
                        remove: edit.no_repository.clone(),
                    },
                    waits_for: Delta {
                        add: edit.waits_for.clone(),
                        remove: edit.no_waits_for.clone(),
                    },
                };
                let item = &edit.item;
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, None, |model| {
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
                let ancestry = GitAncestry::new(context::clones(self.env)?);
                let commit = commit.as_deref().map(split_commit).transpose()?;
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, Some(&ancestry), |model| {
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
                    Ok(json!({"item": id, "state": ItemState::Done, "evidence": evidence}))
                })
            },
            Command::Thread(ThreadCommand::Release { thread }) => self.write(|replica, by| {
                let (id, key) = existing_thread(replica, thread)?;
                Ok(json!({"thread": id, "released": claim::release(replica, &key, by.actor)}))
            }),
            Command::Item(ItemCommand::Reopen { item }) => self.write(|replica, by| {
                let id = Records::of(&Model::read(replica.state())?).item(item)?;
                archive::reopen(replica, &id, by)?;
                Ok(json!({"item": id}))
            }),
            Command::Thread(ThreadCommand::Claim { thread, lease, force }) => {
                self.claim(thread, *lease, *force)
            },
            Command::Thread(ThreadCommand::List) => self.threads(),
            Command::Thread(ThreadCommand::Show { threads }) => self.show_threads(threads),
            Command::Thread(ThreadCommand::Add { id: given, title, brief, repository }) => {
                let fields = ThreadFields {
                    title: Some(title.clone()),
                    brief: Change::of(brief.clone(), false),
                    repositories: Delta { add: repository.clone(), remove: Vec::new() },
                };
                self.write(|replica, by| {
                    let (id, adding) = match given {
                        Some(given) => (edit::given_id(given)?, Adding::Given),
                        None => (acyclic_core::ids::new(), Adding::Minted),
                    };
                    let outcome = edit::commit(replica, by, None, None, |model| {
                        let fields = fields.clone().resolve(model, &Records::of(model), None)?;
                        Ok(edit::thread_changes(&id, &fields, adding))
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"thread": id, "title": title}))
                })
            },
            Command::Thread(ThreadCommand::Edit(edit)) => {
                let fields = ThreadFields {
                    title: edit.title.clone(),
                    brief: Change::of(edit.brief.clone(), edit.no_brief),
                    repositories: Delta {
                        add: edit.repository.clone(),
                        remove: edit.no_repository.clone(),
                    },
                };
                let thread = &edit.thread;
                self.write(|replica, by| {
                    let mut id = String::new();
                    let outcome = edit::commit(replica, by, None, None, |model| {
                        let records = Records::of(model);
                        id = records.thread(thread)?;
                        let fields = fields.clone().resolve(model, &records, Some(&id))?;
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
                    let outcome = edit::commit(replica, by, Some(name), None, |_| {
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
                    let outcome = edit::commit(replica, by, name.as_deref(), None, |model| {
                        id = Records::of(model).repository(repository)?;
                        edit::repository_changes(&id, &fields, Adding::No)
                    })?;
                    self.remember(&outcome.after);
                    Ok(json!({"repository": id}))
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
            Command::Item(ItemCommand::Archive { item }) => {
                let clones = context::clones(self.env)?;
                let ancestry = GitAncestry::new(clones);
                self.write(|replica, by| {
                    let model = Model::read(replica.state())?;
                    let id = Records::of(&model).item(item)?;
                    if !model.items[&id].is_archived_root() {
                        archive::archive(replica, &ancestry, &id, by)?;
                    }
                    Ok(json!({"item": id}))
                })
            },
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

    /// The names [`Session::remember`] kept; else, for a command that neither read nor wrote through the model, such
    /// as `sync` or `doctor`, those of the tracker as it is now, read through the tracker this session opened.
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

    /// Runs one batch as the actor. Lease renewal rides along, so a batch can commit though the command changed
    /// nothing; `changed` says whether the command itself changed anything.
    fn write(
        &self,
        apply: impl FnOnce(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        let result = RefCell::new((Json::Null, false));
        let meta = Meta { actor: self.actor.clone() };
        self.tracker.batch(&meta, self.now, false, &project_root, |replica, at| {
            claim::renew_leases(replica, at, LEASE, MAX_LEASE)?;
            acyclic_core::probe::mark("batch: renew leases");
            let before = replica.changes();
            if let Some(named) = &self.named {
                *named.borrow_mut() = None;
            }
            let data = apply(replica, By { actor: &self.actor, at })?;
            *result.borrow_mut() = (data, replica.changes() != before);
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
        if error.code != "not-found" || error.message != format!("no item {named}") {
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

    /// Reads the tracker and hands `read` a view of it: its items' readiness is judged against the clones' history.
    fn with_view<T>(
        &self,
        read: impl FnOnce(&View<'_>) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        let (state, model) = self.read()?;
        let ancestry = GitAncestry::new(context::clones(self.env)?);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        read(&View { state: &state, model: &model, readiness: &readiness, now: self.now })
    }

    fn ready(&self, include_archived: bool) -> Result<Json, CliError> {
        self.with_view(|view| {
            let ready = view.readiness.ready(include_archived, &self.actor)?;
            acyclic_core::probe::mark("readiness");
            Ok(json!({"ready": view.ready_rows(&ready)?}))
        })
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
            let mut rows = view::thread_rows(view.model);
            let members = thread_members(view.model);
            for row in &mut rows {
                let id = row["id"].as_str().unwrap_or_default().to_owned();
                row["waits_for"] = json!(waves.waits_for.get(&id).cloned().unwrap_or_default());
                row["progress"] =
                    view.thread_progress(members.get(id.as_str()).map_or(&[][..], Vec::as_slice))?;
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
                shown.push(view.thread(&id, &known[id.as_str()])?);
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
            shown.push(view::repo_show(&model, &model.repositories[&id]));
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
        Ok(json!({
            "remote": remote,
            "fetched": synced.fetched,
            "joined": synced.joined,
            "pushed": synced.pushed,
            "changed": synced.joined,
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
            .map(|(key, err)| json!({"entity": key.kind, "id": key.id, "error": err.to_string()}))
            .collect();
        Ok(json!({
            "replica": self.tracker.replica().as_str(),
            "counts": {
                "items": model.items.len(),
                "links": model.links.len(),
                "threads": model.threads.len(),
            },
            "unreadable": unreadable,
        }))
    }

    fn import(&self, file: &Path, dry_run: bool) -> Result<Json, CliError> {
        let path = self.env.cwd.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            CliError::new("not-found", exit::NOT_FOUND, format!("{}: {e}", file.display()))
        })?;
        let doc = Import::from_json(&text)?;
        let ancestry = GitAncestry::new(context::clones(self.env)?);
        if dry_run {
            let (_, state) = self.tracker.load()?;
            let replica = Replica::open(self.tracker.replica().clone(), state);
            let at = replica.stamp(self.now, false)?;
            let report = import::preview(&doc, &replica, By { actor: &self.actor, at }, &ancestry)?;
            return Ok(import_json(&report, true));
        }
        self.write(|replica, by| {
            Ok(import_json(&import::apply(&doc, replica, by, &ancestry)?, false))
        })
    }

    fn claim(&self, reference: &str, lease: Option<u32>, force: bool) -> Result<Json, CliError> {
        let lease = lease.map_or(LEASE, |minutes| Millis::from(minutes) * MINUTE);
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        self.write(|replica, by| {
            let (id, key) = existing_thread(replica, reference)?;
            let thread = id.as_str();
            let ready = (!force).then_some(&ancestry as &dyn Ancestry);
            claim::claim(replica, thread, by.actor, by.at, (lease, MAX_LEASE), ready).map_err(
                |error| match error {
                    claim::ClaimError::NotReady(status) => CliError::refused(
                        "not-ready",
                        format!(
                            "thread {thread} isn't ready to claim: it has no ready work, or someone else holds it; \
                             `acyclic thread show {thread}` says why"
                        ),
                    )
                    .with(json!({"readiness": status})),
                    claim::ClaimError::State(error) => error.into(),
                },
            )?;
            let held = claim::winner(replica.state(), &key, by.at)?;
            Ok(json!({"thread": thread, "claim": held.as_ref().map(view::claim_json)}))
        })
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
/// dangles.
pub(crate) fn thread_members(model: &Model) -> BTreeMap<&str, Vec<Member<'_>>> {
    let mut threads: BTreeMap<&str, Vec<Member<'_>>> =
        model.threads.keys().map(|id| (id.as_str(), Vec::new())).collect();
    for item in model.items.values() {
        let named = item.thread.all();
        for thread in &named {
            if let Some(members) = threads.get_mut(thread.as_str()) {
                members.push(Member { item: &item.id, conflict: item.thread.is_conflict() });
            }
        }
    }
    // Each thread's blocks links among its own members, gathered in one pass rather than once per thread.
    let mut links: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let threads_of =
        |id: &str| model.items.get(id).map(|item| item.thread.all()).unwrap_or_default();
    for link in &model.links {
        let to = threads_of(&link.to);
        for thread in threads_of(&link.from).into_iter().filter(|thread| to.contains(thread)) {
            links.entry(thread.as_str()).or_default().push((&link.from, &link.to));
        }
    }
    for (thread, members) in &mut threads {
        let among = links.get(thread).map_or(&[][..], Vec::as_slice);
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

/// The thread a reference names, and its key, when the thread has a record; naming an item in a thread says which
/// thread was meant.
pub(crate) fn existing_thread(
    replica: &Replica,
    reference: &str,
) -> Result<(String, Key), CliError> {
    let model = Model::read(replica.state())?;
    let records = Records::of(&model);
    match records.thread(reference) {
        Ok(id) if model.threads.contains_key(&id) => {
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

/// The project root an entity's audit events go to: an item's topmost ancestor, or none for a top-level item and
/// anything that isn't an item.
fn project_root(state: &State, key: &Key) -> Option<String> {
    if key.kind != kind::ITEM {
        return None;
    }
    let mut seen = std::collections::BTreeSet::from([key.id.clone()]);
    let mut current = key.clone();
    let mut top = None;
    loop {
        let parent = state
            .field(&current, field::PARENT)
            .and_then(|register| register.values().next())
            .and_then(|value| value.value.as_str().map(str::to_owned));
        match parent {
            Some(parent) if seen.insert(parent.clone()) => {
                current = Key::new(kind::ITEM, &parent).ok()?;
                top = Some(parent);
            },
            _ => return top,
        }
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
    let git = |args: &[&str]| store::git_at(path).args(args).env("LC_ALL", "C").output();
    let is_repo =
        path.exists() && git(&["rev-parse", "--git-dir"]).is_ok_and(|out| out.status.success());
    if is_repo {
        let configured = git(&["config", "--get", store::REPLICA_CONFIG])
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
        match configured {
            Some(existing) if existing == replica => {
                let changed = remote.map(|url| set_origin(&git, url)).transpose()?.unwrap_or(false);
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
        let out =
            git(&["init", "-q"]).map_err(|e| CliError::new("git", exit::STORE, e.to_string()))?;
        if !out.status.success() {
            return Err(CliError::new(
                "git",
                exit::STORE,
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ));
        }
    }
    store::init_replica(path, &id)?;
    if let Some(url) = remote {
        set_origin(&git, url)?;
    }
    Ok(json!({"path": path, "replica": replica, "created": !is_repo, "changed": true}))
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
    for args in [["rev-parse", "--show-toplevel"], ["rev-parse", "--absolute-git-dir"]] {
        let result = store::git_at(existing)
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .map_err(|e| CliError::new("git", exit::STORE, e.to_string()))?;
        if !result.status.success() {
            continue;
        }
        let root = PathBuf::from(String::from_utf8_lossy(&result.stdout).trim());
        if let Ok(root) = std::fs::canonicalize(root) {
            return Ok(destination.starts_with(&root) && destination != root);
        }
    }
    Ok(false)
}

/// Points the clone's `origin` at `url`, adding it when missing; whether that changed anything.
fn set_origin(
    git: &impl Fn(&[&str]) -> std::io::Result<std::process::Output>,
    url: &str,
) -> Result<bool, CliError> {
    let current = git(&["remote", "get-url", store::REMOTE])
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    let args = match current {
        Some(current) if current == url => return Ok(false),
        Some(_) => ["remote", "set-url", store::REMOTE, url],
        None => ["remote", "add", store::REMOTE, url],
    };
    let out = git(&args).map_err(|e| CliError::new("git", exit::STORE, e.to_string()))?;
    if !out.status.success() {
        return Err(CliError::new(
            "git",
            exit::STORE,
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(true)
}

/// An import's result: what it changed, in counts and field by field, and the threads items joined, as
/// `{item, thread}`. A dry run changes nothing.
pub(crate) fn import_json(report: &import::Report, dry_run: bool) -> Json {
    let mut data = json!({
        "dry_run": dry_run,
        "counts": {
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
