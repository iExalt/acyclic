//! Readiness, computed on read from the state alone: a dependency is met once it is done, and an archived dependency
//! always is. Reads never run git: commit evidence is checked when it is written.
//!
//! Only leaves are claimed or ready. A dependency on a parent is met when the parent was marked done by hand, and
//! otherwise once everything under it meets it, each child judged as a dependency of its own, so a part closed by hand
//! counts as closed: a parked parent meets none.
//! A parent's dependencies hold back every item under it.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::claim;
use crate::model::{Item, ItemState, Model, Progress, ProgressState, Value};
use crate::natural;
use crate::state::{Error, Millis, State};

mod waves;
pub use waves::Waves;

/// Why a dependency isn't met yet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Wait {
    /// It isn't done.
    Open,
}

/// An item's readiness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum Readiness {
    /// Ready to work on.
    Ready,
    /// Waiting on waits-for targets, by item id.
    Blocked {
        /// What each unmet dependency waits for.
        #[allow(
            clippy::zero_sized_map_values,
            reason = "each wait says why in the JSON, as `{\"why\": …}`"
        )]
        waiting: BTreeMap<String, Wait>,
    },
    /// In no thread, or in one with no record, so never ready: threads are the unit of work.
    Unthreaded,
    /// Its thread is claimed by one holder.
    Claimed {
        /// The winning holder.
        holder: String,
    },
    /// Its thread is claimed by several holders that didn't see each other; the first is the winner.
    Contested {
        /// The holders, winner first.
        holders: Vec<String>,
    },
    /// On a cycle of links or parents, which pauses it.
    InCycle,
    /// Its state or thread holds conflicting values.
    Conflicted {
        /// The conflicting fields.
        fields: Vec<String>,
    },
    /// Has children, so it is never claimed itself.
    Parent,
    /// Set aside on purpose.
    Parked,
    /// Done.
    Done,
    /// It or an ancestor is archived.
    Archived,
}

impl Readiness {
    /// Whether `actor` may finish an item with this readiness without forcing: when it is ready, when what stands in the
    /// way is the actor's own claim on its thread, alone or as winner of a contest, or when it is finished already,
    /// which finishing again doesn't change. A parent is judged by its open work instead.
    #[must_use]
    pub fn finishable_by(&self, actor: &str) -> bool {
        match self {
            Self::Ready | Self::Done => true,
            Self::Claimed { holder } => holder == actor,
            Self::Contested { holders } => holders.first().is_some_and(|h| h == actor),
            _ => false,
        }
    }
}

/// A thread's readiness: whether it can be claimed, and its members that would be ready while it is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum ThreadReadiness {
    /// No thread has this id.
    Missing,
    /// Unclaimed, with members ready to work on.
    Ready {
        /// The ready members, by id.
        items: Vec<String>,
    },
    /// Unclaimed, with no member ready to work on.
    NothingReady,
    /// Claimed by one holder.
    Claimed {
        /// The winning holder.
        holder: String,
        /// The members ready to work on, by id.
        items: Vec<String>,
    },
    /// Claimed by several holders that didn't see each other; the first is the winner.
    Contested {
        /// The holders, winner first.
        holders: Vec<String>,
        /// The members ready to work on, by id.
        items: Vec<String>,
    },
}

impl ThreadReadiness {
    /// Whether `actor` may claim the thread without forcing: when it is ready, or when `actor` already wins it.
    #[must_use]
    pub fn claimable_by(&self, actor: &str) -> bool {
        match self {
            Self::Ready { .. } => true,
            Self::Claimed { holder, .. } => holder == actor,
            Self::Contested { holders, .. } => holders.first().is_some_and(|h| h == actor),
            _ => false,
        }
    }

    /// The members ready to work on while the thread is held.
    #[must_use]
    pub fn items(&self) -> &[String] {
        match self {
            Self::Ready { items } | Self::Claimed { items, .. } | Self::Contested { items, .. } => {
                items
            },
            _ => &[],
        }
    }
}

/// A thread with work for an actor, as `ready` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyThread {
    /// The thread.
    pub thread: String,
    /// Its ready members, by title in natural order, then id.
    pub items: Vec<String>,
}

/// Where a leaf stands, in six words that tell all of them apart: what the app draws on each item and counts under
/// each parent. In progress means ready in a thread someone holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Done.
    Done,
    /// Ready in a claimed thread.
    InProgress,
    /// Ready to start.
    Ready,
    /// Waiting on something: a dependency, a cycle or a conflict.
    Blocked,
    /// Set aside on purpose, or under a part that is.
    Parked,
    /// In no thread, so never ready.
    Unthreaded,
}

/// A parent's leaves counted by stage, archived ones aside.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Stages {
    pub done: usize,
    pub in_progress: usize,
    pub ready: usize,
    pub blocked: usize,
    pub parked: usize,
    pub unthreaded: usize,
}

impl Stages {
    fn add(&mut self, stage: Stage, count: usize) {
        let slot = match stage {
            Stage::Done => &mut self.done,
            Stage::InProgress => &mut self.in_progress,
            Stage::Ready => &mut self.ready,
            Stage::Blocked => &mut self.blocked,
            Stage::Parked => &mut self.parked,
            Stage::Unthreaded => &mut self.unthreaded,
        };
        *slot += count;
    }

    /// The stage of a parent with these leaves, `claimed` when a live claim holds the thread of an open leaf under it:
    /// done once every leaf is; before that, in progress once any work
    /// below has started and some of what is left isn't parked; otherwise ready, blocked, parked or needs a thread by
    /// its leaves, in that order. `None` with no leaf to count.
    #[must_use]
    pub fn rolled_up(&self, claimed: bool) -> Option<Stage> {
        let finished = self.done;
        if self.total() == 0 {
            return None;
        }
        if finished == self.total() {
            return Some(Stage::Done);
        }
        let open = self.in_progress + self.ready + self.blocked + self.unthreaded;
        if open > 0 && (self.in_progress > 0 || finished > 0 || claimed) {
            return Some(Stage::InProgress);
        }
        [
            (self.ready, Stage::Ready),
            (self.blocked, Stage::Blocked),
            (self.parked, Stage::Parked),
            (self.unthreaded, Stage::Unthreaded),
        ]
        .into_iter()
        .find_map(|(count, stage)| (count > 0).then_some(stage))
    }

    /// Leaves counted by the stage each has; parents and archived items have none, so none count.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn of_leaves<'i>(
        readiness: &ReadinessEvaluator<'_>,
        items: impl IntoIterator<Item = &'i str>,
    ) -> Result<Self, Error> {
        let mut stages = Self::default();
        for id in items {
            if let Some(stage) = readiness.stage(id)? {
                stages.add(stage, 1);
            }
        }
        Ok(stages)
    }

    /// The leaves counted, every stage together.
    #[must_use]
    pub fn total(&self) -> usize {
        self.done + self.in_progress + self.ready + self.blocked + self.parked + self.unthreaded
    }
}

/// A parent's progress as it reads now, with its leaves counted by stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rollup {
    /// Its state, and its leaves done and in all, as the model counts them.
    pub progress: Progress,
    /// The same leaves by stage: the done ones are the model's done.
    pub stages: Stages,
    /// Its stage, from its leaves; `None` with no leaf to count.
    pub stage: Option<Stage>,
}

/// Computes readiness over one state.
pub struct ReadinessEvaluator<'a> {
    state: &'a State,
    model: &'a Model,
    now: Millis,
    cyclic: BTreeSet<String>,
    /// Waits-for links' targets by the item they start from.
    links_from: BTreeMap<String, Vec<String>>,
    archived: RefCell<BTreeMap<String, bool>>,
    /// Each thread's members, built on first use.
    members: OnceCell<BTreeMap<&'a str, Vec<&'a str>>>,
    /// Each thread's winning claim, memoized.
    winners: RefCell<BTreeMap<String, Option<claim::Claim>>>,
    /// Each item's stage, memoized, since a rollup counts every leaf under every parent.
    stages: RefCell<BTreeMap<String, Option<Stage>>>,
}

impl<'a> ReadinessEvaluator<'a> {
    /// Evaluates readiness at `now`.
    #[must_use]
    pub fn new(state: &'a State, model: &'a Model, now: Millis) -> Self {
        let cyclic =
            model.link_cycles().into_iter().chain(model.parent_cycles()).flatten().collect();
        let mut links_from: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for link in &model.links {
            links_from.entry(link.from.clone()).or_default().push(link.to.clone());
        }
        Self {
            state,
            model,
            now,
            cyclic,
            links_from,
            archived: RefCell::default(),
            members: OnceCell::new(),
            winners: RefCell::default(),
            stages: RefCell::default(),
        }
    }

    /// An item's readiness, or `None` for an unknown item.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn of(&self, id: &str) -> Result<Option<Readiness>, Error> {
        let Some(item) = self.model.items.get(id) else { return Ok(None) };
        if let Some(status) = self.unclaimed(id, item) {
            return Ok(Some(status));
        }
        let mut holders = claim::holders(&claim::covering(self.state, self.model, id, self.now)?);
        Ok(Some(match holders.len() {
            0 => Readiness::Ready,
            1 => Readiness::Claimed { holder: holders.remove(0) },
            _ => Readiness::Contested { holders },
        }))
    }

    /// An item's readiness as though no thread were claimed, or `None` when it would be ready.
    fn unclaimed(&self, id: &str, item: &Item) -> Option<Readiness> {
        if self.is_archived(id) {
            return Some(Readiness::Archived);
        }
        let fields: Vec<String> =
            [("state", item.state.is_conflict()), ("thread", item.thread.is_conflict())]
                .into_iter()
                .filter(|(_, conflicted)| *conflicted)
                .map(|(name, _)| name.to_owned())
                .collect();
        if !fields.is_empty() {
            return Some(Readiness::Conflicted { fields });
        }
        if item.is_done() {
            return Some(Readiness::Done);
        }
        if !self.model.children(id).is_empty() {
            return Some(Readiness::Parent);
        }
        if self.cyclic.contains(id) {
            return Some(Readiness::InCycle);
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Readiness::Parked);
        }
        let waiting = self.waiting(id);
        if !waiting.is_empty() {
            return Some(Readiness::Blocked { waiting });
        }
        let threaded =
            item.thread.one().is_some_and(|thread| self.model.threads.contains_key(thread));
        (!threaded).then_some(Readiness::Unthreaded)
    }

    /// A thread's readiness.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn thread(&self, thread: &str) -> Result<ThreadReadiness, Error> {
        if !self.model.threads.contains_key(thread) {
            return Ok(ThreadReadiness::Missing);
        }
        let members = self.members.get_or_init(|| {
            let mut members: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for item in self.model.items.values() {
                if let Some(thread) = item.thread.one() {
                    members.entry(thread).or_default().push(&item.id);
                }
            }
            members
        });
        let items: Vec<String> = members
            .get(thread)
            .into_iter()
            .flatten()
            .copied()
            // Finished work is skipped.
            .filter(|id| !self.model.items[*id].is_done())
            .filter(|id| self.unclaimed(id, &self.model.items[*id]).is_none())
            .map(str::to_owned)
            .collect();
        let key = claim::thread_key(thread)?;
        let mut holders = claim::holders(&claim::live(self.state, &key, self.now)?);
        if holders.len() > 1 {
            return Ok(ThreadReadiness::Contested { holders, items });
        }
        if let Some(holder) = holders.pop() {
            return Ok(ThreadReadiness::Claimed { holder, items });
        }
        Ok(if items.is_empty() {
            ThreadReadiness::NothingReady
        } else {
            ThreadReadiness::Ready { items }
        })
    }

    /// The threads with work `actor` may take, each with its ready members: its own claimed thread first, then the
    /// unclaimed ones by title in natural order and id. Threads others hold are left out, and so are archived subtrees
    /// unless `include_archived` is set, which judges every item as if nothing were archived.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn ready(&self, include_archived: bool, actor: &str) -> Result<Vec<ReadyThread>, Error> {
        if include_archived {
            let mut model = self.model.clone();
            for item in model.items.values_mut() {
                item.archived = Value::None;
            }
            let unarchived = ReadinessEvaluator::new(self.state, &model, self.now);
            return unarchived.ready(false, actor);
        }
        let title =
            |id: &str, title: &Value<String>| title.one().cloned().unwrap_or_else(|| id.to_owned());
        let mut ready = Vec::new();
        for (id, thread) in &self.model.threads {
            let status = self.thread(id)?;
            if !status.claimable_by(actor) || status.items().is_empty() {
                continue;
            }
            let own = !matches!(status, ThreadReadiness::Ready { .. });
            let mut items: Vec<(String, &String)> = status
                .items()
                .iter()
                .map(|item| (title(item, &self.model.items[item].title), item))
                .collect();
            items.sort_by(|a, b| natural::cmp(&a.0, &b.0).then_with(|| a.1.cmp(b.1)));
            let items = items.into_iter().map(|(_, item)| item.clone()).collect();
            let key = (!own, title(id, &thread.title));
            ready.push((key, ReadyThread { thread: id.clone(), items }));
        }
        ready.sort_by(|(a, x), (b, y)| {
            a.0.cmp(&b.0)
                .then_with(|| natural::cmp(&a.1, &b.1))
                .then_with(|| x.thread.cmp(&y.thread))
        });
        Ok(ready.into_iter().map(|(_, thread)| thread).collect())
    }

    /// A leaf's stage, or `None` for a parent, an archived item or an unknown one: its readiness decides, and a claim
    /// on its thread reads as in progress.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn stage(&self, id: &str) -> Result<Option<Stage>, Error> {
        if let Some(known) = self.stages.borrow().get(id) {
            return Ok(*known);
        }
        let Some(item) = self.model.items.get(id) else { return Ok(None) };
        // Done work is done whatever else holds it, such as threads two clones wrote apart: the model counts it so.
        let finished =
            item.is_done() && self.model.children(id).is_empty() && !self.is_archived(id);
        let stage = if finished {
            Some(Stage::Done)
        } else {
            self.of(id)?.and_then(|status| match status {
                Readiness::Archived | Readiness::Parent => None,
                Readiness::Done => Some(Stage::Done),
                Readiness::Parked => Some(Stage::Parked),
                Readiness::Claimed { .. } | Readiness::Contested { .. } => Some(Stage::InProgress),
                Readiness::Ready => Some(Stage::Ready),
                Readiness::Unthreaded => Some(Stage::Unthreaded),
                Readiness::Blocked { .. } | Readiness::InCycle | Readiness::Conflicted { .. } => {
                    Some(Stage::Blocked)
                },
            })
        };
        self.stages.borrow_mut().insert(id.to_owned(), stage);
        Ok(stage)
    }

    /// A parent's progress as it reads now, with its leaves by stage. It reads in progress, rather than backlog, once
    /// a leaf under it is, or while a live claim holds the thread of any open leaf under it, even one still blocked.
    /// A claim names a thread, not an item, and expires, so this is how the parent reads now; nothing is waiting on
    /// it. Under a part parked by hand, this one included, open leaves count as parked and don't make it read in
    /// progress.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn progress(&self, id: &str) -> Result<Option<Rollup>, Error> {
        let Some(mut progress) = self.model.progress(id) else { return Ok(None) };
        let mut stages = Stages::default();
        let parked = progress.state == ProgressState::Parked;
        self.tally(id, parked, &mut BTreeSet::from([id]), &mut stages)?;
        let claimed = self.claimed_under(id, &mut BTreeSet::from([id]))?;
        if progress.state == ProgressState::InProgress || progress.state == ProgressState::Backlog {
            let moving = stages.in_progress > 0 || progress.done > 0 || claimed;
            progress.state =
                if moving { ProgressState::InProgress } else { ProgressState::Backlog };
        }
        Ok(Some(Rollup { progress, stages, stage: stages.rolled_up(claimed) }))
    }

    /// Counts the leaves under `id` by stage, walking as the model's own tally does: archived ones aside, each item
    /// once, every leaf of a part done by hand as done, and the open leaves of a part parked by hand as parked.
    fn tally<'b>(
        &self,
        id: &'b str,
        parked: bool,
        seen: &mut BTreeSet<&'b str>,
        stages: &mut Stages,
    ) -> Result<(), Error>
    where
        'a: 'b,
    {
        for child in self.model.children(id) {
            if !seen.insert(child) || self.is_archived(child) {
                continue;
            }
            let Some(item) = self.model.items.get(child) else { continue };
            if self.model.children(child).is_empty() {
                let stage = self.stage(child)?.unwrap_or(Stage::Blocked);
                let open = stage != Stage::Done;
                stages.add(if parked && open { Stage::Parked } else { stage }, 1);
            } else if item.is_done() {
                let mut closed = Stages::default();
                self.tally(child, false, seen, &mut closed)?;
                stages.add(Stage::Done, closed.total());
            } else {
                let parks = parked || item.state == Value::One(ItemState::Parked);
                self.tally(child, parks, seen, stages)?;
            }
        }
        Ok(())
    }

    /// Whether a live claim holds the thread of an open leaf under `id`. What is parked, or under a parked part, isn't
    /// being worked, so it counts for nothing.
    fn claimed_under<'b>(&self, id: &'b str, seen: &mut BTreeSet<&'b str>) -> Result<bool, Error>
    where
        'a: 'b,
    {
        for child in self.model.children(id) {
            let Some(item) = self.model.items.get(child) else { continue };
            if !seen.insert(child)
                || item.is_done()
                || item.state == Value::One(ItemState::Parked)
                || self.is_archived(child)
            {
                continue;
            }
            if !self.model.children(child).is_empty() {
                if self.claimed_under(child, seen)? {
                    return Ok(true);
                }
            } else if let Some(thread) = item.thread.one()
                && self.winner(thread)?.is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The unmet waits-for targets of an item and its ancestors.
    /// What keeps a parent from being finished: its children that are neither done nor archived, and what would hold
    /// it back as a leaf (a cycle, being parked, or unmet dependencies).
    #[must_use]
    pub fn unfinished(&self, id: &str) -> (Vec<String>, Option<Readiness>) {
        let open = self
            .model
            .children(id)
            .into_iter()
            .filter(|child| {
                self.model.items.get(*child).is_some_and(|c| !c.is_done() && !c.is_archived_root())
            })
            .map(str::to_owned)
            .collect();
        let held = if self.cyclic.contains(id) {
            Some(Readiness::InCycle)
        } else if self.model.items.get(id).is_some_and(|i| i.state == Value::One(ItemState::Parked))
        {
            Some(Readiness::Parked)
        } else {
            let waiting = self.waiting(id);
            (!waiting.is_empty()).then_some(Readiness::Blocked { waiting })
        };
        (open, held)
    }

    #[allow(
        clippy::zero_sized_map_values,
        reason = "each wait says why in the JSON, as `{\"why\": …}`"
    )]
    fn waiting(&self, id: &str) -> BTreeMap<String, Wait> {
        let mut waiting = BTreeMap::new();
        for to in self.linked_from(id) {
            if let Some(wait) = self.dependency_wait(id, to) {
                waiting.insert(to.to_owned(), wait);
            }
        }
        waiting
    }

    /// The targets of an item's and its ancestors' blocks links.
    fn linked_from(&self, id: &str) -> Vec<&str> {
        std::iter::once(id)
            .chain(self.model.ancestors(id))
            .flat_map(|holder| self.links_from.get(holder).into_iter().flatten())
            .map(String::as_str)
            .collect()
    }

    /// A thread's winning claim, memoized.
    fn winner(&self, thread: &str) -> Result<Option<claim::Claim>, Error> {
        if let Some(winner) = self.winners.borrow().get(thread) {
            return Ok(winner.clone());
        }
        let winner = claim::winner(self.state, &claim::thread_key(thread)?, self.now)?;
        self.winners.borrow_mut().insert(thread.to_owned(), winner.clone());
        Ok(winner)
    }

    /// Whether an item is archived, memoized.
    fn is_archived(&self, id: &str) -> bool {
        if let Some(&archived) = self.archived.borrow().get(id) {
            return archived;
        }
        let archived = self.model.is_archived(id);
        self.archived.borrow_mut().insert(id.to_owned(), archived);
        archived
    }

    /// Why `dependency` doesn't yet satisfy `dependent`, or `None` when it does.
    fn dependency_wait(&self, dependent: &str, dependency: &str) -> Option<Wait> {
        let Some(item) = self.model.items.get(dependency) else { return Some(Wait::Open) };
        if self.is_archived(dependency) {
            return None;
        }
        // Reopening archived work flags its dependents but never blocks them again.
        if item.reopened.all().iter().any(|r| r.archived) {
            return None;
        }
        if !item.is_done() {
            if item.state == Value::One(ItemState::Parked)
                || self.model.children(dependency).is_empty()
            {
                return Some(Wait::Open);
            }
            return self.leaves_wait(dependent, dependency);
        }
        None
    }

    /// Why the work under a parent doesn't yet satisfy `dependent`, or `None` when all of it does: each child judged
    /// as a dependency of its own, so a leaf by its state, a parent under it done or parked by hand by that, and any
    /// other by what is under it in turn. A leaf waiting on
    /// its own ancestor is among what it waits on, so only closing the ancestor by hand meets it.
    fn leaves_wait(&self, dependent: &str, parent: &str) -> Option<Wait> {
        // A parent in a cycle meets nothing until the cycle is fixed: a cycle of parents has no bottom to reach.
        if self.cyclic.contains(parent) {
            return Some(Wait::Open);
        }
        self.model
            .children(parent)
            .into_iter()
            .find_map(|child| self.dependency_wait(dependent, child))
    }
}

#[cfg(test)]
mod tests;
