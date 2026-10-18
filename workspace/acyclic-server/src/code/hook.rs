//! The mirrors' pre-receive hook: what checks a run's push before git takes it.
//!
//! The endpoint runs `git receive-pack` with an environment of its own making, and git runs this hook with the pushed
//! objects in quarantine, before it moves any ref. A push may only update the run's own refs, `refs/runs/<run>/*`, to
//! commits. Every object it makes reachable must have come in the push itself: every run's objects share the mirror's
//! store, so a ref naming another run's commit, or a commit built on one, would otherwise hand that run's work over.
//! Then the new commits are scanned: for the deny-list's terms in a public repository, since private records name
//! private terms by design, and with Gitleaks in every repository. Anything else refuses the whole push.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use acyclic_core::private;
use acyclic_guard::{DenyList, scan_range};

/// The command the hook's script runs: `acyclic receive-hook`.
pub const COMMAND: &str = "receive-hook";
/// The run pushing.
pub const RUN: &str = "ACYCLIC_RUN";
/// `1` when the repository is public.
pub const PUBLIC: &str = "ACYCLIC_PUBLIC";
/// The deny-list file, readable by the user alone.
pub const DENY: &str = "ACYCLIC_DENY_FILE";

/// Runs the hook in this process, as git runs it under `git receive-pack`.
#[must_use]
pub fn main() -> ExitCode {
    let given = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(run), Some(public), Some(deny), Some(quarantine)) =
        (given(RUN), given(PUBLIC), given(DENY), given("GIT_QUARANTINE_PATH"))
    else {
        eprintln!("acyclic receive-hook runs only as the mirrors' hook, under git receive-pack");
        return ExitCode::from(2);
    };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("the hook couldn't read the push's updates");
        return ExitCode::FAILURE;
    }
    match check(Path::new("."), &input, &run, public == "1", Path::new(&deny), &quarantine) {
        Ok(()) => ExitCode::SUCCESS,
        Err(refusal) => {
            eprintln!("{refusal}");
            ExitCode::FAILURE
        },
    }
}

/// Checks a push to the mirror at `repo` whose updates git gave as `input`, from run `run`.
fn check(
    repo: &Path,
    input: &str,
    run: &str,
    public: bool,
    deny: &Path,
    quarantine: &str,
) -> Result<(), String> {
    let own = format!("refs/runs/{run}/");
    let zero = |id: &str| id.bytes().all(|byte| byte == b'0');
    let mut news = Vec::new();
    let mut names = Vec::new();
    for line in input.lines().filter(|line| !line.is_empty()) {
        let [_, new, reference] = line.split(' ').collect::<Vec<_>>()[..] else {
            return Err("the hook couldn't read the push's updates".to_owned());
        };
        if !allowed(&own, reference) {
            return Err(format!("this run pushes to {own}* alone, not {reference}"));
        }
        names.push(reference);
        if zero(new) {
            continue;
        }
        if git(repo, &["cat-file", "-t", new])?.trim() != "commit" {
            return Err(format!("{reference} must name a commit"));
        }
        news.push(new.to_owned());
    }
    // Loaded before anything else is read, so it fails closed, and nothing refused echoes its terms.
    let list = private::check(deny, false)
        .map_err(|err| err.to_string())
        .and_then(|_| DenyList::load(deny).map_err(|err| err.to_string()))
        .map_err(|err| format!("the push couldn't be scanned: {err}"))?;
    if news.is_empty() {
        return Ok(());
    }
    // What the run could see before the push: the branches, the tags and its own refs, none of another run's.
    let mut range = news.clone();
    range.extend(
        ["--not", "--exclude=refs/runs/*", "--all", &format!("--glob={own}*")].map(str::to_owned),
    );
    let mut listing = vec!["rev-list", "--objects"];
    listing.extend(range.iter().map(String::as_str));
    let listed = git(repo, &listing)?;
    let objects: Vec<&str> = listed.lines().filter_map(|line| line.split(' ').next()).collect();
    let found = quarantined(repo, &objects, quarantine)?;
    if found.lines().any(|line| line.ends_with(" missing")) {
        return Err("the push references objects this run can't see".to_owned());
    }
    let list = if public { list } else { DenyList::parse("") };
    if names.iter().any(|name| !list.matches(name).is_empty()) {
        return Err("a ref's name holds a term the deny-list forbids".to_owned());
    }
    let report = scan_range(repo, &range, &list)
        .map_err(|err| format!("the push couldn't be scanned: {err}"))?;
    if let Some(finding) = report.findings.first() {
        return Err(format!("the push holds a term the deny-list forbids: {finding}"));
    }
    if report.leaks {
        return Err("Gitleaks found a secret in the push".to_owned());
    }
    Ok(())
}

/// Whether a run whose refs start with `own` may update `reference`.
fn allowed(own: &str, reference: &str) -> bool {
    reference.strip_prefix(own).is_some_and(|rest| !rest.is_empty())
}

/// What `git cat-file --batch-check` says of `objects` in the quarantine alone, the push's own objects.
fn quarantined(repo: &Path, objects: &[&str], quarantine: &str) -> Result<String, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch-check"])
        .env("GIT_OBJECT_DIRECTORY", quarantine)
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("git can't run: {err}"))?;
    let mut stdin = child.stdin.take().ok_or("git took no input")?;
    let text = objects.iter().flat_map(|object| [*object, "\n"]).collect::<String>();
    let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let output = child.wait_with_output().map_err(|err| format!("git can't run: {err}"))?;
    let _ = writer.join();
    if !output.status.success() {
        return Err("git cat-file failed".to_owned());
    }
    String::from_utf8(output.stdout).map_err(|_| "git printed something not UTF-8".to_owned())
}

/// Runs git in the mirror, as the hook's environment points it, quarantine included.
fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|err| format!("git can't run: {err}"))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.first().unwrap_or(&"")));
    }
    String::from_utf8(output.stdout).map_err(|_| "git printed something not UTF-8".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_updates_its_own_refs_alone() {
        let own = "refs/runs/r1/";
        assert!(allowed(own, "refs/runs/r1/main"));
        assert!(allowed(own, "refs/runs/r1/a/b"));
        for refused in [
            "refs/runs/r1/",
            "refs/runs/r1",
            "refs/runs/r12/main",
            "refs/runs/r2/main",
            "refs/heads/main",
            "refs/tags/v1",
        ] {
            assert!(!allowed(own, refused), "{refused}");
        }
    }

    #[test]
    fn the_hook_refuses_to_run_outside_a_push() {
        // The test process has none of the hook's variables.
        assert_eq!(main(), ExitCode::from(2));
    }
}
