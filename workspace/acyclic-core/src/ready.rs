//! Readiness, computed on read under plan D18: done inside a thread, merged across. A dependency in the same thread
//! is met when it is done; one in another thread, or for an item in no thread, is met once its evidence commit is on
//! its repository's default branch, and is "in review" until then. Work with no repository is met when done, and an
//! archived dependency always is.
//!
//! Only leaves are claimed or ready. A dependency on a parent is met when the parent was marked done by hand, and
//! otherwise once everything under it meets it, each child judged as a dependency of its own, so a part closed by hand
//! counts as closed: a parked parent meets none.
//! A parent's dependencies hold back every item under it.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Serialize;

use crate::claim;
use crate::model::{Evidence, Item, ItemState, LinkKind, Model, Progress, Repository, Value};
use crate::natural;
use crate::state::{Error, Millis, State};

mod waves;
pub use waves::Waves;

/// Whether a commit is on a repository's default branch.
pub trait Ancestry {
    /// `Ok(true)` when `commit` is on `repository`'s default branch, `Ok(false)` when it isn't yet, and `Err` with
    /// the reason when that can't be checked.
    ///
    /// # Errors
    ///
    /// Fails when the repository has no clone or default branch, or git fails.
    fn merged(&self, repository: &Repository, commit: &str) -> Result<bool, String>;
}

/// Checks ancestry in local code clones, as they were last fetched.
pub struct GitAncestry<F> {
    clone_of: F,
}

impl<F: Fn(&Repository) -> Option<PathBuf>> GitAncestry<F> {
    /// Checks in the clone `clone_of` names for each repository record.
    pub fn new(clone_of: F) -> Self {
        Self { clone_of }
    }
}

impl<F: Fn(&Repository) -> Option<PathBuf>> Ancestry for GitAncestry<F> {
    fn merged(&self, repository: &Repository, commit: &str) -> Result<bool, String> {
        // Evidence is written by agents and synced, so it never reaches git as anything but a commit id.
        if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("not a commit id".to_owned());
        }
        let clone = (self.clone_of)(repository).ok_or("no clone of the repository is mapped")?;
        let branch =
            repository.default_branch.one().ok_or("the repository has no single default branch")?;
        let git = |args: &[&str]| {
            crate::store::git_at(&clone)
                .args(args)
                .env("LC_ALL", "C")
                .output()
                .map_err(|e| format!("git can't run: {e}"))
        };
        let target = [format!("refs/remotes/origin/{branch}"), format!("refs/heads/{branch}")]
            .into_iter()
            .find(|candidate| {
                git(&["rev-parse", "--verify", "--quiet", &format!("{candidate}^{{commit}}")])
                    .is_ok_and(|out| out.status.success())
            })
            .ok_or_else(|| format!("the clone has no {branch} branch"))?;
        let output = git(&["merge-base", "--is-ancestor", "--end-of-options", commit, &target])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        }
    }
}

/// Why a dependency isn't met yet.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case", tag = "why")]
pub enum Wait {
    /// It isn't done.
    Open,
    /// It is done, but its commit isn't on the default branch yet.
    InReview,
    /// It is done, but whether its commit is merged can't be checked.
    Unverified {
        /// The repository record.
        repository: String,
        /// Why not.
        reason: String,
    },
}

/// An item's readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum Status {
    /// Ready to work on.
    Ready,
    /// Waiting on hard dependencies, by item id.
    Blocked {
        /// What each unmet dependency waits for.
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
    /// Its thread is unclaimed, but another thread's winning claim holds a resource one of its thread's open members
    /// names.
    ResourceBusy {
        /// The resource.
        resource: String,
        /// The thread holding it.
        thread: String,
        /// That thread's holder.
        holder: String,
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
    /// Done, and merged where that matters.
    Done,
    /// Done, but its commit isn't on the default branch yet.
    InReview,
    /// Done, but whether its commit is merged can't be checked.
    Unverified {
        /// The repository record.
        repository: String,
        /// Why not.
        reason: String,
    },
    /// It or an ancestor is archived.
    Archived,
}

/// A thread's readiness: whether it can be claimed, and its members that would be ready while it is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum ThreadStatus {
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
    /// Unclaimed, but another thread's winning claim holds a resource one of its open members names.
    ResourceBusy {
        /// The resource.
        resource: String,
        /// The thread holding it.
        thread: String,
        /// That thread's holder.
        holder: String,
    },
}

impl ThreadStatus {
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

/// Where a leaf stands, in seven words that tell all of them apart: what the app draws on each item and counts under
/// each parent. In progress means started, or ready in a thread someone holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    /// Done, and merged where that matters.
    Done,
    /// Done, but its commit isn't on the default branch yet, or that can't be checked.
    InReview,
    /// Started, or ready in a claimed thread.
    InProgress,
    /// Ready to start.
    Ready,
    /// Waiting on something: a dependency, a cycle, a conflict, or a resource another thread holds.
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
    pub in_review: usize,
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
            Stage::InReview => &mut self.in_review,
            Stage::InProgress => &mut self.in_progress,
            Stage::Ready => &mut self.ready,
            Stage::Blocked => &mut self.blocked,
            Stage::Parked => &mut self.parked,
            Stage::Unthreaded => &mut self.unthreaded,
        };
        *slot += count;
    }

    /// The leaves counted, every stage together.
    #[must_use]
    pub fn total(&self) -> usize {
        self.done
            + self.in_review
            + self.in_progress
            + self.ready
            + self.blocked
            + self.parked
            + self.unthreaded
    }
}

/// A parent's progress as it reads now, with its leaves counted by stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rollup {
    /// Its state, and its leaves done and in all, as the model counts them.
    pub progress: Progress,
    /// The same leaves by stage: the done and in-review ones together are the model's done.
    pub stages: Stages,
}

/// Readiness over one state, with ancestry checks cached by repository and commit.
pub struct Readiness<'a> {
    state: &'a State,
    model: &'a Model,
    now: Millis,
    ancestry: &'a dyn Ancestry,
    merged: RefCell<BTreeMap<(String, String), Result<bool, String>>>,
    cyclic: BTreeSet<String>,
    /// Blocks links' targets by the item they start from.
    links_from: BTreeMap<String, Vec<String>>,
    archived: RefCell<BTreeMap<String, bool>>,
    /// Each thread's members, built on first use.
    members: OnceCell<BTreeMap<&'a str, Vec<&'a str>>>,
    /// The resources each thread's claim holds, built on first use.
    resources: OnceCell<BTreeMap<&'a str, BTreeSet<&'a str>>>,
    /// Each thread's winning claim, memoized.
    winners: RefCell<BTreeMap<String, Option<claim::Claim>>>,
    /// Whether another thread holds a resource each thread needs, memoized.
    busy: RefCell<BTreeMap<String, Option<claim::Held>>>,
    /// Each item's stage, memoized, since a rollup counts every leaf under every parent.
    stages: RefCell<BTreeMap<String, Option<Stage>>>,
}

impl<'a> Readiness<'a> {
    /// Readiness at `now`.
    #[must_use]
    pub fn new(
        state: &'a State,
        model: &'a Model,
        now: Millis,
        ancestry: &'a dyn Ancestry,
    ) -> Self {
        let cyclic =
            model.link_cycles().into_iter().chain(model.parent_cycles()).flatten().collect();
        let mut links_from: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for link in model.links.iter().filter(|link| link.kind == LinkKind::Blocks) {
            links_from.entry(link.from.clone()).or_default().push(link.to.clone());
        }
        Self {
            state,
            model,
            now,
            ancestry,
            merged: RefCell::default(),
            cyclic,
            links_from,
            archived: RefCell::default(),
            members: OnceCell::new(),
            resources: OnceCell::new(),
            winners: RefCell::default(),
            busy: RefCell::default(),
            stages: RefCell::default(),
        }
    }

    /// An item's readiness, or `None` for an unknown item.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn of(&self, id: &str) -> Result<Option<Status>, Error> {
        let Some(item) = self.model.items.get(id) else { return Ok(None) };
        if let Some(status) = self.unclaimed(id, item) {
            return Ok(Some(status));
        }
        let alive = |holder: &str| self.model.run_alive(holder);
        let mut holders: Vec<String> = Vec::new();
        for claim in claim::covering(self.state, self.model, id, self.now, &alive)? {
            if !holders.contains(&claim.holder) {
                holders.push(claim.holder);
            }
        }
        if holders.is_empty()
            && let Some(thread) = item.thread.one()
            && let Some((resource, thread, holder)) = self.thread_busy(thread)?
        {
            return Ok(Some(Status::ResourceBusy { resource, thread, holder }));
        }
        Ok(Some(match holders.len() {
            0 => Status::Ready,
            1 => Status::Claimed { holder: holders.remove(0) },
            _ => Status::Contested { holders },
        }))
    }

    /// An item's readiness as though no thread were claimed, or `None` when it would be ready.
    fn unclaimed(&self, id: &str, item: &Item) -> Option<Status> {
        if self.is_archived(id) {
            return Some(Status::Archived);
        }
        let fields: Vec<String> =
            [("state", item.state.is_conflict()), ("thread", item.thread.is_conflict())]
                .into_iter()
                .filter(|(_, conflicted)| *conflicted)
                .map(|(name, _)| name.to_owned())
                .collect();
        if !fields.is_empty() {
            return Some(Status::Conflicted { fields });
        }
        if item.is_done() {
            return Some(match self.own_wait(item) {
                None => Status::Done,
                Some(Wait::InReview | Wait::Open) => Status::InReview,
                Some(Wait::Unverified { repository, reason }) => {
                    Status::Unverified { repository, reason }
                },
            });
        }
        if !self.model.children(id).is_empty() {
            return Some(Status::Parent);
        }
        if self.cyclic.contains(id) {
            return Some(Status::InCycle);
        }
        if item.state == Value::One(ItemState::Parked) {
            return Some(Status::Parked);
        }
        let waiting = self.waiting(id);
        if !waiting.is_empty() {
            return Some(Status::Blocked { waiting });
        }
        let threaded =
            item.thread.one().is_some_and(|thread| self.model.threads.contains_key(thread));
        (!threaded).then_some(Status::Unthreaded)
    }

    /// A thread's readiness.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn thread(&self, thread: &str) -> Result<ThreadStatus, Error> {
        if !self.model.threads.contains_key(thread) {
            return Ok(ThreadStatus::Missing);
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
            // Finished work is skipped before its merge is checked.
            .filter(|id| !self.model.items[*id].is_done())
            .filter(|id| self.unclaimed(id, &self.model.items[*id]).is_none())
            .map(str::to_owned)
            .collect();
        let alive = |holder: &str| self.model.run_alive(holder);
        let key = claim::thread_key(thread)?;
        let mut holders: Vec<String> = Vec::new();
        for claim in claim::live_with(self.state, &key, self.now, &alive)? {
            if !holders.contains(&claim.holder) {
                holders.push(claim.holder);
            }
        }
        if holders.len() > 1 {
            return Ok(ThreadStatus::Contested { holders, items });
        }
        if let Some(holder) = holders.pop() {
            return Ok(ThreadStatus::Claimed { holder, items });
        }
        if let Some((resource, thread, holder)) = self.thread_busy(thread)? {
            return Ok(ThreadStatus::ResourceBusy { resource, thread, holder });
        }
        Ok(if items.is_empty() {
            ThreadStatus::NothingReady
        } else {
            ThreadStatus::Ready { items }
        })
    }

    /// The threads with work `actor` may take, each with its ready members: its own claimed thread first, then the
    /// unclaimed ones, approved first, then by title in natural order and id. Threads others hold are left out.
    /// Archived subtrees are left out unless `include_archived` is set, which judges every item as if nothing were
    /// archived.
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
            let unarchived = Readiness {
                state: self.state,
                model: &model,
                now: self.now,
                ancestry: self.ancestry,
                merged: RefCell::new(self.merged.borrow().clone()),
                cyclic: self.cyclic.clone(),
                links_from: self.links_from.clone(),
                archived: RefCell::default(),
                members: OnceCell::new(),
                resources: OnceCell::new(),
                winners: RefCell::new(self.winners.borrow().clone()),
                busy: RefCell::default(),
                stages: RefCell::default(),
            };
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
            let own = !matches!(status, ThreadStatus::Ready { .. });
            let mut items: Vec<(String, &String)> = status
                .items()
                .iter()
                .map(|item| (title(item, &self.model.items[item].title), item))
                .collect();
            items.sort_by(|a, b| natural::cmp(&a.0, &b.0).then_with(|| a.1.cmp(b.1)));
            let items = items.into_iter().map(|(_, item)| item.clone()).collect();
            let key = (!own, !thread.is_approved(), title(id, &thread.title));
            ready.push((key, ReadyThread { thread: id.clone(), items }));
        }
        ready.sort_by(|(a, x), (b, y)| {
            (a.0, a.1)
                .cmp(&(b.0, b.1))
                .then_with(|| natural::cmp(&a.2, &b.2))
                .then_with(|| x.thread.cmp(&y.thread))
        });
        Ok(ready.into_iter().map(|(_, thread)| thread).collect())
    }

    /// A leaf's stage, or `None` for a parent, an archived item or an unknown one. A leaf started by hand reads in
    /// progress whatever else holds it, such as a link added since; otherwise its readiness decides.
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
            Some(if self.own_wait(item).is_none() { Stage::Done } else { Stage::InReview })
        } else {
            let started = item.state == Value::One(ItemState::InProgress);
            self.of(id)?.and_then(|status| match status {
                Status::Archived | Status::Parent => None,
                Status::Done => Some(Stage::Done),
                Status::InReview | Status::Unverified { .. } => Some(Stage::InReview),
                Status::Parked => Some(Stage::Parked),
                _ if started => Some(Stage::InProgress),
                Status::Claimed { .. } | Status::Contested { .. } => Some(Stage::InProgress),
                Status::Ready => Some(Stage::Ready),
                Status::Unthreaded => Some(Stage::Unthreaded),
                Status::Blocked { .. }
                | Status::ResourceBusy { .. }
                | Status::InCycle
                | Status::Conflicted { .. } => Some(Stage::Blocked),
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
        let parked = progress.state == ItemState::Parked;
        self.tally(id, parked, &mut BTreeSet::from([id]), &mut stages)?;
        if progress.state == ItemState::InProgress || progress.state == ItemState::Backlog {
            let moving = stages.in_progress > 0
                || progress.done > 0
                || self.claimed_under(id, &mut BTreeSet::from([id]))?;
            progress.state = if moving { ItemState::InProgress } else { ItemState::Backlog };
        }
        Ok(Some(Rollup { progress, stages }))
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
                let open = !matches!(stage, Stage::Done | Stage::InReview);
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

    /// The unmet hard dependencies of an item and its ancestors.
    /// What keeps a parent from being finished: its children that are neither done nor archived, and what would hold
    /// it back as a leaf (a cycle, being parked, or unmet dependencies).
    #[must_use]
    pub fn unfinished(&self, id: &str) -> (Vec<String>, Option<Status>) {
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
            Some(Status::InCycle)
        } else if self.model.items.get(id).is_some_and(|i| i.state == Value::One(ItemState::Parked))
        {
            Some(Status::Parked)
        } else {
            let waiting = self.waiting(id);
            (!waiting.is_empty()).then_some(Status::Blocked { waiting })
        };
        (open, held)
    }

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
        let alive = |holder: &str| self.model.run_alive(holder);
        let winner = claim::winner_with(self.state, &claim::thread_key(thread)?, self.now, &alive)?;
        self.winners.borrow_mut().insert(thread.to_owned(), winner.clone());
        Ok(winner)
    }

    /// A resource the thread's open members name that another thread's winning claim holds, with that thread and its
    /// holder, memoized. Any member's resource counts, so a thread and each of its items agree.
    fn thread_busy(&self, thread: &str) -> Result<Option<claim::Held>, Error> {
        if let Some(busy) = self.busy.borrow().get(thread) {
            return Ok(busy.clone());
        }
        let held = self.resources.get_or_init(|| claim::resources(self.model));
        let busy = claim::resource_held(held, thread, None, |other| self.winner(other))?;
        self.busy.borrow_mut().insert(thread.to_owned(), busy.clone());
        Ok(busy)
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
        let same_thread = self
            .model
            .thread_of(dependent)
            .is_some_and(|thread| self.model.thread_of(dependency) == Some(thread));
        if same_thread { None } else { self.own_wait(item) }
    }

    /// Why the work under a parent doesn't yet satisfy `dependent`, or `None` when all of it does: each child judged
    /// as a dependency of its own, so a leaf by its state, a parent under it done or parked by hand by that, and any
    /// other by what is under it in turn. The first wait wins, one still open before one in review. A leaf waiting on
    /// its own ancestor is among what it waits on, so only closing the ancestor by hand meets it.
    fn leaves_wait(&self, dependent: &str, parent: &str) -> Option<Wait> {
        // A parent in a cycle meets nothing until the cycle is fixed: a cycle of parents has no bottom to reach.
        if self.cyclic.contains(parent) {
            return Some(Wait::Open);
        }
        let mut found = None;
        for child in self.model.children(parent) {
            match self.dependency_wait(dependent, child) {
                Some(Wait::Open) => return Some(Wait::Open),
                Some(wait) => found = found.or(Some(wait)),
                None => {},
            }
        }
        found
    }

    /// Whether a done item's own commits are merged: `None` when every one is, or it has none.
    fn own_wait(&self, item: &Item) -> Option<Wait> {
        for evidence in item.evidence.all() {
            let Evidence { repository: Some(repository), commit: Some(commit), .. } = evidence
            else {
                continue;
            };
            let Some(record) = self.model.repositories.get(repository) else {
                return Some(Wait::Unverified {
                    repository: repository.clone(),
                    reason: "no such repository record".to_owned(),
                });
            };
            let key = (repository.clone(), commit.clone());
            let merged = self
                .merged
                .borrow_mut()
                .entry(key)
                .or_insert_with(|| self.ancestry.merged(record, commit))
                .clone();
            match merged {
                Ok(true) => {},
                Ok(false) => return Some(Wait::InReview),
                Err(reason) => {
                    return Some(Wait::Unverified { repository: repository.clone(), reason });
                },
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
