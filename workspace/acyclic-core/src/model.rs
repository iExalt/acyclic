//! The model: items, links, threads and repositories, read as a typed snapshot of the state and written through
//! checked writes. Nothing derived is stored: roles, membership, repository sets and cycles are computed on read.
//!
//! Every field reads as a [`Value`], so a reader sees concurrent writes as a conflict and never silently picks one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::graph;
use crate::schema::{item as field, kind, link, repository, set, thread};
use crate::state::{Error, Key, Millis, Replica, State};

/// The longest item id: a link's id holds two, and ids are file names.
pub const MAX_ID_LEN: usize = 64;

/// A field's current value: unset, one value, or several written concurrently.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Value<T> {
    /// Unset.
    #[default]
    None,
    /// One value, or several equal ones.
    One(T),
    /// Concurrent, different values, in dot order; `None` is a clear that raced them.
    Conflict(Vec<Option<T>>),
}

/// Unset reads as `null`, one value as itself, and a conflict as `{"conflict": [values]}`, a clear among them as
/// `null`.
impl<T: Serialize> Serialize for Value<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_none(),
            Self::One(value) => value.serialize(serializer),
            Self::Conflict(values) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("conflict", values)?;
                map.end()
            },
        }
    }
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

    /// Every value set: none, one, or each conflicting one; a clear in a conflict sets none.
    #[must_use]
    pub fn all(&self) -> Vec<&T> {
        match self {
            Self::None => Vec::new(),
            Self::One(value) => vec![value],
            Self::Conflict(values) => values.iter().flatten().collect(),
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

impl ItemState {
    /// Every state, in order.
    pub const ALL: [Self; 4] = [Self::Backlog, Self::Parked, Self::InProgress, Self::Done];
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

impl Size {
    /// Every size, smallest first.
    pub const ALL: [Self; 5] = [Self::Xs, Self::S, Self::M, Self::L, Self::Xl];
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
    /// Context only.
    Relates,
}

impl LinkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::Relates => "relates",
        }
    }

    /// The word a command line and its output use for the kind: `waits-for` or `relates-to`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Blocks => "waits-for",
            Self::Relates => "relates-to",
        }
    }

    /// Whether the kind orders work, so it counts in cycle checks.
    #[must_use]
    pub fn orders(self) -> bool {
        self == Self::Blocks
    }
}

/// An autonomy level: which agents' questions reach the needs-you list. Blockers always do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// Ask about everything.
    Everything,
    /// Ask about scope changes and irreversible actions.
    ScopeAndIrreversible,
    /// Ask only about blockers: the default.
    Blockers,
}

impl Level {
    /// Every level, most talkative first.
    pub const ALL: [Self; 3] = [Self::Everything, Self::ScopeAndIrreversible, Self::Blockers];
}

/// What reopening recorded on an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reopened {
    /// Whether it was archived, so its dependents stay unblocked and are only flagged.
    pub archived: bool,
}

/// An item's role, derived from its place in the tree.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// Set while a reopened item is open again.
    pub reopened: Value<Reopened>,
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

/// A parent's progress, from the leaves under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The state it reads as: its own when parked or done by hand, else done when every leaf under it is finished, in
    /// progress once any leaf is started or done, and backlog until then.
    pub state: ItemState,
    /// Its leaves that are done, archived ones aside.
    pub done: usize,
    /// Its leaves, archived ones aside: none when every one is archived, and it then reads as done.
    pub total: usize,
}

/// What counting a parent's leaves finds.
#[derive(Default)]
struct Tally {
    done: usize,
    total: usize,
    started: bool,
}

/// A link as read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Link {
    /// The dependent item's id.
    pub from: String,
    /// The id of the item it depends on.
    pub to: String,
    /// The link's kind.
    pub kind: LinkKind,
}

/// A thread as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Thread {
    /// The thread's id.
    pub id: String,
    /// A short title.
    pub title: Value<String>,
    /// The brief.
    pub brief: Value<String>,
    /// Whether the thread's scope is approved; a conflict counts as approved, which protects more.
    pub approved: Value<bool>,
    /// The members the latest approval saw; none on threads approved before approvals recorded them.
    pub approved_members: Value<Vec<String>>,
    /// Repositories added explicitly, beyond those of its items.
    pub repositories: BTreeSet<String>,
    /// The thread's autonomy level, overriding the global one.
    pub autonomy: Value<Level>,
}

impl Thread {
    /// Whether the thread's scope is approved: any approving value counts, so a conflict protects the scope.
    #[must_use]
    pub fn is_approved(&self) -> bool {
        self.approved.all().into_iter().any(|approved| *approved)
    }

    /// Whether `item` was a member when the thread was approved, or `None` when no approval recorded its members.
    /// Approvals that raced each other each saw what they list, so a member of any of them counts.
    #[must_use]
    pub fn approved_member(&self, item: &str) -> Option<bool> {
        let lists = self.approved_members.all();
        (!lists.is_empty()).then(|| lists.into_iter().flatten().any(|member| member == item))
    }
}

/// A repository record as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Repository {
    /// The record's id.
    pub id: String,
    /// The owner, such as an organization.
    pub owner: Value<String>,
    /// The repository's name.
    pub name: Value<String>,
    /// The default branch.
    pub default_branch: Value<String>,
    /// The App's installation that reaches it.
    pub installation: Value<String>,
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
    /// The global autonomy level.
    pub autonomy: Value<Level>,
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
            model.add(state, key)?;
        }
        model.links.sort();
        Ok(model)
    }

    /// Reads the model, leaving out every entity that can't be read and listing it with why, so a diagnosis can
    /// still show the rest. Every other reader uses [`Model::read`], which never hides such an entity.
    #[must_use]
    pub fn read_lenient(state: &State) -> (Self, Vec<(Key, Error)>) {
        let mut model = Self::default();
        let mut unreadable = Vec::new();
        for (key, _) in state.entities() {
            let mut one = Self::default();
            match one.add(state, key) {
                Ok(()) => model.merge(one),
                Err(err) => unreadable.push((key.clone(), err)),
            }
        }
        model.links.sort();
        model.links.dedup();
        (model, unreadable)
    }

    fn merge(&mut self, other: Self) {
        self.items.extend(other.items);
        self.links.extend(other.links);
        self.threads.extend(other.threads);
        self.repositories.extend(other.repositories);
        if other.autonomy != Value::None {
            self.autonomy = other.autonomy;
        }
        for (parent, children) in other.children {
            self.children.entry(parent).or_default().extend(children);
        }
    }

    fn add(&mut self, state: &State, key: &Key) -> Result<(), Error> {
        match key.kind.as_str() {
            // A deleted item or thread is gone from every reader; see `crate::delete` and `crate::renumber`.
            kind::ITEM | kind::THREAD if is_deleted(state, key) => {},
            kind::ITEM => {
                let item = read_item(state, key)?;
                for parent in item.parent.all() {
                    self.children.entry(parent.clone()).or_default().insert(key.id.clone());
                }
                self.items.insert(key.id.clone(), item);
            },
            kind::LINK => {
                if state.field(key, link::PRESENT).is_none() {
                    return Ok(());
                }
                let (Value::One(from), Value::One(to), Value::One(kind)) = (
                    value::<String>(state, key, link::FROM)?,
                    value::<String>(state, key, link::TO)?,
                    value::<LinkKind>(state, key, link::KIND)?,
                ) else {
                    return Err(Error::Format(format!("link {} lacks its ends or kind", key.id)));
                };
                self.links.push(Link { from, to, kind });
            },
            kind::THREAD => {
                let thread = Thread {
                    id: key.id.clone(),
                    title: value(state, key, thread::TITLE)?,
                    brief: value(state, key, thread::BRIEF)?,
                    approved: value(state, key, thread::APPROVED)?,
                    approved_members: value(state, key, thread::APPROVED_MEMBERS)?,
                    repositories: elements(state, key, set::REPOSITORY),
                    autonomy: value(state, key, thread::AUTONOMY)?,
                };
                self.threads.insert(key.id.clone(), thread);
            },
            kind::REPOSITORY => {
                let record = Repository {
                    id: key.id.clone(),
                    owner: value(state, key, repository::OWNER)?,
                    name: value(state, key, repository::NAME)?,
                    default_branch: value(state, key, repository::DEFAULT_BRANCH)?,
                    installation: value(state, key, repository::INSTALLATION)?,
                };
                self.repositories.insert(key.id.clone(), record);
            },
            kind::SETTINGS if key.id == crate::schema::GLOBAL => {
                self.autonomy = value(state, key, crate::schema::settings::AUTONOMY)?;
            },
            _ => {},
        }
        Ok(())
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

    /// A parent's progress, or `None` for a leaf. A parent's own state counts only when it is parked or done, set by
    /// hand; any other is what reopening a child left, and its leaves decide. A parent under it done by hand counts
    /// every leaf under it as done, so a closed part reads closed from above too.
    #[must_use]
    pub fn progress(&self, id: &str) -> Option<Progress> {
        let item = self.items.get(id)?;
        if self.children(id).is_empty() {
            return None;
        }
        let mut tally = Tally::default();
        self.tally(id, &mut BTreeSet::from([id]), &mut tally);
        let state = if item.state == Value::One(ItemState::Parked) {
            ItemState::Parked
        } else if item.is_done() || tally.done == tally.total {
            ItemState::Done
        } else if tally.started || tally.done > 0 {
            ItemState::InProgress
        } else {
            ItemState::Backlog
        };
        Some(Progress { state, done: tally.done, total: tally.total })
    }

    /// Counts the leaves under `id`, archived ones aside, each item once, so a parent cycle ends the walk.
    fn tally<'a>(&'a self, id: &'a str, seen: &mut BTreeSet<&'a str>, tally: &mut Tally) {
        for child in self.children(id) {
            if !seen.insert(child) || self.is_archived(child) {
                continue;
            }
            let Some(item) = self.items.get(child) else { continue };
            if self.children(child).is_empty() {
                tally.total += 1;
                tally.done += usize::from(item.is_done());
                tally.started |= item.state == Value::One(ItemState::InProgress);
            } else if item.is_done() {
                let mut closed = Tally::default();
                self.tally(child, seen, &mut closed);
                tally.total += closed.total;
                tally.done += closed.total;
            } else {
                self.tally(child, seen, tally);
            }
        }
    }

    /// Whether an item reads as done: a leaf by its own state, a parent by its progress.
    #[must_use]
    pub fn reads_done(&self, id: &str) -> bool {
        match self.progress(id) {
            Some(progress) => progress.state == ItemState::Done,
            None => self.items.get(id).is_some_and(Item::is_done),
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

    /// The cycles among blocks links, each sorted.
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

    /// A path of blocks links from `from` to `to`, both included; a single item when they are the same.
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

    /// A thread's heads; see [`Heads`]. To list many threads' heads, build [`Model::heads_index`] once.
    #[must_use]
    pub fn heads(&self, thread: &str) -> Vec<&str> {
        self.heads_index().of(thread)
    }

    /// What finding heads reads, gathered in one pass over the items and links, for every thread at once.
    #[must_use]
    pub fn heads_index(&self) -> Heads<'_> {
        let mut open: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for item in self.items.values() {
            if let Some(thread) = item.thread.one()
                && !self.reads_done(&item.id)
                && !self.is_archived(&item.id)
            {
                open.entry(thread.as_str()).or_default().push(item.id.as_str());
            }
        }
        let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut prev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for link in self.links.iter().filter(|link| link.kind.orders()) {
            next.entry(link.from.as_str()).or_default().push(link.to.as_str());
            for reached in std::iter::once(link.to.as_str()).chain(self.ancestors(&link.to)) {
                prev.entry(reached).or_default().push(link.from.as_str());
            }
        }
        Heads { model: self, open, next, prev }
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

    /// A thread's autonomy level: its own, else the global one, else asking only about blockers. A conflicted level
    /// counts as the one that asks most.
    #[must_use]
    pub fn level(&self, thread: Option<&str>) -> Level {
        let own = thread.and_then(|t| self.threads.get(t)).map(|t| &t.autonomy);
        let chosen = match own {
            Some(level) if *level != Value::None => level,
            _ => &self.autonomy,
        };
        chosen.all().into_iter().min().copied().unwrap_or(Level::Blockers)
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
    /// The item was deleted, and its id isn't used again.
    Deleted(String),
    /// The write would put an item under itself or its own descendant.
    ParentCycle(String),
    /// The link would close a cycle of blocks links, through these items.
    LinkCycle(Vec<String>),
    /// The field has invariants: write it with its checked write.
    Checked {
        /// The field.
        field: String,
        /// The checked write to use.
        write: &'static str,
    },
    /// Only the user sets this field.
    UserOnly {
        /// The field.
        field: String,
        /// Who tried.
        actor: String,
    },
    /// An item can't link to itself.
    SelfLink(String),
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
            Self::Deleted(id) => write!(f, "item {id} was deleted, and its id isn't used again"),
            Self::ParentCycle(id) => write!(f, "{id} can't go under itself or its own descendant"),
            Self::LinkCycle(path) => {
                write!(f, "the link would close a cycle through {}", path.join(", "))
            },
            Self::Checked { field, write } => {
                write!(f, "{field} has invariants: write it with {write}")
            },
            Self::UserOnly { field, actor } => write!(f, "only the user sets {field}, not {actor}"),
            Self::SelfLink(id) => write!(f, "{id} can't link to itself"),
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

/// The fields only a checked write may set, with the write to use instead.
const CHECKED: [(&str, &str, &str); 7] = [
    (kind::ITEM, field::THREAD, "scope::edit"),
    (kind::ITEM, field::ARCHIVED, "archive::archive"),
    (kind::ITEM, field::DELETED, "delete::delete_item"),
    (kind::ITEM, field::REOPENED, "archive::reopen"),
    (kind::ITEM, field::PARENT, "set_parent"),
    (kind::ITEM, field::STATE, "set_state"),
    (kind::LINK, "", "link or unlink"),
];

/// Fields only the user's batches set: a thread's approval and autonomy level, and every global setting.
const USER_ONLY: [(&str, &str); 4] = [
    (kind::THREAD, thread::APPROVED),
    (kind::THREAD, thread::APPROVED_MEMBERS),
    (kind::THREAD, thread::AUTONOMY),
    (kind::SETTINGS, ""),
];

/// Writes a field of any entity, or clears it with `None`. Fields with invariants (an item's parent, state, thread,
/// archived flag and reopen flag, and a link's fields) are refused: use their checked writes. A thread's approval and
/// autonomy level and the global settings are the user's alone.
///
/// # Errors
///
/// Fails on a checked field, on a user-only field written by anyone else, or when the state layer refuses.
pub fn set<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    value: Option<&T>,
    by: By<'_>,
) -> Result<(), WriteError> {
    let checked = CHECKED
        .iter()
        .find(|(kind, field, _)| key.kind == *kind && (field.is_empty() || name == *field));
    if let Some((_, _, write)) = checked {
        return Err(WriteError::Checked { field: name.to_owned(), write });
    }
    let user_only = USER_ONLY
        .iter()
        .any(|(kind, field)| key.kind == *kind && (field.is_empty() || name == *field));
    if user_only && by.actor != crate::scope::USER {
        return Err(WriteError::UserOnly { field: name.to_owned(), actor: by.actor.to_owned() });
    }
    put(replica, key, name, value, by)
}

/// Writes a single-valued field. A clear writes `null` rather than nothing, so a set it never saw conflicts with it
/// instead of silently outliving it; `null` reads as unset.
pub(crate) fn put<T: Serialize>(
    replica: &mut Replica,
    key: &Key,
    name: &str,
    value: Option<&T>,
    by: By<'_>,
) -> Result<(), WriteError> {
    replica.write(key, name, by.actor, by.at, Some(value.map_or(Json::Null, to_json)))?;
    Ok(())
}

/// A set field, stored a field per element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetField {
    /// Area tags.
    Area,
    /// Repositories; on items and threads they are scope edits, made through `scope::edit`.
    Repository,
    /// Named resources.
    Resource,
}

impl SetField {
    /// The prefix of the set's fields.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Area => set::AREA,
            Self::Repository => set::REPOSITORY,
            Self::Resource => set::RESOURCE,
        }
    }
}

/// Adds an element to a set field, or removes it. Repositories on items and threads are scope edits: use
/// `scope::edit`.
///
/// # Errors
///
/// Fails on a repository of an item or thread, an invalid element name, or when the state layer refuses.
pub fn set_element(
    replica: &mut Replica,
    key: &Key,
    set: SetField,
    element: &str,
    present: bool,
    by: By<'_>,
) -> Result<(), WriteError> {
    if set == SetField::Repository && [kind::ITEM, kind::THREAD].contains(&key.kind.as_str()) {
        return Err(WriteError::Checked {
            field: format!("{}{element}", set.prefix()),
            write: "scope::edit",
        });
    }
    put_element(replica, key, set.prefix(), element, present, by)
}

pub(crate) fn put_element(
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
/// Fails on an invalid id, on the id of a deleted item, or when the state layer refuses.
pub fn add_item(
    replica: &mut Replica,
    id: &str,
    title: &str,
    by: By<'_>,
) -> Result<Key, WriteError> {
    let key = item_key(id)?;
    if is_deleted(&replica.state, &key) {
        return Err(WriteError::Deleted(id.to_owned()));
    }
    put(replica, &key, field::TITLE, Some(&title), by)?;
    put(replica, &key, field::STATE, Some(&ItemState::Backlog), by)?;
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
    if state == ItemState::Done && replica.state.field(&key, field::REOPENED).is_some() {
        put::<Reopened>(replica, &key, field::REOPENED, None, by)?;
    }
    put(replica, &key, field::STATE, Some(&state), by)
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
    put(replica, &key, field::PARENT, parent.map(str::to_owned).as_ref(), by)
}

/// Adds a link from `from` to `to`.
///
/// # Errors
///
/// Fails when either item doesn't exist, when a blocks link would close a cycle,
/// or when the state layer refuses.
pub fn link(
    replica: &mut Replica,
    from: &str,
    to: &str,
    kind: LinkKind,
    by: By<'_>,
) -> Result<Key, WriteError> {
    existing(&replica.state, from)?;
    existing(&replica.state, to)?;
    if from == to {
        return Err(WriteError::SelfLink(from.to_owned()));
    }
    let key = link_key(from, to, kind)?;
    if kind.orders()
        && let Some(path) = Model::read(&replica.state)?.ordering_path(to, from)
    {
        return Err(WriteError::LinkCycle(path));
    }
    if replica.state.field(&key, link::FROM).is_none() {
        put(replica, &key, link::FROM, Some(&from), by)?;
        put(replica, &key, link::TO, Some(&to), by)?;
        put(replica, &key, link::KIND, Some(&kind), by)?;
    }
    put(replica, &key, link::PRESENT, Some(&true), by)?;
    Ok(key)
}

/// Removes a link, writing nothing when it is absent; a concurrent re-add survives. Returns whether it wrote.
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
) -> Result<bool, WriteError> {
    let key = link_key(from, to, kind)?;
    if replica.state.field(&key, link::PRESENT).is_none() {
        return Ok(false);
    }
    // A link is a set element: removing it leaves nothing, so an add it never saw keeps the link.
    replica.write(&key, link::PRESENT, by.actor, by.at, None)?;
    Ok(true)
}

/// Whether an item has been deleted: its tombstone is set, and nothing clears it.
#[must_use]
pub fn is_deleted(state: &State, key: &Key) -> bool {
    state
        .field(key, field::DELETED)
        .is_some_and(|register| register.values().any(|v| v.value == Json::Bool(true)))
}

/// An existing item's key: an item that was never written, or was deleted, isn't one.
///
/// # Errors
///
/// Fails on an invalid id, or when no such item exists.
pub fn existing(state: &State, id: &str) -> Result<Key, WriteError> {
    let key = item_key(id)?;
    if state.entity(&key).is_none() || is_deleted(state, &key) {
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
        reopened: value(state, key, field::REOPENED)?,
    })
}

/// Reads a field as a [`Value`]; equal concurrent values read as one.
pub(crate) fn value<T: DeserializeOwned + PartialEq>(
    state: &State,
    key: &Key,
    name: &str,
) -> Result<Value<T>, Error> {
    let mut values: Vec<Option<T>> = Vec::new();
    for stamped in state.field(key, name).into_iter().flatten().map(|(_, v)| v) {
        let value: Option<T> = serde_json::from_value(stamped.value.clone()).map_err(|e| {
            Error::Format(format!("unreadable {name} on {} {}: {e}", key.kind, key.id))
        })?;
        if !values.contains(&value) {
            values.push(value);
        }
    }
    Ok(match values.len() {
        0 => Value::None,
        1 => values.remove(0).map_or(Value::None, Value::One),
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

/// Every thread's open members and the ordering links, read once, to find threads' heads.
pub struct Heads<'a> {
    model: &'a Model,
    open: BTreeMap<&'a str, Vec<&'a str>>,
    next: BTreeMap<&'a str, Vec<&'a str>>,
    /// The nodes a link reaches, each with the nodes that link from: what waits on a node reaches its ancestors too.
    prev: BTreeMap<&'a str, Vec<&'a str>>,
}

impl<'a> Heads<'a> {
    /// The model it reads.
    #[must_use]
    pub fn model(&self) -> &'a Model {
        self.model
    }

    /// A thread's heads: its open leaves that nothing in the thread orders, by id. A leaf follows another open member
    /// when a path of blocks links, from the leaf or an ancestor, reaches that member or one of its
    /// ancestors. Ids, numbering and membership order nothing, so two or more heads mean the thread's order is partly
    /// unsaid: they may run in parallel, or a link is missing.
    #[must_use]
    pub fn of(&self, thread: &str) -> Vec<&'a str> {
        let model = self.model;
        let open = self.open.get(thread).map_or(&[][..], Vec::as_slice);
        // Each node, with the open members it is or is an ancestor of, built once rather than per leaf.
        let mut covers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for &member in open {
            for node in std::iter::once(member).chain(model.ancestors(member)) {
                covers.entry(node).or_default().insert(member);
            }
        }
        // Only the nodes that can reach a member are worth walking from, so a leaf's walk stays near the thread.
        let mut toward: BTreeSet<&str> = covers.keys().copied().collect();
        let mut back: Vec<&str> = toward.iter().copied().collect();
        while let Some(node) = back.pop() {
            for &from in self.prev.get(node).into_iter().flatten() {
                if toward.insert(from) {
                    back.push(from);
                }
            }
        }
        let follows_another = |leaf: &str| {
            let reaches_another = |node: &str| {
                covers.get(node).is_some_and(|members| members.iter().any(|m| *m != leaf))
            };
            let mut seen: BTreeSet<&str> =
                std::iter::once(leaf).chain(model.ancestors(leaf)).collect();
            let mut frontier: Vec<&str> = seen.iter().copied().collect();
            while let Some(node) = frontier.pop() {
                for &to in self.next.get(node).into_iter().flatten() {
                    if reaches_another(to) {
                        return true;
                    }
                    // What waits on `to` waits on its ancestors' dependencies too.
                    for reached in std::iter::once(to).chain(model.ancestors(to)) {
                        if toward.contains(reached) && seen.insert(reached) {
                            frontier.push(reached);
                        }
                    }
                }
            }
            false
        };
        open.iter()
            .copied()
            .filter(|id| model.children(id).is_empty() && !follows_another(id))
            .collect()
    }
}

#[cfg(test)]
mod tests;
