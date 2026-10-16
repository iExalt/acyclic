//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::RefCell;
use std::path::Path;
use std::process::Command as Process;

use acyclic_core::archive;
use acyclic_core::claim;
use acyclic_core::export;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, Model, SetField};
use acyclic_core::needs;
use acyclic_core::place;
use acyclic_core::ready::{GitAncestry, Readiness, Status};
use acyclic_core::schema::{
    GLOBAL, item as field, kind, link as link_field, repository, settings, thread,
};
use acyclic_core::scope::{self, Outcome as ScopeOutcome, ScopeEdit};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use acyclic_core::suggest;
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{
    Attachment, Cli, Command, GraphFormat, ItemCommand, LinkCommand, ProposalCommand, Renderer,
    RepoCommand, ThreadCommand,
};
use crate::context;
use crate::error::{CliError, exit};
use crate::page;

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
    if let Command::Init { path, replica, remote } = &cli.command {
        return init(&env.cwd.join(path), replica, remote.as_deref()).map(Output::Json);
    }
    if let Command::Prime = &cli.command {
        return Ok(Output::Text(prime()));
    }
    let session = Session::open(cli, env)?;
    if let Command::Graph { format, renderer, open, root, thread, include_archived, output } =
        &cli.command
    {
        let scope = export::Scope {
            root: root.as_deref(),
            thread: thread.as_deref(),
            include_archived: *include_archived,
        };
        let format = format.unwrap_or(if *open { GraphFormat::Html } else { GraphFormat::Dot });
        if renderer.is_some() && format != GraphFormat::Html {
            return Err(CliError::new(
                "usage",
                exit::USAGE,
                "--renderer lays out the HTML page; pass --format html",
            ));
        }
        let renderer = renderer.unwrap_or(Renderer::Graphviz);
        return session.graph(format, renderer, &scope, output.as_deref(), *open);
    }
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
            Command::Init { .. } | Command::Prime | Command::Graph { .. } => {
                unreachable!("handled before dispatch")
            },
            Command::Ready { include_archived } => self.ready(*include_archived),
            Command::List => self.list(),
            Command::Show { items } => self.show(items),
            Command::Needs => self.needs(),
            Command::Sync => self.sync(),
            Command::Doctor => self.doctor(),
            Command::Item(ItemCommand::Add { id, title, parent }) => {
                self.add(id, title, parent.as_deref())
            },
            Command::Item(ItemCommand::Edit {
                id,
                title,
                notes,
                gate,
                size,
                area,
                no_area,
                resource,
                no_resource,
            }) => self.write(|replica, by| {
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
            Command::Item(ItemCommand::Move { id, parent }) => self.write(|replica, by| {
                let key = existing(replica, id)?;
                let mut inherited = Vec::new();
                if single(replica.state().field(&key, field::PARENT))
                    != parent.as_ref().map(|p| json!(p))
                {
                    model::set_parent(replica, id, parent.as_deref(), by)?;
                    inherited = place::place(replica, std::slice::from_ref(id), by)?;
                }
                Ok(with_inherited(json!({"item": id, "parent": parent}), &inherited))
            }),
            Command::Link(LinkCommand::Add { from, kind, to }) => self.write(|replica, by| {
                let key = model::link_key(from, to, *kind)?;
                if replica.state().field(&key, link_field::PRESENT).is_none() {
                    model::link(replica, from, to, *kind, by)?;
                }
                Ok(json!({"from": from, "kind": kind, "to": to}))
            }),
            Command::Link(LinkCommand::Remove { from, kind, to }) => self.write(|replica, by| {
                let key = model::link_key(from, to, *kind)?;
                if replica.state().field(&key, link_field::PRESENT).is_some() {
                    model::unlink(replica, from, to, *kind, by)?;
                }
                Ok(json!({"from": from, "kind": kind, "to": to}))
            }),
            Command::Item(ItemCommand::State { id, state }) => self.write(|replica, by| {
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
                // Finished work holds nothing, not even the resources its claim named.
                claim::release(replica, &key, by.actor);
                Ok(json!({"item": id, "state": ItemState::Done}))
            }),
            Command::Release { id } => self.write(|replica, by| {
                let key = existing(replica, id)?;
                Ok(json!({"item": id, "released": claim::release(replica, &key, by.actor)}))
            }),
            Command::Item(ItemCommand::Reopen { id }) => self.write(|replica, by| {
                archive::reopen(replica, id, by)?;
                Ok(json!({"item": id}))
            }),
            Command::Claim { id, lease, force } => self.claim(id, *lease, *force),
            Command::Thread(ThreadCommand::Set { id, title, brief }) => {
                self.write(|replica, by| {
                    let key = Key::new(kind::THREAD, id)?;
                    set_if(replica, &key, thread::TITLE, title.as_ref(), by)?;
                    set_if(replica, &key, thread::BRIEF, brief.as_ref(), by)?;
                    Ok(json!({"thread": id}))
                })
            },
            Command::Thread(ThreadCommand::Add { item, thread }) => self.member(item, Some(thread)),
            Command::Thread(ThreadCommand::Remove { item }) => self.member(item, None),
            Command::Repo(RepoCommand::Attach(attachment)) => self.attach(attachment, true),
            Command::Repo(RepoCommand::Detach(attachment)) => self.attach(attachment, false),
            Command::Repo(RepoCommand::Set { id, owner, name, public, default_branch }) => self
                .write(|replica, by| {
                    let key = Key::new(kind::REPOSITORY, id)?;
                    set_if(replica, &key, repository::OWNER, owner.as_ref(), by)?;
                    set_if(replica, &key, repository::NAME, name.as_ref(), by)?;
                    set_if(replica, &key, repository::PUBLIC, public.as_ref(), by)?;
                    set_if(replica, &key, repository::DEFAULT_BRANCH, default_branch.as_ref(), by)?;
                    Ok(json!({"repository": id}))
                }),
            Command::Proposal(ProposalCommand::Keep { proposal }) => self.write(|replica, by| {
                scope::keep(replica, proposal, by)?;
                Ok(json!({"proposal": proposal, "decision": "keep"}))
            }),
            Command::Proposal(ProposalCommand::Drop { proposal }) => self.write(|replica, by| {
                scope::drop(replica, proposal, by)?;
                Ok(json!({"proposal": proposal, "decision": "drop"}))
            }),
            Command::Thread(ThreadCommand::Approve { thread: id }) => self.approve(id, true),
            Command::Thread(ThreadCommand::Revoke { thread: id }) => self.approve(id, false),
            Command::Level { level, thread: id } => self.write(|replica, by| {
                let (key, name) = match id {
                    Some(id) => (Key::new(kind::THREAD, id)?, thread::AUTONOMY),
                    None => (Key::new(kind::SETTINGS, GLOBAL)?, settings::AUTONOMY),
                };
                set_if(replica, &key, name, Some(level), by)?;
                Ok(json!({"thread": id, "level": level}))
            }),
            Command::Import { file, dry_run } => self.import(file, *dry_run),
            Command::Item(ItemCommand::Archive { id }) => {
                let clones = context::clones(self.env)?;
                let ancestry = GitAncestry::new(clones);
                self.write(|replica, by| {
                    let model = Model::read(replica.state())?;
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

    fn scope_edit(
        &self,
        change: &ScopeEdit,
        already: impl Fn(&State, &Model) -> Result<bool, CliError>,
    ) -> Result<Json, CliError> {
        self.write(|replica, by| {
            let model = Model::read(replica.state())?;
            if already(replica.state(), &model)? {
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

    /// Puts an item in a thread, or takes it out.
    fn member(&self, item: &str, thread: Option<&String>) -> Result<Json, CliError> {
        let change = ScopeEdit::Member { item: item.to_owned(), thread: thread.cloned() };
        self.scope_edit(&change, |state, model| {
            if model.items.get(item).is_none_or(|i| i.thread.one() != thread) {
                return Ok(false);
            }
            // The user putting an unapproved member in its thread again keeps it, so it writes; an agent's repeat
            // changes nothing.
            Ok(thread.is_none()
                || scope::classify(model, &self.actor) != scope::Actor::User
                || !scope::unapproved(state, model)?.iter().any(|u| u.item == item))
        })
    }

    /// Attaches a repository record to an item or a thread, or detaches it.
    fn attach(&self, attachment: &Attachment, present: bool) -> Result<Json, CliError> {
        let Attachment { repository, item, thread } = attachment;
        let change = match (item, thread) {
            (Some(item), _) => ScopeEdit::ItemRepository {
                item: item.clone(),
                repository: repository.clone(),
                present,
            },
            (None, Some(thread)) => ScopeEdit::ThreadRepository {
                thread: thread.clone(),
                repository: repository.clone(),
                present,
            },
            (None, None) => unreachable!("clap requires one"),
        };
        self.scope_edit(&change, |_, model| {
            let set = match (item, thread) {
                (Some(item), _) => model.items.get(item).map(|i| &i.repositories),
                (None, Some(thread)) => model.threads.get(thread).map(|t| &t.repositories),
                (None, None) => None,
            };
            Ok(set.is_some_and(|set| set.contains(repository)) == present)
        })
    }

    /// Approves a thread's scope, or revokes the approval.
    fn approve(&self, id: &str, approved: bool) -> Result<Json, CliError> {
        self.write(|replica, by| {
            scope::approve(replica, id, approved, by)?;
            Ok(json!({"thread": id, "approved": approved}))
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

    /// The graph as a document: DOT or Mermaid to standard output as text, or any format to a file with a JSON
    /// summary, opened in the browser with `open`.
    fn graph(
        &self,
        format: GraphFormat,
        renderer: Renderer,
        scope: &export::Scope<'_>,
        output: Option<&Path>,
        open: bool,
    ) -> Result<Output, CliError> {
        let usage = |message: &str| CliError::new("usage", exit::USAGE, message);
        if open && format != GraphFormat::Html {
            return Err(usage("--open opens the HTML page; drop --format or pass --format html"));
        }
        if format == GraphFormat::Html && output.is_none() && !open {
            return Err(usage(
                "an HTML page is too large for standard output: pass --output <file> or --open",
            ));
        }
        let (_, model) = self.read()?;
        let view = export::view(&model, scope)
            .map_err(|err| CliError::new("not-found", exit::NOT_FOUND, err.to_string()))?;
        let document = match (format, renderer) {
            (GraphFormat::Dot, _) | (GraphFormat::Html, Renderer::Graphviz) => {
                export::dot(&model, &view)
            },
            (GraphFormat::Mermaid, _) | (GraphFormat::Html, Renderer::Mermaid) => {
                export::mermaid(&model, &view)
            },
        };
        let document = if format == GraphFormat::Html {
            let title = format!("acyclic: {}", export::scope_name(scope));
            page::page(&title, renderer, &document).ok_or_else(|| {
                CliError::refused(
                    "no-renderer",
                    "this acyclic was built without the graph page's renderers: run `mise run vendor:graph` and \
                     rebuild",
                )
            })?
        } else {
            document
        };
        let Some(path) = (match output {
            Some(output) => Some(self.env.cwd.join(output)),
            None if open => Some(
                self.tracker
                    .local_dir()?
                    .join("graph")
                    .join(format!("{}.html", export::scope_name(scope))),
            ),
            None => None,
        }) else {
            return Ok(Output::Text(document));
        };
        let unwritable = |err: std::io::Error| {
            CliError::refused("unwritable", format!("can't write {}: {err}", path.display()))
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(unwritable)?;
        }
        std::fs::write(&path, document).map_err(unwritable)?;
        if open {
            context::open(self.env, &path).map_err(|err| {
                CliError::refused(
                    "unopenable",
                    format!("wrote {} but can't open it: {err}", path.display()),
                )
                .with(json!({"path": path}))
            })?;
        }
        let format = match format {
            GraphFormat::Dot => "dot",
            GraphFormat::Mermaid => "mermaid",
            GraphFormat::Html => "html",
        };
        Ok(Output::Json(json!({
            "path": path,
            "format": format,
            "opened": open,
            "items": view.items.len(),
            "ghosts": view.ghosts.len(),
            "edges": view.edges.len(),
        })))
    }

    fn list(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let items: Vec<Json> = model
            .items
            .values()
            .map(|item| {
                json!({
                    "item": item.id,
                    "title": item.title,
                    "state": item.state,
                    "parent": item.parent,
                    "thread": item.thread,
                })
            })
            .collect();
        Ok(json!({"items": items}))
    }

    fn show(&self, ids: &[String]) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        let alive = |holder: &str| model.run_alive(holder);
        let mut shown = Vec::with_capacity(ids.len());
        for id in ids {
            let item = model.items.get(id).ok_or_else(|| not_found(id))?;
            let claims = claim::live_with(&state, &model::item_key(id)?, self.now, &alive)?;
            shown.push(json!({
                "item": item,
                "role": model.role(id),
                "children": model.children(id),
                "readiness": readiness.of(id)?,
                "claims": claims,
            }));
        }
        Ok(Json::Array(shown))
    }

    fn needs(&self) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        let readiness = Readiness::new(&state, &model, self.now, &ancestry);
        let mut entries = needs::needs_you(&state, &model, &readiness, self.now)?;
        // Local to this clone, so derived here rather than kept in the shared state: asked to push every write, it
        // still holds writes the remote lacks.
        if let Some(remote) = self.tracker.remote()
            && self.tracker.push_after_write()
            && self.tracker.unpushed(remote)?
        {
            entries.push(needs::Entry::Unpushed { remote: remote.to_owned() });
        }
        Ok(json!({"needs": entries}))
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
                    "the tracker clone has no {}; set one with acyclic init <clone> --replica <id> --remote <url>",
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
            // Only a parent places an item: a new top-level item has nothing to inherit and is one leaf.
            let mut placed = false;
            if replica.state().entity(&key).is_some() {
                if single(replica.state().field(&key, field::TITLE)) != Some(json!(title)) {
                    return Err(CliError::refused(
                        "already-exists",
                        format!("item {id} exists with another title"),
                    ));
                }
            } else {
                model::add_item(replica, id, title, by)?;
            }
            if let Some(parent) = parent
                && single(replica.state().field(&key, field::PARENT)) != Some(json!(parent))
            {
                model::set_parent(replica, id, Some(parent), by)?;
                placed = true;
            }
            // Candidates on every add, after placement, so each parent names the thread it would pass down as it
            // stands now. With a parent given, its rank says whether the words agree with it.
            let (inherited, model) = if placed {
                place::place_and_read(replica, &[id.to_owned()], by)?
            } else {
                (Vec::new(), Model::read(replica.state())?)
            };
            let (suggestions, rank) = suggest::suggest(&model, id, parent, suggest::LIMIT);
            let mut candidates = json!(suggestions);
            if parent.is_some() {
                candidates["parent_rank"] = json!(rank);
            }
            Ok(with_inherited(json!({"item": id, "candidates": candidates}), &inherited))
        })
    }

    fn import(&self, file: &Path, dry_run: bool) -> Result<Json, CliError> {
        let path = self.env.cwd.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            CliError::new("not-found", exit::NOT_FOUND, format!("{}: {e}", file.display()))
        })?;
        let doc = Import::from_json(&text)?;
        // Imported work with no thread of its own joins the one passed down to it, as an add's does.
        let keys: Vec<String> = doc.items.iter().map(|item| item.key.clone()).collect();
        if dry_run {
            let (_, state) = self.tracker.load()?;
            let mut replica = Replica::open(self.tracker.replica().clone(), state);
            let at = replica.stamp(self.now, false)?;
            let by = By { actor: &self.actor, at };
            let report = import::apply(&doc, &mut replica, by)?;
            let inherited = place::place(&mut replica, &keys, by)?;
            let data = json!({"report": report, "dry_run": true, "changed": false});
            return Ok(with_inherited(data, &inherited));
        }
        self.write(|replica, by| {
            let report = import::apply(&doc, replica, by)?;
            let inherited = place::place(replica, &keys, by)?;
            Ok(with_inherited(json!({"report": report, "dry_run": false}), &inherited))
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
                let model = Model::read(replica.state())?;
                let status = Readiness::new(replica.state(), &model, by.at, &ancestry).of(id)?;
                if !may_claim(status.as_ref(), by.actor) {
                    return Err(CliError::refused(
                        "not-ready",
                        format!("item {id} isn't ready to claim"),
                    )
                    .with(json!({"readiness": status})));
                }
            }
            claim::claim(replica, &key, by.actor, by.at, lease, MAX_LEASE)?;
            let model = Model::read(replica.state())?;
            let alive = |holder: &str| model.run_alive(holder);
            let held = claim::winner_with(replica.state(), &key, by.at, &alive)?;
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
    if replica.state().entity(&key).is_none() {
        return Err(not_found(id));
    }
    Ok(key)
}

/// A field's value when it holds exactly one; a clear holds none.
fn single(register: Option<&acyclic_core::state::Register>) -> Option<Json> {
    match register {
        Some(register) if register.len() == 1 => {
            register.values().next().map(|v| v.value.clone()).filter(|value| !value.is_null())
        },
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
    if single(replica.state().field(key, name)) != Some(json) {
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
    if replica.state().field(key, &field).is_some() != present {
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
fn init(path: &Path, replica: &str, remote: Option<&str>) -> Result<Json, CliError> {
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

/// Adds the items that joined a thread passed down to them, or asked to, to a command's data, when there are any.
fn with_inherited(mut data: Json, inherited: &[place::Inherited]) -> Json {
    if !inherited.is_empty() {
        data["inherited"] = json!(inherited);
    }
    data
}
