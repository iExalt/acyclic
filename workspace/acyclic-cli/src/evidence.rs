//! Commit evidence is checked once, where a batch writes it: every commit a write adds or changes must be on its
//! repository's default branch in this machine's clone, fetched first when it isn't yet. Reads never check, so this is
//! where a typo or an unmerged commit is caught.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use acyclic_core::merged::{Check, GitAncestry};
use acyclic_core::model::Model;
use serde_json::{Value as Json, json};

use crate::Env;
use crate::context;
use crate::error::CliError;

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
pub(crate) fn preflight(env: &Env, candidates: &BTreeSet<Commit>, model: &Model) -> Prepared {
    let mut prepared = Prepared::default();
    let Ok(clones) = context::clones(env) else { return prepared };
    let ancestry = GitAncestry::new(&clones);
    let mut refreshed: BTreeMap<(PathBuf, String), Result<(), String>> = BTreeMap::new();
    for candidate in candidates {
        let Some(record) = model.repositories.get(&candidate.1) else { continue };
        let id = identity(&clones, record);
        let prior = match ancestry.check(record, &candidate.2) {
            Check::Merged => PriorCheck::Merged,
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
    let clones = match context::clones(env) {
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
                    (PriorCheck::Merged | PriorCheck::Unchecked, _)
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
            Check::NoBranch(_) | Check::Unchecked(_) => {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::process::Command;

    use acyclic_core::model::{Evidence, Item, ItemState, Repository, Value};
    use tempfile::TempDir;

    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn candidate_model(commit: &str) -> Model {
        let mut model = Model::default();
        model.repositories.insert(
            "repo".to_owned(),
            Repository {
                id: "repo".to_owned(),
                owner: Value::One("me".to_owned()),
                name: Value::One("code".to_owned()),
                default_branch: Value::One("main".to_owned()),
            },
        );
        model.items.insert(
            "item".to_owned(),
            Item {
                id: "item".to_owned(),
                title: Value::One("Ship".to_owned()),
                notes: Value::None,
                state: Value::One(ItemState::Done),
                done_at: None,
                parent: Value::None,
                evidence: Value::One(Evidence {
                    repository: Some("repo".to_owned()),
                    commit: Some(commit.to_owned()),
                    text: None,
                }),
                thread: Value::None,
            },
        );
        model
    }

    #[test]
    fn a_refreshed_candidate_cannot_become_uncheckable_inside_the_batch() {
        let temp = TempDir::new().unwrap();
        let origin = temp.path().join("origin");
        let clone = temp.path().join("clone");
        git(temp.path(), &["init", "-q", "-b", "main", origin.to_str().unwrap()]);
        git(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(temp.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        git(&clone, &["switch", "-q", "-c", "work"]);
        git(&origin, &["commit", "-q", "--allow-empty", "-m", "merged"]);
        let commit = git(&origin, &["rev-parse", "HEAD"]);

        let config = temp.path().join(".config/acyclic");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("clones.toml"),
            format!("[clones]\n\"me/code\" = \"{}\"\n", clone.display()),
        )
        .unwrap();
        let env = crate::Env {
            vars: BTreeMap::from([("HOME".to_owned(), temp.path().display().to_string())]),
            cwd: temp.path().to_path_buf(),
            terminal: None,
        };
        let model = candidate_model(&commit);
        let candidates = BTreeSet::from([("item".to_owned(), "repo".to_owned(), commit)]);
        let prepared = preflight(&env, &candidates, &model);

        git(&clone, &["update-ref", "-d", "refs/remotes/origin/main"]);
        git(&clone, &["branch", "-D", "main"]);
        let error = check(&env, &BTreeSet::new(), &model, &prepared).unwrap_err();
        assert_eq!(error.details["reason"], "stale-preflight");
        assert_eq!(error.code, "changed-since");
    }
}
