//! The inbox's pre-receive hook: what takes a run's pushed operations to the tracker.
//!
//! The endpoint runs `git receive-pack` on the run's inbox with an environment of its own making, and git runs this
//! hook with the pushed objects in quarantine before it moves any ref. The hook checks the whole push first: one update,
//! to the run's own ref, a chain of commits that each add the next operation and nothing else, every file a readable
//! operation, and nothing the deny-list or Gitleaks objects to. Anything else refuses the push before a single
//! operation is forwarded. Then it forwards them, as [`crate::forward`] does, and reports each one's outcome and a
//! summary on lines of their own; it accepts the push only if every operation was spent.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use acyclic_cli::{Applied, Env};
use acyclic_core::private;
use acyclic_core::store::ops::{self, Outcome, Summary};
use acyclic_guard::{DenyList, scan_range};

use crate::forward::{self, Operator, Refused, Sent};
use crate::token::Signer;

/// The variables the endpoint gives the hook.
pub const TOKEN: &str = "ACYCLIC_TOKEN";
/// The tracker clone the operator writes.
pub const TRACKER: &str = "ACYCLIC_OPERATOR_TRACKER";
/// The operator's id.
pub const OPERATOR: &str = "ACYCLIC_OPERATOR_ID";
/// The deny-list file, readable by the user alone.
pub const DENY: &str = "ACYCLIC_DENY_FILE";

/// The most operations one push may carry.
const MOST_OPS: usize = 1000;
/// The largest operation file, in bytes.
const MOST_BYTES: u64 = 64 * 1024;

/// Why a push was refused before anything was forwarded: a code for the summary line and a message.
#[derive(Debug)]
struct Refusal(&'static str, String);

fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal(code, message.into())
}

/// Runs the hook in this process: the `acyclic operator-hook` the inbox's pinned hook execs.
#[must_use]
pub fn main() -> ExitCode {
    let env = Env::from_process();
    let given = |name: &str| env.vars.get(name).filter(|value| !value.is_empty()).cloned();
    let in_hook = given("GIT_DIR").is_some() || given("GIT_QUARANTINE_PATH").is_some();
    let (Some(token), Some(tracker), Some(id), Some(deny), Some(home), true) =
        (given(TOKEN), given(TRACKER), given(OPERATOR), given(DENY), given("HOME"), in_hook)
    else {
        eprintln!(
            "acyclic operator-hook runs only as the operator's inbox hook, under git receive-pack"
        );
        return ExitCode::from(2);
    };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return report_refused(&refuse("unreadable", "the hook couldn't read the push's updates"));
    }
    let signer = match Signer::open(Path::new(&home)) {
        Ok((signer, _)) => signer,
        Err(err) => return report_refused(&refuse("operator", err.to_string())),
    };
    let now = acyclic_cli::now(&env);
    let claims = match signer.verifier().verify(&token, now) {
        Ok(claims) => claims,
        Err(err) => return report_refused(&refuse("token", err.to_string())),
    };
    let sent = match check(Path::new("."), &input, &claims.run, Path::new(&deny)) {
        Ok(sent) => sent,
        Err(refusal) => return report_refused(&refusal),
    };
    let operator =
        Operator::new(&id, signer, forward::Clone { tracker: PathBuf::from(tracker), env });
    let seqs: Vec<u64> = sent.iter().map(|sent| sent.seq).collect();
    let forwarded = match operator.forward(&token, sent, now) {
        Ok(forwarded) => forwarded,
        Err(Refused::Token(err)) => return report_refused(&refuse("token", err.to_string())),
        Err(Refused::Inactive { run }) => {
            return report_refused(&refuse("inactive", format!("run {run} isn't under way")));
        },
        Err(Refused::Tracker((_, envelope))) => {
            return report_refused(&refuse("tracker", envelope.to_string()));
        },
    };
    let mut lines = Vec::new();
    for (seq, applied) in seqs.iter().zip(&forwarded.applied) {
        let outcome = match applied {
            Applied::Ran(0, _) | Applied::Duplicate(_) => Outcome::Ok,
            Applied::Ran(_, envelope) => Outcome::Refused(code_of(envelope)),
        };
        lines.push(ops::op_line(*seq, &outcome));
    }
    if let Some((_, envelope)) = &forwarded.failed {
        let seq = seqs[forwarded.applied.len()];
        lines.push(ops::op_line(seq, &Outcome::Failed(code_of(envelope))));
    }
    let count = u64::try_from(lines.len()).unwrap_or(u64::MAX);
    lines.push(ops::summary_line(&Summary::Forwarded(count)));
    eprintln!("{}", lines.join("\n"));
    if forwarded.failed.is_some() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn code_of(envelope: &serde_json::Value) -> String {
    envelope["error"]["code"].as_str().unwrap_or("unknown").to_owned()
}

fn report_refused(refusal: &Refusal) -> ExitCode {
    eprintln!("{}", ops::escape(&refusal.1));
    eprintln!("{}", ops::summary_line(&Summary::Refused(refusal.0.to_owned())));
    ExitCode::FAILURE
}

/// Checks a push to the inbox at `repo` whose updates git gave as `input`, for run `run`, and returns its operations
/// in order.
fn check(repo: &Path, input: &str, run: &str, deny: &Path) -> Result<Vec<Sent>, Refusal> {
    let updates: Vec<Vec<&str>> = input
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split(' ').collect())
        .collect();
    let [update] = updates.as_slice() else {
        return Err(refuse("wrong-ref", "a push updates the run's own ref and nothing else"));
    };
    let [old, new, reference] = update.as_slice() else {
        return Err(refuse("unreadable", "the hook couldn't read the push's update"));
    };
    if *reference != ops::remote_ref(run) {
        return Err(refuse("wrong-ref", format!("this run pushes {} alone", ops::remote_ref(run))));
    }
    let zero = |id: &str| id.bytes().all(|byte| byte == b'0');
    if zero(new) {
        return Err(refuse("wrong-ref", "the run's ref can't be deleted"));
    }
    let old = (!zero(old)).then_some(*old);
    let range = vec![(*new).to_owned(), "--not".to_owned(), "--all".to_owned()];
    let mut highest = match old {
        Some(old) => highest(repo, old)?,
        None => 0,
    };
    let listed =
        git(repo, &["rev-list", "--reverse", "--topo-order", "--parents", new, "--not", "--all"])?;
    let commits: Vec<Vec<&str>> = listed.lines().map(|line| line.split(' ').collect()).collect();
    if commits.len() > MOST_OPS {
        return Err(refuse("too-many", format!("a push carries at most {MOST_OPS} operations")));
    }
    let mut previous = old;
    for line in &commits {
        let (commit, parents) = line.split_first().expect("rev-list prints a commit per line");
        if parents != previous.as_slice() {
            return Err(refuse(
                "not-a-chain",
                format!("{commit} doesn't follow the run's last operation alone"),
            ));
        }
        previous = Some(commit);
    }
    // Scanned before anything is read from the files, so no refusal can echo what the deny-list holds.
    let list = private::check(deny, false)
        .map_err(|err| refuse("scan-failed", err.to_string()))
        .and_then(|_| DenyList::load(deny).map_err(|err| refuse("scan-failed", err.to_string())))?;
    let report =
        scan_range(repo, &range, &list).map_err(|err| refuse("scan-failed", err.to_string()))?;
    if let Some(finding) = report.findings.first() {
        return Err(refuse("deny-listed", finding.to_string()));
    }
    if report.leaks {
        return Err(refuse("leak", "Gitleaks found a secret in the push"));
    }
    let mut sent = Vec::new();
    for line in &commits {
        highest += 1;
        sent.push(operation(repo, line[0], highest)?);
    }
    Ok(sent)
}

/// The operation commit `commit` adds, which must be operation `seq` and nothing else.
fn operation(repo: &Path, commit: &str, seq: u64) -> Result<Sent, Refusal> {
    let changes = git(
        repo,
        &["diff-tree", "-r", "-z", "--root", "--no-commit-id", "--no-renames", "--raw", commit],
    )?;
    let fields: Vec<&str> = changes.split('\0').filter(|field| !field.is_empty()).collect();
    let expected = ops::path(seq);
    let blob = match fields.as_slice() {
        [meta, path] if *path == expected => match meta.split(' ').collect::<Vec<_>>().as_slice() {
            [":000000", "100644", _, blob, "A"] => (*blob).to_owned(),
            _ => {
                return Err(refuse(
                    "not-an-operation",
                    format!("{commit} must add {expected} as a file"),
                ));
            },
        },
        _ => {
            return Err(refuse(
                "not-an-operation",
                format!("{commit} must add {expected} and nothing else"),
            ));
        },
    };
    let size: u64 = git(repo, &["cat-file", "-s", &blob])?.trim().parse().unwrap_or(u64::MAX);
    if size > MOST_BYTES {
        return Err(refuse("not-an-operation", format!("{expected} is over {MOST_BYTES} bytes")));
    }
    let text = git(repo, &["cat-file", "blob", &blob])?;
    let sent: Sent = serde_json::from_str(&text)
        .map_err(|err| refuse("not-an-operation", format!("{expected}: {err}")))?;
    if sent.seq != seq {
        return Err(refuse(
            "not-an-operation",
            format!("{expected} says it is operation {}", sent.seq),
        ));
    }
    Ok(sent)
}

/// The highest operation in a commit already in the inbox.
fn highest(repo: &Path, commit: &str) -> Result<u64, Refusal> {
    let names = git(repo, &["ls-tree", "--name-only", commit, "ops/"])?;
    Ok(names.lines().filter_map(ops::seq_of).max().unwrap_or(0))
}

/// Runs git in the inbox, as the hook's environment points it, quarantine included.
fn git(repo: &Path, args: &[&str]) -> Result<String, Refusal> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| refuse("git", format!("git can't run: {err}")))?;
    if !output.status.success() {
        return Err(refuse("git", format!("git {} failed", args.first().unwrap_or(&""))));
    }
    String::from_utf8(output.stdout).map_err(|_| refuse("git", "git printed something not UTF-8"))
}

/// The environment the endpoint gives `git receive-pack`, and so the hook: nothing of its own but these.
#[must_use]
pub fn environment(
    token: &str,
    tracker: &Path,
    operator: &str,
    deny: &Path,
    home: &Path,
    path: &str,
    now: Option<&str>,
) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::from([
        ("PATH".to_owned(), path.to_owned()),
        ("HOME".to_owned(), home.display().to_string()),
        ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
        ("GIT_CONFIG_GLOBAL".to_owned(), "/dev/null".to_owned()),
        (TOKEN.to_owned(), token.to_owned()),
        (TRACKER.to_owned(), tracker.display().to_string()),
        (OPERATOR.to_owned(), operator.to_owned()),
        (DENY.to_owned(), deny.display().to_string()),
    ]);
    if let Some(now) = now {
        vars.insert("ACYCLIC_NOW".to_owned(), now.to_owned());
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_refuses_to_run_outside_a_push() {
        // The test process has none of the hook's variables.
        assert_eq!(main(), ExitCode::from(2));
    }
}
