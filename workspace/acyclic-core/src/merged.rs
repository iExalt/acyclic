//! Whether a commit is on its repository's default branch, checked in local code clones when commit evidence is
//! written: reads never run git, so a machine with no clones reads the tracker as one with them.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::Repository;
use crate::store::Git;

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

type BranchCache = BTreeMap<(PathBuf, String), Result<Option<Branch>, String>>;

/// Checks ancestry in local code clones, as they were last fetched. Each clone's default branch is looked up once,
/// with its latest commits, so a batch with many commits walks each branch once. Make one for each batch: it doesn't
/// see a branch move after its first look, except through [`refresh`](GitAncestry::refresh).
pub struct GitAncestry<F> {
    clone_of: F,
    branches: RefCell<BranchCache>,
}

/// A clone's default branch: the ref it is read from, and its latest commits, sorted.
struct Branch {
    target: String,
    commits: Vec<String>,
    git: Git,
}

/// The most of a branch's commits looked up at once; an older commit is walked to on its own.
const LATEST: usize = 100_000;

/// The reason a repository with no known clone can't be checked, naming how to give it one.
fn no_clone(repository: &Repository) -> String {
    let name = match (repository.owner.one(), repository.name.one()) {
        (Some(owner), Some(name)) => format!("{owner}/{name}"),
        _ => "the repository".to_owned(),
    };
    format!(
        "no clone of {name} is known on this machine: run any acyclic command inside a clone of it whose origin is on \
         github.com, or map it in ~/.config/acyclic/clones.toml"
    )
}

impl<F: Fn(&Repository) -> Option<PathBuf>> GitAncestry<F> {
    /// Checks in the clone `clone_of` names for each repository record.
    pub fn new(clone_of: F) -> Self {
        Self { clone_of, branches: RefCell::default() }
    }
}

impl Branch {
    /// The ref `clone` reads `branch` from, and its latest commits.
    /// Returns `None` only when both exact refs are absent and no `origin` remote is configured.
    fn of(clone: &Path, branch: &str) -> Result<Option<Self>, String> {
        let git = Git::new(clone);
        let target = [format!("refs/remotes/origin/{branch}"), format!("refs/heads/{branch}")]
            .into_iter()
            .find_map(|candidate| git.resolve(&candidate).ok().flatten());
        let Some(target) = target else {
            let remote_ref = format!("refs/remotes/origin/{branch}");
            let local_ref = format!("refs/heads/{branch}");
            if git
                .is_originless_without_default_refs([&remote_ref, &local_ref])
                .map_err(|error| error.to_string())?
            {
                return Ok(None);
            }
            return Err(format!("the clone has no {branch} branch"));
        };
        // Without them, every commit is checked on its own, which is slower but no less right.
        let mut commits = git.recent_commits(&target, LATEST).unwrap_or_default();
        commits.sort_unstable();
        Ok(Some(Self { target, commits, git }))
    }

    /// The commit `commit` names, in full: an abbreviated id as git names it, so one that names no commit, or several,
    /// names none.
    fn full(&self, commit: &str) -> Option<String> {
        self.git.full_commit(commit)
    }
}

/// What checking a commit against its repository's default branch found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// On the default branch.
    Merged,
    /// A commit the clone knows, not on the default branch.
    NotMerged,
    /// No commit the clone knows: a typo, or one it hasn't fetched.
    UnknownCommit,
    /// The clone has no default branch to look in, as before its first fetch of it; the reason says so.
    NoBranch(String),
    /// Can't be checked on this machine, such as with no clone, no single default branch, an originless clone without
    /// default refs, or a Git failure.
    Unchecked(String),
}

impl<F: Fn(&Repository) -> Option<PathBuf>> GitAncestry<F> {
    /// Whether `commit` is on `repository`'s default branch, as last fetched in its clone.
    pub fn check(&self, repository: &Repository, commit: &str) -> Check {
        // Evidence is written by agents and synced, so it never reaches git as anything but a commit id.
        if !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Check::UnknownCommit;
        }
        let Some(clone) = (self.clone_of)(repository) else {
            return Check::Unchecked(no_clone(repository));
        };
        let Some(branch) = repository.default_branch.one() else {
            return Check::Unchecked("the repository has no single default branch".to_owned());
        };
        let mut branches = self.branches.borrow_mut();
        let found = match branches
            .entry((clone.clone(), branch.to_owned()))
            .or_insert_with(|| Branch::of(&clone, branch))
        {
            Ok(Some(found)) => found,
            Ok(None) => {
                return Check::Unchecked(format!(
                    "the clone has neither an origin remote nor refs for default branch {branch}"
                ));
            },
            Err(reason) => return Check::NoBranch(reason.clone()),
        };
        // An id the clone has no object for is unknown: a typo, or a commit it hasn't fetched.
        let Some(full) = found.full(commit) else { return Check::UnknownCommit };
        if found.commits.binary_search(&full).is_ok() {
            return Check::Merged;
        }
        // Not among the latest: not merged, or older than they go.
        match found.git.is_ancestor_checked(&full, &found.target) {
            Ok(true) => Check::Merged,
            Ok(false) => Check::NotMerged,
            Err(error) => Check::Unchecked(error.to_string()),
        }
    }

    /// Fetches `repository`'s default branch into its clone, so the next [`check`](Self::check) sees a merge made
    /// since the clone's last fetch.
    ///
    /// # Errors
    ///
    /// Fails, with git's reason, when the repository has no clone or single default branch, or the fetch fails.
    pub fn refresh(&self, repository: &Repository) -> Result<(), String> {
        let clone = (self.clone_of)(repository).ok_or_else(|| no_clone(repository))?;
        let branch =
            repository.default_branch.one().ok_or("the repository has no single default branch")?;
        // Only the remote-tracking ref `Branch::of` reads moves: the checkout and its branches are left alone.
        let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        let result = fetch(&clone, refspec, FETCH_TIMEOUT);
        self.branches.borrow_mut().remove(&(clone, branch.to_owned()));
        result
    }
}

/// How long a fetch may take before the write it serves gives up.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Fetches `refspec` from `origin` into `clone`, and gives up after `timeout`: the fetch runs on a thread of its own,
/// left to finish or fail on its own when it takes longer.
fn fetch(clone: &Path, refspec: String, timeout: std::time::Duration) -> Result<(), String> {
    let (sent, received) = std::sync::mpsc::channel();
    let clone = clone.to_owned();
    std::thread::spawn(move || {
        let _ = sent.send(Git::new(&clone).fetch("origin", &[refspec]).map_err(|e| e.to_string()));
    });
    received.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(format!("the fetch took longer than {} seconds", timeout.as_secs()))
    })
}

impl<F: Fn(&Repository) -> Option<PathBuf>> Ancestry for GitAncestry<F> {
    fn merged(&self, repository: &Repository, commit: &str) -> Result<bool, String> {
        match self.check(repository, commit) {
            Check::Merged => Ok(true),
            Check::NotMerged => Ok(false),
            Check::UnknownCommit => Err("no commit the clone knows".to_owned()),
            Check::NoBranch(reason) | Check::Unchecked(reason) => Err(reason),
        }
    }
}

#[cfg(test)]
mod tests;
