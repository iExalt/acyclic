//! Readiness, computed on read from the state alone: a dependency is met once it is done, never by being canceled. Reads never run git: commit evidence is checked when it is written.
//!
//! Only leaves are claimed or ready. A dependency on a parent is met once everything under it meets it, each child
//! judged as a dependency of its own: a parent's stored done is ignored, and a parked parent meets none.
//! A parent's dependencies hold back every item under it.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use serde::Serialize;

use crate::claim;
use crate::model::{Item, ItemState, Leaf, Model, Progress, Value};
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
    /// On a cycle of effective waits-for links among unfinished leaves or parents, which pauses it.
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
    /// Holds only expired claims, yet has members ready to work on: its holder stopped without releasing it. Anyone
    /// may claim it, as an unclaimed thread.
    Stalled {
        /// The holder of the claim that ran out last.
        holder: String,
        /// When that lease ran out, as RFC 3339 in UTC.
        expired_at: String,
        /// The members ready to work on, by id.
        items: Vec<String>,
    },
}

impl ThreadReadiness {
    /// Whether `actor` may claim the thread without forcing: when it is ready, or when `actor` already wins it.
    #[must_use]
    pub fn claimable_by(&self, actor: &str) -> bool {
        match self {
            Self::Ready { .. } | Self::Stalled { .. } => true,
            Self::Claimed { holder, .. } => holder == actor,
            Self::Contested { holders, .. } => holders.first().is_some_and(|h| h == actor),
            _ => false,
        }
    }

    /// The members ready to work on while the thread is held.
    #[must_use]
    pub fn items(&self) -> &[String] {
        match self {
            Self::Ready { items }
            | Self::Claimed { items, .. }
            | Self::Contested { items, .. }
            | Self::Stalled { items, .. } => items,
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
    /// Whether it holds only expired claims: it is listed among the unclaimed threads, marked.
    pub stalled: bool,
}

/// Where a leaf stands, in seven words that tell all of them apart: what the app draws on each item and counts under
/// each parent. In progress means ready in a thread someone holds; stalled, ready in one whose holder's lease ran out.
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
    /// Dropped on purpose, or under a part that is.
    Canceled,
    /// Ready in a thread whose claims have all expired.
    Stalled,
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
    /// Leaves canceled: not among the others, and not in [`Stages::total`].
    pub canceled: usize,
    /// Leaves ready in a thread whose claims have all expired.
    pub stalled: usize,
}

impl Stages {
    fn add(&mut self, stage: Stage, count: usize) {
        let slot = match stage {
            Stage::Done => &mut self.done,
            Stage::InProgress => &mut self.in_progress,
            Stage::Ready => &mut self.ready,
            Stage::Waiting => &mut self.waiting,
            Stage::Parked => &mut self.parked,
            Stage::Canceled => &mut self.canceled,
            Stage::Stalled => &mut self.stalled,
        };
        *slot += count;
    }

    /// The stage of a parent with these leaves, `claimed` when a live claim holds the thread of an open leaf under it:
    /// done once every leaf is; before that, in progress once any work
    /// below has started and some of what is left isn't parked; otherwise stalled, ready, waiting or parked by its
    /// leaves, in that order. `None` with no leaf to count.
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
        let open = self.in_progress + self.stalled + self.ready + self.waiting;
        if open > 0 && (self.in_progress > 0 || finished > 0 || claimed) {
            return Some(Stage::InProgress);
        }
        [
            (self.stalled, Stage::Stalled),
            (self.ready, Stage::Ready),
            (self.waiting, Stage::Waiting),
            (self.parked, Stage::Parked),
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
        self.done + self.in_progress + self.stalled + self.ready + self.waiting + self.parked
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
    /// Each thread's members, built on first use: a loose task is the one member of its thread of one.
    members: OnceCell<BTreeMap<String, Vec<&'a str>>>,
    /// Each thread's winning claim, memoized.
    winners: RefCell<BTreeMap<String, Option<claim::Claim>>>,
    /// Each thread's claim that ran out last while none holds, memoized.
    lapsed: RefCell<BTreeMap<String, Option<claim::Claim>>>,
    /// Each item's stage, memoized, since a rollup counts every leaf under every parent.
    stages: RefCell<BTreeMap<String, Option<Stage>>>,
    /// Each parent's leaves, shared by structural completion and the stage rollup that enriches them.
    leaf_sets: RefCell<BTreeMap<String, LeafSet<'a>>>,
    /// Whether any item's state holds canceled, found on first use.
    any_canceled: OnceCell<bool>,
    /// The items under a part canceled by hand, built on first use.
    under_canceled: OnceCell<BTreeSet<&'a str>>,
    /// Whether each item reads canceled, memoized: [`Model::reads_canceled`] walks the item's subtree or ancestors.
    canceled: RefCell<BTreeMap<String, bool>>,
}

struct LeafSet<'a> {
    leaves: Rc<[Leaf<'a>]>,
    progress: Progress,
}

impl<'a> ReadinessEvaluator<'a> {
    /// Evaluates readiness at `now`.
    #[must_use]
    pub fn new(state: &'a State, model: &'a Model, now: Millis) -> Self {
        let cyclic =
            model.read_wait_cycles().into_iter().chain(model.parent_cycles()).flatten().collect();
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
            lapsed: RefCell::default(),
            stages: RefCell::default(),
            leaf_sets: RefCell::default(),
            any_canceled: OnceCell::new(),
            under_canceled: OnceCell::new(),
            canceled: RefCell::default(),
        }
    }

    /// The parent's leaves, memoized for this immutable model snapshot.
    fn leaves_under(&self, id: &str) -> (Rc<[Leaf<'a>]>, Progress) {
        if let Some(leaf_set) = self.leaf_sets.borrow().get(id) {
            return (Rc::clone(&leaf_set.leaves), leaf_set.progress);
        }
        let leaves: Rc<[Leaf<'a>]> = self.model.leaves_under(id).into();
        let progress = Model::progress_from_leaves(&leaves);
        self.leaf_sets
            .borrow_mut()
            .insert(id.to_owned(), LeafSet { leaves: Rc::clone(&leaves), progress });
        (leaves, progress)
    }

    /// Whether an item reads done, using structural counts that don't depend on readiness or time.
    fn reads_done(&self, id: &str) -> bool {
        let Some(item) = self.model.items.get(id) else { return false };
        if self.model.children(id).is_empty() {
            return item.is_done();
        }
        let progress = self.leaves_under(id).1;
        progress.done == progress.total
            && progress.done > 0
            && item.state != Value::One(ItemState::Parked)
            && item.state != Value::One(ItemState::Canceled)
    }

    /// Whether an item reads canceled, as [`Model::reads_canceled`] says, but built once for every item: the part
    /// canceled by hand is found from its own state, and a parent from its children.
    fn reads_canceled(&self, id: &str) -> bool {
        // Nothing is canceled in most trackers, and then nothing reads canceled: no work to remember.
        let any = *self.any_canceled.get_or_init(|| {
            self.model.items.values().any(|item| item.state.all().contains(&&ItemState::Canceled))
        });
        if !any {
            return false;
        }
        if let Some(known) = self.canceled.borrow().get(id) {
            return *known;
        }
        let Some(item) = self.model.items.get(id) else { return false };
        // A parent cycle ends here: it reads as not canceled while it is being worked out.
        self.canceled.borrow_mut().insert(id.to_owned(), false);
        let under = self.under_canceled.get_or_init(|| {
            let mut under = BTreeSet::new();
            let mut stack: Vec<&str> = self
                .model
                .items
                .values()
                .filter(|item| item.state == Value::One(ItemState::Canceled))
                .map(|item| item.id.as_str())
                .collect();
            while let Some(at) = stack.pop() {
                for child in self.model.children(at) {
                    if under.insert(child) {
                        stack.push(child);
                    }
                }
            }
            under
        });
        let kids = self.model.children(id);
        let reads = if kids.is_empty() {
            !item.is_done() && (item.is_canceled() || under.contains(id))
        } else if item.state == Value::One(ItemState::Canceled) {
            true
        } else if under.contains(id) {
            // Under a part canceled by hand, unless every leaf that is left is done.
            !self.reads_done(id)
        } else {
            !self.reads_done(id) && kids.into_iter().all(|child| self.reads_canceled(child))
        };
        self.canceled.borrow_mut().insert(id.to_owned(), reads);
        reads
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
        let fields: Vec<String> = [
            ("state", item.state.is_conflict()),
            ("thread", self.model.thread_value(item).is_conflict()),
        ]
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
        if self.reads_canceled(id) {
            return Some(Readiness::Canceled);
        }
        if self.cyclic.contains(id) {
            return Some(Readiness::InCycle);
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Readiness::Parked);
        }
        let waiting = self.waiting(id);
        (!waiting.is_empty()).then_some(Readiness::Waiting { waiting })
    }

    /// A thread's readiness.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn thread(&self, thread: &str) -> Result<ThreadReadiness, Error> {
        if !self.model.thread_exists(thread) {
            return Ok(ThreadReadiness::Missing);
        }
        let members = self.members.get_or_init(|| {
            let mut members: BTreeMap<String, Vec<&str>> = BTreeMap::new();
            for item in self.model.items.values() {
                if let Some(thread) = self.model.held_thread(&item.id) {
                    members.entry(thread.into_owned()).or_default().push(&item.id);
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
            .filter(|id| !self.model.items[*id].is_done() && !self.reads_canceled(id))
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
        if items.is_empty() {
            return Ok(ThreadReadiness::NothingReady);
        }
        Ok(match self.lapsed(thread)? {
            Some(lapsed) => ThreadReadiness::Stalled {
                holder: lapsed.holder,
                expired_at: crate::time::rfc3339(lapsed.lease_expires.unwrap_or_default()),
                items,
            },
            None => ThreadReadiness::Ready { items },
        })
    }

    /// Whether an item reads canceled: the model's own answer, remembered for the whole evaluator.
    #[must_use]
    pub fn canceled(&self, id: &str) -> bool {
        self.reads_canceled(id)
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
        for id in &self.model.all_threads() {
            let status = self.thread(id)?;
            if !status.claimable_by(actor) || status.items().is_empty() {
                continue;
            }
            let own = matches!(
                status,
                ThreadReadiness::Claimed { .. } | ThreadReadiness::Contested { .. }
            );
            let stalled = matches!(status, ThreadReadiness::Stalled { .. });
            let mut items = status.items().to_vec();
            items.sort_by(|a, b| natural::cmp(a, b));
            ready.push((!own, ReadyThread { thread: id.clone(), items, stalled }));
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
            match self.of(id)? {
                Some(Readiness::Ready) if self.stalled(id)? => Some(Stage::Stalled),
                found => found.and_then(|status| match status {
                    Readiness::Parent => None,
                    Readiness::Done => Some(Stage::Done),
                    Readiness::Canceled => Some(Stage::Canceled),
                    Readiness::Parked => Some(Stage::Parked),
                    Readiness::Claimed { .. } | Readiness::Contested { .. } => {
                        Some(Stage::InProgress)
                    },
                    Readiness::Ready => Some(Stage::Ready),
                    Readiness::Waiting { .. }
                    | Readiness::InCycle
                    | Readiness::Conflicted { .. } => Some(Stage::Waiting),
                }),
            }
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
        let (leaves, _) = self.leaves_under(id);
        for leaf in leaves.iter() {
            let stage = self.stage(&leaf.item.id)?.unwrap_or(Stage::Waiting);
            let open = !matches!(stage, Stage::Done | Stage::Canceled);
            let stage = if leaf.canceled && open {
                Stage::Canceled
            } else if leaf.parked && open {
                Stage::Parked
            } else {
                stage
            };
            stages.add(stage, 1);
        }
        let claimed = self.claimed_under(id, &mut BTreeSet::from([id]))?;
        let rolled = stages.rolled_up(claimed);
        let stage =
            if item.state == Value::One(ItemState::Canceled) || rolled == Some(Stage::Canceled) {
                Some(Stage::Canceled)
            } else if item.state == Value::One(ItemState::Parked) {
                Some(Stage::Parked)
            } else {
                rolled
            };
        Ok(Some(Rollup { stages, stage }))
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
            } else if let Some(thread) = self.model.held_thread(child)
                && self.winner(&thread)?.is_some()
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

    /// The claim that ran out last on a thread nobody holds now, memoized: `None` while a claim holds it, or when it
    /// never had a claim with a time lease. Such a thread with ready work is stalled.
    fn lapsed(&self, thread: &str) -> Result<Option<claim::Claim>, Error> {
        if let Some(lapsed) = self.lapsed.borrow().get(thread) {
            return Ok(lapsed.clone());
        }
        let lapsed = claim::lapsed(self.state, &claim::thread_key(thread)?, self.now)?;
        self.lapsed.borrow_mut().insert(thread.to_owned(), lapsed.clone());
        Ok(lapsed)
    }

    /// Whether a ready item sits in a stalled thread: one whose claims have all expired.
    fn stalled(&self, id: &str) -> Result<bool, Error> {
        match self.model.held_thread(id) {
            Some(thread) => Ok(self.lapsed(&thread)?.is_some()),
            None => Ok(false),
        }
    }

    /// Why `dependency` doesn't yet satisfy `dependent`, or `None` when it does.
    fn dependency_wait(&self, dependent: &str, dependency: &str) -> Option<Wait> {
        let Some(item) = self.model.items.get(dependency) else { return Some(Wait::Open) };
        if self.model.children(dependency).is_empty() {
            if item.is_done() {
                return None;
            }
            return Some(if self.reads_canceled(dependency) { Wait::Canceled } else { Wait::Open });
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Wait::Open);
        }
        // A part canceled by hand, or with nothing under it but canceled work, is canceled as a whole; one with a
        // canceled leaf among others is waited for as a whole, and proceeds when its other work is done.
        if self.reads_canceled(dependency) {
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
            .filter(|child| !self.reads_canceled(child))
            .find_map(|child| self.dependency_wait(dependent, child))
    }
}

#[cfg(test)]
mod tests;
