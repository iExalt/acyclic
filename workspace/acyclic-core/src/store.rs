//! State stored in a tracker repository: each replica's state ref holds its whole state as a tree, and its audit log
//! keeps the events behind its batches, one ref per month.
//!
//! A batch reads the replica's state, applies a pure closure, and moves the state ref and the month's log ref in one
//! `git update-ref --stdin` transaction with compare-and-swap, so both move or neither does. Losing a race re-reads
//! and re-runs the closure. Only plumbing is used; nothing is checked out.

mod git;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value as Json;

use crate::state::{self, Dot, Key, Millis, Replica, ReplicaId, State};
use git::Git;

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
}

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
    // Howard Hinnant's days-to-civil algorithm.
    let days = at.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}")
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

/// The changed fields between two states, by project root.
fn diff(before: &State, after: &State, root_of: RootOf<'_>) -> BTreeMap<String, Vec<Write>> {
    let keys: BTreeSet<&Key> =
        before.entities().map(|(k, _)| k).chain(after.entities().map(|(k, _)| k)).collect();
    let mut by_root: BTreeMap<String, Vec<Write>> = BTreeMap::new();
    for key in keys {
        let old = before.entity(key);
        let new = after.entity(key);
        let fields: BTreeSet<&String> = old
            .into_iter()
            .flat_map(|e| e.keys())
            .chain(new.into_iter().flat_map(|e| e.keys()))
            .collect();
        for field in fields {
            let was = before.field(key, field);
            let now = after.field(key, field);
            if was == now {
                continue;
            }
            let root = root_of(after, key)
                .or_else(|| root_of(before, key))
                .unwrap_or_else(|| NO_PROJECT.to_owned());
            let values = now
                .into_iter()
                .flatten()
                .map(|(dot, v)| WrittenValue { dot: dot.clone(), value: v.value.clone() })
                .collect();
            by_root.entry(root).or_default().push(Write {
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
        let id = git.text(["config", "--get", REPLICA_CONFIG]).map_err(|_| {
            StoreError::Config(format!("no {REPLICA_CONFIG} in {}", path.display()))
        })?;
        let id = ReplicaId::new(&id)?;
        let unused = git.text(["config", "--get", UNUSED_CONFIG]).is_ok_and(|v| v == id.as_str());
        if git.resolve(&state_ref(&id))?.is_some() {
            return Ok((Self { git, replica: id, unused: AtomicBool::new(false) }, None));
        }
        let mut remembered = BTreeSet::new();
        for (_, commit) in git.refs("refs/heads/state/")? {
            let state = read_state(&git, &commit)?;
            remembered.extend(state.context.seen.keys().cloned());
        }
        if unused && !remembered.contains(&id) {
            return Ok((Self { git, replica: id, unused: AtomicBool::new(true) }, None));
        }
        let fresh = loop {
            let candidate = ReplicaId::new(&format!("{id}-{}", random_suffix()))?;
            if !remembered.contains(&candidate) {
                break candidate;
            }
        };
        init_replica(path, &fresh)?;
        Ok((Self { git, replica: fresh.clone(), unused: AtomicBool::new(true) }, Some(fresh)))
    }

    /// This replica's id.
    #[must_use]
    pub fn replica(&self) -> &ReplicaId {
        &self.replica
    }

    /// The tip of this replica's state ref and the state it holds.
    ///
    /// # Errors
    ///
    /// Fails when git fails or the state can't be read.
    pub fn load(&self) -> Result<(Option<String>, State), StoreError> {
        match self.git.resolve(&state_ref(&self.replica))? {
            Some(tip) => Ok((Some(tip.clone()), read_state(&self.git, &tip)?)),
            None => Ok((None, State::new())),
        }
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
        let (tip, before) = self.load()?;
        let mut replica = Replica::open(self.replica.clone(), before.clone());
        let at = replica.stamp(now, allow_clock_step).map_err(StoreError::State)?;
        apply(&mut replica, at).map_err(BatchError::Apply)?;
        if replica.state == before {
            return Ok(None);
        }
        let commit = self.commit_state(tip.as_deref(), &before, &replica.state, at, "batch")?;

        let event_actor = actor_record(&replica.state, &meta.actor);
        let changes = diff(&before, &replica.state, root_of);
        let month = month_of(at);
        let (segment, segment_tip) = self.current_segment("log", &month)?;
        let mut files = BTreeMap::new();
        let paths: Vec<String> = changes.keys().map(|root| format!("{root}.jsonl")).collect();
        let existing = match &segment_tip {
            Some(tip) => self.git.read_paths(tip, &paths)?,
            None => BTreeMap::new(),
        };
        for (root, writes) in changes {
            let event = Event {
                at,
                actor: &meta.actor,
                actor_record: event_actor.clone(),
                run: meta.run.as_ref().map(|(run, _)| run.as_str()),
                seq: meta.run.as_ref().map(|(_, seq)| *seq),
                writes,
            };
            let path = format!("{root}.jsonl");
            let mut content = existing.get(&path).cloned().unwrap_or_default();
            content.extend(serde_json::to_vec(&event).expect("events serialize"));
            content.push(b'\n');
            files.insert(path, Some(content));
        }
        let log_tree = self.git.write_tree(segment_tip.as_deref(), &files)?;
        let seconds = at.div_euclid(1_000);
        let log_commit = self.git.commit(
            &log_tree,
            segment_tip.as_deref(),
            self.replica.as_str(),
            seconds,
            "events",
        )?;

        if let Some(hook) = &hooks.after_objects {
            hook();
        }
        let mut transaction = self.git.transaction()?;
        transaction.update(&state_ref(&self.replica), &commit, tip.as_deref())?;
        transaction.update(&segment, &log_commit, segment_tip.as_deref())?;
        transaction.prepare()?;
        if let Some(hook) = &hooks.after_prepare {
            hook();
        }
        transaction.commit()?;
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
        let old = before.to_files();
        let new = after.to_files();
        let mut changes: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
        for (path, content) in &new {
            if old.get(path) != Some(content) {
                changes.insert(path.clone(), Some(content.clone().into_bytes()));
            }
        }
        for path in old.keys().filter(|path| !new.contains_key(*path)) {
            changes.insert(path.clone(), None);
        }
        let tree = self.git.write_tree(tip, &changes)?;
        self.git.commit(&tree, tip, self.replica.as_str(), at.div_euclid(1_000), message)
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
                        let root = path.strip_suffix(".jsonl").unwrap_or(path);
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

    /// The month's current segment ref (the highest generation, or generation 1 when none) and its tip.
    fn current_segment(
        &self,
        namespace: &str,
        month: &str,
    ) -> Result<(String, Option<String>), StoreError> {
        let prefix = format!("refs/heads/{namespace}/{}/{month}/", self.replica);
        let existing = self.git.refs(&prefix)?;
        match existing.into_iter().max_by_key(|(name, _)| name.clone()) {
            Some((name, tip)) => Ok((name, Some(tip))),
            None => Ok((segment_ref(namespace, &self.replica, month, 1), None)),
        }
    }

    /// Joins the state held at `reference`, such as another replica's state ref, into this replica's state.
    ///
    /// # Errors
    ///
    /// Fails when the other state has seen more of this replica's writes than it has (another clone uses this
    /// replica's id), or on a lock, git failure or lost races.
    pub fn join_ref(&self, reference: &str) -> Result<Outcome, StoreError> {
        let other_tip = self
            .git
            .resolve(reference)?
            .ok_or_else(|| StoreError::Config(format!("no ref {reference}")))?;
        let other = read_state(&self.git, &other_tip)?;
        let _lock = self.lock()?;
        let started = Instant::now();
        let mut retries = 0;
        loop {
            let (tip, before) = self.load()?;
            let mut replica = Replica::open(self.replica.clone(), before.clone());
            replica.join(&other)?;
            if replica.state == before {
                return Ok(Outcome::Unchanged);
            }
            let at = replica.state.context.clocks.get(&self.replica).copied().unwrap_or(0);
            let commit = self.commit_state(tip.as_deref(), &before, &replica.state, at, "join")?;
            let mut transaction = self.git.transaction()?;
            transaction.update(&state_ref(&self.replica), &commit, tip.as_deref())?;
            match transaction.prepare().and_then(|()| transaction.commit()) {
                Ok(()) => return Ok(Outcome::Committed { commit, retries }),
                Err(StoreError::Conflict) if started.elapsed() < RETRY_DEADLINE => {
                    retries += 1;
                    std::thread::sleep(backoff(retries));
                },
                Err(StoreError::Conflict) => return Err(StoreError::Timeout),
                Err(err) => return Err(err),
            }
        }
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
        let mut transaction = self.git.transaction()?;
        transaction.update(&state_ref(&self.replica), &commit, Some(&tip))?;
        transaction.prepare()?;
        transaction.commit()?;
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

/// Reads the state held in a commit's tree.
fn read_state(git: &Git, commit: &str) -> Result<State, StoreError> {
    let files = git
        .read_tree(commit)?
        .into_iter()
        .map(|(path, content)| {
            String::from_utf8(content).map(|text| (path.clone(), text)).map_err(|_| {
                StoreError::State(state::Error::Format(format!("{path} is not UTF-8")))
            })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(State::from_files(&files)?)
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
    fn backoff_grows_and_stays_bounded() {
        assert!(backoff(1) < Duration::from_millis(15));
        assert!(backoff(30) < Duration::from_millis(1_920));
        assert!(backoff(30) >= Duration::from_millis(640));
    }
}
