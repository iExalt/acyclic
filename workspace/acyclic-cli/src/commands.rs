//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::RefCell;
use std::path::Path;
use std::process::Command as Process;

use acyclic_core::archive;
use acyclic_core::claim;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, Model, SetField};
use acyclic_core::needs;
use acyclic_core::ready::{GitAncestry, Readiness, Status};
use acyclic_core::schema::{
    GLOBAL, item as field, kind, link as link_field, repository, settings, thread,
};
use acyclic_core::scope::{self, Outcome as ScopeOutcome, ScopeEdit};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{Cli, Command};
use crate::context;
use crate::error::{CliError, exit};

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
const MAX_LEASE: Millis = 8 * LEASE;

/// Runs a parsed command.
pub fn run(cli: &Cli, env: &Env) -> Result<Output, CliError> {
    if let Command::Init { path, replica } = &cli.command {
        return init(&env.cwd.join(path), replica).map(Output::Json);
    }
    if let Command::Prime = &cli.command {
        return Ok(Output::Text(prime()));
    }
    let session = Session::open(cli, env)?;
    session.dispatch(&cli.command).map(Output::Json)
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
            Command::Init { .. } | Command::Prime => unreachable!("handled before a session opens"),
            Command::Ready { include_archived } => self.ready(*include_archived),
            Command::Show { item } => self.show(item),
            Command::Needs => self.needs(),
            Command::Doctor => self.doctor(),
            Command::Add { id, title, parent } => self.add(id, title, parent.as_deref()),
            Command::Edit {
                id,
                title,
                notes,
                gate,
                size,
                area,
                no_area,
                resource,
                no_resource,
            } => self.write(|replica, by| {
                let key = existing(replica, id)?;
                set_if(replica, &key, field::TITLE, title.as_ref(), by)?;
                set_if(replica, &key, field::NOTES, notes.as_ref(), by)?;
                set_if(replica, &key, field::GATE, gate.as_ref(), by)?;
                set_if(replica, &key, field::SIZE, size.as_ref(), by)?;
                for (set, names, present) in [
                    (SetField::Area, area, true),
                    (SetField::Area, no_area, false),
                    (SetField::Resource, resource, true),
                    (SetField::Resource, no_resource, false),
                ] {
                    for name in names {
                        element_if(replica, &key, set, name, present, by)?;
                    }
                }
                Ok(json!({"item": id}))
            }),
            Command::Parent { id, parent } => self.write(|replica, by| {
                let key = existing(replica, id)?;
                if single(replica.state.field(&key, field::PARENT))
                    != parent.as_ref().map(|p| json!(p))
                {
                    model::set_parent(replica, id, parent.as_deref(), by)?;
                }
                Ok(json!({"item": id, "parent": parent}))
            }),
            Command::Link { from, kind, to } => self.write(|replica, by| {
                let key = model::link_key(from, to, *kind)?;
                if replica.state.field(&key, link_field::PRESENT).is_none() {
                    model::link(replica, from, to, *kind, by)?;
                }
                Ok(json!({"from": from, "kind": kind, "to": to}))
            }),
            Command::Unlink { from, kind, to } => self.write(|replica, by| {
                let key = model::link_key(from, to, *kind)?;
                if replica.state.field(&key, link_field::PRESENT).is_some() {
                    model::unlink(replica, from, to, *kind, by)?;
                }
                Ok(json!({"from": from, "kind": kind, "to": to}))
            }),
            Command::State { id, state } => self.write(|replica, by| {
                model::set_state(replica, id, *state, by)?;
                Ok(json!({"item": id, "state": state}))
            }),
            Command::Done { id, repository, commit, note } => self.write(|replica, by| {
                let key = existing(replica, id)?;
                if repository.is_some() || note.is_some() {
                    let evidence = Evidence {
                        repository: repository.clone(),
                        commit: commit.clone(),
                        note: note.clone(),
                    };
                    set_if(replica, &key, field::EVIDENCE, Some(&evidence), by)?;
                }
                model::set_state(replica, id, ItemState::Done, by)?;
                Ok(json!({"item": id, "state": ItemState::Done}))
            }),
            Command::Reopen { id } => self.write(|replica, by| {
                archive::reopen(replica, id, by)?;
                Ok(json!({"item": id}))
            }),
            Command::Claim { id, lease, force } => self.claim(id, *lease, *force),
            Command::Thread { id, title, brief } => self.write(|replica, by| {
                let key = Key::new(kind::THREAD, id)?;
                set_if(replica, &key, thread::TITLE, title.as_ref(), by)?;
                set_if(replica, &key, thread::BRIEF, brief.as_ref(), by)?;
                Ok(json!({"thread": id}))
            }),
            Command::Member { item, thread } => {
                let change = ScopeEdit::Member { item: item.clone(), thread: thread.clone() };
                self.scope_edit(&change, |model| {
                    model.items.get(item).is_some_and(|i| i.thread.one() == thread.as_ref())
                })
            },
            Command::Touch { repository, item, thread, remove } => {
                let change = match (item, thread) {
                    (Some(item), _) => ScopeEdit::ItemRepository {
                        item: item.clone(),
                        repository: repository.clone(),
                        present: !remove,
                    },
                    (None, Some(thread)) => ScopeEdit::ThreadRepository {
                        thread: thread.clone(),
                        repository: repository.clone(),
                        present: !remove,
                    },
                    (None, None) => unreachable!("clap requires one"),
                };
                self.scope_edit(&change, |model| {
                    let set = match (item, thread) {
                        (Some(item), _) => model.items.get(item).map(|i| &i.repositories),
                        (None, Some(thread)) => model.threads.get(thread).map(|t| &t.repositories),
                        (None, None) => None,
                    };
                    set.is_some_and(|set| set.contains(repository)) != *remove
                })
            },
            Command::Repository { id, owner, name, public, default_branch } => {
                self.write(|replica, by| {
                    let key = Key::new(kind::REPOSITORY, id)?;
                    set_if(replica, &key, repository::OWNER, owner.as_ref(), by)?;
                    set_if(replica, &key, repository::NAME, name.as_ref(), by)?;
                    set_if(replica, &key, repository::PUBLIC, public.as_ref(), by)?;
                    set_if(replica, &key, repository::DEFAULT_BRANCH, default_branch.as_ref(), by)?;
                    Ok(json!({"repository": id}))
                })
            },
            Command::Keep { proposal } => self.write(|replica, by| {
                scope::keep(replica, proposal, by)?;
                Ok(json!({"proposal": proposal, "decision": "keep"}))
            }),
            Command::Drop { proposal } => self.write(|replica, by| {
                scope::drop(replica, proposal, by)?;
                Ok(json!({"proposal": proposal, "decision": "drop"}))
            }),
            Command::Approve { thread: id, revoke } => self.write(|replica, by| {
                let key = Key::new(kind::THREAD, id)?;
                set_if(replica, &key, thread::APPROVED, Some(&!revoke), by)?;
                Ok(json!({"thread": id, "approved": !revoke}))
            }),
            Command::Level { level, thread: id } => self.write(|replica, by| {
                let (key, name) = match id {
                    Some(id) => (Key::new(kind::THREAD, id)?, thread::AUTONOMY),
                    None => (Key::new(kind::SETTINGS, GLOBAL)?, settings::AUTONOMY),
                };
                set_if(replica, &key, name, Some(level), by)?;
                Ok(json!({"thread": id, "level": level}))
            }),
            Command::Import { file, dry_run } => self.import(file, *dry_run),
            Command::Archive { id } => {
                let clones = context::clones(self.env)?;
                let ancestry = GitAncestry::new(clones);
                self.write(|replica, by| {
                    let model = Model::read(&replica.state)?;
                    let item = model.items.get(id).ok_or_else(|| not_found(id))?;
                    if !item.is_archived_root() {
                        archive::archive(replica, &ancestry, id, by)?;
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

    /// Runs one batch as the actor. Sync's lease renewal rides along; `changed` says whether the command itself
    /// changed anything.
    fn write(
        &self,
        apply: impl Fn(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        let result = RefCell::new((Json::Null, false));
        let meta = Meta { actor: self.actor.clone(), run: None };
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
        }
        Ok(data)
    }

    fn scope_edit(
        &self,
        change: &ScopeEdit,
        already: impl Fn(&Model) -> bool,
    ) -> Result<Json, CliError> {
        self.write(|replica, by| {
            let model = Model::read(&replica.state)?;
            if already(&model) {
                return Ok(json!({"edit": change, "outcome": "unchanged"}));
            }
            Ok(match scope::edit(replica, change, by)? {
                ScopeOutcome::Applied => json!({"edit": change, "outcome": "applied"}),
                ScopeOutcome::Proposed(id) => {
                    json!({"edit": change, "outcome": "proposed", "proposal": id})
                },
            })
        })
    }

    fn ready(&self, include_archived: bool) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        let ready = readiness.ready(include_archived)?;
        acyclic_core::probe::mark("readiness");
        let ready: Vec<Json> = ready
            .into_iter()
            .map(|(id, rank)| json!({"item": id, "rank": rank, "title": model.items[&id].title}))
            .collect();
        Ok(json!({"ready": ready}))
    }

    fn show(&self, id: &str) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let item = model.items.get(id).ok_or_else(|| not_found(id))?;
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        let alive = |holder: &str| model.run_alive(holder);
        let claims = claim::live_with(&state, &model::item_key(id)?, self.now, &alive)?;
        Ok(json!({
            "item": item,
            "role": model.role(id),
            "children": model.children(id),
            "readiness": readiness.of(id)?,
            "claims": claims,
        }))
    }

    fn needs(&self) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        Ok(json!({"needs": needs::needs_you(&state, &model, &readiness, self.now)?}))
    }

    fn doctor(&self) -> Result<Json, CliError> {
        // From Git, refusing non-canonical files, rather than from the cache.
        let (_, state) = self.tracker.load_strict()?;
        let (model, unreadable) = Model::read_lenient(&state);
        let unreadable: Vec<Json> = unreadable
            .into_iter()
            .map(|(key, err)| json!({"kind": key.kind, "id": key.id, "error": err.to_string()}))
            .collect();
        Ok(json!({
            "replica": self.tracker.replica().as_str(),
            "items": model.items.len(),
            "links": model.links.len(),
            "threads": model.threads.len(),
            "unreadable": unreadable,
        }))
    }

    fn add(&self, id: &str, title: &str, parent: Option<&str>) -> Result<Json, CliError> {
        self.write(|replica, by| {
            let key = model::item_key(id)?;
            if replica.state.entity(&key).is_some() {
                if single(replica.state.field(&key, field::TITLE)) != Some(json!(title)) {
                    return Err(CliError::refused(
                        "already-exists",
                        format!("item {id} exists with another title"),
                    ));
                }
            } else {
                model::add_item(replica, id, title, by)?;
            }
            if let Some(parent) = parent
                && single(replica.state.field(&key, field::PARENT)) != Some(json!(parent))
            {
                model::set_parent(replica, id, Some(parent), by)?;
            }
            Ok(json!({"item": id}))
        })
    }

    fn import(&self, file: &Path, dry_run: bool) -> Result<Json, CliError> {
        let path = self.env.cwd.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            CliError::new("not-found", exit::NOT_FOUND, format!("{}: {e}", file.display()))
        })?;
        let doc = Import::from_json(&text)?;
        if dry_run {
            let (_, state) = self.tracker.load()?;
            let mut replica = Replica::open(self.tracker.replica().clone(), state);
            let at = replica.stamp(self.now, false)?;
            let report = import::apply(&doc, &mut replica, By { actor: &self.actor, at })?;
            return Ok(json!({"report": report, "dry_run": true, "changed": false}));
        }
        self.write(|replica, by| {
            let report = import::apply(&doc, replica, by)?;
            Ok(json!({"report": report, "dry_run": false}))
        })
    }

    fn claim(&self, id: &str, lease: Option<u32>, force: bool) -> Result<Json, CliError> {
        let is_run = self.actor.starts_with("run/");
        let lease = match (lease, is_run) {
            (None, true) => None,
            (None, false) => Some(LEASE),
            (Some(minutes), _) => Some(Millis::from(minutes) * MINUTE),
        };
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        self.write(|replica, by| {
            let key = existing(replica, id)?;
            if !force {
                let model = Model::read(&replica.state)?;
                let status = Readiness::new(&replica.state, &model, by.at, &ancestry).of(id)?;
                if !may_claim(status.as_ref(), by.actor) {
                    return Err(CliError::refused(
                        "not-ready",
                        format!("item {id} isn't ready to claim"),
                    )
                    .with(json!({"readiness": status})));
                }
            }
            claim::claim(replica, &key, by.actor, by.at, lease, MAX_LEASE)?;
            let model = Model::read(&replica.state)?;
            let alive = |holder: &str| model.run_alive(holder);
            let held = claim::winner_with(&replica.state, &key, by.at, &alive)?;
            Ok(json!({"item": id, "claim": held}))
        })
    }
}

/// Whether `actor` may claim an item in `status` without forcing: when it's ready, or when what stands in the way is
/// the actor's own claim, as winner of a contest or as holder of a shared resource. Core allows the same.
pub(crate) fn may_claim(status: Option<&Status>, actor: &str) -> bool {
    match status {
        Some(Status::Ready { .. }) => true,
        Some(Status::Claimed { holder } | Status::ResourceBusy { holder, .. }) => holder == actor,
        Some(Status::Contested { holders }) => holders.first().is_some_and(|h| h == actor),
        _ => false,
    }
}

/// The agent block, with this build's version.
pub(crate) fn prime() -> String {
    include_str!("prime.md").replace("{version}", acyclic_core::VERSION)
}

fn not_found(id: &str) -> CliError {
    CliError::new("not-found", exit::NOT_FOUND, format!("no item {id}"))
}

fn existing(replica: &Replica, id: &str) -> Result<Key, CliError> {
    let key = model::item_key(id)?;
    if replica.state.entity(&key).is_none() {
        return Err(not_found(id));
    }
    Ok(key)
}

/// A field's value when it holds exactly one.
fn single(register: Option<&acyclic_core::state::Register>) -> Option<Json> {
    match register {
        Some(register) if register.len() == 1 => register.values().next().map(|v| v.value.clone()),
        _ => None,
    }
}

/// Sets a field unless it already holds exactly that value; `None` leaves it alone.
fn set_if<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    value: Option<&T>,
    by: By<'_>,
) -> Result<(), CliError> {
    let Some(value) = value else { return Ok(()) };
    let json = serde_json::to_value(value).expect("model values serialize");
    if single(replica.state.field(key, name)) != Some(json) {
        model::set(replica, key, name, Some(value), by)?;
    }
    Ok(())
}

/// Adds or removes a set element unless it already is so.
fn element_if(
    replica: &mut Replica,
    key: &Key,
    set: SetField,
    name: &str,
    present: bool,
    by: By<'_>,
) -> Result<(), CliError> {
    let field = format!("{}{name}", set.prefix());
    if replica.state.field(key, &field).is_some() != present {
        model::set_element(replica, key, set, name, present, by)?;
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
fn init(path: &Path, replica: &str) -> Result<Json, CliError> {
    let id = ReplicaId::new(replica)?;
    let git = |args: &[&str]| {
        Process::new("git").arg("-C").arg(path).args(args).env("LC_ALL", "C").output()
    };
    let is_repo =
        path.exists() && git(&["rev-parse", "--git-dir"]).is_ok_and(|out| out.status.success());
    if is_repo {
        let configured = git(&["config", "--get", store::REPLICA_CONFIG])
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
        match configured {
            Some(existing) if existing == replica => {
                return Ok(
                    json!({"path": path, "replica": replica, "created": false, "changed": false}),
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
    Ok(json!({"path": path, "replica": replica, "created": !is_repo, "changed": true}))
}
