//! State stored in a tracker repository: each replica's state ref holds its whole state as a tree, and its audit log
//! keeps the events behind its batches, one ref per month.
//!
//! A batch reads the replica's state, applies a pure closure, writes the changed files and the day's events with one
//! `git fast-import` into a single pack, and moves the state ref and the month's log ref in one
//! `git update-ref --stdin` transaction with compare-and-swap, so both move or neither does. Losing a race re-reads
//! and re-runs the closure. Only plumbing is used; nothing is checked out.
//!
//! Reads come from a cache of the parsed state kept beside the repository for the state ref's current tip, and the
//! store repacks itself once packs pile up, so git's share of a command stays small and steady; parsing and
//! rewriting the state still grow with the tracker. The repack runs `git gc --auto`, which skips repacking for a day
//! after a run that warned, as git records in `gc.log`; packs pile up meanwhile.

mod cache;
mod git;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value as Json;

use crate::state::{self, Dot, Journal, Key, Millis, Replica, ReplicaId, State};
use crate::time::civil;
use cache::{Cache, Found};
use git::{Git, NewCommit};

/// How long a batch keeps retrying lost races.
const RETRY_DEADLINE: Duration = Duration::from_secs(30);

/// The git config key holding this clone's replica id.
pub const REPLICA_CONFIG: &str = "acyclic.replica";

/// The audit-log file for items in no project.
pub const NO_PROJECT: &str = "_";

/// Why a store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// A git command failed.
    Git(String),
    /// Another writer moved a ref first; retried internally until the deadline.
    Conflict,
    /// A ref lock is held. Locks are never removed automatically: a stalled git process could still finish.
    Locked {
        /// The lock file.
        path: PathBuf,
        /// How old it is, when known.
        age_seconds: Option<u64>,
    },
    /// Lost races until the deadline.
    Timeout,
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
            Self::Conflict => write!(f, "another writer moved the ref first"),
            Self::Locked { path, age_seconds } => {
                write!(f, "a ref is locked by {}", path.display())?;
                if let Some(age) = age_seconds {
                    write!(f, " ({age} s old)")?;
                }
                write!(f, "; if no git process holds it, remove it with the lock recovery command")
            },
            Self::Timeout => {
                write!(f, "gave up after losing races to other writers for {RETRY_DEADLINE:?}")
            },
            Self::State(err) => err.fmt(f),
            Self::Config(message) => write!(f, "tracker not set up: {message}"),
            Self::RemoteMoved => {
                write!(f, "the remote's state ref moved since the last sync; fetch and join first")
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

/// Who a batch is for: the actor and, for a run's operations, the run and its sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// The actor behind the batch, such as `actor/user` or `run/r1`.
    pub actor: String,
    /// The run and sequence number, for a batch applying a run's operation.
    pub run: Option<(String, u64)>,
}

/// Maps an entity to its project root's id, or `None` for no project.
pub type RootOf<'a> = &'a dyn Fn(&State, &Key) -> Option<String>;

/// Failpoints for crash tests.
#[doc(hidden)]
#[derive(Default)]
pub struct Hooks {
    /// Runs after every object is written, before the transaction starts.
    pub after_objects: Option<Box<dyn Fn()>>,
    /// Runs after the transaction is prepared, before it commits.
    pub after_prepare: Option<Box<dyn Fn()>>,
}

/// What a batch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The closure changed nothing.
    Unchanged,
    /// Committed as this state commit, after this many lost races.
    Committed {
        /// The new state commit.
        commit: String,
        /// Races lost before it committed.
        retries: u32,
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
    let git = Git::new(path);
    git.run(["config", REPLICA_CONFIG, id.as_str()], None, &[])?;
    git.run(["config", UNUSED_CONFIG, id.as_str()], None, &[])?;
    Ok(())
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

/// The ref of a log or usage segment: `refs/heads/<namespace>/<replica>/<yyyy-mm>/<generation>`.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
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
    /// records it and reports the change.
    ///
    /// # Errors
    ///
    /// Fails when the replica id is missing or invalid, or git fails.
    pub fn open(path: &Path) -> Result<(Self, Option<ReplicaId>), StoreError> {
        let git = Git::new(path);
        // The replica's settings and git's repacking thresholds in one process; git prints names in lower case.
        let settings = git
            .text([
                "config",
                "--get-regexp",
                r"^(acyclic\.|gc\.auto$|gc\.autopacklimit$|remote\.origin\.url$)",
            ])
            .unwrap_or_default();
        let setting = |name: &str| {
            settings.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
        };
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
        crate::probe::mark("open: git rev-parse");
        let tracker = |git, replica, unused, tip| Self {
            git,
            replica,
            unused: AtomicBool::new(unused),
            opened: std::sync::Mutex::new(Some(OpenedTip(tip))),
            upkeep,
            origin,
            push_after_write,
        };
        if tip.is_some() {
            return Ok((tracker(git, id, false, tip), None));
        }
        let mut remembered = BTreeSet::new();
        for (_, commit) in git.refs("refs/heads/state/")? {
            let state = read_state(&git, &commit)?;
            remembered.extend(state.context.seen.keys().cloned());
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
            let _ = self.git.run(["gc", "--auto", "--quiet"], None, &[]);
        }
    }

    /// Deletes a scratch ref, best effort: a repack holding the lock only means it lingers until the next try.
    fn drop_scratch(&self, scratch: &str) {
        let _ = self.git.run(["update-ref", "-d", scratch], None, &[]);
    }

    fn cache(&self) -> Result<Cache, StoreError> {
        Ok(Cache::new(&self.git.common_dir()?, &self.replica))
    }

    /// Runs a batch: `apply` receives the replica's current state and must be pure, since a lost race runs it again
    /// on the newer state. `now` is stamped through the clock guard.
    ///
    /// # Errors
    ///
    /// Fails when the closure or the clock guard refuses, a lock is held, git fails, or races are lost until the
    /// deadline.
    pub fn batch<E>(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: impl Fn(&mut Replica, Millis) -> Result<(), E>,
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
        apply: impl Fn(&mut Replica, Millis) -> Result<(), E>,
        hooks: &Hooks,
    ) -> Result<Outcome, BatchError<E>> {
        crate::probe::mark("before the batch");
        let _lock = self.lock()?;
        let started = Instant::now();
        let mut retries = 0;
        loop {
            match self.try_batch(meta, now, allow_clock_step, root_of, &apply, hooks) {
                Ok(Some(commit)) => return Ok(Outcome::Committed { commit, retries }),
                Ok(None) => return Ok(Outcome::Unchanged),
                Err(BatchError::Store(StoreError::Conflict))
                    if started.elapsed() < RETRY_DEADLINE =>
                {
                    retries += 1;
                    std::thread::sleep(backoff(retries));
                },
                Err(BatchError::Store(StoreError::Conflict)) => {
                    return Err(BatchError::Store(StoreError::Timeout));
                },
                Err(err) => return Err(err),
            }
        }
    }

    fn try_batch<E>(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: &impl Fn(&mut Replica, Millis) -> Result<(), E>,
        hooks: &Hooks,
    ) -> Result<Option<String>, BatchError<E>> {
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
            return Ok(None);
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
            let event = Event {
                at,
                actor: &meta.actor,
                actor_record: event_actor.clone(),
                run: meta.run.as_ref().map(|(run, _)| run.as_str()),
                seq: meta.run.as_ref().map(|(_, seq)| *seq),
                writes,
            };
            let path = log_path(&root, &day);
            let mut content = existing.get(&path).cloned().unwrap_or_default();
            content.extend(serde_json::to_vec(&event).expect("events serialize"));
            content.push(b'\n');
            files.insert(path, Some(content));
        }
        crate::probe::mark("batch: log events");
        let seconds = at.div_euclid(1_000);
        let commit = |scratch, parent, files, message| NewCommit {
            scratch,
            parent,
            files,
            replica: self.replica.as_str(),
            seconds,
            message,
        };
        let ids = self.git.import_commits(&[
            commit(SCRATCH_STATE, tip.as_deref(), &state_files, "batch"),
            commit(SCRATCH_LOG, segment_tip.as_deref(), &files, "events"),
        ])?;
        let [commit, log_commit] = [&ids[0], &ids[1]].map(String::clone);
        crate::probe::mark("batch: git fast-import");

        if let Some(hook) = &hooks.after_objects {
            hook();
        }
        let mut transaction = self.git.transaction()?;
        transaction.update(&state_name, &commit, tip.as_deref())?;
        transaction.update(&segment, &log_commit, segment_tip.as_deref())?;
        transaction.prepare()?;
        if let Some(hook) = &hooks.after_prepare {
            hook();
        }
        transaction.commit()?;
        crate::probe::mark("batch: git update-ref");
        self.cache()?.write_after(&found, &commit, &state, journal.keys());
        self.maintain();
        crate::probe::mark("batch: upkeep");
        if self.unused.swap(false, Ordering::Relaxed) {
            // Another worktree may have cleared it already; the state ref now vouches for the id either way.
            let _ = self.git.run(["config", "--unset", UNUSED_CONFIG], None, &[]);
        }
        Ok(Some(commit))
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
        Ok(file)
    }

    /// Writes `after` as a commit on `tip`, storing only the files that changed since `before`.
    fn commit_state(
        &self,
        tip: Option<&str>,
        before: &State,
        after: &State,
        at: Millis,
        message: &str,
    ) -> Result<String, StoreError> {
        let files = file_bytes(after.changed_files(before));
        let commit = NewCommit {
            scratch: SCRATCH_STATE,
            parent: tip,
            files: &files,
            replica: self.replica.as_str(),
            seconds: at.div_euclid(1_000),
            message,
        };
        let mut ids = self.git.import_commits(&[commit])?;
        Ok(ids.remove(0))
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
                    let seconds = self
                        .git
                        .text(["show", "-s", "--format=%ct", &segment.tip])?
                        .parse()
                        .unwrap_or(0);
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
        if let Some(mut transaction) = transaction {
            transaction.prepare()?;
            transaction.commit()?;
            self.drop_scratch(SCRATCH_LOG);
        }
        Ok(dropped)
    }

    /// Fetches every replica's state ref from `remote` into private fetched refs, ready to join; this
    /// replica's own one is what the next push leases against.
    ///
    /// # Errors
    ///
    /// Fails when git fails.
    pub fn fetch(&self, remote: &str) -> Result<(), StoreError> {
        let tracking = tracking_prefix(remote);
        self.git.run(
            ["fetch", "--quiet", "--prune", remote, &format!("+refs/heads/state/*:{tracking}*")],
            None,
            &[],
        )?;
        Ok(())
    }

    /// Pushes this replica's refs to `remote` in one atomic push: the state ref, every log and usage segment that is
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
        let segment_prefixes: Vec<String> =
            NAMESPACES.iter().map(|ns| format!("refs/heads/{ns}/{}/", self.replica)).collect();
        let listing = self.git.text(
            ["ls-remote", remote, mine.as_str()]
                .into_iter()
                .map(str::to_owned)
                .chain(segment_prefixes.iter().map(|prefix| format!("{prefix}*"))),
        )?;
        let theirs: BTreeMap<String, String> = listing
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(id, name)| (name.to_owned(), id.to_owned()))
            .collect();
        let mut ours = self.git.refs(&mine)?;
        let mut pending = BTreeMap::new();
        for prefix in &segment_prefixes {
            ours.extend(self.git.refs(prefix)?);
            pending.extend(self.git.refs(&pruned_ref(prefix))?);
        }
        let mut args = vec!["push".to_owned(), "--atomic".to_owned(), "--porcelain".to_owned()];
        let mut refspecs = Vec::new();
        for (name, id) in &ours {
            if theirs.get(name) == Some(id) {
                continue;
            }
            if *name == mine {
                let lease = self.git.resolve(&tracking)?.unwrap_or_default();
                args.push(format!("--force-with-lease={name}:{lease}"));
            }
            refspecs.push(format!("{name}:{name}"));
        }
        let mut settled = Vec::new();
        for (marker, tip) in &pending {
            let name = format!("refs/heads/{}", &marker[PRUNED.len()..]);
            match theirs.get(&name) {
                None => settled.push((marker, tip)),
                // Delete only what this replica wrote: the pruned tip or an earlier push of it.
                Some(remote_tip)
                    if self
                        .git
                        .run(["merge-base", "--is-ancestor", remote_tip, tip], None, &[])
                        .is_ok() =>
                {
                    args.push(format!("--force-with-lease={name}:{remote_tip}"));
                    refspecs.push(format!(":{name}"));
                    settled.push((marker, tip));
                },
                Some(_) => {},
            }
        }
        if !refspecs.is_empty() {
            args.push(remote.to_owned());
            args.extend(refspecs.iter().cloned());
            let output = self.git.output(&args)?;
            if !output.status.success() {
                return Err(push_failure(&mine, &output));
            }
        }
        if let Some(tip) = ours.get(&mine) {
            self.git.run(["update-ref", &tracking, tip], None, &[])?;
        }
        for (marker, tip) in settled {
            self.git.run(["update-ref", "-d", marker, tip], None, &[])?;
        }
        Ok(refspecs.len())
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

    /// Joins the state held at `reference`, such as another replica's state ref, into this replica's state.
    ///
    /// # Errors
    ///
    /// Fails when the other state has seen more of this replica's writes than it has (another clone uses this
    /// replica's id), or on a lock, git failure or lost races.
    pub fn join_ref(&self, reference: &str) -> Result<Outcome, StoreError> {
        self.join_refs(&[reference.to_owned()])
    }

    /// Joins the states held at `references` into this replica's state, in one commit.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::join_ref`] does.
    pub fn join_refs(&self, references: &[String]) -> Result<Outcome, StoreError> {
        let mut others = Vec::with_capacity(references.len());
        for reference in references {
            let tip = self
                .git
                .resolve(reference)?
                .ok_or_else(|| StoreError::Config(format!("no ref {reference}")))?;
            others.push(read_state(&self.git, &tip)?);
        }
        let _lock = self.lock()?;
        let started = Instant::now();
        let mut retries = 0;
        loop {
            let (tip, before) = self.load()?;
            let mut replica = Replica::open(self.replica.clone(), before.clone());
            for other in &others {
                replica.join(other)?;
            }
            if replica.state == before {
                return Ok(Outcome::Unchanged);
            }
            let at = replica.state.context.clocks.get(&self.replica).copied().unwrap_or(0);
            let commit = self.commit_state(tip.as_deref(), &before, &replica.state, at, "join")?;
            let mut transaction = self.git.transaction()?;
            transaction.update(&state_ref(&self.replica), &commit, tip.as_deref())?;
            match transaction.prepare().and_then(|()| transaction.commit()) {
                Ok(()) => {
                    self.cache()?.write(&commit, &replica.state);
                    return Ok(Outcome::Committed { commit, retries });
                },
                Err(StoreError::Conflict) if started.elapsed() < RETRY_DEADLINE => {
                    retries += 1;
                    std::thread::sleep(backoff(retries));
                },
                Err(StoreError::Conflict) => return Err(StoreError::Timeout),
                Err(err) => return Err(err),
            }
        }
    }

    /// Syncs with `remote`: fetches every replica's state ref, joins them all into this replica's state, its own
    /// fetched one included, then pushes. A remote copy of this replica's state ref that isn't this clone's own
    /// earlier push means another clone writes under this replica's id, and the sync stops before its push could
    /// overwrite that clone's work. One case passes unseen: a push whose tracking update was lost, followed by a local
    /// squash before the next sync, can't be told from a duplicate's write and is refused as one. Only the join holds
    /// the write lock, not the network calls.
    ///
    /// # Errors
    ///
    /// [`StoreError::State`] with an id reuse when another clone writes as this replica, and the errors of
    /// [`Self::fetch`], [`Self::join_refs`] and [`Self::push`]; a failed push never retries, since right after a
    /// fetch a moved ref means something else writes this replica's refs.
    pub fn sync(&self, remote: &str) -> Result<Synced, StoreError> {
        let prefix = tracking_prefix(remote);
        let own = format!("{prefix}{}", self.replica);
        let last_seen = self.git.resolve(&own)?;
        self.fetch(remote)?;
        // Only this replica writes its state ref, so a move of the remote's copy is this clone's own push whose
        // tracking update was lost (an ancestor of the local tip), a deleted remote ref (pushed again), or another
        // clone writing as this replica. A join can't tell that last case apart when both clones wrote as many
        // operations, so it is checked here.
        let fetched_own = self.git.resolve(&own)?;
        if fetched_own != last_seen
            && let Some(remote_tip) = &fetched_own
        {
            let local = self.git.resolve(&state_ref(&self.replica))?;
            let ours = local.is_some_and(|local| {
                self.git.run(["merge-base", "--is-ancestor", remote_tip, &local], None, &[]).is_ok()
            });
            if !ours {
                match &last_seen {
                    Some(tip) => self.git.run(["update-ref", &own, tip], None, &[])?,
                    None => self.git.run(["update-ref", "-d", &own], None, &[])?,
                };
                return Err(StoreError::Duplicate(self.replica.clone()));
            }
        }
        let fetched = self.git.refs(&prefix)?;
        let names: Vec<String> = fetched.keys().cloned().collect();
        let joined = !names.is_empty() && self.join_refs(&names)? != Outcome::Unchanged;
        let pushed = self.push(remote)?;
        let replicas = names.iter().map(|name| name[prefix.len()..].to_owned()).collect();
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
        let tree = self.git.text(["rev-parse", &format!("{tip}^{{tree}}")])?;
        let seconds = self.git.text(["show", "-s", "--format=%ct", &tip])?.parse().unwrap_or(0);
        let commit = self.git.commit(&tree, None, self.replica.as_str(), seconds, "squash")?;
        let cache = self.cache()?;
        let state = cache.read(&tip).map(|(state, _)| state);
        let mut transaction = self.git.transaction()?;
        transaction.update(&state_ref(&self.replica), &commit, Some(&tip))?;
        transaction.prepare()?;
        transaction.commit()?;
        self.drop_scratch(SCRATCH_STATE);
        if let Some(state) = state {
            // The same tree under a new tip.
            cache.write(&commit, &state);
        }
        Ok(Some(commit))
    }
}

/// The namespaces holding monthly segments: audit logs and usage records.
const NAMESPACES: [&str; 2] = ["log", "usage"];

/// Where prune records the segments it deleted, until a push deletes them from the remote too.
const PRUNED: &str = "refs/acyclic/pruned/";

/// The pending-deletion marker for a segment ref (or a prefix of them), holding the segment's last tip.
fn pruned_ref(segment: &str) -> String {
    format!("{PRUNED}{}", segment.trim_start_matches("refs/heads/"))
}

/// The prefix of the fetched state refs for `remote`, a name or a URL. They live outside `refs/remotes/`, which a
/// plain `git fetch` updates without a join; a lease against such a ref would protect nothing.
fn tracking_prefix(remote: &str) -> String {
    let name: String = remote
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("refs/acyclic/remotes/{name}/state/")
}

/// Classifies a failed push from its porcelain output: a stale lease on the state ref, another refused ref, or a
/// failure to reach the remote at all.
fn push_failure(state: &str, output: &std::process::Output) -> StoreError {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let refused = stdout.lines().filter_map(|line| {
        let mut fields = line.strip_prefix("!\t")?.split('\t');
        let destination = fields.next()?.rsplit(':').next()?.to_owned();
        let reason = fields.next().unwrap_or_default().to_owned();
        Some((destination, reason))
    });
    // An atomic push reports every other ref as refused too; the cause is the one refused for itself.
    let mut cause = None;
    for (reference, reason) in refused {
        if !reason.contains("atomic push failed") {
            cause = Some((reference, reason));
            break;
        }
        cause.get_or_insert((reference, reason));
    }
    match cause {
        Some((reference, reason)) if reference == state && reason.contains("stale info") => {
            StoreError::RemoteMoved
        },
        Some((reference, reason)) => StoreError::Rejected { reference, reason },
        None => StoreError::Git(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
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

/// Exponential backoff from about 5 ms to under 2 s: half the step plus up to a whole step of jitter, so racing
/// writers spread out.
fn backoff(retries: u32) -> Duration {
    let base = 5u64 << retries.min(8);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let jitter = u64::from(nanos) % base.max(1);
    Duration::from_millis(base / 2 + jitter)
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
        assert_eq!(
            segment_ref("usage", &replica, "2026-10", 12),
            "refs/heads/usage/mac/2026-10/0012"
        );
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

    #[test]
    fn backoff_grows_and_stays_bounded() {
        assert!(backoff(1) < Duration::from_millis(15));
        assert!(backoff(30) < Duration::from_millis(1_920));
        assert!(backoff(30) >= Duration::from_millis(640));
    }
}
