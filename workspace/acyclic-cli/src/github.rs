//! Asking GitHub, through `gh`, whether a commit is on the default branch of a repository this machine has no clone
//! of. It happens where commit evidence is checked: when a write is prepared, before it takes the tracker's lock, and
//! in `doctor`; never in a read. Any failure leaves the commit unchecked, as it was without the question.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use acyclic_core::merged::{Check, github_check};
use acyclic_core::model::Repository;
use serde::{Deserialize, Serialize};

use crate::Env;

/// How long one `gh` call may take before it is killed.
const TIMEOUT: Duration = Duration::from_secs(15);

type Runner = Box<dyn Fn(&[&str]) -> Result<String, String>>;

/// `gh` with the arguments given, as `env` would run it: found on its `PATH`, never reading standard input or
/// prompting, and killed when it takes too long. With no `PATH` in `env` there is no `gh` to run.
pub(crate) fn runner(env: &Env) -> Runner {
    let vars = env.vars.clone();
    #[cfg(debug_assertions)]
    let timeout = env
        .var("ACYCLIC_GH_TIMEOUT_MS")
        .and_then(|ms| ms.parse().ok())
        .map_or(TIMEOUT, Duration::from_millis);
    #[cfg(not(debug_assertions))]
    let timeout = TIMEOUT;
    Box::new(move |args| {
        let program = vars
            .get("PATH")
            .and_then(|path| {
                std::env::split_paths(path).map(|dir| dir.join("gh")).find(|gh| gh.is_file())
            })
            .ok_or("gh isn't installed: there is none on PATH")?;
        run(&program, args, &vars, timeout)
    })
}

fn run(
    program: &Path,
    args: &[&str],
    vars: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(vars)
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gh didn't start: {e}"))?;
    // Read both streams as they fill, so a chatty `gh` never blocks on a full pipe. The readers are left to end on
    // their own: a killed `gh` may leave a descendant holding the pipes open.
    let drain = |mut stream: Option<Box<dyn std::io::Read + Send>>| {
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(stream) = stream.as_mut() {
                let mut bytes = Vec::new();
                let _ = stream.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            let _ = sent.send(text);
        });
        received
    };
    let stdout = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let stderr = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "gh took longer than {} seconds and was stopped",
                    timeout.as_secs_f32()
                ));
            },
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("gh couldn't be waited for: {e}")),
        }
    };
    let grace = Duration::from_secs(2);
    let out = stdout.recv_timeout(grace).unwrap_or_default();
    let err = stderr.recv_timeout(grace).unwrap_or_default();
    if status.success() {
        return Ok(out);
    }
    let said = format!("{err}{out}").trim().to_owned();
    Err(if said.is_empty() { format!("gh exited with {status}") } else { said })
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Merged {
    /// `owner/name@branch@commit`, each known to be on the branch: a merge is never undone.
    merged: BTreeSet<String>,
}

/// Verdicts from GitHub for one command, asked once each and shared by every check of it.
pub(crate) struct Github {
    run: Runner,
    cache: Option<PathBuf>,
    merged: RefCell<Merged>,
    memo: RefCell<BTreeMap<String, Check>>,
    /// Why GitHub couldn't be reached, once it couldn't: the rest of the command asks no more.
    down: RefCell<Option<String>>,
    asked: Cell<usize>,
}

impl Github {
    pub(crate) fn new(env: &Env) -> Self {
        Self::with(runner(env), crate::learned::state_file(env, "merged.json"))
    }

    fn with(run: Runner, cache: Option<PathBuf>) -> Self {
        let merged = cache.as_deref().map(crate::learned::read::<Merged>).unwrap_or_default();
        Self {
            run,
            cache,
            merged: RefCell::new(merged),
            memo: RefCell::default(),
            down: RefCell::new(None),
            asked: Cell::new(0),
        }
    }

    /// Whether `commit` is on `repository`'s default branch, by asking GitHub unless this machine already learned it
    /// is, or has asked this command, or GitHub proved unreachable.
    pub(crate) fn check(&self, repository: &Repository, commit: &str) -> Check {
        let name =
            crate::resolve::repository_name(repository).unwrap_or_default().to_ascii_lowercase();
        let branch = repository.default_branch.one().cloned().unwrap_or_default();
        let key = format!("{name}@{branch}@{}", commit.to_ascii_lowercase());
        if let Some(known) = self.memo.borrow().get(&key) {
            return known.clone();
        }
        let check = if self.merged.borrow().merged.contains(&key) {
            Check::Merged
        } else if let Some(reason) = &*self.down.borrow() {
            Check::Unchecked(reason.clone())
        } else {
            self.asked.set(self.asked.get() + 1);
            let asked = github_check(&*self.run, repository, commit);
            if asked.transport_failed
                && let Check::Unchecked(reason) = &asked.check
            {
                *self.down.borrow_mut() =
                    Some(format!("{reason} (not asked again by this command)"));
            }
            if asked.check == Check::Merged {
                self.merged.borrow_mut().merged.insert(key.clone());
                if let Some(path) = &self.cache {
                    let _ = crate::learned::write(path, &*self.merged.borrow());
                }
            }
            asked.check
        };
        self.memo.borrow_mut().insert(key, check.clone());
        check
    }
}

#[cfg(test)]
mod tests;
