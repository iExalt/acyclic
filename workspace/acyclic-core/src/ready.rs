//! Readiness, computed on read from the state alone: a dependency is met once it is done, never by being canceled. Reads never run git: commit evidence is checked when it is written.
//!
//! Only leaves are claimed or ready. A dependency on a parent is met once everything under it meets it, each child
//! judged as a dependency of its own: a parent's stored done is ignored, and a parked parent meets none.
//! A parent's dependencies hold back every item under it.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::claim;
use crate::model::{Item, ItemState, Model, Value};
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
    /// It is canceled, which meets no dependency: what waits for it is the user's to settle.
    Canceled,
}

/// An item's readiness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "readiness")]
pub enum Readiness {
    /// Ready to work on.
    Ready,
    /// Waiting on waits-for targets, by item id.
    Waiting {
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
    /// Canceled.
    Canceled,
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
#[serde(rename_all = "kebab-case", tag = "readiness")]
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
    /// Its ready members, by id in natural order.
    pub items: Vec<String>,
}

/// Where a leaf stands, in seven words that tell all of them apart: what the app draws on each item and counts under
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
    Waiting,
    /// Set aside on purpose, or under a part that is.
    Parked,
    /// In no thread, so never ready.
    Unthreaded,
    /// Dropped on purpose, or under a part that is.
    Canceled,
}

/// A parent's leaves counted by stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Stages {
    pub done: usize,
    pub in_progress: usize,
    pub ready: usize,
    pub waiting: usize,
    pub parked: usize,
    pub unthreaded: usize,
    /// Leaves canceled: not among the others, and not in [`Stages::total`].
    pub canceled: usize,
}

impl Stages {
    fn add(&mut self, stage: Stage, count: usize) {
        let slot = match stage {
            Stage::Done => &mut self.done,
            Stage::InProgress => &mut self.in_progress,
            Stage::Ready => &mut self.ready,
            Stage::Waiting => &mut self.waiting,
            Stage::Parked => &mut self.parked,
            Stage::Unthreaded => &mut self.unthreaded,
            Stage::Canceled => &mut self.canceled,
        };
        *slot += count;
    }

    /// The stage of a parent with these leaves, `claimed` when a live claim holds the thread of an open leaf under it:
    /// done once every leaf is; before that, in progress once any work
    /// below has started and some of what is left isn't parked; otherwise ready, waiting, parked or needs a thread by
    /// its leaves, in that order. `None` with no leaf to count.
    #[must_use]
    pub fn rolled_up(&self, claimed: bool) -> Option<Stage> {
        let finished = self.done;
        // Canceled leaves are left out of the roll-up; a parent with nothing else reads canceled.
        if self.total() == 0 {
            return (self.canceled > 0).then_some(Stage::Canceled);
        }
        if finished == self.total() {
            return Some(Stage::Done);
        }
        let open = self.in_progress + self.ready + self.waiting + self.unthreaded;
        if open > 0 && (self.in_progress > 0 || finished > 0 || claimed) {
            return Some(Stage::InProgress);
        }
        [
            (self.ready, Stage::Ready),
            (self.waiting, Stage::Waiting),
            (self.parked, Stage::Parked),
            (self.unthreaded, Stage::Unthreaded),
        ]
        .into_iter()
        .find_map(|(count, stage)| (count > 0).then_some(stage))
    }

    /// Leaves counted by the stage each has; parents have none, so none count.
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

    /// Whether a thread with these member leaves is finished: it has some, and each is done or canceled. An empty
    /// thread is not finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.total() == self.done && self.done + self.canceled > 0
    }

    /// The leaves counted, every stage together but canceled.
    #[must_use]
    pub fn total(&self) -> usize {
        self.done + self.in_progress + self.ready + self.waiting + self.parked + self.unthreaded
    }
}

/// A parent's progress as it reads now: its leaves counted by stage, and the stage they make it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rollup {
    /// Its leaves by stage: in all and done, the model's own counts.
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
            model.waits_for_cycles().into_iter().chain(model.parent_cycles()).flatten().collect();
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
        let fields: Vec<String> =
            [("state", item.state.is_conflict()), ("thread", item.thread.is_conflict())]
                .into_iter()
                .filter(|(_, conflicted)| *conflicted)
                .map(|(name, _)| name.to_owned())
                .collect();
        if !fields.is_empty() {
            return Some(Readiness::Conflicted { fields });
        }
        // A parent's done is its leaves', so a stored one is ignored.
        if !self.model.children(id).is_empty() {
            return Some(Readiness::Parent);
        }
        if item.is_done() {
            return Some(Readiness::Done);
        }
        if self.model.reads_canceled(id) {
            return Some(Readiness::Canceled);
        }
        if self.cyclic.contains(id) {
            return Some(Readiness::InCycle);
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Readiness::Parked);
        }
        let waiting = self.waiting(id);
        if !waiting.is_empty() {
            return Some(Readiness::Waiting { waiting });
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
            .filter(|id| !self.model.reads_finished(id))
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

    /// The holders of the live claims covering an item: those on its thread.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn holders(&self, id: &str) -> Result<Vec<String>, Error> {
        Ok(claim::holders(&claim::covering(self.state, self.model, id, self.now)?))
    }

    /// The threads with work `actor` may take, each with its ready members: its own claimed thread first, then the
    /// unclaimed ones by id in natural order. Threads others hold are left out.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn ready(&self, actor: &str) -> Result<Vec<ReadyThread>, Error> {
        let mut ready = Vec::new();
        for id in self.model.threads.keys() {
            let status = self.thread(id)?;
            if !status.claimable_by(actor) || status.items().is_empty() {
                continue;
            }
            let own = !matches!(status, ThreadReadiness::Ready { .. });
            let mut items = status.items().to_vec();
            items.sort_by(|a, b| natural::cmp(a, b));
            ready.push((!own, ReadyThread { thread: id.clone(), items }));
        }
        ready.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| natural::cmp(&x.thread, &y.thread)));
        Ok(ready.into_iter().map(|(_, thread)| thread).collect())
    }

    /// A leaf's stage, or `None` for a parent or an unknown one: its readiness decides, and a claim
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
        // Canceled work likewise.
        let leaf = self.model.children(id).is_empty();
        let stage = if leaf && item.is_done() {
            Some(Stage::Done)
        } else if leaf && item.is_canceled() {
            Some(Stage::Canceled)
        } else {
            self.of(id)?.and_then(|status| match status {
                Readiness::Parent => None,
                Readiness::Done => Some(Stage::Done),
                Readiness::Canceled => Some(Stage::Canceled),
                Readiness::Parked => Some(Stage::Parked),
                Readiness::Claimed { .. } | Readiness::Contested { .. } => Some(Stage::InProgress),
                Readiness::Ready => Some(Stage::Ready),
                Readiness::Unthreaded => Some(Stage::Unthreaded),
                Readiness::Waiting { .. } | Readiness::InCycle | Readiness::Conflicted { .. } => {
                    Some(Stage::Waiting)
                },
            })
        };
        self.stages.borrow_mut().insert(id.to_owned(), stage);
        Ok(stage)
    }

    /// A parent's progress as it reads now, with its leaves by stage. Its stage is in progress once a leaf under it
    /// is, or while a live claim holds the thread of any open leaf under it, even one still waiting. A claim names a
    /// thread, not an item, and expires, so this is how the parent reads now; nothing is waiting on it. Under a part
    /// parked by hand, this one included, open leaves count as parked and don't make it read in progress.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn progress(&self, id: &str) -> Result<Option<Rollup>, Error> {
        let Some(item) = self.model.items.get(id) else { return Ok(None) };
        if self.model.children(id).is_empty() {
            return Ok(None);
        }
        let mut stages = Stages::default();
        let parked = item.state == Value::One(ItemState::Parked);
        let canceled = item.state == Value::One(ItemState::Canceled);
        self.tally(id, parked, canceled, &mut BTreeSet::from([id]), &mut stages)?;
        let claimed = self.claimed_under(id, &mut BTreeSet::from([id]))?;
        // A part canceled by hand reads canceled, whatever its leaves say.
        let stage = if canceled { Some(Stage::Canceled) } else { stages.rolled_up(claimed) };
        Ok(Some(Rollup { stages, stage }))
    }

    /// Counts the leaves under `id` by stage, walking as the model's own tally does: each item once, and the open
    /// leaves of a part parked by hand as parked, or canceled by hand as canceled.
    fn tally<'b>(
        &self,
        id: &'b str,
        parked: bool,
        canceled: bool,
        seen: &mut BTreeSet<&'b str>,
        stages: &mut Stages,
    ) -> Result<(), Error>
    where
        'a: 'b,
    {
        for child in self.model.children(id) {
            if !seen.insert(child) {
                continue;
            }
            let Some(item) = self.model.items.get(child) else { continue };
            if self.model.children(child).is_empty() {
                let stage = self.stage(child)?.unwrap_or(Stage::Waiting);
                let open = !matches!(stage, Stage::Done | Stage::Canceled);
                let stage = if canceled && open {
                    Stage::Canceled
                } else if parked && open {
                    Stage::Parked
                } else {
                    stage
                };
                stages.add(stage, 1);
            } else {
                let parks = parked || item.state == Value::One(ItemState::Parked);
                let cancels = canceled || item.state == Value::One(ItemState::Canceled);
                self.tally(child, parks, cancels, seen, stages)?;
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
            // A done stored on a parent, as older versions wrote, is ignored: its leaves decide.
            let finished = item.is_finished() && self.model.children(child).is_empty();
            let set_aside =
                matches!(item.state, Value::One(ItemState::Parked | ItemState::Canceled));
            if !seen.insert(child) || finished || set_aside {
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

    /// Why `dependency` doesn't yet satisfy `dependent`, or `None` when it does.
    fn dependency_wait(&self, dependent: &str, dependency: &str) -> Option<Wait> {
        let Some(item) = self.model.items.get(dependency) else { return Some(Wait::Open) };
        if self.model.children(dependency).is_empty() {
            if item.is_done() {
                return None;
            }
            return Some(if self.model.reads_canceled(dependency) {
                Wait::Canceled
            } else {
                Wait::Open
            });
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Wait::Open);
        }
        // A part canceled by hand, or with nothing under it but canceled work, is canceled as a whole; one with a
        // canceled leaf among others is waited for as a whole, and proceeds when its other work is done.
        if self.model.reads_canceled(dependency) {
            return Some(Wait::Canceled);
        }
        self.leaves_wait(dependent, dependency)
    }

    /// Why the work under a parent doesn't yet satisfy `dependent`, or `None` when all of it does: each child judged
    /// as a dependency of its own, so a leaf by its state, a parent under it parked by hand by that, and any other by
    /// what is under it in turn.
    fn leaves_wait(&self, dependent: &str, parent: &str) -> Option<Wait> {
        // A parent in a cycle meets nothing until the cycle is fixed: a cycle of parents has no bottom to reach.
        if self.cyclic.contains(parent) {
            return Some(Wait::Open);
        }
        self.model
            .children(parent)
            .into_iter()
            .filter(|child| !self.model.reads_canceled(child))
            .find_map(|child| self.dependency_wait(dependent, child))
    }
}

#[cfg(test)]
mod tests;
