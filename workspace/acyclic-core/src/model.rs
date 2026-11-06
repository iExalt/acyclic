//! The model: initiatives, items, links, threads and repositories, read as a typed snapshot of the state. Records are written
//! through the write module's judge; the per-field writes here serve it, deletes and, with the
//! `fixtures` feature, tests. Nothing derived is stored: roles, membership and cycles are computed on read.
//!
//! Every field reads as a [`Value`], so a reader sees concurrent writes as a conflict and never silently picks one.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::graph;
use crate::schema::{
    RETIRED, RETIRED_SETS, initiative, item as field, kind, link, repository, thread,
};
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

impl<T: PartialEq> Value<T> {
    /// Whether writing `value` would change nothing: the field holds exactly it, or for `None`, nothing. A conflict
    /// holds neither, so a write settles it.
    #[must_use]
    pub fn holds(&self, value: Option<&T>) -> bool {
        match (self, value) {
            (Self::None, None) => true,
            (Self::One(held), Some(value)) => held == value,
            _ => false,
        }
    }
}

/// An item's state, as stored. Reads and commands say [`ItemState::word`]: an item not started is `open`, stored as
/// `backlog` so replicas of older versions keep reading what newer ones write. `open` is read too, but never written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemState {
    /// Not started: open, stored as `backlog`.
    #[serde(alias = "open")]
    Backlog,
    /// Set aside on purpose.
    Parked,
    /// Finished.
    Done,
    /// Dropped on purpose, so it was never delivered: it counts as neither done nor open. Written only by an
    /// explicit state edit or an import that says so, and unreadable to replicas older than it.
    Canceled,
}

impl ItemState {
    /// The state as reads show it and commands take it: `open`, `parked`, `done` or `canceled`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Backlog => "open",
            Self::Parked => "parked",
            Self::Done => "done",
            Self::Canceled => "canceled",
        }
    }
}

/// A stored state as reads show it: the stored `backlog` is `open`, in one value or each of a conflict's
/// (`{"conflict": [...]}` or a bare list); anything else is as given.
#[must_use]
pub fn shown_state(value: &Json) -> Json {
    match value {
        Json::String(stored) if stored == "backlog" => Json::from(ItemState::Backlog.word()),
        Json::Array(values) => Json::Array(values.iter().map(shown_state).collect()),
        Json::Object(fields) => {
            Json::Object(fields.iter().map(|(key, v)| (key.clone(), shown_state(v))).collect())
        },
        other => other.clone(),
    }
}

/// A stored field's name as reads show it: a thread's `brief` is its notes. Older replicas read the stored name, so it
/// stays, as does a need's key, which names the field as stored.
#[must_use]
pub fn shown_field<'a>(kind: &str, field: &'a str) -> &'a str {
    if (kind, field) == (crate::schema::kind::THREAD, crate::schema::thread::BRIEF) {
        "notes"
    } else {
        field
    }
}

/// What shows a done item was done.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The id of the repository record holding the commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The commit that completed the item: its SHA, apart from the repository, so renaming the repository renames
    /// its evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Evidence in words, such as where a record lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A link's stored kind. Only one is left: `from` waits for `to`. A link stored with any other kind, such as the
/// `relates` or `after` of older versions, can't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LinkKind {
    /// A hard dependency: `from` waits for `to`.
    Blocks,
}

/// An item's role, derived from its place in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// An item with children directly under an initiative, or in no initiative at all, which reads as the level an
    /// initiative's work sits at.
    Epic,
    /// Any other item: one with no children, or one deeper than an epic.
    Task,
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
    /// The item's state; unset reads as open.
    pub state: Value<ItemState>,
    /// When the item was marked done: the earliest of its current done values.
    pub done_at: Option<Millis>,
    /// The parent item's id.
    pub parent: Value<String>,
    /// Evidence of completion.
    pub evidence: Value<Evidence>,
    /// The thread the item belongs to.
    pub thread: Value<String>,
}

impl Item {
    /// Whether the item is done; a conflicted state is done only when every value is.
    #[must_use]
    pub fn is_done(&self) -> bool {
        let states = self.state.all();
        !states.is_empty() && states.iter().all(|state| **state == ItemState::Done)
    }

    /// Whether the item is canceled; a conflicted state is canceled only when every value is.
    #[must_use]
    pub fn is_canceled(&self) -> bool {
        let states = self.state.all();
        !states.is_empty() && states.iter().all(|state| **state == ItemState::Canceled)
    }

    /// Whether the item is finished: done or canceled. A state in conflict between the two is neither.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.is_done() || self.is_canceled()
    }
}

/// A parent's leaves, counted: where its work stands is its stage, from [`crate::ready`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Its leaves that are done.
    pub done: usize,
    /// Its leaves that are canceled, or open under a part canceled by hand; they are not in `total`.
    pub canceled: usize,
    /// Its other leaves, done or not.
    pub total: usize,
}

/// What counting a parent's leaves finds.
#[derive(Default)]
struct Tally {
    done: usize,
    canceled: usize,
    total: usize,
}

/// A link as read: `from` waits for `to`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Link {
    /// The dependent item's id.
    pub from: String,
    /// The id of the item it depends on.
    pub to: String,
}

/// An initiative as read: a long-lived body of work, a record of its own that items sit under. It is never work, so it
/// has no state, evidence, thread or links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Initiative {
    /// The initiative's id, which an item's `parent` names.
    pub id: String,
    /// A short title.
    pub title: Value<String>,
    /// Its notes.
    pub notes: Value<String>,
}

/// A thread as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Thread {
    /// The thread's id.
    pub id: String,
    /// A short title.
    pub title: Value<String>,
    /// Its notes: what the work is and how to go about it, stored under the older field name `brief`.
    pub notes: Value<String>,
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
}

/// A snapshot of the model, read once from a state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    /// Initiatives by id.
    pub initiatives: BTreeMap<String, Initiative>,
    /// Items by id.
    pub items: BTreeMap<String, Item>,
    /// Present links, sorted.
    pub links: Vec<Link>,
    /// Threads by id.
    pub threads: BTreeMap<String, Thread>,
    /// Repository records by id.
    pub repositories: BTreeMap<String, Repository>,
    /// The keys of deleted initiatives, items and threads, which no other field holds: an id deleted is never used again.
    pub(crate) deleted: BTreeSet<Key>,
    /// Each live entity's fields that hold different values, as `(kind, id, field)`.
    pub(crate) conflicts: BTreeSet<(String, String, String)>,
    /// The ids of link entities that were removed, so restoring one writes only its presence.
    pub(crate) removed_links: BTreeSet<String>,
    pub(crate) children: BTreeMap<String, BTreeSet<String>>,
}

impl Model {
    /// Reads the model from a state.
    ///
    /// # Errors
    ///
    /// Fails when a value can't be read, such as one written by a newer version: hiding it could hide a blocker.
    pub fn read(state: &State) -> Result<Self, Error> {
        let mut model = Self::default();
        for (key, entity) in state.entities() {
            model.add(state, key)?;
            model.note_conflicts(key, entity);
        }
        model.links.sort();
        model.name_threads_of_one();
        Ok(model)
    }

    /// Gives a loose task's thread of one a record while its task names it, though it holds no field: releasing the
    /// only claim on one takes its stored record with it, and the task still belongs to its thread.
    fn name_threads_of_one(&mut self) {
        let named: Vec<String> = self
            .items
            .values()
            .flat_map(|item| item.thread.all())
            .filter(|thread| {
                !self.threads.contains_key(thread.as_str())
                    && crate::ids::task_of_one(thread)
                        .is_some_and(|task| self.items.contains_key(task))
            })
            .cloned()
            .collect();
        for id in named {
            let thread = Thread { id: id.clone(), title: Value::None, notes: Value::None };
            self.threads.insert(id, thread);
        }
    }

    /// Reads the model, leaving out every entity that can't be read and listing it with why, so a diagnosis can
    /// still show the rest. Every other reader uses [`Model::read`], which never hides such an entity.
    #[must_use]
    pub fn read_lenient(state: &State) -> (Self, Vec<(Key, Error)>) {
        let mut model = Self::default();
        let mut unreadable = Vec::new();
        for (key, entity) in state.entities() {
            let mut one = Self::default();
            match one.add(state, key) {
                Ok(()) => {
                    one.note_conflicts(key, entity);
                    model.merge(one);
                },
                Err(err) => unreadable.push((key.clone(), err)),
            }
        }
        model.links.sort();
        model.links.dedup();
        model.name_threads_of_one();
        (model, unreadable)
    }

    fn merge(&mut self, other: Self) {
        self.initiatives.extend(other.initiatives);
        self.items.extend(other.items);
        self.links.extend(other.links);
        self.threads.extend(other.threads);
        self.repositories.extend(other.repositories);
        self.deleted.extend(other.deleted);
        self.conflicts.extend(other.conflicts);
        self.removed_links.extend(other.removed_links);
        for (parent, children) in other.children {
            self.children.entry(parent).or_default().extend(children);
        }
    }

    /// Notes each field of a live entity that holds different values, read or not, so a field a newer version wrote
    /// still shows; claims contest rather than conflict, and a retired field or set lies dormant, since nothing could
    /// settle it.
    fn note_conflicts(&mut self, key: &Key, entity: &crate::state::Entity) {
        if self.deleted.contains(key) {
            return;
        }
        for (name, register) in entity {
            if name == crate::claim::FIELD
                || RETIRED.contains(&(key.kind.as_str(), name.as_str()))
                || RETIRED_SETS.iter().any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            let mut values = register.values().map(|stamped| &stamped.value);
            if let Some(first) = values.next()
                && values.any(|value| value != first)
            {
                self.conflicts.insert((key.kind.clone(), key.id.clone(), name.clone()));
            }
        }
    }

    /// Every field of a live entity that holds different values written apart, as `(kind, id, field)`, sorted: the
    /// fields this model reads and any it doesn't. A deleted record's are gone with it.
    pub fn conflicts(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.conflicts.iter().map(|(kind, id, field)| (kind.as_str(), id.as_str(), field.as_str()))
    }

    fn add(&mut self, state: &State, key: &Key) -> Result<(), Error> {
        match key.kind.as_str() {
            // A deleted initiative, item or thread is gone from every reader; see `crate::delete`.
            kind::INITIATIVE | kind::ITEM | kind::THREAD if is_deleted(state, key) => {
                self.deleted.insert(key.clone());
            },
            kind::INITIATIVE => {
                let record = Initiative {
                    id: key.id.clone(),
                    title: value(state, key, initiative::TITLE)?,
                    notes: value(state, key, initiative::NOTES)?,
                };
                self.initiatives.insert(key.id.clone(), record);
            },
            kind::ITEM => {
                let item = read_item(state, key)?;
                for parent in item.parent.all() {
                    self.children.entry(parent.clone()).or_default().insert(key.id.clone());
                }
                self.items.insert(key.id.clone(), item);
            },
            kind::LINK => {
                if state.field(key, link::PRESENT).is_none() {
                    self.removed_links.insert(key.id.clone());
                    return Ok(());
                }
                let (Value::One(from), Value::One(to), Value::One(LinkKind::Blocks)) = (
                    value::<String>(state, key, link::FROM)?,
                    value::<String>(state, key, link::TO)?,
                    value::<LinkKind>(state, key, link::KIND)?,
                ) else {
                    return Err(Error::Format(format!("link {} lacks its ends or kind", key.id)));
                };
                self.links.push(Link { from, to });
            },
            kind::THREAD => {
                let thread = Thread {
                    id: key.id.clone(),
                    title: value(state, key, thread::TITLE)?,
                    notes: value(state, key, thread::BRIEF)?,
                };
                self.threads.insert(key.id.clone(), thread);
            },
            kind::REPOSITORY => {
                let record = Repository {
                    id: key.id.clone(),
                    owner: value(state, key, repository::OWNER)?,
                    name: value(state, key, repository::NAME)?,
                    default_branch: value(state, key, repository::DEFAULT_BRANCH)?,
                };
                self.repositories.insert(key.id.clone(), record);
            },
            _ => {},
        }
        Ok(())
    }

    /// Whether an item or thread was deleted. A deleted record is in none of the model's maps, so this is only for
    /// keeping its id from being used again.
    #[must_use]
    pub fn is_deleted(&self, key: &Key) -> bool {
        self.deleted.contains(key)
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

    /// An item's role: an epic when it has children and sits directly under an initiative, or under none at all, and a
    /// task otherwise. A conflicted parent is no initiative.
    #[must_use]
    pub fn role(&self, id: &str) -> Role {
        let Some(item) = self.items.get(id) else { return Role::Task };
        let top = match &item.parent {
            Value::None => true,
            Value::One(parent) => self.initiatives.contains_key(parent),
            Value::Conflict(_) => false,
        };
        if top && !self.children(id).is_empty() { Role::Epic } else { Role::Task }
    }

    /// The initiatives an item is under, sorted: those at the top of the chain of its ancestors, more than one only
    /// where a conflicted parent leads to several. None means the item is in no initiative.
    #[must_use]
    pub fn initiatives_of(&self, id: &str) -> Vec<&str> {
        let mut found: Vec<&str> = self
            .ancestors(id)
            .into_iter()
            .filter_map(|above| self.initiatives.get_key_value(above).map(|(id, _)| id.as_str()))
            .collect();
        found.sort_unstable();
        found
    }

    /// The initiative an item is under, when it is under exactly one.
    #[must_use]
    pub fn initiative_of(&self, id: &str) -> Option<&str> {
        match self.initiatives_of(id)[..] {
            [only] => Some(only),
            _ => None,
        }
    }

    /// Whether `id` names an initiative or an item, live or deleted, other than a record of kind `own`: an item's `parent`
    /// names either by its id alone, so no id may name both.
    #[must_use]
    pub fn id_held_beyond(&self, own: &str, id: &str) -> bool {
        let held = |of: &str, live: bool| -> bool {
            live || Key::new(of, id).is_ok_and(|key| self.deleted.contains(&key))
        };
        let others: [(&str, bool); 2] = [
            (kind::INITIATIVE, self.initiatives.contains_key(id)),
            (kind::ITEM, self.items.contains_key(id)),
        ];
        others.into_iter().any(|(of, live)| of != own && held(of, live))
    }

    /// A parent's leaves done and in all, or `None` for a leaf. A done stored on a parent, as older versions wrote,
    /// is ignored: its leaves count.
    #[must_use]
    pub fn progress(&self, id: &str) -> Option<Progress> {
        self.items.get(id)?;
        if self.children(id).is_empty() {
            return None;
        }
        let mut tally = Tally::default();
        let canceled = self.items[id].state == Value::One(ItemState::Canceled);
        self.tally(id, canceled, &mut BTreeSet::from([id]), &mut tally);
        Some(Progress { done: tally.done, canceled: tally.canceled, total: tally.total })
    }

    /// Counts the leaves under `id`, each item once, so a parent cycle ends the walk. The open leaves of a part
    /// canceled by hand count as canceled.
    fn tally<'a>(
        &'a self,
        id: &'a str,
        canceled: bool,
        seen: &mut BTreeSet<&'a str>,
        tally: &mut Tally,
    ) {
        for child in self.children(id) {
            if !seen.insert(child) {
                continue;
            }
            let Some(item) = self.items.get(child) else { continue };
            if self.children(child).is_empty() {
                if item.is_canceled() || (canceled && !item.is_done()) {
                    tally.canceled += 1;
                } else {
                    tally.total += 1;
                    tally.done += usize::from(item.is_done());
                }
            } else {
                // A parent's stored done is ignored: its leaves say whether it is done.
                let cancels = canceled || item.state == Value::One(ItemState::Canceled);
                self.tally(child, cancels, seen, tally);
            }
        }
    }

    /// Whether an item reads as done: a leaf by its own state, a parent once every leaf under it that isn't canceled
    /// is, and one is, unless it is parked or canceled by hand.
    #[must_use]
    pub fn reads_done(&self, id: &str) -> bool {
        let Some(item) = self.items.get(id) else { return false };
        match self.progress(id) {
            Some(progress) => {
                progress.done == progress.total
                    && progress.done > 0
                    && item.state != Value::One(ItemState::Parked)
                    && item.state != Value::One(ItemState::Canceled)
            },
            None => item.is_done(),
        }
    }

    /// Whether an item reads as canceled, unless it reads done: a leaf by its own state, a parent canceled by hand or
    /// with every leaf under it canceled, and anything under a part canceled by hand, as that part cancels what is
    /// open beneath it. A leaf reading canceled has the stage canceled.
    #[must_use]
    pub fn reads_canceled(&self, id: &str) -> bool {
        let Some(item) = self.items.get(id) else { return false };
        if self.reads_done(id) {
            return false;
        }
        let own = match self.progress(id) {
            Some(progress) => {
                item.state == Value::One(ItemState::Canceled)
                    || (progress.total == 0 && progress.canceled > 0)
            },
            None => item.is_canceled(),
        };
        own || self.ancestors(id).into_iter().any(|above| {
            self.items.get(above).is_some_and(|a| a.state == Value::One(ItemState::Canceled))
        })
    }

    /// Whether an item reads as finished: done or canceled.
    #[must_use]
    pub fn reads_finished(&self, id: &str) -> bool {
        self.reads_done(id) || self.reads_canceled(id)
    }

    /// An item's thread as reads see it: the one stored on it, unless it has children. A thread stored on a parent,
    /// as a sync can leave one, is ignored, and Needs you lists it; see [`Self::ignored_thread`].
    #[must_use]
    pub fn thread_value<'m>(&'m self, item: &'m Item) -> &'m Value<String> {
        static NONE: Value<String> = Value::None;
        if self.children.contains_key(&item.id) { &NONE } else { &item.thread }
    }

    /// The thread stored on an item that has children, which reads ignore.
    #[must_use]
    pub fn ignored_thread(&self, id: &str) -> Option<&str> {
        let item = self.items.get(id)?;
        if self.children.contains_key(id) { item.thread.one().map(String::as_str) } else { None }
    }

    /// The items of a thread: those whose thread field, as read, is that thread alone. An item with a conflicted
    /// thread is in neither, and nor is a parent.
    #[must_use]
    pub fn members(&self, thread: &str) -> Vec<&str> {
        self.items
            .values()
            .filter(|item| self.thread_value(item).one().is_some_and(|t| t == thread))
            .map(|item| item.id.as_str())
            .collect()
    }

    /// Whether an item is a loose task: an open leaf in no thread, or in one with no record, whose thread of one is
    /// therefore its own. A leaf whose thread is in conflict is left to the conflict.
    #[must_use]
    pub fn is_loose(&self, id: &str) -> bool {
        let Some(item) = self.items.get(id) else { return false };
        self.children(id).is_empty()
            && !item.is_finished()
            && !item.thread.is_conflict()
            && item.thread.one().is_none_or(|thread| !self.threads.contains_key(thread))
    }

    /// The thread an item works in: the one it names when that has a record, else the thread of one derived from it
    /// when it is a loose task. A finished item with no thread record is in none.
    #[must_use]
    pub fn held_thread(&self, id: &str) -> Option<std::borrow::Cow<'_, str>> {
        use std::borrow::Cow;
        match self.thread_of(id).filter(|thread| self.threads.contains_key(*thread)) {
            Some(thread) => Some(Cow::Borrowed(thread)),
            None => self.is_loose(id).then(|| Cow::Owned(crate::ids::one(id))),
        }
    }

    /// The task whose thread of one `thread` is, when it is one and the task exists.
    #[must_use]
    pub fn task_of(&self, thread: &str) -> Option<&str> {
        let task = crate::ids::task_of_one(thread)?;
        self.items.get_key_value(task).map(|(id, _)| id.as_str())
    }

    /// Whether a thread exists: it has a record, or it is a loose task's thread of one, which needs none until it is
    /// claimed.
    #[must_use]
    pub fn thread_exists(&self, thread: &str) -> bool {
        self.threads.contains_key(thread)
            || crate::ids::task_of_one(thread).is_some_and(|task| self.is_loose(task))
    }

    /// Every thread: those with a record, and the thread of one of each loose task, by id.
    #[must_use]
    pub fn all_threads(&self) -> BTreeSet<String> {
        let loose = self.items.keys().filter(|id| self.is_loose(id)).map(|id| crate::ids::one(id));
        self.threads.keys().cloned().chain(loose).collect()
    }

    /// Whether `thread` is a thread of one that no item names, such as one whose members moved out: it is hidden, and
    /// whatever title it kept when it graduated stays free for others to take.
    #[must_use]
    pub fn is_emptied_one(&self, thread: &str) -> bool {
        crate::ids::task_of_one(thread).is_some()
            && !self.items.values().any(|item| {
                self.thread_value(item).all().iter().any(|named| named.as_str() == thread)
            })
    }

    /// The task a thread's row shows in place of a title: `thread` is a thread of one with no stored title, or an
    /// emptied one whose title is free, and its task exists.
    #[must_use]
    pub fn shown_as_task(&self, thread: &str) -> Option<&str> {
        let task = self.task_of(thread)?;
        let titled = self.threads.get(thread).is_some_and(|record| record.title != Value::None);
        (!titled || self.is_emptied_one(thread)).then_some(task)
    }

    /// The thread an item belongs to, when it is in exactly one.
    #[must_use]
    pub fn thread_of(&self, id: &str) -> Option<&str> {
        let item = self.items.get(id)?;
        self.thread_value(item).one().map(String::as_str)
    }

    /// The cycles among waits-for links, each sorted.
    #[must_use]
    pub fn waits_for_cycles(&self) -> Vec<Vec<String>> {
        let edges: Vec<(String, String)> =
            self.links.iter().map(|link| (link.from.clone(), link.to.clone())).collect();
        graph::cycles(&edges)
    }

    /// A path of blocks links from `from` to `to`, both included; a single item when they are the same.
    #[must_use]
    pub fn ordering_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for link in &self.links {
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
    /// The item or thread was deleted, and its id isn't used again.
    Deleted(String),
    /// The write would put an item under itself or its own descendant.
    ParentCycle(String),
    /// The link would close a cycle of waits-for links, through these items.
    WaitsForCycle(Vec<String>),
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
            Self::Deleted(id) => write!(f, "{id} was deleted, and its id isn't used again"),
            Self::ParentCycle(id) => write!(f, "{id} can't go under itself or its own descendant"),
            Self::WaitsForCycle(path) => {
                write!(f, "the link would close a cycle through {}", path.join(", "))
            },
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
pub fn link_key(from: &str, to: &str) -> Result<Key, WriteError> {
    let (from, to) = (name(from)?, name(to)?);
    Ok(Key::new(kind::LINK, &format!("{}-{}-{from}-{to}", link::BLOCKS, from.len()))?)
}

fn to_json<T: Serialize>(value: &T) -> Json {
    serde_json::to_value(value).expect("model values always serialize")
}

fixture! {
    /// Writes a field of any entity, or clears it with `None`, past every invariant the write judge keeps.
    ///
    /// # Errors
    ///
    /// Fails when the state layer refuses.
    fn set<T: Serialize>(
        replica: &mut Replica,
        key: &Key,
        name: &str,
        value: Option<&T>,
        by: By<'_>,
    ) -> Result<(), WriteError> {
        put(replica, key, name, value, by)
    }
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

fixture! {
    /// Creates an item with a title, open.
    ///
    /// # Errors
    ///
    /// Fails on an invalid id, on the id of a deleted item, or when the state layer refuses.
    fn add_item(
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
}

fixture! {
    /// Creates an initiative with a title.
    ///
    /// # Errors
    ///
    /// Fails on an invalid id, on the id of a deleted initiative, or when the state layer refuses.
    fn add_initiative(
        replica: &mut Replica,
        id: &str,
        title: &str,
        by: By<'_>,
    ) -> Result<Key, WriteError> {
        let key = Key::new(kind::INITIATIVE, name(id)?)?;
        if is_deleted(&replica.state, &key) {
            return Err(WriteError::Deleted(id.to_owned()));
        }
        put(replica, &key, initiative::TITLE, Some(&title), by)?;
        Ok(key)
    }
}

fixture! {
    /// Sets an item's state. Setting the state it already has, alone, writes nothing, so re-marking done never moves
    /// when it was done.
    ///
    /// # Errors
    ///
    /// Fails when the item doesn't exist or the state layer refuses.
    fn set_state(
        replica: &mut Replica,
        id: &str,
        state: ItemState,
        by: By<'_>,
    ) -> Result<(), WriteError> {
        let key = existing(&replica.state, id)?;
        if value::<ItemState>(&replica.state, &key, field::STATE)? == Value::One(state) {
            return Ok(());
        }
        put(replica, &key, field::STATE, Some(&state), by)
    }
}

fixture! {
    /// Moves an item under `parent`, an item or an initiative, or to the top level with `None`.
    ///
    /// # Errors
    ///
    /// Fails when the item or the parent doesn't exist, when the move would put the item under itself or a descendant,
    /// or when the state layer refuses.
    fn set_parent(
        replica: &mut Replica,
        id: &str,
        parent: Option<&str>,
        by: By<'_>,
    ) -> Result<(), WriteError> {
        let key = existing(&replica.state, id)?;
        if let Some(parent) = parent {
            let initiative = Key::new(kind::INITIATIVE, name(parent)?)?;
            if replica.state.entity(&initiative).is_none() || is_deleted(&replica.state, &initiative) {
                existing(&replica.state, parent)?;
            }
            let model = Model::read(&replica.state)?;
            if parent == id || model.ancestors(parent).contains(&id) {
                return Err(WriteError::ParentCycle(id.to_owned()));
            }
        }
        put(replica, &key, field::PARENT, parent.map(str::to_owned).as_ref(), by)
    }
}

fixture! {
    /// Makes `from` wait for `to`.
    ///
    /// # Errors
    ///
    /// Fails when either item doesn't exist, when the link would close a cycle, or when the state layer refuses.
    fn link(replica: &mut Replica, from: &str, to: &str, by: By<'_>) -> Result<Key, WriteError> {
        existing(&replica.state, from)?;
        existing(&replica.state, to)?;
        if from == to {
            return Err(WriteError::SelfLink(from.to_owned()));
        }
        let key = link_key(from, to)?;
        if let Some(path) = Model::read(&replica.state)?.ordering_path(to, from) {
            return Err(WriteError::WaitsForCycle(path));
        }
        if replica.state.field(&key, link::FROM).is_none() {
            put(replica, &key, link::FROM, Some(&from), by)?;
            put(replica, &key, link::TO, Some(&to), by)?;
            put(replica, &key, link::KIND, Some(&link::BLOCKS), by)?;
        }
        put(replica, &key, link::PRESENT, Some(&true), by)?;
        Ok(key)
    }
}

fixture! {
    /// Removes a link, writing nothing when it is absent; a concurrent re-add survives. Returns whether it wrote.
    ///
    /// # Errors
    ///
    /// Fails on an invalid id or when the state layer refuses.
    fn unlink(replica: &mut Replica, from: &str, to: &str, by: By<'_>) -> Result<bool, WriteError> {
        let key = link_key(from, to)?;
        if replica.state.field(&key, link::PRESENT).is_none() {
            return Ok(false);
        }
        // A link is a set element: removing it leaves nothing, so an add it never saw keeps the link.
        replica.write(&key, link::PRESENT, by.actor, by.at, None)?;
        Ok(true)
    }
}

/// Whether an item or thread has been deleted: its tombstone is set, and nothing clears it. The one exception is a
/// loose task's thread of one: an older version, which let it be deleted, may have written a tombstone, and honouring
/// it would leave the task unclaimable for good, so it is ignored.
#[must_use]
pub fn is_deleted(state: &State, key: &Key) -> bool {
    if key.kind == kind::THREAD && crate::ids::task_of_one(&key.id).is_some() {
        return false;
    }
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
        state: value(state, key, field::STATE)?,
        done_at,
        parent: value(state, key, field::PARENT)?,
        evidence: value(state, key, field::EVIDENCE)?,
        thread: value(state, key, field::THREAD)?,
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
            Error::Format(format!("invalid {name} on {} {}: {e}", key.kind, key.id))
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

#[cfg(test)]
mod tests;
