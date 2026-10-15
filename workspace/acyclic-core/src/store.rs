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
        }
    }
}

impl std::error::Error for StoreError {}

impl From<state::Error> for StoreError {
    fn from(err: state::Error) -> Self {
        Self::State(err)
    }
}

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
    /// A replica whose own state ref is missing but which other replicas' states remember has lost its state, so it
    /// takes a new id (`<id>-2`, `-3`, …), records it in the git config and reports the change; reusing the old id
    /// could reuse its dots.
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
        if git.resolve(&state_ref(&id))?.is_some() {
            return Ok((Self { git, replica: id }, None));
        }
        let mut remembered = BTreeSet::new();
        for (_, commit) in git.refs("refs/heads/state/")? {
            let state = read_state(&git, &commit)?;
            remembered.extend(state.context.seen.keys().cloned());
        }
        if !remembered.contains(&id) {
            return Ok((Self { git, replica: id }, None));
        }
        let mut suffix = 2u32;
        let fresh = loop {
            let candidate = ReplicaId::new(&format!("{id}-{suffix}"))?;
            if !remembered.contains(&candidate) {
                break candidate;
            }
            suffix += 1;
        };
        git.run(["config", REPLICA_CONFIG, fresh.as_str()], None, &[])?;
        Ok((Self { git, replica: fresh.clone() }, Some(fresh)))
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
    pub fn batch(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: impl Fn(&mut Replica, Millis) -> Result<(), state::Error>,
    ) -> Result<Outcome, StoreError> {
        self.batch_with(meta, now, allow_clock_step, root_of, apply, &Hooks::default())
    }

    #[doc(hidden)]
    #[allow(clippy::missing_errors_doc)]
    pub fn batch_with(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: impl Fn(&mut Replica, Millis) -> Result<(), state::Error>,
        hooks: &Hooks,
    ) -> Result<Outcome, StoreError> {
        let _lock = self.lock()?;
        let started = Instant::now();
        let mut retries = 0;
        loop {
            match self.try_batch(meta, now, allow_clock_step, root_of, &apply, hooks) {
                Ok(Some(commit)) => return Ok(Outcome::Committed { commit, retries }),
                Ok(None) => return Ok(Outcome::Unchanged),
                Err(StoreError::Conflict) if started.elapsed() < RETRY_DEADLINE => {
                    retries += 1;
                    std::thread::sleep(backoff(retries));
                },
                Err(StoreError::Conflict) => return Err(StoreError::Timeout),
                Err(err) => return Err(err),
            }
        }
    }

    fn try_batch(
        &self,
        meta: &Meta,
        now: Millis,
        allow_clock_step: bool,
        root_of: RootOf<'_>,
        apply: &impl Fn(&mut Replica, Millis) -> Result<(), state::Error>,
        hooks: &Hooks,
    ) -> Result<Option<String>, StoreError> {
        let (tip, before) = self.load()?;
        let mut replica = Replica::open(self.replica.clone(), before.clone());
        let at = replica.stamp(now, allow_clock_step)?;
        apply(&mut replica, at)?;
        if replica.state == before {
            return Ok(None);
        }
        let commit = self.commit_state(tip.as_deref(), &before, &replica.state, at, "batch")?;

        let event_actor = actor_record(&replica.state, &meta.actor);
        let changes = diff(&before, &replica.state, root_of);
        let month = month_of(at);
        let (segment, segment_tip) = self.current_segment("log", &month)?;
        let mut files = BTreeMap::new();
        let existing = match &segment_tip {
            Some(tip) => self.git.read_tree(tip)?,
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
    fn backoff_grows_and_stays_bounded() {
        assert!(backoff(1) < Duration::from_millis(15));
        assert!(backoff(30) < Duration::from_millis(1_920));
        assert!(backoff(30) >= Duration::from_millis(640));
    }
}
