//! Readiness, computed on read under plan D18: done inside a thread, merged across. A dependency in the same thread
//! is met when it is done; one in another thread, or for an item in no thread, is met once its evidence commit is on
//! its repository's default branch, and is "in review" until then. Work with no repository is met when done, and an
//! archived dependency always is.
//!
//! Only leaves are claimed or ready. A dependency on a parent is met by the parent's own state, and a parent's
//! dependencies hold back every item under it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::claim;
use crate::model::{Evidence, Item, ItemState, LinkKind, Model, Repository, Value};
use crate::state::{Error, Millis, State};

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
            Command::new("git")
                .arg("-C")
                .arg(&clone)
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
    /// Claimed by one holder.
    Claimed {
        /// The winning holder.
        holder: String,
    },
    /// Claimed by several holders that didn't see each other; the first is the winner.
    Contested {
        /// The holders, winner first.
        holders: Vec<String>,
    },
    /// Unclaimed, but another item's winning claim holds one of its named resources.
    ResourceBusy {
        /// The resource.
        resource: String,
        /// The item holding it.
        item: String,
        /// That item's holder.
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
        }
    }

    /// An item's readiness, or `None` for an unknown item.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn of(&self, id: &str) -> Result<Option<Status>, Error> {
        let Some(item) = self.model.items.get(id) else { return Ok(None) };
        if self.is_archived(id) {
            return Ok(Some(Status::Archived));
        }
        let fields: Vec<String> =
            [("state", item.state.is_conflict()), ("thread", item.thread.is_conflict())]
                .into_iter()
                .filter(|(_, conflicted)| *conflicted)
                .map(|(name, _)| name.to_owned())
                .collect();
        if !fields.is_empty() {
            return Ok(Some(Status::Conflicted { fields }));
        }
        if item.is_done() {
            return Ok(Some(match self.own_wait(item) {
                None => Status::Done,
                Some(Wait::InReview | Wait::Open) => Status::InReview,
                Some(Wait::Unverified { repository, reason }) => {
                    Status::Unverified { repository, reason }
                },
            }));
        }
        if !self.model.children(id).is_empty() {
            return Ok(Some(Status::Parent));
        }
        if self.cyclic.contains(id) {
            return Ok(Some(Status::InCycle));
        }
        if item.state == Value::One(ItemState::Parked) {
            return Ok(Some(Status::Parked));
        }
        let waiting = self.waiting(id);
        if !waiting.is_empty() {
            return Ok(Some(Status::Blocked { waiting }));
        }
        let key = crate::model::item_key(id).map_err(|_| Error::InvalidName(id.to_owned()))?;
        let alive = |holder: &str| self.model.run_alive(holder);
        let claims = claim::live_with(self.state, &key, self.now, &alive)?;
        let mut holders: Vec<String> = Vec::new();
        for claim in claims {
            if !holders.contains(&claim.holder) {
                holders.push(claim.holder);
            }
        }
        if holders.is_empty()
            && let Some(busy) = self.resource_busy(item)?
        {
            return Ok(Some(busy));
        }
        Ok(Some(match holders.len() {
            0 => Status::Ready,
            1 => Status::Claimed { holder: holders.remove(0) },
            _ => Status::Contested { holders },
        }))
    }

    /// The ready, unclaimed leaves, by id. Archived subtrees are left out unless
    /// `include_archived` is set, which judges every item as if nothing were archived.
    ///
    /// # Errors
    ///
    /// Fails when a claim can't be read.
    pub fn ready(&self, include_archived: bool) -> Result<Vec<String>, Error> {
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
            };
            return unarchived.ready(false);
        }
        let mut ready = Vec::new();
        for (id, item) in &self.model.items {
            if item.is_done() || !self.model.children(id).is_empty() {
                continue;
            }
            if let Some(Status::Ready) = self.of(id)? {
                ready.push(id.clone());
            }
        }
        Ok(ready)
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

    /// The first named resource of an unclaimed item that another item's winning claim holds.
    fn resource_busy(&self, item: &Item) -> Result<Option<Status>, Error> {
        let alive = |holder: &str| self.model.run_alive(holder);
        for resource in &item.resources {
            for other in self.model.items.values() {
                if other.id == item.id || !other.resources.contains(resource) {
                    continue;
                }
                let key = crate::model::item_key(&other.id)
                    .map_err(|_| Error::InvalidName(other.id.clone()))?;
                if let Some(claim) = claim::winner_with(self.state, &key, self.now, &alive)? {
                    return Ok(Some(Status::ResourceBusy {
                        resource: resource.clone(),
                        item: other.id.clone(),
                        holder: claim.holder,
                    }));
                }
            }
        }
        Ok(None)
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
            return Some(Wait::Open);
        }
        let same_thread = self
            .model
            .thread_of(dependent)
            .is_some_and(|thread| self.model.thread_of(dependency) == Some(thread));
        if same_thread { None } else { self.own_wait(item) }
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
