//! Commit evidence is checked once, where a batch writes it: every commit a write adds or changes must be on its
//! repository's default branch in this machine's clone, fetched first when it isn't yet, or, for a repository with no
//! clone, on GitHub's word, asked once before the write takes the tracker's lock. Reads never check, so this is where
//! a typo or an unmerged commit is caught.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;

use acyclic_core::merged::{Check, GitAncestry};
use acyclic_core::model::Model;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::context;
use crate::error::CliError;
use crate::github::Github;
use crate::learned::Learned;

/// One commit evidence: the item it finishes, its repository record and the commit.
type Commit = (String, String, String);

/// A clone and branch selection that a write's commit evidence was checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    clone: Option<PathBuf>,
    branch: Option<String>,
}

#[derive(Clone)]
enum PriorCheck {
    Merged,
    Unchecked,
    Refreshed(Result<(), String>),
    /// Asked GitHub, for a repository with no clone: there is nothing to refresh, and the verdict stands.
    GitHub(Check),
}

#[derive(Clone)]
struct Candidate {
    identity: Identity,
    prior: PriorCheck,
}

/// Refreshes completed before the store batch, keyed by each added evidence record.
#[derive(Default)]
pub(crate) struct Prepared(BTreeMap<Commit, Candidate>);

/// Every commit evidence in `model`.
pub(crate) fn commits(model: &Model) -> BTreeSet<Commit> {
    model
        .items
        .iter()
        .flat_map(|(id, item)| {
            item.evidence.all().into_iter().filter_map(move |evidence| {
                Some((id.clone(), evidence.repository.clone()?, evidence.commit.clone()?))
            })
        })
        .collect()
}

/// Fetches any default branches needed to check commits a prospective write adds, before its store batch.
pub(crate) fn preflight(
    env: &Env,
    learned: &Rc<Learned>,
    candidates: &BTreeSet<Commit>,
    model: &Model,
) -> Prepared {
    let mut prepared = Prepared::default();
    let Ok(clones) = context::clones_with(env, learned) else { return prepared };
    let ancestry = GitAncestry::new(&clones);
    let github = Github::new(env);
    let mut refreshed: BTreeMap<(PathBuf, String), Result<(), String>> = BTreeMap::new();
    for candidate in candidates {
        let Some(record) = model.repositories.get(&candidate.1) else { continue };
        let id = identity(&clones, record);
        let prior = match ancestry.check(record, &candidate.2) {
            Check::Merged => PriorCheck::Merged,
            Check::Unchecked(_) if id.clone.is_none() => {
                PriorCheck::GitHub(github.check(record, &candidate.2))
            },
            Check::Unchecked(_) => PriorCheck::Unchecked,
            Check::NotMerged | Check::UnknownCommit | Check::NoBranch(_) => {
                let refresh = match (&id.clone, &id.branch) {
                    (Some(clone), Some(branch)) => Some(
                        refreshed
                            .entry((clone.clone(), branch.clone()))
                            .or_insert_with(|| ancestry.refresh(record))
                            .clone(),
                    ),
                    _ => None,
                };
                PriorCheck::Refreshed(refresh.unwrap_or_else(|| {
                    Err("no clone or single default branch is mapped".to_owned())
                }))
            },
        };
        prepared.0.insert(candidate.clone(), Candidate { identity: id, prior });
    }
    prepared
}

/// Checks the commits `after` holds that `before` didn't, without fetching: refreshes needed by the candidate write
/// have already run before the store batch. Returns commits this machine can't check, and refuses unmerged evidence.
pub(crate) fn check(
    env: &Env,
    learned: &Rc<Learned>,
    before: &BTreeSet<Commit>,
    after: &Model,
    prepared: &Prepared,
) -> Result<Vec<Json>, CliError> {
    let added: Vec<Commit> = commits(after).difference(before).cloned().collect();
    if added.is_empty() {
        return Ok(Vec::new());
    }
    let unchecked = |item: &str, repository: &str, commit: &str, reason: &str| {
        json!({
            "item": item, "repository": name_of(after, repository), "commit": commit, "reason": reason,
        })
    };
    let clones = match context::clones_with(env, learned) {
        Ok(clones) => clones,
        Err(error) => {
            let mut found = Vec::new();
            for (item, repository, commit) in &added {
                let candidate = (item.clone(), repository.clone(), commit.clone());
                if prepared
                    .0
                    .get(&candidate)
                    .is_some_and(|candidate| !matches!(&candidate.prior, PriorCheck::Unchecked))
                {
                    return Err(stale_preflight(after, (item, repository, commit)));
                }
                found.push(unchecked(item, repository, commit, &error.message));
            }
            return Ok(found);
        },
    };
    let ancestry = GitAncestry::new(&clones);
    let mut found = Vec::new();
    for (item, repository, commit) in &added {
        let Some(record) = after.repositories.get(repository) else {
            found.push(unchecked(item, repository, commit, "no such repository record"));
            continue;
        };
        let candidate = (item.clone(), repository.clone(), commit.clone());
        let current_identity = identity(&clones, record);
        let prepared_candidate = prepared.0.get(&candidate);
        let checked = ancestry.check(record, commit);
        if !matches!(&checked, Check::Merged) {
            if let Some(candidate) = prepared_candidate {
                if candidate.identity != current_identity {
                    return Err(stale_preflight(after, (item, repository, commit)));
                }
                match (&candidate.prior, &checked) {
                    (PriorCheck::Unchecked, Check::Unchecked(_)) => {},
                    (PriorCheck::GitHub(verdict), Check::Unchecked(_)) => {
                        if let Some(reason) = settle(after, (item, repository, commit), verdict)? {
                            found.push(unchecked(item, repository, commit, &reason));
                        }
                        continue;
                    },
                    (PriorCheck::Merged | PriorCheck::Unchecked | PriorCheck::GitHub(_), _)
                    | (PriorCheck::Refreshed(Ok(())), Check::NoBranch(_) | Check::Unchecked(_)) => {
                        return Err(stale_preflight(after, (item, repository, commit)));
                    },
                    (PriorCheck::Refreshed(Err(reason)), _) => {
                        let mut refused =
                            refusal(after, (item, repository, commit), "fetch-failed");
                        refused.details["fetch_error"] = reason.clone().into();
                        return Err(refused);
                    },
                    (PriorCheck::Refreshed(Ok(())), Check::NotMerged) => {
                        return Err(refusal(after, (item, repository, commit), "not-merged"));
                    },
                    (PriorCheck::Refreshed(Ok(())), Check::UnknownCommit) => {
                        return Err(refusal(after, (item, repository, commit), "unknown-commit"));
                    },
                    (PriorCheck::Refreshed(Ok(())), Check::Merged) => unreachable!(),
                }
            } else {
                match &checked {
                    Check::Unchecked(reason) => {
                        if current_identity.clone.is_none() || current_identity.branch.is_none() {
                            found.push(unchecked(item, repository, commit, reason));
                            continue;
                        }
                        return Err(stale_preflight(after, (item, repository, commit)));
                    },
                    _ => return Err(stale_preflight(after, (item, repository, commit))),
                }
            }
        }
        match checked {
            Check::Merged => {},
            Check::NotMerged => {
                return Err(refusal(after, (item, repository, commit), "not-merged"));
            },
            Check::UnknownCommit => {
                return Err(refusal(after, (item, repository, commit), "unknown-commit"));
            },
            Check::NoBranch(reason) | Check::Unchecked(reason) => {
                found.push(unchecked(item, repository, commit, &reason));
            },
        }
    }
    Ok(found)
}

/// What GitHub's verdict on a commit means for the write: a refusal, or the reason it stays unchecked.
fn settle(
    model: &Model,
    evidence: (&str, &str, &str),
    verdict: &Check,
) -> Result<Option<String>, CliError> {
    match verdict {
        Check::Merged => Ok(None),
        Check::NotMerged => Err(refusal(model, evidence, "not-merged")),
        Check::UnknownCommit => Err(refusal(model, evidence, "unknown-commit")),
        Check::NoBranch(reason) | Check::Unchecked(reason) => Ok(Some(reason.clone())),
    }
}

fn identity(
    clones: &impl Fn(&acyclic_core::model::Repository) -> Option<PathBuf>,
    repository: &acyclic_core::model::Repository,
) -> Identity {
    Identity { clone: clones(repository), branch: repository.default_branch.one().cloned() }
}

fn stale_preflight(model: &Model, evidence: (&str, &str, &str)) -> CliError {
    let (item, repository, commit) = evidence;
    CliError::refused(
        "changed-since",
        "the repository clone mapping or default branch changed while checking commit evidence; retry the write",
    )
    .with(json!({
        "item": item,
        "repository": name_of(model, repository),
        "commit": commit,
        "reason": "stale-preflight",
    }))
}

/// A repository record's `owner/name`, or its id when it has none.
fn name_of(model: &Model, repository: &str) -> String {
    model
        .repositories
        .get(repository)
        .and_then(crate::resolve::repository_name)
        .unwrap_or_else(|| repository.to_owned())
}

fn refusal(model: &Model, (item, repository, commit): (&str, &str, &str), why: &str) -> CliError {
    let name = name_of(model, repository);
    let branch = model
        .repositories
        .get(repository)
        .and_then(|record| record.default_branch.one().cloned())
        .unwrap_or_else(|| "default branch".to_owned());
    let message = match why {
        "not-merged" => format!(
            "commit {commit} isn't on {name}'s {branch}: finish the item after its pull request merges, with the \
             commit that landed"
        ),
        "unknown-commit" => {
            format!("{name} has no commit {commit}, even after fetching {branch}: check the commit")
        },
        _ => format!(
            "couldn't fetch {name}'s {branch} to check commit {commit}; nothing was written, so try again"
        ),
    };
    CliError::refused("unmerged", format!("{message} (forcing doesn't skip this check)")).with(
        json!({
            "item": item, "repository": name, "commit": commit, "reason": why,
        }),
    )
}

/// Every commit evidence in `model` checked as this machine's clones were last fetched, fetching nothing, and, for a
/// repository with no clone, on GitHub's word, asked once for each commit and no more once GitHub proves unreachable:
/// those not on their default branch, or unknown, by item, how many were checked and how many couldn't be, and each
/// of those, with the reason.
pub(crate) fn audit(env: &Env, model: &Model) -> Json {
    let all = commits(model);
    let unchecked_row = |item: &str, repository: &str, commit: &str, reason: &str| {
        json!({
            "item": item, "repository": name_of(model, repository), "commit": commit, "reason": reason,
        })
    };
    let clones = match context::clones(env) {
        Ok(clones) => clones,
        Err(error) => {
            let rows: Vec<Json> = all
                .iter()
                .map(|(item, repository, commit)| {
                    unchecked_row(item, repository, commit, &error.message)
                })
                .collect();
            return json!({"checked": 0, "unchecked": rows.len(), "unmerged": [], "unchecked_commits": rows});
        },
    };
    let ancestry = GitAncestry::new(&clones);
    let github = Github::new(env);
    let (mut checked, mut unchecked, mut unmerged) = (0, Vec::new(), Vec::new());
    for (item, repository, commit) in &all {
        let Some(record) = model.repositories.get(repository) else {
            unchecked.push(unchecked_row(item, repository, commit, "no such repository record"));
            continue;
        };
        let mut verdict = ancestry.check(record, commit);
        if matches!(verdict, Check::Unchecked(_)) && clones(record).is_none() {
            verdict = github.check(record, commit);
        }
        let why = match verdict {
            Check::Merged => None,
            Check::NotMerged => Some("not-merged"),
            Check::UnknownCommit => Some("unknown-commit"),
            Check::NoBranch(reason) | Check::Unchecked(reason) => {
                unchecked.push(unchecked_row(item, repository, commit, &reason));
                continue;
            },
        };
        checked += 1;
        if let Some(why) = why {
            unmerged.push(json!({
                "item": item, "repository": name_of(model, repository), "commit": commit, "reason": why,
            }));
        }
    }
    json!({"checked": checked, "unchecked": unchecked.len(), "unmerged": unmerged, "unchecked_commits": unchecked})
}

#[cfg(test)]
mod tests;
