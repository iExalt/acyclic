//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use acyclic_core::archive;
use acyclic_core::claim;
use acyclic_core::delete;
use acyclic_core::ids;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, Model};
use acyclic_core::needs;
use acyclic_core::place;
use acyclic_core::ready::{Ancestry, GitAncestry, Readiness, Status};
use acyclic_core::schema::{item as field, kind};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{
    Class, Cli, Command, ItemCommand, LEASE_MAX_MINUTES, RepositoryCommand, StateWord,
    ThreadCommand,
};
use crate::context;
use crate::edit::{self, Change, Delta, ItemFields, RepositoryFields, ThreadFields};
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

/// Runs a parsed command.
pub fn run(cli: &Cli, env: &Env) -> Result<Output, CliError> {
    match (cli.command.class(), &cli.command) {
        (Class::Standalone, Command::Init { path, replica, remote }) => {
            init(&env.cwd.join(path), replica, remote.as_deref()).map(Output::Json)
        },
        (Class::Standalone, Command::Prime) => Ok(Output::Text(prime())),
        (Class::Standalone, Command::Id { count }) => {
            Ok(Output::Text((0..*count).map(|_| ids::new() + "\n").collect()))
        },
        (Class::Standalone, Command::Completions { shell }) => {
            Ok(Output::Text(completions(*shell)))
        },
        // `import --schema`, the only other command needing no tracker.
        (Class::Standalone, _) => Ok(Output::Text(import::SCHEMA.to_owned())),
        (Class::Server, _) => {
            Err(CliError::new("no-server", exit::CONFIG, "this build of acyclic has no server"))
        },
        (Class::Clone | Class::Read | Class::Write, command) => {
            Session::open(cli, env)?.dispatch(command).map(Output::Json)
        },
    }
}

/// What pretty output calls each record, by id: items and threads by the shortest ending of their ids that names them,
/// and repositories by `owner/name`. Read after the command, so a record it added is named too; empty for a command
/// with no tracker, or one that can't be read.
pub fn names(cli: &Cli, env: &Env) -> BTreeMap<String, String> {
    let read = || -> Result<BTreeMap<String, String>, CliError> {
        let (tracker, _) = Tracker::open(&context::tracker(cli, env)?)?;
        let (_, state) = tracker.load()?;
        let model = Model::read(&state)?;
        let records = Records::of(&model);
        let mut names = records.endings(true);
        names.extend(records.endings(false));
        for record in model.repositories.values() {
            if let Some(name) = crate::resolve::repository_name(record) {
                names.insert(record.id.clone(), name);
            }
        }
        Ok(names)
    };
    match cli.command.class() {
        Class::Clone | Class::Read | Class::Write => read().unwrap_or_default(),
        Class::Standalone | Class::Server => BTreeMap::new(),
    }
}

struct Session<'a> {
    tracker: Tracker,
    actor: String,
    now: Millis,
    env: &'a Env,
}

impl<'a> Session<'a> {
    fn open(cli: &Cli, env: &'a Env) -> Result<Self, CliError> {
        let actor = context::actor(cli, env)?;
        let path = context::tracker(cli, env)?;
        let (tracker, renamed) = Tracker::open(&path)?;
        if let Some(renamed) = renamed {
            // A clone without its own state took a fresh id; say so once, but carry on.
            eprintln!("acyclic: this clone is now replica {renamed}");
        }
        Ok(Self { tracker, actor, now: context::now(env), env })
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
                    let id = edit::new_id(replica, kind::ITEM, given.as_deref(), title)?;
                    let fields = fields.clone().resolve(&Model::read(replica.state())?, None)?;
                    let mut data = edit::add_item(replica, &id, &fields, by)?.item(&id);
                    data["title"] = json!(title);
                    Ok(data)
                })
                .map_err(|error| {
                    self.misplaced(error, parent.as_deref(), || {
                        let id =
                            given.as_deref().map(|id| format!(" --id {id}")).unwrap_or_default();
                        format!(
                            "acyclic item add{id} --title <TITLE> --thread {}",
                            parent.as_deref().unwrap_or_default()
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
                    let model = Model::read(replica.state())?;
                    let id = Records::of(&model).item(item)?;
                    let fields = fields.clone().resolve(&model, Some(&id))?;
                    Ok(edit::edit_item(replica, &id, &fields, by)?.item(&id))
                })
                .map_err(|error| {
                    self.misplaced(error, edit.parent.as_deref(), || {
                        format!(
                            "acyclic item edit {item} --thread {}",
                            edit.parent.as_deref().unwrap_or_default()
                        )
                    })
                })
            },
            Command::Item(ItemCommand::Done { item, commit, evidence: text, force }) => {
                let ancestry = GitAncestry::new(context::clones(self.env)?);
                let commit = commit.as_deref().map(split_commit).transpose()?;
                self.write(|replica, by| {
                    let model = Model::read(replica.state())?;
                    let records = Records::of(&model);
                    let id = &records.item(item)?;
                    let key = existing(replica, id)?;
                    if !force {
                        check_finish(replica, id, by, &ancestry)?;
                    }
                    if commit.is_some() || text.is_some() {
                        let repository = commit.map(|(r, _)| records.repository(r)).transpose()?;
                        let evidence = Evidence {
                            repository,
                            commit: commit.map(|(_, sha)| sha.to_owned()),
                            text: text.clone(),
                        };
                        check_evidence(&model, &evidence)?;
                        if !force && overwrites(&model.items[id], &evidence) {
                            return Err(CliError::refused(
                                "already-done",
                                format!("item {id} is done with other evidence; pass --force to replace it"),
                            )
                            .with(json!({"evidence": view::evidence(&model, &model.items[id].evidence)})));
                        }
                        set_if(replica, &key, field::EVIDENCE, Some(&evidence), by)?;
                    }
                    model::set_state(replica, id, ItemState::Done, by)?;
                    // The evidence it holds now, so a caller sees what was recorded.
                    let model = Model::read(replica.state())?;
                    let evidence = view::evidence(&model, &model.items[id].evidence);
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
                    let id = edit::new_id(replica, kind::THREAD, given.as_deref(), title)?;
                    let fields = fields.clone().resolve(&Model::read(replica.state())?, None)?;
                    edit::add_thread(replica, &id, &fields, by)?;
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
                    let model = Model::read(replica.state())?;
                    let id = Records::of(&model).thread(thread)?;
                    let fields = fields.clone().resolve(&model, Some(&id))?;
                    edit::edit_thread(replica, &id, &fields, by)?;
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
                    edit::add_repository(replica, &id, &fields, by)?;
                    Ok(json!({"repository": id}))
                })
            },
            Command::Repository(RepositoryCommand::Edit { repository, name, default_branch }) => {
                let fields =
                    RepositoryFields { name: name.clone(), default_branch: default_branch.clone() };
                self.write(|replica, by| {
                    let id = Records::of(&Model::read(replica.state())?).repository(repository)?;
                    edit::edit_repository(replica, &id, &fields, by)?;
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
        Ok((state, model))
    }

    /// Runs one batch as the actor. Lease renewal rides along, so a batch can commit though the command changed
    /// nothing; `changed` says whether the command itself changed anything.
    fn write(
        &self,
        apply: impl Fn(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        let result = RefCell::new((Json::Null, false));
        let meta = Meta { actor: self.actor.clone() };
        self.tracker.batch(&meta, self.now, false, &project_root, |replica, at| {
            claim::renew_leases(replica, at, LEASE, MAX_LEASE)?;
            acyclic_core::probe::mark("batch: renew leases");
            let before = replica.changes();
            let data = apply(replica, By { actor: &self.actor, at })?;
            *result.borrow_mut() = (data, replica.changes() != before);
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
            for row in &mut rows {
                let waits = row["id"].as_str().and_then(|id| waves.waits_for.get(id));
                row["waits_for"] = json!(waits.cloned().unwrap_or_default());
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
        // Imported work with no thread of its own joins the one passed down to it, as an add's does.
        let keys: Vec<String> = doc.items.iter().map(|item| item.id.clone()).collect();
        if dry_run {
            let (_, state) = self.tracker.load()?;
            let mut replica = Replica::open(self.tracker.replica().clone(), state);
            let at = replica.stamp(self.now, false)?;
            let by = By { actor: &self.actor, at };
            let report = import::apply(&doc, &mut replica, by)?;
            let placed = place::place(&mut replica, &keys, by)?;
            return Ok(import_json(&report, &placed, true));
        }
        self.write(|replica, by| {
            let report = import::apply(&doc, replica, by)?;
            let placed = place::place(replica, &keys, by)?;
            Ok(import_json(&report, &placed, false))
        })
    }

    fn claim(&self, reference: &str, lease: Option<u32>, force: bool) -> Result<Json, CliError> {
        let lease = lease.map_or(LEASE, |minutes| Millis::from(minutes) * MINUTE);
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        self.write(|replica, by| {
            let (id, key) = existing_thread(replica, reference)?;
            let thread = id.as_str();
            if !force {
                let model = Model::read(replica.state())?;
                let status = Readiness::new(replica.state(), &model, by.at, &ancestry).thread(thread)?;
                if !status.claimable_by(by.actor) {
                    return Err(CliError::refused(
                        "not-ready",
                        format!(
                            "thread {thread} isn't ready to claim: it has no ready work, or someone else holds it or \
                             a resource it needs; `acyclic thread show {thread}` says why"
                        ),
                    )
                    .with(json!({"readiness": status})));
                }
            }
            claim::claim(replica, thread, by.actor, by.at, lease, MAX_LEASE)?;
            let held = claim::winner(replica.state(), &key, by.at)?;
            Ok(json!({"thread": thread, "claim": held.as_ref().map(view::claim_json)}))
        })
    }
}

/// Whether `actor` may work on an item in `status` without forcing: when it's ready, or when what stands in the way is
/// the actor's own claim on its thread, alone or as winner of a contest.
pub(crate) fn may_claim(status: Option<&Status>, actor: &str) -> bool {
    match status {
        Some(Status::Ready) => true,
        Some(Status::Claimed { holder }) => holder == actor,
        Some(Status::Contested { holders }) => holders.first().is_some_and(|h| h == actor),
        _ => false,
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

/// Refuses evidence that could never verify, without asking the network: the commit must look like an object id and
/// name a complete repository record.
pub(crate) fn check_evidence(model: &Model, evidence: &Evidence) -> Result<(), CliError> {
    if evidence.commit.as_ref().is_some_and(|commit| !acyclic_core::import::is_object_id(commit)) {
        return Err(CliError::new(
            "invalid-commit",
            exit::USAGE,
            "a commit is 7 to 64 hexadecimal characters, as git prints it",
        ));
    }
    let Some(id) = &evidence.repository else { return Ok(()) };
    let Some(record) = model.repositories.get(id) else {
        return Err(CliError::new(
            "not-found",
            exit::NOT_FOUND,
            format!("no repository {id}; `acyclic repository list` lists them"),
        ));
    };
    let missing: Vec<&str> = [
        ("owner", record.owner.one().is_some()),
        ("name", record.name.one().is_some()),
        ("default-branch", record.default_branch.one().is_some()),
    ]
    .into_iter()
    .filter_map(|(field, set)| (!set).then_some(field))
    .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(CliError::refused(
        "incomplete-repository",
        format!(
            "repository {id} is missing {}; set it with `acyclic repository edit {id}`",
            missing.join(", ")
        ),
    )
    .with(json!({"missing": missing.iter().map(|f| f.replace('-', "_")).collect::<Vec<_>>()})))
}

/// Whether recording `evidence` would replace different evidence on an item that is already done.
pub(crate) fn overwrites(item: &model::Item, evidence: &Evidence) -> bool {
    item.is_done() && !item.evidence.all().is_empty() && item.evidence.one() != Some(evidence)
}

/// Refuses to finish an item that isn't ready or that someone else holds, unless forced. A finished item may be finished
/// again, which changes nothing.
fn check_finish(
    replica: &Replica,
    id: &str,
    by: By<'_>,
    ancestry: &dyn Ancestry,
) -> Result<(), CliError> {
    let model = Model::read(replica.state())?;
    let readiness = Readiness::new(replica.state(), &model, by.at, ancestry);
    let status = readiness.of(id)?;
    if may_finish(status.as_ref(), by.actor) {
        return Ok(());
    }
    // A parent is finished once its children are, unless it would be held back as a leaf.
    if status == Some(Status::Parent) {
        let (open, held) = readiness.unfinished(id);
        if open.is_empty() && held.is_none() {
            return Ok(());
        }
        return Err(CliError::refused(
            "not-ready",
            format!("item {id} can't be finished yet: it has open children or is held back; `acyclic item show {id}` says which"),
        )
        .with(json!({"readiness": status, "open": open, "held": held})));
    }
    Err(CliError::refused(
        "not-ready",
        format!("item {id} isn't yours to finish: it isn't ready, or someone else holds it; `acyclic item show {id}` says which"),
    )
    .with(json!({"readiness": status})))
}

/// Whether an actor may finish an item in `status` without forcing: when it may claim it, or it is finished already.
pub(crate) fn may_finish(status: Option<&Status>, actor: &str) -> bool {
    matches!(status, Some(Status::Done | Status::InReview | Status::Unverified { .. }))
        || may_claim(status, actor)
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

/// Every thread by id, those with fields and those only named by an item, with its members in the order its blocks
/// links run them: ties by id, and members on a cycle last. An item raced into two threads is in both.
pub(crate) fn thread_members(model: &Model) -> BTreeMap<&str, Vec<Member<'_>>> {
    let mut threads: BTreeMap<&str, Vec<Member<'_>>> =
        model.threads.keys().map(|id| (id.as_str(), Vec::new())).collect();
    for item in model.items.values() {
        let named = item.thread.all();
        for thread in &named {
            let member = Member { item: &item.id, conflict: item.thread.is_conflict() };
            threads.entry(thread.as_str()).or_default().push(member);
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

/// An item's key, refusing one that doesn't exist or was deleted.
pub(crate) fn existing(replica: &Replica, id: &str) -> Result<Key, CliError> {
    Ok(model::existing(replica.state(), id)?)
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

/// A field's value when it holds exactly one; a clear holds none.
pub(crate) fn single(register: Option<&acyclic_core::state::Register>) -> Option<Json> {
    match register {
        Some(register) if register.len() == 1 => {
            register.values().next().map(|v| v.value.clone()).filter(|value| !value.is_null())
        },
        _ => None,
    }
}

/// Whether a field holds only `value`, or, for `None`, nothing but clears. A conflict holds neither, so writing
/// either resolves it.
pub(crate) fn holds_only(
    register: Option<&acyclic_core::state::Register>,
    value: Option<Json>,
) -> bool {
    let values: Vec<&Json> = register.into_iter().flatten().map(|(_, v)| &v.value).collect();
    match value {
        // Equal values written apart read as one, so they hold it too.
        Some(value) => !values.is_empty() && values.iter().all(|held| **held == value),
        None => values.iter().all(|held| held.is_null()),
    }
}

/// Sets a field unless it already holds exactly that value; `None` leaves it alone.
pub(crate) fn set_if<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    value: Option<&T>,
    by: By<'_>,
) -> Result<(), CliError> {
    let Some(value) = value else { return Ok(()) };
    let json = serde_json::to_value(value).expect("model values serialize");
    if single(replica.state().field(key, name)) != Some(json) {
        model::set(replica, key, name, Some(value), by)?;
    }
    Ok(())
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
pub(crate) fn import_json(
    report: &import::Report,
    inherited: &[place::Inherited],
    dry_run: bool,
) -> Json {
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
        "placed": inherited,
    });
    if dry_run {
        data["changed"] = json!(false);
    }
    data
}
