//! Stored state: entities of fields, each field a multi-value register of dotted values, with a causal context.
//!
//! A replica's state is the whole current state; joining two states is commutative, associative and idempotent, so
//! any order of syncs gives the same result. Concurrent writes to a field keep every value until a later write that
//! has seen them supersedes them. The layer doesn't interpret entities or fields, so unknown ones pass through
//! unchanged; the model gives them meaning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The newest state format this build reads.
pub const FORMAT_VERSION: u32 = 2;

/// How far a writer's clock may step back before a write is refused, in milliseconds.
pub const CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

/// A long-lived writer of state: the Mac's replica or a server's. Runs are not replicas.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplicaId(String);

impl ReplicaId {
    /// A replica id of lowercase ASCII letters, digits and `-`.
    ///
    /// # Errors
    ///
    /// Fails on an empty id or any other character.
    pub fn new(id: &str) -> Result<Self, Error> {
        if id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(Error::InvalidName(id.to_owned()));
        }
        Ok(Self(id.to_owned()))
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A write's identity: the replica that applied it and that replica's counter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Dot(pub ReplicaId, pub u64);

/// An entity's address: its kind, such as `item`, and its id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Key {
    /// The entity's kind.
    pub kind: String,
    /// The entity's id within its kind.
    pub id: String,
}

impl Key {
    /// A key whose kind and id are safe as path segments on any file system, case-insensitive ones included:
    /// lowercase ASCII letters, digits, `-`, `_` and `.`, not starting with `.`.
    ///
    /// # Errors
    ///
    /// Fails when the kind or id is empty or unsafe.
    pub fn new(kind: &str, id: &str) -> Result<Self, Error> {
        for part in [kind, id] {
            let safe = !part.is_empty()
                && !part.starts_with('.')
                && part.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
                });
            if !safe {
                return Err(Error::InvalidName(part.to_owned()));
            }
        }
        Ok(Self { kind: kind.to_owned(), id: id.to_owned() })
    }

    /// The entity's file path, fanned out over 256 directories by a hash of the id, so that no directory grows
    /// large however ids are chosen: Git rewrites a whole directory's listing for a change to one file in it.
    #[must_use]
    pub fn path(&self) -> String {
        format!("{}/{:02x}/{}.json", self.kind, fan(&self.id), self.id)
    }
}

/// 32-bit FNV-1a of the id, folded to a byte. Written out here, since the layout must never change with a library's
/// hasher.
fn fan(id: &str) -> u8 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let [a, b, c, d] = hash.to_le_bytes();
    a ^ b ^ c ^ d
}

/// One value of a field, with who wrote it and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamped {
    /// The actor behind the write: a person, an agent or a run (`run/<id>`).
    pub actor: String,
    /// When the writer applied it.
    pub at: Millis,
    /// The value. Numbers must be integers.
    pub value: Json,
}

/// A field: every current value, by the dot that wrote it. More than one value is a conflict.
pub type Register = BTreeMap<Dot, Stamped>;

/// An entity: its fields by name.
pub type Entity = BTreeMap<String, Register>;

/// What a replica has seen: for each writer, the dots `1..=n`, and the writer's latest timestamp.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// The highest counter seen from each writer; every lower one was seen too.
    pub seen: BTreeMap<ReplicaId, u64>,
    /// The latest timestamp seen from each writer.
    pub clocks: BTreeMap<ReplicaId, Millis>,
}

impl Context {
    /// Whether this context has seen a dot.
    #[must_use]
    pub fn covers(&self, dot: &Dot) -> bool {
        self.seen.get(&dot.0).is_some_and(|&n| dot.1 <= n)
    }

    fn join(&mut self, other: &Self) {
        for (replica, &n) in &other.seen {
            let entry = self.seen.entry(replica.clone()).or_default();
            *entry = (*entry).max(n);
        }
        for (replica, &at) in &other.clocks {
            let entry = self.clocks.entry(replica.clone()).or_insert(at);
            *entry = (*entry).max(at);
        }
    }
}

/// A replica's whole state.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// What the state has seen.
    pub context: Context,
    entities: BTreeMap<Key, Entity>,
    /// While recording, what the state was before each change: see [`State::record`].
    journal: Option<Journal>,
}

/// Two states are equal when they hold the same context and entities, whatever either is recording.
impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && self.entities == other.entities
    }
}

impl Eq for State {}

/// A state's changes since it began recording: the context then, and each entity changed since as it was then
/// (`None` where it was absent). A batch commits, logs and caches only these, rather than comparing whole states.
#[derive(Debug, Clone, Default)]
pub(crate) struct Journal {
    context: Context,
    entities: BTreeMap<Key, Option<Entity>>,
}

impl Journal {
    /// The keys of the entities that changed.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &Key> {
        self.entities.keys()
    }

    /// A changed entity as it was before; `None` when it was absent.
    pub(crate) fn before(&self, key: &Key) -> Option<&Entity> {
        self.entities.get(key).and_then(Option::as_ref)
    }
}

/// Why a write, an operation or a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A name that isn't safe as an id or path segment.
    InvalidName(String),
    /// A value holding a non-integer number.
    InvalidValue,
    /// The writer's clock stepped back by more than [`CLOCK_SKEW_MS`].
    ClockJump {
        /// The clock's reading.
        now: Millis,
        /// The writer's latest recorded timestamp.
        last: Millis,
    },
    /// A rebuilt replica tried to write before its own entry was restored, which could reuse its dots.
    NotContinuous(ReplicaId),
    /// A joined state has seen more of this replica's writes than the replica has.
    IdReused {
        /// The replica.
        replica: ReplicaId,
        /// Its own highest counter.
        mine: u64,
        /// The joined state's entry for it.
        theirs: u64,
    },
    /// A run operation skipped a sequence number.
    Gap {
        /// The run.
        run: String,
        /// The sequence number expected next.
        expected: u64,
        /// The sequence number received.
        got: u64,
    },
    /// A run operation reached a replica other than the one applying the run.
    WrongReplica {
        /// The run.
        run: String,
        /// The replica applying it.
        applied_by: String,
    },
    /// The item already has a live claim in the writer's view.
    Claimed(Key),
    /// Only leaves are claimed; the item has children.
    NotALeaf(Key),
    /// A holder other than a run claimed without a time lease.
    LeaseRequired(String),
    /// Another holder's live claim on another item holds a named resource this item needs.
    ResourceHeld {
        /// The resource.
        resource: String,
        /// The item holding it.
        item: Key,
        /// Its holder.
        holder: String,
    },
    /// A state file that couldn't be read.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(f, "{name:?} is not a valid name"),
            Self::InvalidValue => write!(f, "values may hold integers but no other numbers"),
            Self::ClockJump { now, last } => {
                write!(
                    f,
                    "the clock reads {} but this replica last wrote at {}; it stepped back",
                    crate::time::rfc3339(*now),
                    crate::time::rfc3339(*last)
                )
            },
            Self::NotContinuous(replica) => {
                write!(f, "replica {replica} was rebuilt and hasn't recovered its own writes yet")
            },
            Self::IdReused { replica, mine, theirs } => write!(
                f,
                "another writer used replica id {replica} ({theirs} writes seen, this replica has {mine}), or this \
                 replica lost state; take a new replica id"
            ),
            Self::Gap { run, expected, got } => {
                write!(f, "run {run} sent operation {got} but {expected} comes next")
            },
            Self::WrongReplica { run, applied_by } => {
                write!(f, "run {run} is applied by replica {applied_by}")
            },
            Self::Claimed(key) => write!(f, "{} {} already has a live claim", key.kind, key.id),
            Self::LeaseRequired(holder) => {
                write!(f, "{holder} isn't a run, so its claim needs a time lease")
            },
            Self::NotALeaf(key) => {
                write!(f, "{} {} has children; claim one of them", key.kind, key.id)
            },
            Self::ResourceHeld { resource, item, holder } => {
                write!(f, "{holder} holds {resource} through {} {}", item.kind, item.id)
            },
            Self::Format(message) => write!(f, "unreadable state: {message}"),
        }
    }
}

impl std::error::Error for Error {}

fn check_value(value: &Json) -> Result<(), Error> {
    match value {
        Json::Number(n) if !(n.is_i64() || n.is_u64()) => Err(Error::InvalidValue),
        Json::Array(items) => items.iter().try_for_each(check_value),
        Json::Object(fields) => fields.values().try_for_each(check_value),
        _ => Ok(()),
    }
}

fn join_registers(a: &Register, a_seen: &Context, b: &Register, b_seen: &Context) -> Register {
    let keep =
        |mine: &Register, theirs: &Register, their_context: &Context| -> Vec<(Dot, Stamped)> {
            mine.iter()
                .filter(|(dot, _)| theirs.contains_key(dot) || !their_context.covers(dot))
                .map(|(dot, value)| (dot.clone(), value.clone()))
                .collect()
        };
    keep(a, b, b_seen).into_iter().chain(keep(b, a, a_seen)).collect()
}

impl State {
    /// An empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An entity, if any of its fields has a value.
    #[must_use]
    pub fn entity(&self, key: &Key) -> Option<&Entity> {
        self.entities.get(key)
    }

    /// A field's current values; empty when unset.
    #[must_use]
    pub fn field(&self, key: &Key, field: &str) -> Option<&Register> {
        self.entities.get(key).and_then(|entity| entity.get(field))
    }

    /// Every entity, in key order.
    pub fn entities(&self) -> impl Iterator<Item = (&Key, &Entity)> {
        self.entities.iter()
    }

    /// Joins another state into this one, reporting whether any entity changed.
    pub fn join(&mut self, other: &Self) -> bool {
        let journal = &mut self.journal;
        let keys: BTreeSet<&Key> = self.entities.keys().chain(other.entities.keys()).collect();
        let empty = Entity::new();
        let mut joined = BTreeMap::new();
        let mut changed = false;
        for key in keys {
            let mine = self.entities.get(key).unwrap_or(&empty);
            let theirs = other.entities.get(key).unwrap_or(&empty);
            let names: BTreeSet<&String> = mine.keys().chain(theirs.keys()).collect();
            let mut entity = Entity::new();
            for name in names {
                let none = Register::new();
                let register = join_registers(
                    mine.get(name).unwrap_or(&none),
                    &self.context,
                    theirs.get(name).unwrap_or(&none),
                    &other.context,
                );
                if !register.is_empty() {
                    entity.insert(name.clone(), register);
                }
            }
            if *mine != entity {
                changed = true;
                if let Some(journal) = journal {
                    journal
                        .entities
                        .entry(key.clone())
                        .or_insert_with(|| self.entities.get(key).cloned());
                }
            }
            if !entity.is_empty() {
                joined.insert(key.clone(), entity);
            }
        }
        self.entities = joined;
        self.context.join(&other.context);
        changed
    }

    /// Checks that every stored dot is covered by the state's own context, so `seen` means `1..=n` for each writer.
    ///
    /// # Errors
    ///
    /// Fails on a dot beyond its writer's entry.
    pub fn check(&self) -> Result<(), Error> {
        for (key, entity) in &self.entities {
            for (name, register) in entity {
                if let Some(dot) = register.keys().find(|dot| !self.context.covers(dot)) {
                    return Err(Error::Format(format!(
                        "{}/{} field {name} holds dot {}:{} beyond the context",
                        key.kind, key.id, dot.0, dot.1
                    )));
                }
            }
        }
        Ok(())
    }

    /// Sets a field, reporting whether its values changed.
    fn set(&mut self, key: &Key, field: &str, register: Register) -> bool {
        let current = self.field(key, field);
        if current.map_or(register.is_empty(), |current| *current == register) {
            return false;
        }
        if let Some(journal) = &mut self.journal {
            journal.entities.entry(key.clone()).or_insert_with(|| self.entities.get(key).cloned());
        }
        if register.is_empty() {
            if let Some(entity) = self.entities.get_mut(key) {
                entity.remove(field);
                if entity.is_empty() {
                    self.entities.remove(key);
                }
            }
        } else {
            self.entities.entry(key.clone()).or_default().insert(field.to_owned(), register);
        }
        true
    }

    /// Starts recording changes afresh, for [`State::take_journal`].
    pub(crate) fn record(&mut self) {
        self.journal = Some(Journal { context: self.context.clone(), entities: BTreeMap::new() });
    }

    /// Stops recording and returns the changes since [`State::record`], leaving out entities changed back.
    pub(crate) fn take_journal(&mut self) -> Option<Journal> {
        let mut journal = self.journal.take()?;
        journal.entities.retain(|key, before| self.entities.get(key) != before.as_ref());
        Some(journal)
    }

    /// Whether nothing changed since the journal began.
    pub(crate) fn unchanged_since(&self, journal: &Journal) -> bool {
        journal.entities.is_empty() && journal.context == self.context
    }

    /// Swaps the journal's earlier context and entities with the current ones, turning this state into the one it
    /// was when recording began and the journal into a record of the later one; swapping again turns both back.
    /// Costs only the changed entities.
    pub(crate) fn swap(&mut self, journal: &mut Journal) {
        std::mem::swap(&mut self.context, &mut journal.context);
        for (key, other) in &mut journal.entities {
            let current = match other.take() {
                Some(entity) => self.entities.insert(key.clone(), entity),
                None => self.entities.remove(key),
            };
            *other = current;
        }
    }
}

/// Whether a replica may mint dots: only once its own entry is known to be its true highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuity {
    /// The replica's own state is intact, so its entry is its highest; it may write offline.
    Intact,
    /// The replica was rebuilt without its own state; it may write once it has joined a state holding its entry, or
    /// it must take a new id.
    Rebuilt,
}

/// A replica writing to its state.
#[derive(Debug, Clone)]
pub struct Replica {
    id: ReplicaId,
    continuity: Continuity,
    /// How many writes and joins changed an entity.
    changes: u64,
    /// Read-only outside the crate: a batch records the changes made through the replica's own methods, and
    /// replacing the state wholesale would hide them.
    pub(crate) state: State,
}

/// An operation from a run, applied by the replica that runs it under the replica's own dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    /// The run's id.
    pub run: String,
    /// The run's sequence number for this operation, from 1.
    pub seq: u64,
    /// The entity written.
    pub key: Key,
    /// The field written.
    pub field: String,
    /// The new value, or `None` to clear the field.
    pub value: Option<Json>,
    /// The dots of the field's values the run had seen; they are superseded, and others survive as conflicts.
    pub seen: Vec<Dot>,
}

/// What applying a run operation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// Applied, under this dot unless it cleared the field.
    Applied(Option<Dot>),
    /// Already applied; nothing changed.
    Duplicate,
}

/// The run entity's field holding the last applied sequence number.
pub const RUN_APPLIED_SEQ: &str = "applied-seq";
/// The run entity's field naming the replica that applies the run.
pub const RUN_APPLIED_BY: &str = "applied-by";

impl Replica {
    /// A replica whose own state is intact.
    #[must_use]
    pub fn open(id: ReplicaId, mut state: State) -> Self {
        state.journal = None;
        Self { id, continuity: Continuity::Intact, changes: 0, state }
    }

    /// A replica rebuilt without its own state, from whatever it could fetch.
    #[must_use]
    pub fn rebuilt(id: ReplicaId, mut state: State) -> Self {
        state.journal = None;
        Self { id, continuity: Continuity::Rebuilt, changes: 0, state }
    }

    /// The replica's state.
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The replica's state, taken out of it.
    #[must_use]
    pub fn into_state(self) -> State {
        self.state
    }

    /// Marks a rebuilt replica as holding its true highest counter again. Call it only after joining every state ref
    /// freshly fetched from the remote, its own pushed ref included: a stale peer's entry for this replica is not
    /// enough. Otherwise take a new replica id.
    pub fn mark_restored(&mut self) {
        self.continuity = Continuity::Intact;
    }

    /// The replica's id.
    #[must_use]
    pub fn id(&self) -> &ReplicaId {
        &self.id
    }

    /// How many of this replica's writes and joins changed an entity: compare two readings to learn whether
    /// something in between did. A clear changes an entity without minting a dot.
    #[must_use]
    pub fn changes(&self) -> u64 {
        self.changes
    }

    fn set(&mut self, key: &Key, field: &str, register: Register) {
        if self.state.set(key, field, register) {
            self.changes += 1;
        }
    }

    /// Joins another state.
    ///
    /// # Errors
    ///
    /// An intact replica refuses a state that has seen more of its own writes than it has: another clone is writing
    /// under this replica's id, or this replica lost state. Joining it could reuse dots and lose writes silently.
    pub fn join(&mut self, other: &State) -> Result<(), Error> {
        if self.continuity == Continuity::Intact {
            let mine = self.state.context.seen.get(&self.id).copied().unwrap_or(0);
            let theirs = other.context.seen.get(&self.id).copied().unwrap_or(0);
            if theirs > mine {
                return Err(Error::IdReused { replica: self.id.clone(), mine, theirs });
            }
        }
        if self.state.join(other) {
            self.changes += 1;
        }
        Ok(())
    }

    /// The timestamp for a write now: never earlier than this replica's last, and refused when the clock stepped back
    /// by more than [`CLOCK_SKEW_MS`] unless `allow_clock_step` is set.
    ///
    /// # Errors
    ///
    /// Fails on a backward step beyond the skew.
    pub fn stamp(&self, now: Millis, allow_clock_step: bool) -> Result<Millis, Error> {
        let last = self.state.context.clocks.get(&self.id).copied().unwrap_or(Millis::MIN);
        if now < last.saturating_sub(CLOCK_SKEW_MS) && !allow_clock_step {
            return Err(Error::ClockJump { now, last });
        }
        Ok(now.max(last))
    }

    /// Mints the next dot, with `at` raised to the replica's latest timestamp if it is earlier.
    fn mint(&mut self, at: Millis) -> Result<(Dot, Millis), Error> {
        if self.continuity == Continuity::Rebuilt {
            return Err(Error::NotContinuous(self.id.clone()));
        }
        let counter = self.state.context.seen.entry(self.id.clone()).or_default();
        *counter += 1;
        let dot = Dot(self.id.clone(), *counter);
        let clock = self.state.context.clocks.entry(self.id.clone()).or_insert(at);
        *clock = (*clock).max(at);
        Ok((dot, *clock))
    }

    /// Drops the field's values that `keep` rejects, minting no dot: the context still covers them, so a join removes
    /// them from every replica, while values written elsewhere unseen survive. Returns whether anything was dropped.
    pub fn retain(&mut self, key: &Key, field: &str, keep: impl Fn(&Stamped) -> bool) -> bool {
        let Some(current) = self.state.field(key, field) else { return false };
        let kept: Register =
            current.iter().filter(|(_, v)| keep(v)).map(|(d, v)| (d.clone(), v.clone())).collect();
        if kept.len() == current.len() {
            return false;
        }
        self.set(key, field, kept);
        true
    }

    /// Writes a field, superseding every value this replica has seen. `None` clears it.
    ///
    /// # Errors
    ///
    /// Fails on a non-integer number or when the replica can't mint dots yet.
    pub fn write(
        &mut self,
        key: &Key,
        field: &str,
        actor: &str,
        at: Millis,
        value: Option<Json>,
    ) -> Result<Option<Dot>, Error> {
        if let Some(value) = &value {
            check_value(value)?;
        }
        let Some(value) = value else {
            self.set(key, field, Register::new());
            return Ok(None);
        };
        let (dot, at) = self.mint(at)?;
        let stamped = Stamped { actor: actor.to_owned(), at, value };
        self.set(key, field, Register::from([(dot.clone(), stamped)]));
        Ok(Some(dot))
    }

    /// Applies a run's operation. Sequence numbers must arrive in order from 1: a repeat is a duplicate and a gap is
    /// refused. The new value supersedes the values the run saw and the run's own earlier values; values it didn't
    /// see survive beside it as a conflict.
    ///
    /// # Errors
    ///
    /// Fails on a gap, on a run applied by another replica, on a non-integer number, or when the replica can't mint.
    pub fn apply(&mut self, op: &Op, at: Millis) -> Result<Applied, Error> {
        let run = Key::new("run", &op.run)?;
        let applied_by = self
            .state
            .field(&run, RUN_APPLIED_BY)
            .and_then(|r| r.values().find_map(|v| v.value.as_str().map(str::to_owned)));
        if let Some(applied_by) = applied_by
            && applied_by != self.id.as_str()
        {
            return Err(Error::WrongReplica { run: op.run.clone(), applied_by });
        }
        let last = self
            .state
            .field(&run, RUN_APPLIED_SEQ)
            .and_then(|r| r.values().filter_map(|v| v.value.as_u64()).max())
            .unwrap_or(0);
        if op.seq <= last {
            return Ok(Applied::Duplicate);
        }
        if op.seq != last + 1 {
            return Err(Error::Gap { run: op.run.clone(), expected: last + 1, got: op.seq });
        }
        if let Some(value) = &op.value {
            check_value(value)?;
        }

        let actor = format!("run/{}", op.run);
        let seen: BTreeSet<&Dot> = op.seen.iter().collect();
        let mut register: Register = self
            .state
            .field(&op.key, &op.field)
            .into_iter()
            .flatten()
            .filter(|(dot, value)| !seen.contains(dot) && value.actor != actor)
            .map(|(dot, value)| (dot.clone(), value.clone()))
            .collect();
        let dot = match op.value.clone() {
            Some(value) => {
                let (dot, at) = self.mint(at)?;
                register.insert(dot.clone(), Stamped { actor: actor.clone(), at, value });
                Some(dot)
            },
            None => None,
        };
        self.set(&op.key, &op.field, register);
        let system = format!("replica/{}", self.id);
        self.write(&run, RUN_APPLIED_SEQ, &system, at, Some(Json::from(op.seq)))?;
        if self.state.field(&run, RUN_APPLIED_BY).is_none() {
            self.write(&run, RUN_APPLIED_BY, &system, at, Some(Json::from(self.id.as_str())))?;
        }
        Ok(Applied::Applied(dot))
    }
}

/// A state's `state.json`: its format version and context.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format_version: u32,
    context: Context,
}

/// A whole state in one document: see [`State::to_snapshot`].
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    format_version: u32,
    context: Context,
    entities: Vec<EntityFile>,
}

/// Changed entities for the cache's delta: the whole context, each changed entity, and each removed one's key.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delta {
    context: Context,
    entities: Vec<EntityFile>,
    removed: Vec<Key>,
}

/// An entity file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityFile {
    kind: String,
    id: String,
    fields: BTreeMap<String, Vec<ValueFile>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueFile {
    dot: Dot,
    actor: String,
    at: Millis,
    value: Json,
}

/// The path of the state's header file.
pub const HEADER_PATH: &str = "state.json";

/// Serializes `value` with object keys sorted, whatever `serde_json`'s features.
fn canonical<T: Serialize>(value: &T) -> String {
    fn sort(value: Json) -> Json {
        match value {
            Json::Object(map) => {
                let sorted: BTreeMap<String, Json> =
                    map.into_iter().map(|(k, v)| (k, sort(v))).collect();
                Json::Object(sorted.into_iter().collect())
            },
            Json::Array(items) => Json::Array(items.into_iter().map(sort).collect()),
            other => other,
        }
    }
    let value = serde_json::to_value(value).expect("state always serializes");
    let mut text = serde_json::to_string_pretty(&sort(value)).expect("state always serializes");
    text.push('\n');
    text
}

fn entity_file(key: &Key, entity: &Entity) -> String {
    canonical(&entity_record(key, entity))
}

fn entity_record(key: &Key, entity: &Entity) -> EntityFile {
    let fields = entity
        .iter()
        .map(|(name, register)| {
            let values = register
                .iter()
                .map(|(dot, v)| ValueFile {
                    dot: dot.clone(),
                    actor: v.actor.clone(),
                    at: v.at,
                    value: v.value.clone(),
                })
                .collect();
            (name.clone(), values)
        })
        .collect();
    EntityFile { kind: key.kind.clone(), id: key.id.clone(), fields }
}

impl State {
    /// The state as files: [`HEADER_PATH`] and one file per entity at [`Key::path`], in canonical JSON.
    #[must_use]
    pub fn to_files(&self) -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        files.insert(HEADER_PATH.to_owned(), self.header_file());
        for (key, entity) in &self.entities {
            files.insert(key.path(), entity_file(key, entity));
        }
        files
    }

    /// The files changed since the journal began, as [`State::changed_files`] gives them; costs only the changes.
    pub(crate) fn changed_files_since(
        &self,
        journal: &Journal,
    ) -> BTreeMap<String, Option<String>> {
        let mut changes = BTreeMap::new();
        // Rewriting the header only when the context moves keeps an older version's header until then; that is safe
        // because any value an older build can't read (a clear's null) arrives with a new dot, so the context moves.
        if self.context != journal.context {
            changes.insert(HEADER_PATH.to_owned(), Some(self.header_file()));
        }
        for key in journal.keys() {
            let file = self.entities.get(key).map(|entity| entity_file(key, entity));
            changes.insert(key.path(), file);
        }
        changes
    }

    /// The files that differ from `before`'s: `Some` content for each new or changed file, `None` for each removed
    /// one. Serializes only what changed, so it costs nothing like [`State::to_files`] twice.
    #[must_use]
    pub fn changed_files(&self, before: &State) -> BTreeMap<String, Option<String>> {
        let mut changes = BTreeMap::new();
        if self.context != before.context {
            changes.insert(HEADER_PATH.to_owned(), Some(self.header_file()));
        }
        for (key, entity) in &self.entities {
            if before.entities.get(key) != Some(entity) {
                changes.insert(key.path(), Some(entity_file(key, entity)));
            }
        }
        for key in before.entities.keys().filter(|key| !self.entities.contains_key(*key)) {
            changes.insert(key.path(), None);
        }
        changes
    }

    fn header_file(&self) -> String {
        canonical(&Header { format_version: FORMAT_VERSION, context: self.context.clone() })
    }

    /// Reads a state from its files, refusing any that aren't in canonical form, as when they come from another
    /// replica.
    ///
    /// # Errors
    ///
    /// Fails on a missing or newer header, a malformed file, a file at the wrong path, a dot beyond the context, or a
    /// file not in canonical form.
    pub fn from_files(files: &BTreeMap<String, String>) -> Result<Self, Error> {
        let state = Self::from_own_files(files)?;
        let mut expected = state.to_files();
        // An older version's header stays canonical under its own number until the next write rewrites it.
        let version = files
            .get(HEADER_PATH)
            .and_then(|text| serde_json::from_str::<Json>(text).ok())
            .and_then(|header| header.get("format_version").and_then(Json::as_u64))
            .and_then(|version| u32::try_from(version).ok())
            .unwrap_or(FORMAT_VERSION);
        expected.insert(
            HEADER_PATH.to_owned(),
            canonical(&Header { format_version: version, context: state.context.clone() }),
        );
        if expected != *files {
            return Err(Error::Format("files are not in canonical form".to_owned()));
        }
        Ok(state)
    }

    /// Reads a state from files this replica wrote itself, skipping the canonical-form check, which re-serializes
    /// the whole state; every other check stays.
    ///
    /// # Errors
    ///
    /// Fails on a missing or newer header, a malformed file, a file at the wrong path, or a dot beyond the context.
    pub fn from_own_files(files: &BTreeMap<String, String>) -> Result<Self, Error> {
        let header =
            files.get(HEADER_PATH).ok_or_else(|| Error::Format("no state.json".to_owned()))?;
        let header: Json =
            serde_json::from_str(header).map_err(|e| Error::Format(format!("state.json: {e}")))?;
        check_version(&header)?;
        let header: Header = serde_json::from_value(header)
            .map_err(|e| Error::Format(format!("state.json: {e}")))?;
        let entities =
            files.iter().filter(|(path, _)| path.as_str() != HEADER_PATH).map(|(path, text)| {
                let file: EntityFile = serde_json::from_str(text)
                    .map_err(|e| Error::Format(format!("{path}: {e}")))?;
                Ok((Some(path.as_str()), file))
            });
        Self::assemble(header.context, entities)
    }

    /// The whole state as one compact document, for a local cache. It has no canonical form, since nothing hashes
    /// it.
    ///
    /// # Panics
    ///
    /// Never: a state holds only strings, integers and the like.
    #[must_use]
    pub fn to_snapshot(&self) -> String {
        let entities =
            self.entities.iter().map(|(key, entity)| entity_record(key, entity)).collect();
        let snapshot =
            Snapshot { format_version: FORMAT_VERSION, context: self.context.clone(), entities };
        serde_json::to_string(&snapshot).expect("state always serializes")
    }

    /// Reads a document written by [`State::to_snapshot`], with every check but the canonical form's.
    ///
    /// # Errors
    ///
    /// Fails on another format version, a malformed document or a dot beyond the context.
    pub fn from_snapshot(text: &str) -> Result<Self, Error> {
        let snapshot: Snapshot =
            serde_json::from_str(text).map_err(|e| Error::Format(format!("snapshot: {e}")))?;
        if snapshot.format_version != FORMAT_VERSION {
            return Err(Error::Format(format!(
                "snapshot format version {} isn't {FORMAT_VERSION}",
                snapshot.format_version
            )));
        }
        Self::assemble(snapshot.context, snapshot.entities.into_iter().map(|file| Ok((None, file))))
    }

    /// Builds a state from its context and entity files, checking each file's place (when it has a path), its
    /// values and finally the dots against the context.
    fn assemble<'a>(
        context: Context,
        files: impl Iterator<Item = Result<(Option<&'a str>, EntityFile), Error>>,
    ) -> Result<Self, Error> {
        let mut state = Self { context, ..Self::default() };
        for file in files {
            let (path, file) = file?;
            let (key, entity) = read_entity(path, file)?;
            if !entity.is_empty() {
                state.entities.insert(key, entity);
            }
        }
        state.check()?;
        Ok(state)
    }

    /// The context and the entities at `keys` as one compact document, for the cache's delta: a removed entity
    /// becomes a tombstone.
    pub(crate) fn to_delta<'a>(&self, keys: impl Iterator<Item = &'a Key>) -> String {
        let (mut entities, mut removed) = (Vec::new(), Vec::new());
        for key in keys {
            match self.entities.get(key) {
                Some(entity) => entities.push(entity_record(key, entity)),
                None => removed.push(key.clone()),
            }
        }
        let delta = Delta { context: self.context.clone(), entities, removed };
        serde_json::to_string(&delta).expect("state always serializes")
    }

    /// Applies a document written by [`State::to_delta`], returning the keys it held.
    ///
    /// # Errors
    ///
    /// Fails on a malformed document or a dot beyond the context; the state is then partly changed.
    pub(crate) fn apply_delta(&mut self, text: &str) -> Result<BTreeSet<Key>, Error> {
        let delta: Delta =
            serde_json::from_str(text).map_err(|e| Error::Format(format!("delta: {e}")))?;
        let mut keys = BTreeSet::new();
        self.context = delta.context;
        for file in delta.entities {
            let (key, entity) = read_entity(None, file)?;
            if entity.is_empty() {
                self.entities.remove(&key);
            } else {
                self.entities.insert(key.clone(), entity);
            }
            keys.insert(key);
        }
        for key in delta.removed {
            let key = Key::new(&key.kind, &key.id)?;
            self.entities.remove(&key);
            keys.insert(key);
        }
        // Contexts only grow, so only the delta's own entities can hold a dot beyond the new one.
        for key in &keys {
            let dots =
                self.entities.get(key).into_iter().flat_map(|e| e.values()).flat_map(|r| r.keys());
            if let Some(dot) = dots.into_iter().find(|dot| !self.context.covers(dot)) {
                return Err(Error::Format(format!(
                    "delta: {}/{} holds dot {}:{} beyond the context",
                    key.kind, key.id, dot.0, dot.1
                )));
            }
        }
        Ok(keys)
    }
}

/// An entity file's key and entity, checking its place (when it has a path) and its values.
fn read_entity(path: Option<&str>, file: EntityFile) -> Result<(Key, Entity), Error> {
    let key = Key::new(&file.kind, &file.id)?;
    let at = path.map_or_else(|| key.path(), str::to_owned);
    if path.is_some_and(|path| key.path() != path) {
        return Err(Error::Format(format!("{at} holds {}", key.path())));
    }
    for value in file.fields.values().flatten() {
        check_value(&value.value).map_err(|e| Error::Format(format!("{at}: {e}")))?;
    }
    let mut entity = Entity::new();
    for (name, values) in file.fields {
        let register: Register = values
            .into_iter()
            .map(|v| (v.dot, Stamped { actor: v.actor, at: v.at, value: v.value }))
            .collect();
        if !register.is_empty() {
            entity.insert(name, register);
        }
    }
    Ok((key, entity))
}

fn check_version(header: &Json) -> Result<(), Error> {
    let version = header.get("format_version").and_then(Json::as_u64).unwrap_or(0);
    if version == 0 || version > u64::from(FORMAT_VERSION) {
        return Err(Error::Format(format!(
            "format version {version} is not supported; this build reads 1 to {FORMAT_VERSION}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
