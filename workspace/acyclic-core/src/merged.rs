//! Whether a commit is on its repository's default branch, checked in local code clones when commit evidence is
//! written: reads never run git, so a machine with no clones reads the tracker as one with them.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::model::Repository;

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

/// Checks ancestry in local code clones, as they were last fetched. Each clone's default branch is looked up once,
/// with its latest commits, and abbreviated ids are named in full by one git process kept open, so a read with many
/// done dependencies starts a few git processes rather than two for each. Make one for each read: it doesn't see a
/// branch move after its first look.
pub struct GitAncestry<F> {
    clone_of: F,
    branches: RefCell<BTreeMap<(PathBuf, String), Result<Branch, String>>>,
}

/// A clone's default branch: the ref it is read from, and its latest commits, sorted.
struct Branch {
    target: String,
    commits: Vec<String>,
    names: Option<Names>,
}

/// The most of a branch's commits looked up at once; an older commit takes a git process of its own.
const LATEST: usize = 100_000;

impl<F: Fn(&Repository) -> Option<PathBuf>> GitAncestry<F> {
    /// Checks in the clone `clone_of` names for each repository record.
    pub fn new(clone_of: F) -> Self {
        Self { clone_of, branches: RefCell::default() }
    }
}

/// Git in `clone`, in the C locale.
fn git_in(clone: &Path) -> std::process::Command {
    let mut git = crate::store::git_at(clone);
    git.env("LC_ALL", "C");
    git
}

/// Git's output in `clone`.
fn git_output(clone: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    git_in(clone).args(args).output().map_err(|e| format!("git can't run: {e}"))
}

impl Branch {
    /// The ref `clone` reads `branch` from, and its latest commits.
    fn of(clone: &Path, branch: &str) -> Result<Self, String> {
        let target = [format!("refs/remotes/origin/{branch}"), format!("refs/heads/{branch}")]
            .into_iter()
            .find(|candidate| {
                git_output(
                    clone,
                    &["rev-parse", "--verify", "--quiet", &format!("{candidate}^{{commit}}")],
                )
                .is_ok_and(|out| out.status.success())
            })
            .ok_or_else(|| format!("the clone has no {branch} branch"))?;
        // Without them, every commit is checked on its own, which is slower but no less right.
        let mut commits =
            git_output(clone, &["rev-list", &format!("--max-count={LATEST}"), &target])
                .ok()
                .filter(|out| out.status.success())
                .map(|out| {
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
        commits.sort_unstable();
        Ok(Self { target, commits, names: None })
    }

    /// Whether `commit` is among the latest. An abbreviated id is named in full first, as git names it: one that
    /// names no commit, or several, isn't.
    fn has(&mut self, clone: &Path, commit: &str) -> bool {
        let full = if matches!(commit.len(), 40 | 64) {
            Some(commit.to_ascii_lowercase())
        } else {
            self.names.get_or_insert_with(|| Names::start(clone)).full(commit)
        };
        full.is_some_and(|full| self.commits.binary_search(&full).is_ok())
    }
}

/// A `git cat-file` that names abbreviated commit ids in full, one a line, started on the first.
struct Names(
    Option<(std::process::Child, std::process::ChildStdin, BufReader<std::process::ChildStdout>)>,
);

impl Names {
    fn start(clone: &Path) -> Self {
        let started = git_in(clone)
            .args(["cat-file", "--batch-check"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        Self(started.ok().and_then(|mut child| {
            let (stdin, stdout) = (child.stdin.take()?, child.stdout.take()?);
            Some((child, stdin, BufReader::new(stdout)))
        }))
    }

    /// The commit `id` names, in full, or `None` when it names none, several objects, another kind of object, or git
    /// failed.
    fn full(&mut self, id: &str) -> Option<String> {
        let (_, stdin, stdout) = self.0.as_mut()?;
        // Bare, as merge-base takes it: an id that also starts another kind of object is ambiguous to both.
        writeln!(stdin, "{id}").and_then(|()| stdin.flush()).ok()?;
        let mut line = String::new();
        stdout.read_line(&mut line).ok()?;
        // `<id> commit <size>` when found, else the id asked for and why not, such as `missing` or `ambiguous`.
        match line.split_whitespace().collect::<Vec<_>>()[..] {
            [full, "commit", _] => Some(full.to_owned()),
            _ => None,
        }
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        if let Some((mut child, stdin, _)) = self.0.take() {
            // Its input closed, it exits.
            drop(stdin);
            let _ = child.wait();
        }
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
    /// Can't be checked on this machine, and why: no clone mapped, no single default branch, or git failing.
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
            return Check::Unchecked("no clone of the repository is mapped".to_owned());
        };
        let Some(branch) = repository.default_branch.one() else {
            return Check::Unchecked("the repository has no single default branch".to_owned());
        };
        let mut branches = self.branches.borrow_mut();
        let found = match branches
            .entry((clone.clone(), branch.to_owned()))
            .or_insert_with(|| Branch::of(&clone, branch))
        {
            Ok(found) => found,
            Err(reason) => return Check::Unchecked(reason.clone()),
        };
        if found.has(&clone, commit) {
            return Check::Merged;
        }
        // Not among the latest: not merged, older than they go, or no commit git knows, which git tells apart.
        let output = match git_output(
            &clone,
            &["merge-base", "--is-ancestor", "--end-of-options", commit, &found.target],
        ) {
            Ok(output) => output,
            Err(reason) => return Check::Unchecked(reason),
        };
        match output.status.code() {
            Some(0) => Check::Merged,
            Some(1) => Check::NotMerged,
            _ if !knows(&clone, commit) => Check::UnknownCommit,
            _ => Check::Unchecked(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        }
    }

    /// Fetches `repository`'s default branch into its clone, so the next [`check`](Self::check) sees a merge made
    /// since the clone's last fetch.
    ///
    /// # Errors
    ///
    /// Fails, with git's reason, when the repository has no clone or single default branch, or the fetch fails.
    pub fn refresh(&self, repository: &Repository) -> Result<(), String> {
        let clone = (self.clone_of)(repository).ok_or("no clone of the repository is mapped")?;
        let branch =
            repository.default_branch.one().ok_or("the repository has no single default branch")?;
        // Only the remote-tracking ref `Branch::of` reads moves: the checkout and its branches are left alone.
        let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        let result = fetch(&clone, &refspec, FETCH_TIMEOUT);
        self.branches.borrow_mut().remove(&(clone, branch.to_owned()));
        result
    }
}

/// How long a fetch may take before the write it serves gives up.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Fetches `refspec` from `origin` into `clone`, never asking for credentials, and gives up after `timeout`.
fn fetch(clone: &Path, refspec: &str, timeout: std::time::Duration) -> Result<(), String> {
    let mut git = git_in(clone);
    git.args(["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "origin", refspec])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        git.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = git.spawn().map_err(|e| format!("git can't run: {e}"))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut pipe, &mut stderr);
                }
                return if status.success() { Ok(()) } else { Err(stderr.trim().to_owned()) };
            },
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the fetch took longer than {} seconds", timeout.as_secs()));
            },
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(error) => return Err(format!("git can't run: {error}")),
        }
    }
}

/// Whether `clone` has the commit `commit` names.
fn knows(clone: &Path, commit: &str) -> bool {
    git_output(clone, &["cat-file", "-e", "--end-of-options", &format!("{commit}^{{commit}}")])
        .is_ok_and(|out| out.status.success())
}

impl<F: Fn(&Repository) -> Option<PathBuf>> Ancestry for GitAncestry<F> {
    fn merged(&self, repository: &Repository, commit: &str) -> Result<bool, String> {
        match self.check(repository, commit) {
            Check::Merged => Ok(true),
            Check::NotMerged => Ok(false),
            Check::UnknownCommit => Err("no commit the clone knows".to_owned()),
            Check::Unchecked(reason) => Err(reason),
        }
    }
}

#[cfg(test)]
mod tests;
