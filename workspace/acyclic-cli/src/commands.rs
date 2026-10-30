//! The commands. Reads build one model; writes run as one batch each and write nothing that is already so, so a
//! repeated command changes nothing and commits nothing.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use acyclic_core::archive;
use acyclic_core::claim;
use acyclic_core::delete;
use acyclic_core::export;
use acyclic_core::import::{self, Import};
use acyclic_core::model::{self, By, Evidence, ItemState, LinkKind, Model, SetField, Value};
use acyclic_core::needs;
use acyclic_core::place;
use acyclic_core::ready::{Ancestry, GitAncestry, Readiness, Status};
use acyclic_core::schema::{GLOBAL, item as field, kind, link as link_field, settings};
use acyclic_core::scope;
use acyclic_core::state::{Context, Key, Millis, Replica, ReplicaId, Sequence, State};
use acyclic_core::store::{self, Meta, Tracker};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::args::{
    Cli, Command, GraphFormat, ItemCommand, LEASE_MAX_MINUTES, LinkCommand, ProposalCommand,
    Renderer, RepoCommand, StateWord, ThreadCommand,
};
use crate::context;
use crate::edit::{self, Change, Delta, ItemFields, RepositoryFields, ThreadFields};
use crate::error::{CliError, exit};
use crate::page;
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

/// What a run's command must find in the tracker, inside the batch that writes it, to stay within the run's thread.
#[derive(Debug)]
enum Within {
    /// The item is in the thread.
    Item(String),
    /// The link is the run's own: only the run wrote it.
    OwnLink(String, model::LinkKind, String),
}

fn outside(thread: &str, what: impl std::fmt::Display) -> CliError {
    CliError::refused("not-in-thread", format!("{what} is outside this run's thread {thread}"))
}

/// What `command` must find to stay within `thread`, or why it can't: a claim or a release of another thread, an add
/// outside the thread, or an edit that moves an item's thread or drops its parent, which only the user does.
fn within(command: &Command, thread: &str) -> Result<Vec<Within>, CliError> {
    use Within::{Item, OwnLink};
    let parent = |parent: &Option<String>| {
        parent.iter().map(|parent| Item(parent.clone())).collect::<Vec<_>>()
    };
    match command {
        Command::Claim { thread: named, .. } | Command::Release { thread: named } => {
            if named == thread {
                Ok(vec![])
            } else {
                Err(outside(thread, format!("thread {named}")))
            }
        },
        Command::Done { item, .. } | Command::Item(ItemCommand::Reopen { item }) => {
            Ok(vec![Item(item.clone())])
        },
        Command::Item(ItemCommand::Add { item, thread: named, parent: up, .. }) => {
            if named.as_deref() == Some(thread) {
                Ok(parent(up))
            } else {
                Err(outside(thread, format!("new item {item}, added with no thread or another")))
            }
        },
        Command::Item(ItemCommand::Edit(edit)) => {
            if edit.thread.is_some() || edit.no_thread || edit.no_parent {
                return Err(outside(
                    thread,
                    format!("moving {} between threads or out from its parent", edit.item),
                ));
            }
            Ok([Item(edit.item.clone())].into_iter().chain(parent(&edit.parent)).collect())
        },
        Command::Link(LinkCommand::Add { from, .. }) => Ok(vec![Item(from.clone())]),
        Command::Link(LinkCommand::Remove { from, kind, to }) => {
            Ok(vec![Item(from.clone()), OwnLink(from.clone(), kind.kind(), to.clone())])
        },
        _ => Ok(vec![]),
    }
}

/// Refuses what `within` asks of the state now: an item counts as in the thread only when every value of its thread
/// field names it, so a conflict over its thread keeps it out.
fn check_within(
    replica: &Replica,
    needs: &[Within],
    thread: &str,
    actor: &str,
) -> Result<(), CliError> {
    if needs.is_empty() {
        return Ok(());
    }
    let model = Model::read(replica.state())?;
    for need in needs {
        match need {
            Within::Item(id) => {
                let inside = model.items.get(id).is_some_and(|item| match &item.thread {
                    Value::One(named) => named == thread,
                    Value::Conflict(values) => {
                        values.iter().all(|named| named.as_deref() == Some(thread))
                    },
                    Value::None => false,
                });
                if !inside {
                    return Err(outside(thread, format!("item {id}")));
                }
            },
            Within::OwnLink(from, kind, to) => {
                let key = model::link_key(from, to, *kind)?;
                let writers =
                    replica.state().field(&key, link_field::PRESENT).into_iter().flatten();
                if !writers.map(|(_, value)| value.actor.as_str()).all(|writer| writer == actor) {
                    return Err(CliError::refused(
                        "not-the-runs",
                        format!(
                            "the link from {from} to {to} is someone else's: a run removes only its own links"
                        ),
                    ));
                }
            },
        }
    }
    Ok(())
}

/// Checks that `op` is the run's next operation, refusing a duplicate with [`DUPLICATE`] and its recorded outcome.
fn next(replica: &Replica, op: &Operation, hash: &str) -> Result<(), CliError> {
    match replica.sequence(&op.run, op.seq, hash)? {
        Sequence::Next => Ok(()),
        Sequence::Duplicate { latest, refusal } => {
            let outcome = match (latest, refusal) {
                (true, Some(refusal)) => json!({"ok": false, "code": refusal["code"]}),
                (true, None) => json!({"ok": true}),
                (false, _) => Json::Null,
            };
            Err(CliError::new(DUPLICATE, exit::OK, "the operation was applied already")
                .with(outcome))
        },
    }
}

/// Applies a run's operation: its command, as the run, in a batch that checks and records its number. A command
/// that writes nothing still has its number recorded, in a batch of its own.
///
/// # Errors
///
/// The command's error, or [`DUPLICATE`] with the recorded outcome when the number was applied already.
pub fn apply(cli: &Cli, env: &Env, operation: Operation) -> Result<Json, CliError> {
    let mut session = Session::open(cli, env)?;
    session.within = within(&cli.command, &operation.thread)?;
    session.hash = hash(&cli.command);
    session.operation = Some(operation);
    let data = session.dispatch(&cli.command)?;
    if !session.recorded.get() {
        session.record(None)?;
    }
    Ok(data)
}

/// Records a run's operation whose command the tracker refused, with the refusal as its outcome, so that sending it
/// again gets the same answer rather than running it again, and the run's later operations aren't stuck behind it.
///
/// # Errors
///
/// When the store fails, or [`DUPLICATE`] when the number was recorded meanwhile.
pub fn refuse(
    cli: &Cli,
    env: &Env,
    operation: Operation,
    refusal: &CliError,
) -> Result<(), CliError> {
    let mut session = Session::open(cli, env)?;
    session.hash = hash(&cli.command);
    session.operation = Some(operation);
    session.record(Some(&json!({"code": refusal.code})))
}

/// Starts run `run` on `thread`, as the operator `operator`, which no command line may name.
///
/// # Errors
///
/// When the tracker can't be opened, the thread already has a run under way, or the store fails.
pub fn start_run(
    tracker: &Path,
    env: &Env,
    operator: &str,
    run: &str,
    thread: &str,
) -> Result<Json, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let session = Session {
        tracker: opened,
        actor: operator.to_owned(),
        now: context::now(env),
        env,
        operation: None,
        recorded: Cell::new(false),
        within: Vec::new(),
        hash: String::new(),
    };
    session.write(|replica, by| {
        model::start_run(replica, run, thread, by)?;
        Ok(json!({"run": run, "thread": thread}))
    })
}

/// Moves run `run` to `state`, as the operator `operator`.
///
/// # Errors
///
/// When the tracker can't be opened, the run can't move that way, or the store fails.
pub fn set_run_state(
    tracker: &Path,
    env: &Env,
    operator: &str,
    run: &str,
    state: model::RunState,
) -> Result<Json, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let session = Session {
        tracker: opened,
        actor: operator.to_owned(),
        now: context::now(env),
        env,
        operation: None,
        recorded: Cell::new(false),
        within: Vec::new(),
        hash: String::new(),
    };
    session.write(|replica, by| {
        model::set_run_state(replica, run, state, by)?;
        Ok(json!({"run": run, "state": state}))
    })
}

/// Whether run `run` is under way on `thread` in the clone at `tracker`.
///
/// # Errors
///
/// When the tracker can't be opened or read.
pub fn run_active(tracker: &Path, run: &str, thread: &str) -> Result<bool, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let (_, state) = opened.load()?;
    Ok(model::run_active(&state, run, thread))
}

/// What a run may reach of a repository it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// The run isn't under way.
    Inactive,
    /// No repository record names it, or the run's thread doesn't hold it.
    NotInThread,
    /// More than one record in the run's thread names it, differing only in case.
    Ambiguous,
    /// The record, in the run's thread.
    Found(model::Repository),
}

/// What run `run` on `thread` may reach of repository `owner`/`name` in the clone at `tracker`.
pub fn reach(
    tracker: &Path,
    run: &str,
    thread: &str,
    owner: &str,
    name: &str,
) -> Result<Reach, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let (_, state) = opened.load()?;
    if !model::run_active(&state, run, thread) {
        return Ok(Reach::Inactive);
    }
    let model = Model::read(&state)?;
    // Only records in the thread count, so a run learns nothing of those outside it.
    let set = model.repository_set(thread);
    let named: Vec<_> = model
        .repositories_named(owner, name)
        .into_iter()
        .filter(|record| set.contains(&record.id))
        .collect();
    Ok(match named.as_slice() {
        [] => Reach::NotInThread,
        [record] => Reach::Found((*record).clone()),
        _ => Reach::Ambiguous,
    })
}

/// Which of `threads` are finished in the clone at `tracker`.
pub fn finished(tracker: &Path, threads: &BTreeSet<String>) -> Result<BTreeSet<String>, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let (_, state) = opened.load()?;
    let model = Model::read(&state)?;
    Ok(threads.iter().filter(|thread| model.thread_finished(thread)).cloned().collect())
}

/// The repository records in the clone at `tracker`.
pub fn repositories(tracker: &Path) -> Result<Vec<model::Repository>, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let (_, state) = opened.load()?;
    Ok(Model::read(&state)?.repositories.into_values().collect())
}

/// The latest operation of run `run` applied in the clone at `tracker`, 0 for none.
pub fn applied(tracker: &Path, run: &str) -> Result<u64, CliError> {
    let (opened, _) = Tracker::open(tracker)?;
    let (_, state) = opened.load()?;
    Ok(state.applied_seq(run)?)
}

/// Runs a parsed command.
pub fn run(cli: &Cli, env: &Env) -> Result<Output, CliError> {
    if let Command::Init { path, replica, remote } = &cli.command {
        return init(&env.cwd.join(path), replica, remote.as_deref()).map(Output::Json);
    }
    if let Command::Prime = &cli.command {
        return Ok(Output::Text(prime()));
    }
    if let Command::Completions { shell } = &cli.command {
        return Ok(Output::Text(completions(*shell)));
    }
    if let Command::Import { schema: true, .. } = &cli.command {
        return Ok(Output::Text(import::SCHEMA.to_owned()));
    }
    if let Command::Serve(_) | Command::Up(_) = &cli.command {
        return Err(CliError::new(
            "no-server",
            exit::CONFIG,
            "this build of acyclic has no server",
        ));
    }
    let session = Session::open(cli, env)?;
    if let Command::Graph { format, renderer, open, root, thread, include_archived, file } =
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
        return session.graph(format, renderer, &scope, file.as_deref(), *open);
    }
    session.dispatch(&cli.command).map(Output::Json)
}

struct Session<'a> {
    tracker: Tracker,
    actor: String,
    now: Millis,
    env: &'a Env,
    /// The run's operation this session applies, if it applies one.
    operation: Option<Operation>,
    /// Whether the operation's number has been recorded.
    recorded: Cell<bool>,
    /// What the operation's command must find to stay within the run's thread.
    within: Vec<Within>,
    /// The operation's command's hash, recorded with its number so that another command sent with the same number is
    /// told apart from the same one sent again.
    hash: String,
}

/// A short hash of a command, as JSON.
fn hash(command: &Command) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let json = serde_json::to_vec(command).unwrap_or_default();
    sha2::Sha256::digest(json).iter().take(8).fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// A run's operation: one command, numbered in the run's sequence from 1, with the run's view of the tracker when it
/// made it. The command's batch checks the number, writes under the run's view, and records the number with it.
#[derive(Debug, Clone)]
pub struct Operation {
    /// The run's id.
    pub run: String,
    /// The thread the run works on, which its operations may not reach outside.
    pub thread: String,
    /// Its number in the run's sequence.
    pub seq: u64,
    /// What the run had seen.
    pub seen: Context,
}

/// The code of the error that says an operation was applied already; its details are the recorded outcome.
pub const DUPLICATE: &str = "duplicate-operation";

impl<'a> Session<'a> {
    fn open(cli: &Cli, env: &'a Env) -> Result<Self, CliError> {
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
            operation: None,
            recorded: Cell::new(false),
            within: Vec::new(),
            hash: String::new(),
        })
    }

    #[expect(clippy::too_many_lines, reason = "one short arm per command")]
    fn dispatch(&self, command: &Command) -> Result<Json, CliError> {
        match command {
            Command::Init { .. }
            | Command::Prime
            | Command::Graph { .. }
            | Command::Import { file: None, .. }
            | Command::Serve(_)
            | Command::Up(_)
            | Command::Completions { .. } => {
                unreachable!("handled before dispatch")
            },
            Command::Ready { include_archived, approved } => {
                self.ready(*include_archived, *approved)
            },
            Command::List => self.list(),
            Command::Show { items } => self.show(items),
            Command::Needs => self.needs(),
            Command::Sync { with_usage } => self.sync(*with_usage),
            Command::Doctor => self.doctor(),
            Command::Item(ItemCommand::Add {
                item,
                title,
                parent,
                thread,
                state,
                notes,
                gate,
                size,
                area,
                resource,
                repository,
            }) => {
                let fields = ItemFields {
                    title: Some(title.clone()),
                    parent: Change::of(parent.clone(), false),
                    thread: Change::of(thread.clone(), false),
                    state: state.map(StateWord::state),
                    notes: Change::of(notes.clone(), false),
                    gate: Change::of(gate.clone(), false),
                    size: Change::of(*size, false),
                    areas: Delta { add: area.clone(), remove: Vec::new() },
                    resources: Delta { add: resource.clone(), remove: Vec::new() },
                    repositories: Delta { add: repository.clone(), remove: Vec::new() },
                };
                self.write(|replica, by| Ok(edit::add_item(replica, item, &fields, by)?.item(item)))
                    .map_err(|error| {
                        self.misplaced(error, parent.as_deref(), || {
                            format!(
                                "acyclic item add {item} --title <TITLE> --thread {}",
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
                    gate: Change::of(edit.gate.clone(), edit.no_gate),
                    size: Change::of(edit.size, edit.no_size),
                    areas: Delta { add: edit.area.clone(), remove: edit.no_area.clone() },
                    resources: Delta {
                        add: edit.resource.clone(),
                        remove: edit.no_resource.clone(),
                    },
                    repositories: Delta {
                        add: edit.repository.clone(),
                        remove: edit.no_repository.clone(),
                    },
                };
                let item = &edit.item;
                self.write(
                    |replica, by| Ok(edit::edit_item(replica, item, &fields, by)?.item(item)),
                )
                .map_err(|error| {
                    self.misplaced(error, edit.parent.as_deref(), || {
                        format!(
                            "acyclic item edit {item} --thread {}",
                            edit.parent.as_deref().unwrap_or_default()
                        )
                    })
                })
            },
            Command::Link(LinkCommand::Add { from, kind: word, to }) => {
                self.write(|replica, by| {
                    let kind = word.kind();
                    let key = model::link_key(from, to, kind)?;
                    if replica.state().field(&key, link_field::PRESENT).is_none() {
                        model::link(replica, from, to, kind, by)?;
                    }
                    Ok(json!({"from": from, "kind": word, "to": to}))
                })
            },
            Command::Link(LinkCommand::Remove { from, kind: word, to }) => {
                self.write(|replica, by| {
                    model::unlink(replica, from, to, word.kind(), by)?;
                    Ok(json!({"from": from, "kind": word, "to": to}))
                })
            },
            Command::Done { item: id, repository, commit, note, force } => {
                let ancestry = GitAncestry::new(context::clones(self.env)?);
                self.write(|replica, by| {
                    let key = existing(replica, id)?;
                    if !force {
                        check_finish(replica, id, by, &ancestry)?;
                    }
                    if repository.is_some() || note.is_some() {
                        let evidence = Evidence {
                            repository: repository.clone(),
                            commit: commit.clone(),
                            note: note.clone(),
                        };
                        let model = Model::read(replica.state())?;
                        check_evidence(&model, &evidence)?;
                        if !force && overwrites(&model.items[id], &evidence) {
                            return Err(CliError::refused(
                                "already-done",
                                format!("item {id} is done with other evidence; pass --force to replace it"),
                            )
                            .with(json!({"evidence": view::evidence(&model.items[id].evidence)})));
                        }
                        set_if(replica, &key, field::EVIDENCE, Some(&evidence), by)?;
                    }
                    model::set_state(replica, id, ItemState::Done, by)?;
                    // The evidence it holds now, so a caller sees what was recorded.
                    let model = Model::read(replica.state())?;
                    let evidence = view::evidence(&model.items[id].evidence);
                    Ok(json!({"item": id, "state": ItemState::Done, "evidence": evidence}))
                })
            },
            Command::Release { thread } => self.write(|replica, by| {
                let key = existing_thread(replica, thread)?;
                Ok(json!({"thread": thread, "released": claim::release(replica, &key, by.actor)}))
            }),
            Command::Item(ItemCommand::Reopen { item: id }) => self.write(|replica, by| {
                archive::reopen(replica, id, by)?;
                Ok(json!({"item": id}))
            }),
            Command::Claim { thread, lease, force } => self.claim(thread, *lease, *force),
            Command::Thread(ThreadCommand::List) => self.threads(),
            Command::Thread(ThreadCommand::Show { threads }) => self.show_threads(threads),
            Command::Thread(ThreadCommand::Add { thread, title, brief, repository, level }) => {
                let fields = ThreadFields {
                    title: Some(title.clone()),
                    brief: Change::of(brief.clone(), false),
                    level: Change::of(*level, false),
                    pm: Change::Keep,
                    repositories: Delta { add: repository.clone(), remove: Vec::new() },
                };
                self.write(|replica, by| {
                    Ok(edit::add_thread(replica, thread, &fields, by)?.thread(thread))
                })
            },
            Command::Thread(ThreadCommand::Edit(edit)) => {
                let fields = ThreadFields {
                    title: edit.title.clone(),
                    brief: Change::of(edit.brief.clone(), edit.no_brief),
                    level: Change::of(edit.level, edit.no_level),
                    pm: Change::of(edit.pm.then_some(true), edit.no_pm),
                    repositories: Delta {
                        add: edit.repository.clone(),
                        remove: edit.no_repository.clone(),
                    },
                };
                let thread = &edit.thread;
                self.write(|replica, by| {
                    Ok(edit::edit_thread(replica, thread, &fields, by)?.thread(thread))
                })
            },
            Command::Repo(RepoCommand::List) => self.repositories(),
            Command::Repo(RepoCommand::Show { repositories }) => {
                self.show_repositories(repositories)
            },
            Command::Repo(RepoCommand::Add {
                repository,
                owner,
                name,
                default_branch,
                public,
                ..
            }) => {
                let fields = RepositoryFields {
                    owner: Some(owner.clone()),
                    name: Some(name.clone()),
                    default_branch: Some(default_branch.clone()),
                    public: Some(*public),
                };
                self.write(|replica, by| {
                    edit::add_repository(replica, repository, &fields, by)?;
                    Ok(json!({"repository": repository}))
                })
            },
            Command::Repo(RepoCommand::Edit {
                repository,
                owner,
                name,
                default_branch,
                public,
                private,
            }) => {
                let fields = RepositoryFields {
                    owner: owner.clone(),
                    name: name.clone(),
                    default_branch: default_branch.clone(),
                    public: (*public || *private).then_some(*public),
                };
                self.write(|replica, by| {
                    edit::edit_repository(replica, repository, &fields, by)?;
                    Ok(json!({"repository": repository}))
                })
            },
            Command::Proposal(ProposalCommand::List) => self.proposal_list(),
            Command::Proposal(ProposalCommand::Show { proposals }) => {
                self.show_proposals(proposals)
            },
            Command::Proposal(ProposalCommand::Keep { proposal }) => {
                self.decide(proposal, scope::keep, "keep")
            },
            Command::Proposal(ProposalCommand::Drop { proposal }) => {
                self.decide(proposal, scope::drop, "drop")
            },
            Command::Thread(ThreadCommand::Approve { thread: id }) => self.approve(id, true),
            Command::Thread(ThreadCommand::Revoke { thread: id }) => self.approve(id, false),
            Command::Level { level: None } => self.levels(),
            Command::Level { level: Some(level) } => self.write(|replica, by| {
                let key = Key::new(kind::SETTINGS, GLOBAL)?;
                set_if(replica, &key, settings::AUTONOMY, Some(level), by)?;
                Ok(json!({"level": level}))
            }),
            Command::Import { file: Some(file), dry_run, .. } => self.import(file, *dry_run),
            Command::Item(ItemCommand::Delete { item }) => self.write(|replica, by| {
                delete::delete_item(replica, item, by)?;
                Ok(json!({"item": item}))
            }),
            Command::Item(ItemCommand::Archive { item: id }) => {
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

    /// Records the operation's number alone, in a batch that checks it again.
    fn record(&self, refusal: Option<&Json>) -> Result<(), CliError> {
        let Some(op) = &self.operation else { return Ok(()) };
        let meta = Meta { actor: self.actor.clone(), run: Some((op.run.clone(), op.seq)) };
        self.tracker.batch(&meta, self.now, false, &project_root, |replica, at| {
            next(replica, op, &self.hash)?;
            Ok::<_, CliError>(replica.record(&op.run, op.seq, &self.hash, refusal, at)?)
        })?;
        Ok(())
    }

    /// Runs one batch as the actor. Sync's lease renewal rides along; `changed` says whether the command itself
    /// changed anything.
    fn write(
        &self,
        apply: impl Fn(&mut Replica, By<'_>) -> Result<Json, CliError>,
    ) -> Result<Json, CliError> {
        let result = RefCell::new((Json::Null, false));
        let meta = Meta {
            actor: self.actor.clone(),
            run: self.operation.as_ref().map(|op| (op.run.clone(), op.seq)),
        };
        let operation = self.operation.as_ref();
        self.tracker.batch(&meta, self.now, false, &project_root, |replica, at| {
            if let Some(op) = operation {
                next(replica, op, &self.hash)?;
                // Checked again here, on the state the write goes into: a run ended since its request arrived writes
                // nothing more.
                if !model::run_active(replica.state(), &op.run, &op.thread) {
                    return Err(CliError::refused(
                        "inactive",
                        format!("run {} isn't under way on thread {}", op.run, op.thread),
                    ));
                }
                check_within(replica, &self.within, &op.thread, &self.actor)?;
            }
            claim::renew_leases(replica, at, LEASE, MAX_LEASE)?;
            acyclic_core::probe::mark("batch: renew leases");
            let before = replica.changes();
            let seen = operation.map(|op| &op.seen);
            let data = apply(replica, By { actor: &self.actor, at, seen })?;
            *result.borrow_mut() = (data, replica.changes() != before);
            if let Some(op) = operation {
                replica.record(&op.run, op.seq, &self.hash, None, at)?;
            }
            Ok(())
        })?;
        self.recorded.set(operation.is_some());
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
            Ok((_, model)) if model.threads.contains_key(named) => {
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

    /// Decides a proposal, and says which member and thread the edit named, and whether that member's work is
    /// approved afterwards: a kept membership edit into an approved thread extends the approval.
    fn decide(
        &self,
        proposal: &str,
        decide: fn(&mut Replica, &str, By<'_>) -> Result<(), scope::ScopeError>,
        decision: &str,
    ) -> Result<Json, CliError> {
        self.write(|replica, by| {
            let edit = scope::proposals(replica.state())?
                .get(proposal)
                .and_then(|p| p.edit.one().cloned());
            decide(replica, proposal, by)?;
            let (item, thread) = match edit {
                Some(scope::ScopeEdit::Member { item, thread }) => (Some(item), thread),
                _ => (None, None),
            };
            let approved = match &item {
                Some(item) => view::approval(&Model::read(replica.state())?, item).1,
                None => false,
            };
            Ok(json!({"proposal": proposal, "decision": decision, "item": item, "thread": thread,
                      "approved": approved}))
        })
    }

    /// Approves a thread's scope, or revokes the approval, naming the thread by its title too, so a wrong id shows. A
    /// thread only its items name, or whose title is in conflict, has none.
    fn approve(&self, id: &str, approved: bool) -> Result<Json, CliError> {
        self.write(|replica, by| {
            scope::approve(replica, id, approved, by)?;
            let model = Model::read(replica.state())?;
            let title = model.threads.get(id).and_then(|thread| thread.title.one());
            Ok(json!({"thread": id, "title": title, "approved": approved}))
        })
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

    fn ready(&self, include_archived: bool, only_approved: bool) -> Result<Json, CliError> {
        self.with_view(|view| {
            let ready = view.readiness.ready(include_archived, &self.actor)?;
            acyclic_core::probe::mark("readiness");
            Ok(json!({"ready": view.ready_rows(&ready, only_approved)?}))
        })
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
                "an HTML page is too large for standard output: pass --file <FILE> or --open",
            ));
        }
        let (_, model) = self.read()?;
        let view = export::view(&model, scope)
            .map_err(|err| CliError::new("not-found", exit::NOT_FOUND, err.to_string()))?;
        let document = match (format, renderer) {
            (GraphFormat::Dot, _) => export::dot_with_legend(&model, &view),
            // The page draws its own legend.
            (GraphFormat::Html, Renderer::Graphviz) => export::dot(&model, &view),
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
            "counts": {
                "items": view.items.len(),
                "ghosts": view.ghosts.len(),
                "edges": view.edges.len(),
            },
        })))
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
            let mut shown = Vec::with_capacity(ids.len());
            for id in ids {
                let item = view.model.items.get(id).ok_or_else(|| not_found(id))?;
                shown.push(view.item(id, item)?);
            }
            Ok(Json::Array(shown))
        })
    }

    fn threads(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        Ok(json!({"threads": view::thread_rows(&model)}))
    }

    fn show_threads(&self, ids: &[String]) -> Result<Json, CliError> {
        self.with_view(|view| {
            let known = thread_members(view.model);
            let mut shown = Vec::with_capacity(ids.len());
            for id in ids {
                let Some(listed) = known.get(id.as_str()) else {
                    return Err(CliError::new(
                        "not-found",
                        exit::NOT_FOUND,
                        format!("no thread {id}; `acyclic thread list` lists them"),
                    ));
                };
                shown.push(view.thread(id, listed)?);
            }
            Ok(Json::Array(shown))
        })
    }

    fn levels(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let own: Vec<Json> = model
            .threads
            .iter()
            .filter(|(_, thread)| thread.autonomy != Value::None)
            .map(|(id, _)| json!({"id": id, "level": model.level(Some(id))}))
            .collect();
        Ok(json!({"global": model.level(None), "threads": own}))
    }

    fn repositories(&self) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let records: Vec<Json> = model.repositories.values().map(view::repo_row).collect();
        Ok(json!({"repositories": records}))
    }

    fn show_repositories(&self, ids: &[String]) -> Result<Json, CliError> {
        let (_, model) = self.read()?;
        let mut shown = Vec::with_capacity(ids.len());
        for id in ids {
            let record = model.repositories.get(id).ok_or_else(|| {
                CliError::new(
                    "not-found",
                    exit::NOT_FOUND,
                    format!("no repository record {id}; `acyclic repo list` lists them"),
                )
            })?;
            shown.push(view::repo_show(&model, record));
        }
        Ok(Json::Array(shown))
    }

    fn proposal_list(&self) -> Result<Json, CliError> {
        self.with_view(|view| Ok(json!({"proposals": view.proposals()?})))
    }

    fn show_proposals(&self, ids: &[String]) -> Result<Json, CliError> {
        let (state, model) = self.read()?;
        let all = scope::proposals(&state)?;
        let mut shown = Vec::with_capacity(ids.len());
        for id in ids {
            let proposal = all.get(id).ok_or_else(|| {
                CliError::new(
                    "not-found",
                    exit::NOT_FOUND,
                    format!("no proposal {id}; `acyclic proposal list` lists them"),
                )
            })?;
            shown.push(view::proposal_show(&model, proposal));
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

    fn sync(&self, with_usage: bool) -> Result<Json, CliError> {
        let remote = self.remote()?;
        let synced = self.tracker.sync_with(remote, store::Fetch { usage: with_usage })?;
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
        let keys: Vec<String> = doc.items.iter().map(|item| item.key.clone()).collect();
        if dry_run {
            let (_, state) = self.tracker.load()?;
            let mut replica = Replica::open(self.tracker.replica().clone(), state);
            let at = replica.stamp(self.now, false)?;
            let by = By { actor: &self.actor, at, seen: None };
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

    fn claim(&self, thread: &str, lease: Option<u32>, force: bool) -> Result<Json, CliError> {
        let is_run = self.actor.starts_with("run/");
        let lease = match (lease, is_run) {
            (None, true) => None,
            (None, false) => Some(LEASE),
            (Some(minutes), _) => Some(Millis::from(minutes) * MINUTE),
        };
        let clones = context::clones(self.env)?;
        let ancestry = GitAncestry::new(clones);
        self.write(|replica, by| {
            let key = existing_thread(replica, thread)?;
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
            let model = Model::read(replica.state())?;
            let alive = |holder: &str| model.run_alive(holder);
            let held = claim::winner_with(replica.state(), &key, by.at, &alive)?;
            Ok(json!({"thread": thread, "claim": held.as_ref().map(view::claim_json)}))
        })
    }
}

/// Whether `actor` may work on an item in `status` without forcing: when it's ready, or when what stands in the way is
/// the actor's own claim on its thread, as winner of a contest, or on the thread holding a resource it needs.
pub(crate) fn may_claim(status: Option<&Status>, actor: &str) -> bool {
    match status {
        Some(Status::Ready) => true,
        Some(Status::Claimed { holder } | Status::ResourceBusy { holder, .. }) => holder == actor,
        Some(Status::Contested { holders }) => holders.first().is_some_and(|h| h == actor),
        _ => false,
    }
}

/// Refuses evidence that could never verify, without asking the network: the commit must look like an object id and
/// name a complete repository record.
pub(crate) fn check_evidence(model: &Model, evidence: &Evidence) -> Result<(), CliError> {
    let looks_like_object_id = |commit: &String| {
        (7..=64).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit())
    };
    if evidence.commit.as_ref().is_some_and(|commit| !looks_like_object_id(commit)) {
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
            format!("no repository record {id}; `acyclic repo list` lists them"),
        ));
    };
    let missing: Vec<&str> = [
        ("owner", record.owner.one().is_some()),
        ("name", record.name.one().is_some()),
        ("public", record.public.one().is_some()),
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
            "repository record {id} is missing {}; set it with `acyclic repo edit {id}`",
            missing.join(", ")
        ),
    )
    .with(json!({"missing": missing.iter().map(|f| f.replace('-', "_")).collect::<Vec<_>>()})))
}

/// Whether recording `evidence` would replace different evidence on an item that is already done.
pub(crate) fn overwrites(item: &model::Item, evidence: &Evidence) -> bool {
    item.is_done() && !item.evidence.all().is_empty() && item.evidence.one() != Some(evidence)
}

/// Refuses to let an agent finish an item that isn't ready or that someone else holds; the user may finish anything.
/// A finished item may be finished again, which changes nothing.
fn check_finish(
    replica: &Replica,
    id: &str,
    by: By<'_>,
    ancestry: &dyn Ancestry,
) -> Result<(), CliError> {
    if by.actor == scope::USER {
        return Ok(());
    }
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
            format!("item {id} can't be finished yet: it has open children or is held back; `acyclic show {id}` says which"),
        )
        .with(json!({"readiness": status, "open": open, "held": held})));
    }
    Err(CliError::refused(
        "not-ready",
        format!("item {id} isn't yours to finish: it isn't ready, or someone else holds it; `acyclic show {id}` says which"),
    )
    .with(json!({"readiness": status})))
}

/// Whether an agent may finish an item in `status` without forcing: when it may claim it, or it is finished already.
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
    for link in model.links.iter().filter(|link| link.kind == LinkKind::Blocks) {
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

pub(crate) fn not_found(id: &str) -> CliError {
    CliError::new("not-found", exit::NOT_FOUND, format!("no item {id}"))
}

/// An item's key, refusing one that doesn't exist or was deleted.
pub(crate) fn existing(replica: &Replica, id: &str) -> Result<Key, CliError> {
    Ok(model::existing(replica.state(), id)?)
}

/// A thread's key, when the thread has a record; naming an item in a thread says which thread was meant.
pub(crate) fn existing_thread(replica: &Replica, thread: &str) -> Result<Key, CliError> {
    let model = Model::read(replica.state())?;
    if !model.threads.contains_key(thread) {
        let error: CliError = acyclic_core::state::Error::NoThread(thread.to_owned()).into();
        return Err(match model.thread_of(thread) {
            Some(meant) => error.with(json!({"thread": meant})),
            None => error,
        });
    }
    Ok(claim::thread_key(thread)?)
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

/// Adds or removes a set element unless it already is so.
pub(crate) fn element_if(
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

/// The items that joined a thread passed down to them, or asked to, as `{item, thread, outcome, proposal}`: applied at
/// once, or proposed when the thread's scope is approved and the actor may only propose.
pub(crate) fn placed(inherited: &[place::Inherited]) -> Json {
    inherited
        .iter()
        .map(|p| {
            json!({"item": p.item, "thread": p.thread,
                   "outcome": if p.proposal.is_some() { "proposed" } else { "applied" },
                   "proposal": p.proposal})
        })
        .collect()
}

/// An import's result: what it changed, in counts and field by field, the proposals it left and the threads items
/// joined. A dry run changes nothing.
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
        "proposals": crate::edit::all_proposals(&report.proposals, inherited),
        "placed": placed(inherited),
    });
    if dry_run {
        data["changed"] = json!(false);
    }
    data
}
