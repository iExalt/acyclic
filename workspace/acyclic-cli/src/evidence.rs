//! Commit evidence is checked once, where a batch writes it: every commit a write adds or changes must be on its
//! repository's default branch in this machine's clone, fetched first when it isn't yet. Reads never check, so this is
//! where a typo or an unmerged commit is caught.

use std::collections::{BTreeMap, BTreeSet};

use acyclic_core::merged::{Check, GitAncestry};
use acyclic_core::model::Model;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::context;
use crate::error::CliError;

/// One commit evidence: the item it finishes, its repository record and the commit.
type Commit = (String, String, String);

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

/// Checks the commits `after` holds that `before` didn't. Returns those this machine can't check, as JSON for the
/// result, and refuses the batch when one is unmerged or unknown, even after fetching its repository's default branch.
pub(crate) fn check(
    env: &Env,
    before: &BTreeSet<Commit>,
    after: &Model,
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
    let clones = match context::clones(env) {
        Ok(clones) => clones,
        Err(error) => {
            return Ok(added
                .iter()
                .map(|(item, repository, commit)| {
                    unchecked(item, repository, commit, &error.message)
                })
                .collect());
        },
    };
    let ancestry = GitAncestry::new(clones);
    let mut refreshed: BTreeMap<&str, Result<(), String>> = BTreeMap::new();
    let mut found = Vec::new();
    for (item, repository, commit) in &added {
        let Some(record) = after.repositories.get(repository) else {
            found.push(unchecked(item, repository, commit, "no such repository record"));
            continue;
        };
        let mut checked = ancestry.check(record, commit);
        if matches!(checked, Check::NotMerged | Check::UnknownCommit) {
            // A squash commit merged a moment ago isn't in the clone until it fetches.
            let fetched = refreshed.entry(repository).or_insert_with(|| ancestry.refresh(record));
            if let Err(reason) = fetched {
                let mut refused = refusal(after, (item, repository, commit), "fetch-failed");
                refused.details["fetch_error"] = reason.clone().into();
                return Err(refused);
            }
            checked = ancestry.check(record, commit);
        }
        match checked {
            Check::Merged => {},
            Check::NotMerged => {
                return Err(refusal(after, (item, repository, commit), "not-merged"));
            },
            Check::UnknownCommit => {
                return Err(refusal(after, (item, repository, commit), "unknown-commit"));
            },
            Check::Unchecked(reason) => found.push(unchecked(item, repository, commit, &reason)),
        }
    }
    Ok(found)
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

/// Every commit evidence in `model` checked as this machine's clones were last fetched, fetching nothing: those not on
/// their default branch, or unknown, by item, and how many were checked and how many couldn't be.
pub(crate) fn audit(env: &Env, model: &Model) -> Json {
    let all = commits(model);
    let Ok(clones) = context::clones(env) else {
        return json!({"checked": 0, "unchecked": all.len(), "unmerged": []});
    };
    let ancestry = GitAncestry::new(clones);
    let (mut checked, mut unchecked, mut unmerged) = (0, 0, Vec::new());
    for (item, repository, commit) in &all {
        let Some(record) = model.repositories.get(repository) else {
            unchecked += 1;
            continue;
        };
        let why = match ancestry.check(record, commit) {
            Check::Merged => None,
            Check::NotMerged => Some("not-merged"),
            Check::UnknownCommit => Some("unknown-commit"),
            Check::Unchecked(_) => {
                unchecked += 1;
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
    json!({"checked": checked, "unchecked": unchecked, "unmerged": unmerged})
}
