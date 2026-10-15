//! The model: items, links, threads, repositories and runs, read as a typed snapshot of the state and written through
//! checked writes. Nothing derived is stored: roles, membership, repository sets and cycles are computed on read.
//!
//! Every field reads as a [`Value`], so a reader sees concurrent writes as a conflict and never silently picks one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::graph;
use crate::schema::{item as field, kind, link, repository, run, set, thread};
use crate::state::{Error, Key, Millis, Replica, State};

/// The longest item id: a link's id holds two, and ids are file names.
pub const MAX_ID_LEN: usize = 64;

/// A field's current value: unset, one value, or several written concurrently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value<T> {
    /// Unset.
    None,
    /// One value, or several equal ones.
    One(T),
    /// Concurrent, different values, in dot order.
    Conflict(Vec<T>),
}

impl<T> Value<T> {
    /// The value when there is exactly one.
    #[must_use]
    pub fn one(&self) -> Option<&T> {
        match self {
            Self::One(value) => Some(value),
            _ => None,
        }
    }

    /// Every value: none, one, or each conflicting one.
    #[must_use]
    pub fn all(&self) -> Vec<&T> {
        match self {
            Self::None => Vec::new(),
            Self::One(value) => vec![value],
            Self::Conflict(values) => values.iter().collect(),
        }
    }

    /// Whether the field holds conflicting values.
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict(_))
    }
}

/// An item's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemState {
    /// Not started.
    Backlog,
    /// Set aside on purpose.
    Parked,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
}

/// A rough size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Size {
    /// Extra small.
    Xs,
    /// Small.
    S,
    /// Medium.
    M,
    /// Large.
    L,
    /// Extra large.
    Xl,
}

/// What shows a done item was done.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The id of the repository record holding the commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The commit that completed the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// A free-form note, such as where a record lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A link's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    /// A hard dependency: `from` waits for `to`.
    Blocks,
    /// A soft dependency: `from` prefers to follow `to`.
    After,
    /// Context only.
    Relates,
}

impl LinkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::After => "after",
            Self::Relates => "relates",
        }
    }

    /// Whether the kind orders work, so it counts in cycle checks.
    #[must_use]
    pub fn orders(self) -> bool {
        matches!(self, Self::Blocks | Self::After)
    }
}

/// A run's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// Launched, not yet working.
    Starting,
    /// Working.
    Running,
    /// Stopped before its thread was finished.
    Stopped,
    /// Finished its work.
    Done,
}

impl RunState {
    /// Whether the run is under way.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

/// An item's role, derived from its place in the tree (plan D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// No children.
    Item,
    /// Children and a gate.
    Goal,
    /// Children and no gate.
    Group,
    /// Top level with children, gate or not.
    Project,
}

/// An item as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The item's id.
    pub id: String,
    /// A short title.
    pub title: Value<String>,
    /// Design notes.
    pub notes: Value<String>,
    /// The exit check.
    pub gate: Value<String>,
    /// The item's state; unset reads as backlog.
    pub state: Value<ItemState>,
    /// When the item was marked done: the earliest of its current done values.
    pub done_at: Option<Millis>,
    /// The parent item's id.
    pub parent: Value<String>,
    /// A rough size.
    pub size: Value<Size>,
    /// Whether this item is the root of an archived subtree.
    pub archived: Value<bool>,
    /// Evidence of completion.
    pub evidence: Value<Evidence>,
    /// The thread the item belongs to.
    pub thread: Value<String>,
    /// Area tags.
    pub areas: BTreeSet<String>,
    /// Ids of the repositories the item touches.
    pub repositories: BTreeSet<String>,
    /// Named resources the item holds while worked on.
    pub resources: BTreeSet<String>,
}

impl Item {
    /// Whether the item is done; a conflicted state is done only when every value is.
    #[must_use]
    pub fn is_done(&self) -> bool {
        let states = self.state.all();
        !states.is_empty() && states.iter().all(|state| **state == ItemState::Done)
    }

    /// Whether the item is marked as an archived subtree's root; a conflict counts only if every value says so.
    #[must_use]
    pub fn is_archived_root(&self) -> bool {
        let values = self.archived.all();
        !values.is_empty() && values.iter().all(|archived| **archived)
    }
}

/// A link as read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Link {
    /// The dependent item's id.
    pub from: String,
    /// The id of the item it depends on.
    pub to: String,
    /// The link's kind.
    pub kind: LinkKind,
}

/// A thread as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// The thread's id.
    pub id: String,
    /// A short title.
    pub title: Value<String>,
    /// The brief.
    pub brief: Value<String>,
    /// Whether the thread's scope is approved; a conflict counts as approved, which protects more.
    pub approved: Value<bool>,
    /// Whether the thread's agent is a project-management agent; a conflict counts as not.
    pub pm_flag: Value<bool>,
    /// Repositories added explicitly, beyond those of its items.
    pub repositories: BTreeSet<String>,
}

impl Thread {
    /// Whether the thread's scope is approved: any approving value counts, so a conflict protects the scope.
    #[must_use]
    pub fn is_approved(&self) -> bool {
        self.approved.all().into_iter().any(|approved| *approved)
    }

    /// Whether the thread's agent is a project-management agent: only when every value says so.
    #[must_use]
    pub fn is_pm(&self) -> bool {
        let values = self.pm_flag.all();
        !values.is_empty() && values.into_iter().all(|flag| *flag)
    }
}

/// A repository record as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The record's id.
    pub id: String,
    /// The owner, such as an organization.
    pub owner: Value<String>,
    /// The repository's name.
    pub name: Value<String>,
    /// Whether the repository is public.
    pub public: Value<bool>,
    /// The default branch.
    pub default_branch: Value<String>,
    /// The App's installation that reaches it.
    pub installation: Value<String>,
}

/// A run record as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The run's id.
    pub id: String,
    /// The thread it works on.
    pub thread: Value<String>,
    /// Its state.
    pub state: Value<RunState>,
}

impl Run {
    /// Whether the run may be under way: any active value counts.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.all().into_iter().any(|state| state.is_active())
    }
}

/// A snapshot of the model, read once from a state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    /// Items by id.
    pub items: BTreeMap<String, Item>,
    /// Present links, sorted.
    pub links: Vec<Link>,
    /// Threads by id.
    pub threads: BTreeMap<String, Thread>,
    /// Repository records by id.
    pub repositories: BTreeMap<String, Repository>,
    /// Runs by id.
    pub runs: BTreeMap<String, Run>,
    children: BTreeMap<String, BTreeSet<String>>,
}

impl Model {
    /// Reads the model from a state.
    ///
    /// # Errors
    ///
    /// Fails when a value can't be read, such as one written by a newer version: hiding it could hide a blocker.
    pub fn read(state: &State) -> Result<Self, Error> {
        let mut model = Self::default();
        for (key, _) in state.entities() {
            match key.kind.as_str() {
                kind::ITEM => {
                    let item = read_item(state, key)?;
                    for parent in item.parent.all() {
                        model.children.entry(parent.clone()).or_default().insert(key.id.clone());
                    }
                    model.items.insert(key.id.clone(), item);
                },
                kind::LINK => {
                    if state.field(key, link::PRESENT).is_none() {
                        continue;
                    }
                    let (Value::One(from), Value::One(to), Value::One(kind)) = (
                        value::<String>(state, key, link::FROM)?,
                        value::<String>(state, key, link::TO)?,
                        value::<LinkKind>(state, key, link::KIND)?,
                    ) else {
                        return Err(Error::Format(format!(
                            "link {} lacks its ends or kind",
                            key.id
                        )));
                    };
                    model.links.push(Link { from, to, kind });
                },
                kind::THREAD => {
                    let thread = Thread {
                        id: key.id.clone(),
                        title: value(state, key, thread::TITLE)?,
                        brief: value(state, key, thread::BRIEF)?,
                        approved: value(state, key, thread::APPROVED)?,
                        pm_flag: value(state, key, thread::PM_FLAG)?,
                        repositories: elements(state, key, set::REPOSITORY),
                    };
                    model.threads.insert(key.id.clone(), thread);
                },
                kind::REPOSITORY => {
                    let record = Repository {
                        id: key.id.clone(),
                        owner: value(state, key, repository::OWNER)?,
                        name: value(state, key, repository::NAME)?,
                        public: value(state, key, repository::PUBLIC)?,
                        default_branch: value(state, key, repository::DEFAULT_BRANCH)?,
                        installation: value(state, key, repository::INSTALLATION)?,
                    };
                    model.repositories.insert(key.id.clone(), record);
                },
                kind::RUN => {
                    let record = Run {
                        id: key.id.clone(),
                        thread: value(state, key, run::THREAD)?,
                        state: value(state, key, run::STATE)?,
                    };
                    model.runs.insert(key.id.clone(), record);
                },
                _ => {},
            }
        }
        model.links.sort();
        Ok(model)
    }

    /// An item's children.
    #[must_use]
    pub fn children(&self, id: &str) -> Vec<&str> {
        self.children.get(id).into_iter().flatten().map(String::as_str).collect()
    }

    /// An item's ancestors, nearest first, following every value of a conflicted parent and each item once, so a
    /// parent cycle made by a join ends the walk.
    #[must_use]
    pub fn ancestors(&self, id: &str) -> Vec<&str> {
        let mut seen = BTreeSet::from([id]);
        let mut found = Vec::new();
        let mut frontier = vec![id];
        while let Some(next) = frontier.pop() {
            let Some(item) = self.items.get(next) else { continue };
            for parent in item.parent.all() {
                if seen.insert(parent.as_str()) {
                    found.push(parent.as_str());
                    frontier.push(parent.as_str());
                }
            }
        }
        found
    }

    /// An item and every item under it, each once.
    #[must_use]
    pub fn subtree<'a>(&'a self, id: &'a str) -> Vec<&'a str> {
        let mut seen = BTreeSet::new();
        let mut found = Vec::new();
        let mut frontier = vec![id];
        while let Some(next) = frontier.pop() {
            if seen.insert(next) {
                found.push(next);
                frontier.extend(self.children(next));
            }
        }
        found
    }

    /// An item's role. A top-level item with children is a project even with a gate; a conflicted parent isn't top
    /// level, and a conflicted gate counts as a gate.
    #[must_use]
    pub fn role(&self, id: &str) -> Role {
        let Some(item) = self.items.get(id) else { return Role::Item };
        if self.children(id).is_empty() {
            Role::Item
        } else if item.parent == Value::None {
            Role::Project
        } else if item.gate == Value::None {
            Role::Group
        } else {
            Role::Goal
        }
    }

    /// Whether an item or any ancestor is an archived subtree's root.
    #[must_use]
    pub fn is_archived(&self, id: &str) -> bool {
        std::iter::once(id)
            .chain(self.ancestors(id))
            .any(|id| self.items.get(id).is_some_and(Item::is_archived_root))
    }

    /// The items of a thread: those whose thread field is that thread alone. An item with a conflicted thread is in
    /// neither.
    #[must_use]
    pub fn members(&self, thread: &str) -> Vec<&str> {
        self.items
            .values()
            .filter(|item| item.thread.one().is_some_and(|t| t == thread))
            .map(|item| item.id.as_str())
            .collect()
    }

    /// The thread an item belongs to, when it is in exactly one.
    #[must_use]
    pub fn thread_of(&self, id: &str) -> Option<&str> {
        self.items.get(id).and_then(|item| item.thread.one()).map(String::as_str)
    }

    /// A thread's repository set: its items' repositories and those added explicitly.
    #[must_use]
    pub fn repository_set(&self, thread: &str) -> BTreeSet<String> {
        let mut repositories =
            self.threads.get(thread).map(|t| t.repositories.clone()).unwrap_or_default();
        for member in self.members(thread) {
            repositories.extend(self.items[member].repositories.iter().cloned());
        }
        repositories
    }

    /// The cycles among blocks and after links, each sorted.
    #[must_use]
    pub fn link_cycles(&self) -> Vec<Vec<String>> {
        let edges: Vec<(String, String)> = self
            .links
            .iter()
            .filter(|link| link.kind.orders())
            .map(|link| (link.from.clone(), link.to.clone()))
            .collect();
        graph::cycles(&edges)
    }

    /// A path of blocks and after links from `from` to `to`, both included; a single item when they are the same.
    #[must_use]
    pub fn ordering_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for link in self.links.iter().filter(|link| link.kind.orders()) {
            next.entry(link.from.as_str()).or_default().push(link.to.as_str());
        }
        let mut came_from: BTreeMap<&str, &str> = BTreeMap::from([(from, from)]);
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(node) = queue.pop_front() {
            if node == to {
                let mut path = vec![to.to_owned()];
                let mut at = to;
                while at != from {
                    at = came_from[at];
                    path.push(at.to_owned());
                }
                path.reverse();
                return Some(path);
            }
            for &after in next.get(node).into_iter().flatten() {
                if !came_from.contains_key(after) {
                    came_from.insert(after, node);
                    queue.push_back(after);
                }
            }
        }
        None
    }

    /// The cycles among parents, which only a join can make, each sorted.
    #[must_use]
    pub fn parent_cycles(&self) -> Vec<Vec<String>> {
        let edges: Vec<(String, String)> = self
            .items
            .values()
            .flat_map(|item| item.parent.all().into_iter().map(|p| (item.id.clone(), p.clone())))
            .collect();
        graph::cycles(&edges)
    }

    /// Threads with more than one active run, which only a join can make.
    #[must_use]
    pub fn run_clashes(&self) -> BTreeMap<String, Vec<String>> {
        let mut active: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for run in self.runs.values().filter(|run| run.is_active()) {
            for thread in run.thread.all() {
                active.entry(thread.clone()).or_default().push(run.id.clone());
            }
        }
        active.retain(|_, runs| runs.len() > 1);
        active
    }
}

/// A refused write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The state layer refused.
    State(Error),
    /// An id or element name that isn't valid.
    InvalidName(String),
    /// The item doesn't exist.
    NoItem(String),
    /// The write would put an item under itself or its own descendant.
    ParentCycle(String),
    /// The link would close a cycle of blocks and after links, through these items.
    LinkCycle(Vec<String>),
    /// The thread already has an active run.
    RunActive {
        /// The thread.
        thread: String,
        /// Its active run.
        run: String,
    },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(err) => err.fmt(f),
            Self::InvalidName(name) => write!(
                f,
                "{name:?} is not a valid name: use at most {MAX_ID_LEN} lowercase letters, digits, '-', '_' or '.'"
            ),
            Self::NoItem(id) => write!(f, "no item {id}"),
            Self::ParentCycle(id) => write!(f, "{id} can't go under itself or its own descendant"),
            Self::LinkCycle(path) => {
                write!(f, "the link would close a cycle through {}", path.join(", "))
            },
            Self::RunActive { thread, run } => {
                write!(f, "thread {thread} already has run {run} under way")
            },
        }
    }
}

impl std::error::Error for WriteError {}

impl From<Error> for WriteError {
    fn from(err: Error) -> Self {
        Self::State(err)
    }
}

/// Who writes, and when.
#[derive(Debug, Clone, Copy)]
pub struct By<'a> {
    /// The actor.
    pub actor: &'a str,
    /// The write's timestamp.
    pub at: Millis,
}

/// A checked id or element name: a valid key id of at most [`MAX_ID_LEN`] characters.
fn name(name: &str) -> Result<&str, WriteError> {
    if name.len() <= MAX_ID_LEN && Key::new(kind::ITEM, name).is_ok() {
        Ok(name)
    } else {
        Err(WriteError::InvalidName(name.to_owned()))
    }
}

/// An item's key, with its id checked.
///
/// # Errors
///
/// Fails on an invalid or overlong id.
pub fn item_key(id: &str) -> Result<Key, WriteError> {
    Ok(Key::new(kind::ITEM, name(id)?)?)
}

/// A link's key: its kind and both ends, length-prefixed so it is unambiguous.
///
/// # Errors
///
/// Fails on an invalid or overlong item id.
pub fn link_key(from: &str, to: &str, kind: LinkKind) -> Result<Key, WriteError> {
    let (from, to) = (name(from)?, name(to)?);
    Ok(Key::new(kind::LINK, &format!("{}-{}-{from}-{to}", kind.as_str(), from.len()))?)
}

fn to_json<T: Serialize>(value: &T) -> Json {
    serde_json::to_value(value).expect("model values always serialize")
}

/// Writes a field of any entity, or clears it with `None`.
///
/// # Errors
///
/// Fails when the state layer refuses.
pub fn set<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    value: Option<&T>,
    by: By<'_>,
) -> Result<(), WriteError> {
    replica.write(key, name, by.actor, by.at, value.map(to_json))?;
    Ok(())
}

/// Adds an element to a set field, or removes it.
///
/// # Errors
///
/// Fails on an invalid element name or when the state layer refuses.
pub fn set_element(
    replica: &mut Replica,
    key: &Key,
    prefix: &str,
    element: &str,
    present: bool,
    by: By<'_>,
) -> Result<(), WriteError> {
    let field = format!("{prefix}{}", name(element)?);
    replica.write(key, &field, by.actor, by.at, present.then_some(Json::Bool(true)))?;
    Ok(())
}

/// Creates an item with a title, in the backlog.
///
/// # Errors
///
/// Fails on an invalid id or when the state layer refuses.
pub fn add_item(
    replica: &mut Replica,
    id: &str,
    title: &str,
    by: By<'_>,
) -> Result<Key, WriteError> {
    let key = item_key(id)?;
    set(replica, &key, field::TITLE, Some(&title), by)?;
    set(replica, &key, field::STATE, Some(&ItemState::Backlog), by)?;
    Ok(key)
}

/// Sets an item's state. Setting the state it already has, alone, writes nothing, so re-marking done never moves
/// when it was done.
///
/// # Errors
///
/// Fails when the item doesn't exist or the state layer refuses.
pub fn set_state(
    replica: &mut Replica,
    id: &str,
    state: ItemState,
    by: By<'_>,
) -> Result<(), WriteError> {
    let key = existing(&replica.state, id)?;
    if value::<ItemState>(&replica.state, &key, field::STATE)? == Value::One(state) {
        return Ok(());
    }
    set(replica, &key, field::STATE, Some(&state), by)
}

/// Moves an item under `parent`, or to the top level with `None`.
///
/// # Errors
///
/// Fails when either item doesn't exist, when the move would put the item under itself or a descendant, or when the
/// state layer refuses.
pub fn set_parent(
    replica: &mut Replica,
    id: &str,
    parent: Option<&str>,
    by: By<'_>,
) -> Result<(), WriteError> {
    let key = existing(&replica.state, id)?;
    if let Some(parent) = parent {
        existing(&replica.state, parent)?;
        let model = Model::read(&replica.state)?;
        if parent == id || model.ancestors(parent).contains(&id) {
            return Err(WriteError::ParentCycle(id.to_owned()));
        }
    }
    set(replica, &key, field::PARENT, parent.map(str::to_owned).as_ref(), by)
}

/// Adds a link from `from` to `to`.
///
/// # Errors
///
/// Fails when either item doesn't exist, when a blocks or after link would close a cycle, or when the state layer
/// refuses.
pub fn link(
    replica: &mut Replica,
    from: &str,
    to: &str,
    kind: LinkKind,
    by: By<'_>,
) -> Result<Key, WriteError> {
    existing(&replica.state, from)?;
    existing(&replica.state, to)?;
    let key = link_key(from, to, kind)?;
    if kind.orders()
        && let Some(path) = Model::read(&replica.state)?.ordering_path(to, from)
    {
        return Err(WriteError::LinkCycle(path));
    }
    if replica.state.field(&key, link::FROM).is_none() {
        set(replica, &key, link::FROM, Some(&from), by)?;
        set(replica, &key, link::TO, Some(&to), by)?;
        set(replica, &key, link::KIND, Some(&kind), by)?;
    }
    set(replica, &key, link::PRESENT, Some(&true), by)?;
    Ok(key)
}

/// Removes a link; a concurrent re-add survives.
///
/// # Errors
///
/// Fails on an invalid id or when the state layer refuses.
pub fn unlink(
    replica: &mut Replica,
    from: &str,
    to: &str,
    kind: LinkKind,
    by: By<'_>,
) -> Result<(), WriteError> {
    let key = link_key(from, to, kind)?;
    set::<bool>(replica, &key, link::PRESENT, None, by)
}

/// Records a run starting on a thread.
///
/// # Errors
///
/// Fails when the thread already has an active run in this replica's view, or the state layer refuses.
pub fn start_run(
    replica: &mut Replica,
    id: &str,
    thread: &str,
    by: By<'_>,
) -> Result<Key, WriteError> {
    let model = Model::read(&replica.state)?;
    if let Some(active) = model
        .runs
        .values()
        .find(|run| run.is_active() && run.thread.all().iter().any(|t| *t == thread))
    {
        return Err(WriteError::RunActive { thread: thread.to_owned(), run: active.id.clone() });
    }
    let key = Key::new(kind::RUN, name(id)?)?;
    set(replica, &key, run::THREAD, Some(&thread), by)?;
    set(replica, &key, run::STATE, Some(&RunState::Starting), by)?;
    Ok(key)
}

fn existing(state: &State, id: &str) -> Result<Key, WriteError> {
    let key = item_key(id)?;
    if state.entity(&key).is_none() {
        return Err(WriteError::NoItem(id.to_owned()));
    }
    Ok(key)
}

fn read_item(state: &State, key: &Key) -> Result<Item, Error> {
    let done_at = state
        .field(key, field::STATE)
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.value == to_json(&ItemState::Done))
        .map(|(_, v)| v.at)
        .min();
    Ok(Item {
        id: key.id.clone(),
        title: value(state, key, field::TITLE)?,
        notes: value(state, key, field::NOTES)?,
        gate: value(state, key, field::GATE)?,
        state: value(state, key, field::STATE)?,
        done_at,
        parent: value(state, key, field::PARENT)?,
        size: value(state, key, field::SIZE)?,
        archived: value(state, key, field::ARCHIVED)?,
        evidence: value(state, key, field::EVIDENCE)?,
        thread: value(state, key, field::THREAD)?,
        areas: elements(state, key, set::AREA),
        repositories: elements(state, key, set::REPOSITORY),
        resources: elements(state, key, set::RESOURCE),
    })
}

/// Reads a field as a [`Value`]; equal concurrent values read as one.
fn value<T: DeserializeOwned + PartialEq>(
    state: &State,
    key: &Key,
    name: &str,
) -> Result<Value<T>, Error> {
    let mut values: Vec<T> = Vec::new();
    for stamped in state.field(key, name).into_iter().flatten().map(|(_, v)| v) {
        let value: T = serde_json::from_value(stamped.value.clone()).map_err(|e| {
            Error::Format(format!("unreadable {name} on {} {}: {e}", key.kind, key.id))
        })?;
        if !values.contains(&value) {
            values.push(value);
        }
    }
    Ok(match values.len() {
        0 => Value::None,
        1 => Value::One(values.remove(0)),
        _ => Value::Conflict(values),
    })
}

/// The elements of a set field: each field under `prefix` that holds a value.
fn elements(state: &State, key: &Key, prefix: &str) -> BTreeSet<String> {
    state
        .entity(key)
        .into_iter()
        .flatten()
        .filter_map(|(name, _)| name.strip_prefix(prefix))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests;
