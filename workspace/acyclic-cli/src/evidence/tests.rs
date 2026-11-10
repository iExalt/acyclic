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
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
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
    let prepared = preflight(&env, &Rc::default(), &candidates, &model);

    git(&clone, &["update-ref", "-d", "refs/remotes/origin/main"]);
    git(&clone, &["remote", "remove", "origin"]);
    git(&clone, &["branch", "-D", "main"]);
    let candidate = candidates.iter().next().unwrap().clone();
    let mut failed_refresh = Prepared(prepared.0.clone());
    failed_refresh.0.get_mut(&candidate).unwrap().prior =
        PriorCheck::Refreshed(Err("synthetic fetch refusal".to_owned()));
    let error = check(&env, &Rc::default(), &BTreeSet::new(), &model, &failed_refresh).unwrap_err();
    assert_eq!(error.details["reason"], "fetch-failed");
    assert_eq!(error.details["fetch_error"], "synthetic fetch refusal");

    let error = check(&env, &Rc::default(), &BTreeSet::new(), &model, &prepared).unwrap_err();
    assert_eq!(error.details["reason"], "stale-preflight");
    assert_eq!(error.code, "changed-since");
}

#[test]
fn doctor_lists_each_unchecked_commit_with_its_reason() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().display().to_string();
    let env = |home: &str| crate::Env {
        vars: BTreeMap::from([("HOME".to_owned(), home.to_owned())]),
        cwd: temp.path().to_path_buf(),
        terminal: None,
    };
    let mut model = candidate_model("abcdef1");
    // A commit whose repository record is gone.
    model.items.get_mut("item").unwrap().evidence.clone_from(&Value::One(Evidence {
        repository: Some("gone".to_owned()),
        commit: Some("abcdef1".to_owned()),
        text: None,
    }));
    let audited = audit(&env(&home), &model);
    assert_eq!(
        audited,
        json!({
            "checked": 0, "unchecked": 1, "unmerged": [],
            "unchecked_commits": [{
                "item": "item", "repository": "gone", "commit": "abcdef1", "reason": "no such repository record",
            }],
        })
    );

    // A repository with no clone says why it can't be checked.
    let model = candidate_model("abcdef1");
    let audited = audit(&env(&home), &model);
    assert_eq!(audited["unchecked"], 1);
    assert_eq!(audited["unchecked_commits"][0]["repository"], "me/code");
    assert_eq!(audited["unchecked_commits"][0]["commit"], "abcdef1");

    // A broken clones.toml lists every commit with the config error as its reason.
    let config = temp.path().join(".config/acyclic");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("clones.toml"), "[clones\n").unwrap();
    let audited = audit(&env(&home), &model);
    assert_eq!((&audited["checked"], &audited["unchecked"]), (&json!(0), &json!(1)));
    let reason = audited["unchecked_commits"][0]["reason"].as_str().unwrap();
    assert!(reason.contains("clones.toml"), "{reason}");
}
