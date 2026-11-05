//! State stored in a tracker repository: each replica's state ref holds its whole state as a tree, and its audit log
//! keeps the events behind its batches, one ref per month.
//!
//! A batch reads the replica's state, applies a pure closure, writes the changed files and the day's events as one
//! pack, and moves the state ref and the month's log ref in one ref transaction with compare-and-swap, so both move or
//! neither does. Writers queue on a lock, so a lost race means something outside acyclic moved a ref, and the batch
//! fails. Only plumbing is used; nothing is checked out, and no `git` program is run: the repository is read and
//! written through `gix`.
//!
//! Reads come from a cache of the parsed state kept beside the repository for the state ref's current tip, and the
//! store repacks itself once packs pile up, so the repository's share of a command stays small and steady; parsing and
//! rewriting the state still grow with the tracker. The repack is `gc` with `--auto`'s conditions, started by the
//! command that crossed them and run beside it (see [`Maintenance`]); one that began in the last 30 seconds keeps the
//! next from starting, so a repack that fails is retried at that pace.

mod cache;
mod git;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value as Json;

use crate::state::{self, Dot, Journal, Key, Millis, Replica, ReplicaId, State};
use crate::time::civil;
use cache::{Cache, Found};
pub(crate) use git::Git;
use git::{NewCommit, PushRef, Refused};

/// What `acyclic init` needs from a repository, without a `git` process.
pub mod setup {
    use std::path::{Path, PathBuf};

    use super::{Git, StoreError};

    /// Whether `path` is inside a repository.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        path.exists() && Git::new(path).handle().is_ok()
    }

    /// The value of a configuration key, from every source: the last one.
    ///
    /// # Errors
    ///
    /// Fails when `path` isn't a repository.
    pub fn config_value(path: &Path, key: &str) -> Result<Option<String>, StoreError> {
        Git::new(path).config_value(key)
    }

    /// The URL of the remote `name`, with `insteadOf` rules applied, when it exists.
    ///
    /// # Errors
    ///
    /// Fails when `path` isn't a repository.
    pub fn remote_url(path: &Path, name: &str) -> Result<Option<String>, StoreError> {
        Git::new(path).remote_url(name)
    }

    /// Points the remote `name` at `url`, adding it when missing; whether that changed anything.
    ///
    /// # Errors
    ///
    /// Fails when the configuration can't be written.
    pub fn set_remote_url(path: &Path, name: &str, url: &str) -> Result<bool, StoreError> {
        let git = Git::new(path);
        if git.remote_url(name)?.as_deref() == Some(url) {
            return Ok(false);
        }
        git.set_remote_url(name, url)?;
        Ok(true)
    }

    /// Creates a repository with a work tree at `path`.
    ///
    /// # Errors
    ///
    /// Fails when the repository can't be created.
    pub fn init_repository(path: &Path) -> Result<(), StoreError> {
        super::git::init(path)
    }

    /// Whether `remote` has the ref `reference`, asked from the repository at `path`.
    ///
    /// # Errors
    ///
    /// Fails when the remote can't be reached.
    pub fn remote_has_ref(path: &Path, remote: &str, reference: &str) -> Result<bool, StoreError> {
        let patterns = [reference.to_owned()];
        let found = if Git::new(path).handle().is_ok() {
            Git::new(path).list_remote(remote, &patterns)?
        } else {
            super::git::list_remote_anywhere(path, remote, &patterns)?
        };
        Ok(found.contains_key(reference))
    }

    /// The work tree, or else the directory, of the repository that contains `path`, when there is one.
    #[must_use]
    pub fn enclosing_repository(path: &Path) -> Option<PathBuf> {
        let git = Git::new(path);
        git.toplevel().ok().or_else(|| git.git_dir().ok())
    }
}

/// The git config key holding this clone's replica id.
pub const REPLICA_CONFIG: &str = "acyclic.replica";

/// The audit-log file for items in no project.
pub const NO_PROJECT: &str = "_";

/// Why a store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// A git command failed.
    Git(String),
    /// Something moved a ref between its read and its update, though writers hold the batch lock.
    Conflict,
    /// A ref lock is held. A lock on this replica's own refs that is a minute old is a dead writer's and is removed by
    /// the next writer; any other is left alone, since the process holding it could still finish.
    Locked {
        /// The lock file.
        path: PathBuf,
        /// How old it is, when known.
        age_seconds: Option<u64>,
    },
    /// The state layer refused.
    State(state::Error),
    /// The tracker isn't set up.
    Config(String),
    /// The remote's state ref moved since this replica last pushed or fetched it: fetch and join, then push again.
    RemoteMoved,
    /// The remote's copy of this replica's state ref moved since this clone last synced: another clone writes as this
    /// replica.
    Duplicate(ReplicaId),
    /// The remote refused a ref, so the atomic push moved nothing.
    Rejected {
        /// The refused ref.
        reference: String,
        /// Why, as git reports it.
        reason: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(message) => write!(f, "git failed: {message}"),
            Self::Conflict => write!(f, "something outside the batch lock moved the ref first"),
            Self::Locked { path, age_seconds } => {
                write!(f, "a ref is locked by {}", path.display())?;
                if let Some(age) = age_seconds {
                    write!(f, " ({age} s old)")?;
                }
                write!(f, "; if nothing holds it, delete that file")
            },
            Self::State(err) => err.fmt(f),
            Self::Config(message) => write!(f, "tracker not set up: {message}"),
            Self::RemoteMoved => {
                write!(
                    f,
                    "the remote's state ref moved since the last sync; run `acyclic sync` first"
                )
            },
            Self::Duplicate(replica) => write!(
                f,
                "the remote's state of replica {replica} holds writes this clone never made: another clone writes as \
                 {replica}, and only one clone may use a replica's id. Nothing was joined or pushed; don't retry, \
                 tell the user"
            ),
            Self::Rejected { reference, reason } => {
                write!(f, "the remote refused {reference}: {reason}")
            },
        }
    }
}

impl std::error::Error for StoreError {}

impl From<state::Error> for StoreError {
    fn from(err: state::Error) -> Self {
        Self::State(err)
    }
}

/// Why a batch failed: the store, or the batch's own writes.
#[derive(Debug)]
pub enum BatchError<E> {
    /// The store failed or refused.
    Store(StoreError),
    /// The batch's writes refused.
    Apply(E),
}

impl<E> From<StoreError> for BatchError<E> {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl<E: fmt::Display> fmt::Display for BatchError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(err) => err.fmt(f),
            Self::Apply(err) => err.fmt(f),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for BatchError<E> {}

/// Who a batch is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// The actor behind the batch, such as `actor/user` or `session/mac`.
    pub actor: String,
}

/// Maps an entity to its project root's id, or `None` for no project.
pub type RootOf<'a> = &'a dyn Fn(&State, &Key) -> Option<String>;

/// Failpoints for crash tests.
#[doc(hidden)]
#[derive(Default)]
pub struct Hooks {
    /// Runs after the transaction is prepared, before it commits.
    pub after_prepare: Option<Box<dyn Fn()>>,
}

/// What a batch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The closure changed nothing.
    Unchanged,
    /// Committed as this state commit.
    Committed {
        /// The new state commit.
        commit: String,
    },
}

/// A tracker repository opened as one replica.
#[derive(Debug)]
pub struct Tracker {
    git: Git,
    replica: ReplicaId,
    /// Whether the id is still marked unused; cleared after the first committed batch.
    unused: AtomicBool,
    /// The state ref's tip as `open` found it, which the first read reuses rather than resolving it again. Taking
    /// the batch lock clears it, so a write always starts from a fresh tip; a read using it sees the state as of
    /// opening.
    opened: std::sync::Mutex<Option<OpenedTip>>,
    /// The clone's `gc.auto` and `gc.autoPackLimit`, or git's defaults: when to repack.
    upkeep: Upkeep,
    /// Whether the clone has a remote named [`REMOTE`].
    origin: bool,
    /// Whether every write ends with a push: `acyclic.pushAfterWrite` in the clone's git config.
    push_after_write: bool,
    /// How the repack runs once it is due.
    maintenance: Maintenance,
}

/// How the store repacks itself once git's thresholds are crossed. `git gc --auto` detached into the background and
/// outlived the command; a repack inside the command would make it wait, once in dozens of writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maintenance {
    /// Repack before the command returns: simple and deterministic, but the command waits for it.
    Inline,
    /// Repack on a thread and go on, for a process that lives on, such as the server.
    Thread,
    /// Repack in a process of its own that outlives the command, for the command line. The binary must call
    /// [`run_maintenance_child`] first thing in `main`; a run began less than 30 seconds ago keeps the next from starting.
    Process,
}

static MAINTENANCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn default_maintenance() -> Maintenance {
    match MAINTENANCE.load(Ordering::Relaxed) {
        1 => Maintenance::Thread,
        2 => Maintenance::Process,
        _ => Maintenance::Inline,
    }
}

/// Sets how trackers opened from now on repack: the process's choice, made once at startup.
pub fn set_maintenance(how: Maintenance) {
    MAINTENANCE.store(
        match how {
            Maintenance::Inline => 0,
            Maintenance::Thread => 1,
            Maintenance::Process => 2,
        },
        Ordering::Relaxed,
    );
}

/// Runs the repack that a parent started with [`Maintenance::Process`], when this process was started for it: call
/// this first thing in `main` and return when it says so.
#[must_use]
pub fn run_maintenance_child() -> bool {
    git::maintenance_child()
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    /// The replicas whose state refs the remote holds, this one's included once it has pushed.
    pub fetched: Vec<String>,
    /// Whether joining them changed this replica's state.
    pub joined: bool,
    /// How many refs the push updated or deleted.
    pub pushed: usize,
}

/// The remote a tracker clone syncs with.
pub const REMOTE: &str = "origin";

/// The git config key that makes every write end with a push, as git lists it: in lower case.
pub const PUSH_AFTER_WRITE_CONFIG: &str = "acyclic.pushafterwrite";

#[derive(Debug, Clone, Copy)]
struct Upkeep {
    loose: usize,
    packs: usize,
}

/// The state ref's tip when the tracker was opened: `None` when it had no state yet.
#[derive(Debug)]
struct OpenedTip(Option<String>);

/// The git config key naming a new replica id that hasn't written yet.
pub const UNUSED_CONFIG: &str = "acyclic.replica-unused";

/// Sets up the tracker clone at `path` as a new replica named `id`, marked unused until its first write.
///
/// # Errors
///
/// Fails when git fails.
pub fn init_replica(path: &Path, id: &ReplicaId) -> Result<(), StoreError> {
    Git::new(path).set_config(&[(REPLICA_CONFIG, id.as_str()), (UNUSED_CONFIG, id.as_str())])
}

/// Six hex digits that differ between clones and runs.
fn random_suffix() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hasher.write_u32(std::process::id());
    format!("{:06x}", hasher.finish() & 0x00ff_ffff)
}

/// The ref holding a replica's state.
#[must_use]
pub fn state_ref(replica: &ReplicaId) -> String {
    format!("refs/heads/state/{replica}")
}

/// The ref of a log segment: `refs/heads/<namespace>/<replica>/<yyyy-mm>/<generation>`.
#[must_use]
pub fn segment_ref(namespace: &str, replica: &ReplicaId, month: &str, generation: u32) -> String {
    format!("refs/heads/{namespace}/{replica}/{month}/{generation:04}")
}

/// The calendar month of a timestamp, as `yyyy-mm` in UTC.
#[must_use]
pub fn month_of(at: Millis) -> String {
    let (year, month, _) = civil(at);
    format!("{year:04}-{month:02}")
}

/// The UTC day of the month for a time, as two digits.
fn day_of(at: Millis) -> String {
    format!("{:02}", civil(at).2)
}

/// A log file's path in its month's segment: a directory per project root and a file per day, so a batch appends
/// to one day's events rather than rewriting the month's.
fn log_path(root: &str, day: &str) -> String {
    format!("{root}/{day}.jsonl")
}

/// The scratch refs `git fast-import` leaves a batch's commits on until its transaction moves the real refs; only the
/// holder of the batch lock writes them. They are never deleted in a transaction, since deleting a packed ref takes
/// the lock a background repack may hold. The next batch overwrites them, and prune and squash drop them so they
/// don't keep what those make unreachable.
const SCRATCH_STATE: &str = "refs/acyclic/scratch/state";
const SCRATCH_LOG: &str = "refs/acyclic/scratch/log";

/// Changed state files as bytes, for a commit.
fn file_bytes(files: BTreeMap<String, Option<String>>) -> BTreeMap<String, Option<Vec<u8>>> {
    files.into_iter().map(|(path, content)| (path, content.map(String::into_bytes))).collect()
}

#[derive(Serialize)]
struct Event<'a> {
    at: Millis,
    actor: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor_record: Option<Json>,
    writes: Vec<Write>,
}

#[derive(Serialize)]
struct Write {
    kind: String,
    id: String,
    field: String,
    /// The field's values after the batch; empty when cleared.
    values: Vec<WrittenValue>,
}

#[derive(Serialize)]
struct WrittenValue {
    dot: Dot,
    value: Json,
}

/// The changed fields since the journal began, by project root. An entity's root is found in the state after the
/// batch, or failing that in the state before it, which a swap with the journal restores for the moment.
fn diff(
    after: &mut State,
    journal: &mut Journal,
    root_of: RootOf<'_>,
) -> BTreeMap<String, Vec<Write>> {
    let keys: Vec<Key> = journal.keys().cloned().collect();
    let mut roots: Vec<Option<String>> = keys.iter().map(|key| root_of(after, key)).collect();
    if roots.iter().any(Option::is_none) {
        after.swap(journal);
        for (key, root) in keys.iter().zip(&mut roots).filter(|(_, root)| root.is_none()) {
            *root = root_of(after, key);
        }
        after.swap(journal);
    }
    let mut by_root: BTreeMap<String, Vec<Write>> = BTreeMap::new();
    for (key, root) in keys.iter().zip(roots) {
        let old = journal.before(key);
        let new = after.entity(key);
        let fields: BTreeSet<&String> = old
            .into_iter()
            .flat_map(|e| e.keys())
            .chain(new.into_iter().flat_map(|e| e.keys()))
            .collect();
        let root = root.unwrap_or_else(|| NO_PROJECT.to_owned());
        for field in fields {
            let was = old.and_then(|entity| entity.get(field));
            let now = new.and_then(|entity| entity.get(field));
            if was == now {
                continue;
            }
            let values = now
                .into_iter()
                .flatten()
                .map(|(dot, v)| WrittenValue { dot: dot.clone(), value: v.value.clone() })
                .collect();
            by_root.entry(root.clone()).or_default().push(Write {
                kind: key.kind.clone(),
                id: key.id.clone(),
                field: field.clone(),
                values,
            });
        }
    }
    by_root
}

/// The actor's record, when the actor names an entity as `kind/id`, with each field's values.
fn actor_record(state: &State, actor: &str) -> Option<Json> {
    let (kind, id) = actor.split_once('/')?;
    let key = Key::new(kind, id).ok()?;
    let entity = state.entity(&key)?;
    let fields: serde_json::Map<String, Json> = entity
        .iter()
        .map(|(name, register)| {
            (name.clone(), Json::Array(register.values().map(|v| v.value.clone()).collect()))
        })
        .collect();
    Some(Json::Object(fields))
}

impl Tracker {
    /// Opens the tracker at `path` as the replica named in its git config.
    ///
    /// A replica id is only trusted while this clone holds its state ref, or while the id is still marked unused by
    /// [`init_replica`]. Otherwise this clone may be a fresh or rebuilt copy of a replica that wrote before, and
    /// reusing its id could reuse its dots, which peers would silently drop. So it takes a new id, `<id>-<random>`,
    /// records it and reports the change. Whether an id was used is judged only from the state refs under
    /// `refs/heads/state/`, which a sync never fills (it keeps what it fetches under `refs/acyclic/`), so an id
    /// marked unused by [`init_replica`] is trusted even when another clone has written under it.
    ///
    /// # Errors
    ///
    /// Fails when the replica id is missing or invalid, or git fails.
    pub fn open(path: &Path) -> Result<(Self, Option<ReplicaId>), StoreError> {
        let git = Git::new(path);
        // The replica's settings and git's repacking thresholds: the first value in git's order, as `git config
        // --get-regexp` lists them and the first match of each is taken.
        let settings = git
            .config_values(&[
                REPLICA_CONFIG,
                UNUSED_CONFIG,
                PUSH_AFTER_WRITE_CONFIG,
                "gc.auto",
                "gc.autopacklimit",
                "remote.origin.url",
            ])
            // A directory that isn't a repository has no replica id, which is what the error below says.
            .unwrap_or_default();
        let setting = |name: &str| settings.get(name).map(String::as_str);
        let number = |name, default| setting(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        let upkeep =
            Upkeep { loose: number("gc.auto", 6_700), packs: number("gc.autopacklimit", 50) };
        let origin = setting("remote.origin.url").is_some();
        let push_after_write = setting(PUSH_AFTER_WRITE_CONFIG).is_some_and(|v| {
            matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
        });
        let id = setting(REPLICA_CONFIG).ok_or_else(|| {
            StoreError::Config(format!("no {REPLICA_CONFIG} in {}", path.display()))
        })?;
        let id = ReplicaId::new(id)?;
        let unused = setting(UNUSED_CONFIG) == Some(id.as_str());
        crate::probe::mark("open: git config");
        let tip = git.resolve(&state_ref(&id))?;
        crate::probe::mark("open: resolve the state ref");
        let tracker = |git, replica, unused, tip| Self {
            git,
            replica,
            unused: AtomicBool::new(unused),
            opened: std::sync::Mutex::new(Some(OpenedTip(tip))),
            upkeep,
            origin,
            push_after_write,
            maintenance: default_maintenance(),
        };
        if tip.is_some() {
            return Ok((tracker(git, id, false, tip), None));
        }
        let mut remembered = BTreeSet::new();
        for (_, commit) in git.refs("refs/heads/state/")? {
            remembered.extend(seen_replicas(&git, &commit)?);
        }
        if unused && !remembered.contains(&id) {
            return Ok((tracker(git, id, true, None), None));
        }
        let fresh = loop {
            let candidate = ReplicaId::new(&format!("{id}-{}", random_suffix()))?;
            if !remembered.contains(&candidate) {
                break candidate;
            }
        };
        init_replica(path, &fresh)?;
        Ok((tracker(git, fresh.clone(), true, None), Some(fresh)))
    }

    /// Chooses how this tracker repacks, instead of the process's choice.
    #[must_use]
    pub fn with_maintenance(mut self, how: Maintenance) -> Self {
        self.maintenance = how;
        self
    }

    /// This replica's id.
    #[must_use]
    pub fn replica(&self) -> &ReplicaId {
        &self.replica
    }

    /// The directory acyclic keeps its local files in, under the clone's common Git directory, so every worktree of
    /// the clone shares it.
    ///
    /// # Errors
    ///
    /// Fails when Git can't name the common directory.
    pub fn local_dir(&self) -> Result<PathBuf, StoreError> {
        Ok(self.git.common_dir()?.join("acyclic"))
    }

    /// The tip of this replica's state ref and the state it holds.
    ///
    /// # Errors
    ///
    /// Fails when git fails or the state can't be read.
    pub fn load(&self) -> Result<(Option<String>, State), StoreError> {
        let opened = self.opened.lock().ok().and_then(|mut opened| opened.take());
        let tip = match opened {
            Some(OpenedTip(tip)) => tip,
            None => self.git.resolve(&state_ref(&self.replica))?,
        };
        let (tip, state, _) = self.load_at(tip)?;
        Ok((tip, state))
    }

    /// The state at `tip`, from the cache when it holds that tip, and what the cache held then.
    fn load_at(&self, tip: Option<String>) -> Result<(Option<String>, State, Found), StoreError> {
        let Some(tip) = tip else {
            return Ok((None, State::new(), Found::default()));
        };
        let cache = self.cache()?;
        if let Some((state, found)) = cache.read(&tip) {
            return Ok((Some(tip), state, found));
        }
        let files = read_files(&self.git, &tip)?;
        crate::probe::mark("load: git ls-tree and cat-file");
        let state = State::from_own_files(&files)?;
        drop(files);
        crate::probe::mark("load: parse files");
        let found = cache.write(&tip, &state);
        Ok((Some(tip), state, found))
    }

    /// Like [`Tracker::load`], but always from Git and refusing files that aren't in canonical form, as a check of
    /// the stored state should.
    ///
    /// # Errors
    ///
    /// Fails when git fails or the state can't be read.
    pub fn load_strict(&self) -> Result<(Option<String>, State), StoreError> {
        if let Ok(mut opened) = self.opened.lock() {
            opened.take();
        }
        match self.git.resolve(&state_ref(&self.replica))? {
            Some(tip) => Ok((Some(tip.clone()), read_state(&self.git, &tip)?)),
            None => Ok((None, State::new())),
        }
    }

    /// Keeps the object store packed: when git's thresholds are probably crossed (`gc.auto` loose objects, judged as
    /// git does from one of the 256 object directories, or more than `gc.autoPackLimit` packs), runs `git gc --auto`,
    /// which checks them itself and detaches to repack in the background. Costs a directory read or two otherwise;
    /// `gc.auto` of 0 turns it off, as in git. Best effort.
    fn maintain(&self) {
        let Ok(common) = self.git.common_dir() else { return };
        let objects = common.join("objects");
        let count = |dir: &Path, keep: &dyn Fn(&str) -> bool| {
            std::fs::read_dir(dir).map_or(0, |entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.file_name().to_str().is_some_and(keep))
                    .count()
            })
        };
        let loose = count(&objects.join("17"), &|name| {
            name.len() >= 38 && name.bytes().all(|b| b.is_ascii_hexdigit())
        });
        let packs = count(&objects.join("pack"), &|name| {
            Path::new(name).extension().is_some_and(|ext| ext == "pack")
        });
        let Upkeep { loose: max_loose, packs: max_packs } = self.upkeep;
        if max_loose > 0 && (loose * 256 > max_loose || (max_packs > 0 && packs > max_packs)) {
            self.git.auto_maintenance(self.maintenance);
        }
    }

    /// Deletes a scratch ref, best effort: a repack holding the lock only means it lingers until the next try.
    fn drop_scratch(&self, scratch: &str) {
        let _ = self.git.delete_ref(scratch, None);
    }

    fn cache(&self) -> Result<Cache, StoreError> {
        Ok(Cache::new(&self.git.common_dir()?, &self.replica))
    }

    /// Runs a batch: `apply` receives the replica's current state, under the batch lock. `now` is stamped through the
    /// clock guard.
    ///
    /// # Errors
    ///
    /// Fails when the closure or the clock guard refuses, a ref lock is held, git fails, or something outside the
    /// batch lock moved a ref ([`StoreError::Conflict`]).
    pub fn batch<E>(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: impl FnOnce(&mut Replica, Millis) -> Result<(), E>,
    ) -> Result<Outcome, BatchError<E>> {
        self.batch_with(meta, now, allow_clock_step, root_of, apply, &Hooks::default())
    }

    #[doc(hidden)]
    #[allow(clippy::missing_errors_doc)]
    pub fn batch_with<E>(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: impl FnOnce(&mut Replica, Millis) -> Result<(), E>,
        hooks: &Hooks,
    ) -> Result<Outcome, BatchError<E>> {
        crate::probe::mark("before the batch");
        let _lock = self.lock()?;
        // The state's tip and the log's segments in one process.
        let state_name = state_ref(&self.replica);
        crate::probe::mark("batch: lock");
        let log_prefix = format!("refs/heads/log/{}/", self.replica);
        let refs = self.git.refs_under(&[&state_name, &log_prefix])?;
        crate::probe::mark("batch: git for-each-ref");
        let (tip, state, found) = self.load_at(refs.get(&state_name).cloned())?;
        let mut replica = Replica::open(self.replica.clone(), state);
        // Recording starts before the stamp, so a stamp alone is no change.
        replica.state.record();
        let at = replica.stamp(now, allow_clock_step).map_err(StoreError::State)?;
        apply(&mut replica, at).map_err(BatchError::Apply)?;
        crate::probe::mark("batch: apply command");
        let mut state = replica.into_state();
        let mut journal = state.take_journal().expect("the batch records");
        if state.unchanged_since(&journal) {
            return Ok(Outcome::Unchanged);
        }
        let state_files = file_bytes(state.changed_files_since(&journal));
        crate::probe::mark("batch: changed files");
        let event_actor = actor_record(&state, &meta.actor);
        let changes = diff(&mut state, &mut journal, root_of);
        let month = month_of(at);
        let day = day_of(at);
        let (segment, segment_tip) = self.segment_in(&refs, "log", &month);
        let mut files = BTreeMap::new();
        let paths: Vec<String> = changes.keys().map(|root| log_path(root, &day)).collect();
        crate::probe::mark("batch: log diff");
        let existing = match &segment_tip {
            Some(tip) => self.git.read_paths(tip, &paths)?,
            None => BTreeMap::new(),
        };
        crate::probe::mark("batch: git cat-file for the log");
        for (root, writes) in changes {
            let event = Event { at, actor: &meta.actor, actor_record: event_actor.clone(), writes };
            let path = log_path(&root, &day);
            let mut content = existing.get(&path).cloned().unwrap_or_default();
            content.extend(serde_json::to_vec(&event).expect("events serialize"));
            content.push(b'\n');
            files.insert(path, Some(content));
        }
        crate::probe::mark("batch: log events");
        let seconds = at.div_euclid(1_000);
        let commit = |parent, files, message| NewCommit {
            parent,
            files,
            replica: self.replica.as_str(),
            seconds,
            message,
        };
        let ids = self.git.import_commits(&[
            commit(tip.as_deref(), &state_files, "batch"),
            commit(segment_tip.as_deref(), &files, "events"),
        ])?;
        let [commit, log_commit] = [&ids[0], &ids[1]].map(String::clone);
        crate::probe::mark("batch: write the objects");

        let mut transaction = self.git.transaction()?;
        transaction.update(&state_name, &commit, tip.as_deref())?;
        transaction.update(&segment, &log_commit, segment_tip.as_deref())?;
        transaction.commit(hooks.after_prepare.as_deref())?;
        crate::probe::mark("batch: update the refs");
        self.cache()?.write_after(&found, &commit, &state, journal.keys());
        self.maintain();
        crate::probe::mark("batch: upkeep");
        if self.unused.swap(false, Ordering::Relaxed) {
            // Another worktree may have cleared it already; the state ref now vouches for the id either way.
            let _ = self.git.unset_config(UNUSED_CONFIG);
        }
        Ok(Outcome::Committed { commit })
    }

    /// Serializes this clone's writers, across its worktrees and processes, so they queue instead of racing; the
    /// operating system releases the lock when its holder exits, so a crash never leaves it stale. Compare-and-swap
    /// still guards the refs.
    fn lock(&self) -> Result<std::fs::File, StoreError> {
        // A writer starts from a fresh tip, never the one seen at opening.
        if let Ok(mut opened) = self.opened.lock() {
            opened.take();
        }
        let path = self.git.common_dir()?.join("acyclic-batch.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| StoreError::Git(format!("can't open {}: {e}", path.display())))?;
        file.lock().map_err(|e| StoreError::Git(format!("can't lock {}: {e}", path.display())))?;
        // Every writer of this replica's refs holds this lock, so a lock file on them that has been there for a minute
        // is left by a writer that died between locking and committing; `git update-ref` released its locks when its
        // parent died, a process of ours can't.
        let common = self.git.common_dir()?;
        let own = [
            format!("refs/heads/state/{}.lock", self.replica),
            format!("refs/heads/log/{}", self.replica),
            format!("refs/acyclic/pruned/log/{}", self.replica),
        ];
        for entry in own {
            remove_stale_locks(&common.join(entry));
        }
        Ok(file)
    }

    /// This replica's segments in a namespace: for each month, its highest generation's ref and tip. Readers take the
    /// highest generation.
    fn segments(&self, namespace: &str) -> Result<BTreeMap<String, Segment>, StoreError> {
        let prefix = format!("refs/heads/{namespace}/{}/", self.replica);
        let mut segments: BTreeMap<String, Segment> = BTreeMap::new();
        for (name, tip) in self.git.refs(&prefix)? {
            let rest = &name[prefix.len()..];
            let Some((month, generation)) = rest.split_once('/') else { continue };
            let Ok(generation) = generation.parse::<u32>() else { continue };
            let newer = segments.get(month).is_none_or(|s| s.generation < generation);
            if newer {
                segments
                    .insert(month.to_owned(), Segment { reference: name.clone(), generation, tip });
            }
        }
        Ok(segments)
    }

    /// Prunes sealed segments: a project root's file is dropped once every event in it is older than its retention
    /// in months (with one month's retention, September's file goes on 1 November), but only where `archived_ok`
    /// confirms the archived subtrees it covers have their records. A segment is replaced by the next generation, a
    /// parentless commit without the dropped files, so the old events become unreachable; a segment left empty is
    /// deleted. Every replacement happens in one transaction, which also records each deleted segment for the next
    /// push to delete from the remote.
    ///
    /// # Errors
    ///
    /// Fails on a lock, a lost race or a git failure; nothing changes then.
    pub fn prune(
        &self,
        now: Millis,
        retention: &dyn Fn(&str) -> Option<u32>,
        archived_ok: &dyn Fn(&str, &str) -> bool,
    ) -> Result<Vec<String>, StoreError> {
        let _lock = self.lock()?;
        let current = month_of(now);
        let mut transaction = None;
        let mut dropped = Vec::new();
        for namespace in NAMESPACES {
            for (month, segment) in
                self.segments(namespace)?.into_iter().filter(|(month, _)| *month < current)
            {
                let files = self.git.read_tree(&segment.tip)?;
                let expired: Vec<&String> = files
                    .keys()
                    .filter(|path| {
                        let root = path.split_once('/').map_or(path.as_str(), |(root, _)| root);
                        retention(root)
                            .is_some_and(|months| months_between(&month, &current) > months)
                            && archived_ok(root, &month)
                    })
                    .collect();
                if expired.is_empty() {
                    continue;
                }
                let transaction = match &mut transaction {
                    Some(t) => t,
                    None => transaction.insert(self.git.transaction()?),
                };
                let kept: BTreeMap<String, Option<Vec<u8>>> = files
                    .iter()
                    .filter(|(path, _)| !expired.contains(path))
                    .map(|(path, content)| (path.clone(), Some(content.clone())))
                    .collect();
                if !kept.is_empty() {
                    let tree = self.git.write_tree(None, &kept)?;
                    let seconds = self.git.commit_seconds(&segment.tip)?;
                    let commit =
                        self.git.commit(&tree, None, self.replica.as_str(), seconds, "prune")?;
                    let next =
                        segment_ref(namespace, &self.replica, &month, segment.generation + 1);
                    transaction.update(&next, &commit, None)?;
                }
                transaction.delete(&segment.reference, &segment.tip)?;
                transaction.update(&pruned_ref(&segment.reference), &segment.tip, None)?;
                dropped.extend(expired.iter().map(|path| format!("{}:{path}", segment.reference)));
            }
        }
        if let Some(transaction) = transaction {
            transaction.commit(None)?;
            self.drop_scratch(SCRATCH_LOG);
        }
        Ok(dropped)
    }

    /// Fetches every replica's state ref from `remote` into private fetched refs, ready to join; this
    /// replica's own one is what the next push leases against. This replica's own segments come along, so a sync
    /// knows what its push must send without asking the remote again.
    ///
    /// # Errors
    ///
    /// Fails when git fails.
    pub fn fetch(&self, remote: &str) -> Result<(), StoreError> {
        let base = remote_base(remote);
        let mut specs = vec![format!("+refs/heads/state/*:{base}state/*")];
        for prefix in self.segment_prefixes() {
            let path = &prefix["refs/heads/".len()..];
            specs.push(format!("+{prefix}*:{base}{path}*"));
        }
        self.git.fetch(remote, &specs)
    }

    /// The prefixes of this replica's log segments.
    fn segment_prefixes(&self) -> Vec<String> {
        NAMESPACES.iter().map(|ns| format!("refs/heads/{ns}/{}/", self.replica)).collect()
    }

    /// Pushes this replica's refs to `remote` in one atomic push: the state ref, every log segment that is
    /// new or has grown, and the deletion of every segment this replica's own prune removed. A segment the remote
    /// holds and this clone doesn't is left alone. A squash rewrites the state ref, so it is pushed with a lease on
    /// the tip this clone last pushed or fetched. Segments only ever fast-forward, so a ruleset refusing forced
    /// updates to them accepts the push.
    ///
    /// # Errors
    ///
    /// [`StoreError::RemoteMoved`] when the remote's state ref moved since this clone last saw it, and
    /// [`StoreError::Rejected`] when the remote refuses another ref; nothing moves then. Fails too when git fails.
    pub fn push(&self, remote: &str) -> Result<usize, StoreError> {
        let mine = state_ref(&self.replica);
        let tracking = format!("{}{}", tracking_prefix(remote), self.replica);
        let segment_prefixes = self.segment_prefixes();
        let patterns: Vec<String> =
            std::iter::once(mine.clone()).chain(segment_prefixes.iter().cloned()).collect();
        let theirs = self.git.list_remote(remote, &patterns)?;
        // This replica's refs, its pending deletions and the lease's tip, in one process.
        let pruned_prefixes: Vec<String> = segment_prefixes.iter().map(|p| pruned_ref(p)).collect();
        let patterns: Vec<&str> = [mine.as_str(), tracking.as_str()]
            .into_iter()
            .chain(segment_prefixes.iter().map(String::as_str))
            .chain(pruned_prefixes.iter().map(String::as_str))
            .collect();
        let local = self.git.refs_under(&patterns)?;
        self.send(remote, &theirs, local)
    }

    /// Pushes what [`Self::push`] would, given the remote's refs by name and this clone's: its state ref, the
    /// lease's tip under its fetched name, its segments and its pending deletions.
    fn send(
        &self,
        remote: &str,
        theirs: &BTreeMap<String, String>,
        mut local: BTreeMap<String, String>,
    ) -> Result<usize, StoreError> {
        let mine = state_ref(&self.replica);
        let tracking = format!("{}{}", tracking_prefix(remote), self.replica);
        let lease = local.remove(&tracking).unwrap_or_default();
        let (pending, ours): (BTreeMap<String, String>, BTreeMap<String, String>) =
            local.into_iter().partition(|(name, _)| name.starts_with(PRUNED));
        let mut refs = Vec::new();
        for (name, id) in &ours {
            if theirs.get(name) == Some(id) {
                continue;
            }
            // The state ref moves under a lease on the tip this clone last pushed or fetched, since a squash
            // rewrites it; a segment only grows, so it is pushed against what the remote has.
            let is_state = *name == mine;
            let lease = if is_state {
                Some(lease.clone()).filter(|l| !l.is_empty())
            } else {
                theirs.get(name).cloned()
            };
            refs.push(PushRef {
                name: name.clone(),
                lease,
                new: Some(id.clone()),
                rewrite: is_state,
            });
        }
        let mut settled = Vec::new();
        for (marker, tip) in &pending {
            let name = format!("refs/heads/{}", &marker[PRUNED.len()..]);
            match theirs.get(&name) {
                None => settled.push((marker, tip)),
                // Delete only what this replica wrote: the pruned tip or an earlier push of it.
                Some(remote_tip) if self.git.is_ancestor(remote_tip, tip) => {
                    refs.push(PushRef {
                        name,
                        lease: Some(remote_tip.clone()),
                        new: None,
                        rewrite: false,
                    });
                    settled.push((marker, tip));
                },
                Some(_) => {},
            }
        }
        if !refs.is_empty()
            && let Err(refused) = self.git.push(remote, &refs)?
        {
            return Err(push_failure(&mine, refused));
        }
        if let Some(tip) = ours.get(&mine)
            && *tip != lease
        {
            self.git.set_ref(&tracking, tip)?;
        }
        for (marker, tip) in settled {
            self.git.delete_ref(marker, Some(tip))?;
        }
        Ok(refs.len())
    }

    /// The month's current segment ref among `refs` (the highest generation, or generation 1 when none) and its tip.
    fn segment_in(
        &self,
        refs: &BTreeMap<String, String>,
        namespace: &str,
        month: &str,
    ) -> (String, Option<String>) {
        let prefix = format!("refs/heads/{namespace}/{}/{month}/", self.replica);
        match refs
            .iter()
            .filter(|(name, _)| name.starts_with(&prefix))
            .max_by_key(|(name, _)| *name)
        {
            Some((name, tip)) => (name.clone(), Some(tip.clone())),
            None => (segment_ref(namespace, &self.replica, month, 1), None),
        }
    }

    /// Joins the states held at these commits into this replica's state, in one commit. `markers` are refs to set
    /// along with it, or alone when the join changes nothing.
    fn join_tips(
        &self,
        tips: &[String],
        markers: &[(String, String)],
    ) -> Result<Outcome, StoreError> {
        let mut others = Vec::with_capacity(tips.len());
        for tip in tips {
            others.push(read_state(&self.git, tip)?);
        }
        crate::probe::mark("join: read states");
        let _lock = self.lock()?;
        let name = state_ref(&self.replica);
        // Read under the lock: a write since the sync listed the refs moved the tip, and nothing moves it now.
        let (tip, before, found) = self.load_at(self.git.resolve(&name)?)?;
        let mut replica = Replica::open(self.replica.clone(), before);
        replica.state.record();
        crate::probe::mark("join: load own state");
        for other in &others {
            replica.join(other)?;
        }
        crate::probe::mark("join: join");
        let mut state = replica.into_state();
        let journal = state.take_journal().expect("the join records");
        let outcome = if state.unchanged_since(&journal) {
            if markers.is_empty() {
                return Ok(Outcome::Unchanged);
            }
            Outcome::Unchanged
        } else {
            let at = state.context.clocks.get(&self.replica).copied().unwrap_or(0);
            let files = file_bytes(state.changed_files_since(&journal));
            let commit = NewCommit {
                parent: tip.as_deref(),
                files: &files,
                replica: self.replica.as_str(),
                seconds: at.div_euclid(1_000),
                message: "join",
            };
            let commit = self.git.import_commits(&[commit])?.remove(0);
            crate::probe::mark("join: write the objects");
            Outcome::Committed { commit }
        };
        let mut transaction = self.git.transaction()?;
        if let Outcome::Committed { commit } = &outcome {
            transaction.update(&name, commit, tip.as_deref())?;
        }
        for (marker, tip) in markers {
            transaction.set(marker, tip)?;
        }
        transaction.commit(None)?;
        crate::probe::mark("join: update refs");
        if let Outcome::Committed { commit } = &outcome {
            self.cache()?.write_after(&found, commit, &state, journal.keys());
        }
        Ok(outcome)
    }

    /// Syncs with `remote`: fetches every replica's state ref, joins those that moved since this replica last joined
    /// them into its state, then pushes what the remote lacks. A remote copy of this replica's state ref that isn't
    /// this clone's own earlier push means another clone writes under this replica's id, and the sync stops before its
    /// push could overwrite that clone's work. One case passes unseen: a push whose tracking update was lost, followed
    /// by a local squash before the next sync, can't be told from a duplicate's write and is refused as one. Only the
    /// join holds the write lock, not the network calls.
    ///
    /// # Errors
    ///
    /// [`StoreError::Duplicate`] when the remote's copy of this replica's state ref shows another clone writing as
    /// this replica, and [`StoreError::State`] with an id reuse when another replica's state has seen more of this
    /// one's writes than it has; otherwise the errors of [`Self::fetch`], the join and [`Self::push`]. A failed push
    /// isn't retried, since right after a fetch a moved ref means something else writes this replica's refs.
    pub fn sync(&self, remote: &str) -> Result<Synced, StoreError> {
        let prefix = tracking_prefix(remote);
        let own = format!("{prefix}{}", self.replica);
        let joined_prefix = joined_prefix(&self.replica, remote);
        let mine = state_ref(&self.replica);
        let last_seen = self.git.resolve(&own)?;
        self.fetch(remote)?;
        crate::probe::mark("sync: fetch");
        // One listing serves the whole sync: what was fetched, what was joined before, and what a push compares.
        let base = remote_base(remote);
        let segments = self.segment_prefixes();
        let pruned: Vec<String> = segments.iter().map(|prefix| pruned_ref(prefix)).collect();
        let patterns: Vec<&str> = [base.as_str(), joined_prefix.as_str(), mine.as_str()]
            .into_iter()
            .chain(segments.iter().chain(&pruned).map(String::as_str))
            .collect();
        let mut fetched = BTreeMap::new();
        let mut done = BTreeMap::new();
        let mut theirs = BTreeMap::new();
        let mut local = BTreeMap::new();
        for (name, tip) in self.git.refs_under(&patterns)? {
            if let Some(path) = name.strip_prefix(&base) {
                theirs.insert(format!("refs/heads/{path}"), tip.clone());
                if name.starts_with(&prefix) {
                    if name == own {
                        local.insert(name.clone(), tip.clone());
                    }
                    fetched.insert(name, tip);
                }
            } else if name.starts_with(&joined_prefix) {
                done.insert(name, tip);
            } else {
                local.insert(name, tip);
            }
        }
        // Only this replica writes its state ref, so a move of the remote's copy is this clone's own push whose
        // tracking update was lost (an ancestor of the local tip), a deleted remote ref (pushed again), or another
        // clone writing as this replica. A join can't tell that last case apart when both clones wrote as many
        // operations, so it is checked here.
        let fetched_own = fetched.get(&own).cloned();
        if fetched_own != last_seen
            && let Some(remote_tip) = &fetched_own
        {
            let ours =
                local.get(&mine).is_some_and(|local| self.git.is_ancestor(remote_tip, local));
            if !ours {
                match &last_seen {
                    Some(tip) => self.git.set_ref(&own, tip)?,
                    None => self.git.delete_ref(&own, None)?,
                }
                return Err(StoreError::Duplicate(self.replica.clone()));
            }
        }
        // This replica's own state is already in its tip by now. Another's needs joining only when it moved since
        // this replica last joined it, which a marker per replica records once the join commits. A replica with no
        // state yet, such as a fresh id taken after its state ref went, trusts no marker.
        let has_state = local.contains_key(&mine);
        let fresh: Vec<(String, String)> = fetched
            .iter()
            .filter(|(name, _)| **name != own)
            .map(|(name, tip)| (format!("{joined_prefix}{}", &name[prefix.len()..]), tip.clone()))
            .filter(|(marker, tip)| !has_state || done.get(marker) != Some(tip))
            .collect();
        let mut joined = false;
        if !fresh.is_empty() {
            let tips: Vec<String> = fresh.iter().map(|(_, tip)| tip.clone()).collect();
            if let Outcome::Committed { commit } = self.join_tips(&tips, &fresh)? {
                local.insert(mine.clone(), commit);
                joined = true;
            }
        }
        // Right after the fetch, its refs say what the remote holds, so the push needn't ask.
        let pushed = self.send(remote, &theirs, local)?;
        crate::probe::mark("sync: push");
        // Fetch and push skip git's own upkeep, so a clone that mostly syncs gets the store's here.
        self.maintain();
        crate::probe::mark("sync: upkeep");
        let replicas = fetched.keys().map(|name| name[prefix.len()..].to_owned()).collect();
        Ok(Synced { fetched: replicas, joined, pushed })
    }

    /// Whether this replica's state ref is ahead of what it last pushed to or fetched from `remote`, judged without
    /// reaching the remote.
    ///
    /// # Errors
    ///
    /// Fails when git fails.
    pub fn unpushed(&self, remote: &str) -> Result<bool, StoreError> {
        let mine = self.git.resolve(&state_ref(&self.replica))?;
        let tracking = format!("{}{}", tracking_prefix(remote), self.replica);
        Ok(mine.is_some() && mine != self.git.resolve(&tracking)?)
    }

    /// The remote this clone syncs with, `origin`, when it has one.
    #[must_use]
    pub fn remote(&self) -> Option<&'static str> {
        self.origin.then_some(REMOTE)
    }

    /// Whether every write ends with a push, as `acyclic.pushAfterWrite` in the clone's git config asks.
    #[must_use]
    pub fn push_after_write(&self) -> bool {
        self.push_after_write
    }

    /// Squashes this replica's state ref to a parentless commit of the same tree, under a lease on the tip it read.
    ///
    /// # Errors
    ///
    /// Fails when the ref moved since it was read, a lock is held or git fails.
    pub fn squash(&self) -> Result<Option<String>, StoreError> {
        let _lock = self.lock()?;
        let Some(tip) = self.git.resolve(&state_ref(&self.replica))? else {
            return Ok(None);
        };
        let tree = self.git.tree_of(&tip)?;
        let seconds = self.git.commit_seconds(&tip)?;
        let commit = self.git.commit(&tree, None, self.replica.as_str(), seconds, "squash")?;
        let cache = self.cache()?;
        let state = cache.read(&tip).map(|(state, _)| state);
        let mut transaction = self.git.transaction()?;
        transaction.update(&state_ref(&self.replica), &commit, Some(&tip))?;
        transaction.commit(None)?;
        self.drop_scratch(SCRATCH_STATE);
        if let Some(state) = state {
            // The same tree under a new tip.
            cache.write(&commit, &state);
        }
        Ok(Some(commit))
    }
}

/// The namespaces holding monthly segments: audit logs.
const NAMESPACES: [&str; 1] = ["log"];

/// Where prune records the segments it deleted, until a push deletes them from the remote too.
const PRUNED: &str = "refs/acyclic/pruned/";

/// The pending-deletion marker for a segment ref (or a prefix of them), holding the segment's last tip.
fn pruned_ref(segment: &str) -> String {
    format!("{PRUNED}{}", segment.trim_start_matches("refs/heads/"))
}

/// The prefix of the refs fetched from `remote`, a name or a URL, each named as on the remote less `refs/heads/`.
/// They live outside `refs/remotes/`, which a plain `git fetch` updates without a join; a lease against such a ref
/// would protect nothing.
fn remote_base(remote: &str) -> String {
    let name: String = remote
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("refs/acyclic/remotes/{name}/")
}

/// The prefix of the fetched state refs for `remote`.
fn tracking_prefix(remote: &str) -> String {
    format!("{}state/", remote_base(remote))
}

/// The prefix of the markers recording, per other replica, the state tip from `remote` that `replica` last joined.
fn joined_prefix(replica: &ReplicaId, remote: &str) -> String {
    tracking_prefix(remote).replacen("/remotes/", &format!("/joined/{replica}/"), 1)
}

/// Classifies a refused push: a stale lease on the state ref, or another refused ref. An atomic push moves nothing when
/// any ref is refused, so the cause is the state ref's stale lease when there is one, else the first refusal.
fn push_failure(state: &str, refused: Vec<Refused>) -> StoreError {
    if refused.iter().any(|r| matches!(r, Refused::Stale(name) if name == state)) {
        return StoreError::RemoteMoved;
    }
    match refused.into_iter().next() {
        Some(Refused::Stale(reference)) => {
            StoreError::Rejected { reference, reason: "stale info".to_owned() }
        },
        Some(Refused::Rejected(reference, reason)) => StoreError::Rejected { reference, reason },
        None => StoreError::Git("the push was refused".to_owned()),
    }
}

/// How old a lock file on this replica's own refs is before the next writer removes it.
const STALE_LOCK: std::time::Duration = std::time::Duration::from_secs(60);

/// Removes the lock files at `path` or below it that are older than [`STALE_LOCK`]. Best effort.
fn remove_stale_locks(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return };
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().filter_map(Result::ok) {
            remove_stale_locks(&entry.path());
        }
    } else if path.extension().is_some_and(|ext| ext == "lock")
        && meta
            .modified()
            .ok()
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age >= STALE_LOCK)
    {
        let _ = std::fs::remove_file(path);
    }
}

/// A month's current segment.
struct Segment {
    reference: String,
    generation: u32,
    tip: String,
}

/// Whole calendar months from `from` to `to`, both `yyyy-mm`; zero when `to` isn't later.
fn months_between(from: &str, to: &str) -> u32 {
    let index = |month: &str| {
        let (year, month) = month.split_once('-').unwrap_or(("0", "1"));
        year.parse::<i64>().unwrap_or(0) * 12 + month.parse::<i64>().unwrap_or(1)
    };
    u32::try_from((index(to) - index(from)).max(0)).unwrap_or(u32::MAX)
}

/// The replicas a peer's state has seen, read from its header alone: a peer's entities, which this build may not read
/// strictly, don't matter to which ids are taken.
fn seen_replicas(git: &Git, commit: &str) -> Result<Vec<ReplicaId>, StoreError> {
    let header = git.read_paths(commit, &[state::HEADER_PATH.to_owned()])?;
    let unreadable =
        || StoreError::State(state::Error::Format(format!("no readable header in state {commit}")));
    let header: Json =
        serde_json::from_slice(header.get(state::HEADER_PATH).ok_or_else(unreadable)?)
            .map_err(|_| unreadable())?;
    let seen = header.pointer("/context/seen").and_then(Json::as_object).ok_or_else(unreadable)?;
    // An id this build can't parse can't clash with one it makes.
    Ok(seen.keys().filter_map(|id| ReplicaId::new(id).ok()).collect())
}

/// Reads a state from a commit's tree, refusing files that aren't in canonical form.
fn read_state(git: &Git, commit: &str) -> Result<State, StoreError> {
    Ok(State::from_files(&read_files(git, commit)?)?)
}

fn read_files(git: &Git, commit: &str) -> Result<BTreeMap<String, String>, StoreError> {
    git.read_tree(commit)?
        .into_iter()
        .map(|(path, content)| {
            String::from_utf8(content).map(|text| (path.clone(), text)).map_err(|_| {
                StoreError::State(state::Error::Format(format!("{path} is not UTF-8")))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_follow_the_utc_calendar() {
        assert_eq!(month_of(0), "1970-01");
        assert_eq!(month_of(951_782_400_000), "2000-02"); // 2000-02-29
        assert_eq!(month_of(951_868_799_999), "2000-02");
        assert_eq!(month_of(951_868_800_000), "2000-03");
        assert_eq!(month_of(1_798_761_599_999), "2026-12");
        assert_eq!(month_of(1_798_761_600_000), "2027-01");
        assert_eq!(month_of(-1), "1969-12");
        assert_eq!(day_of(951_782_400_000), "29");
        assert_eq!(day_of(951_868_800_000), "01");
        assert_eq!(day_of(-1), "31");
    }

    #[test]
    fn refs_are_named_by_replica_month_and_generation() {
        let replica = ReplicaId::new("mac").unwrap();
        assert_eq!(state_ref(&replica), "refs/heads/state/mac");
        assert_eq!(segment_ref("log", &replica, "2026-10", 1), "refs/heads/log/mac/2026-10/0001");
        assert_eq!(segment_ref("log", &replica, "2026-10", 12), "refs/heads/log/mac/2026-10/0012");
    }

    #[test]
    fn months_between_counts_calendar_months() {
        assert_eq!(months_between("2026-07", "2026-10"), 3);
        assert_eq!(months_between("2025-12", "2026-01"), 1);
        assert_eq!(months_between("2026-10", "2026-10"), 0);
        assert_eq!(months_between("2026-11", "2026-10"), 0);
    }

    #[test]
    fn a_log_entry_falls_back_to_the_root_before_the_batch() {
        use serde_json::json;
        // The root is a field of the entity itself, so clearing it leaves only the earlier state to name it.
        let root_of = |state: &State, key: &Key| {
            let register = state.field(key, "root")?;
            register.values().find_map(|v| v.value.as_str().map(str::to_owned))
        };
        let (moved, gone) = (Key::new("item", "a1").unwrap(), Key::new("item", "a2").unwrap());
        let mut replica = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
        for key in [&moved, &gone] {
            replica.write(key, "root", "actor/user", 1, Some(json!("p1"))).unwrap();
        }
        replica.state.record();
        replica.write(&moved, "root", "actor/user", 2, Some(json!("p2"))).unwrap();
        replica.write(&gone, "root", "actor/user", 2, None).unwrap();
        let mut state = replica.into_state();
        let mut journal = state.take_journal().unwrap();
        let after = state.clone();
        let logged: BTreeMap<String, Vec<(String, usize)>> =
            diff(&mut state, &mut journal, &root_of)
                .into_iter()
                .map(|(root, writes)| {
                    (root, writes.into_iter().map(|w| (w.id, w.values.len())).collect())
                })
                .collect();
        let expected = [("p1", vec![("a2".to_owned(), 0)]), ("p2", vec![("a1".to_owned(), 1)])];
        assert_eq!(logged, expected.map(|(root, writes)| (root.to_owned(), writes)).into());
        assert_eq!(state, after, "and the state is as the batch left it");
    }
}
