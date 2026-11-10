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

/// An environment whose `gh` is a script that records each call in `calls`, then prints `stdout`, `stderr` and exits
/// with `code`.
fn gh_env(temp: &TempDir, stdout: &str, stderr: &str, code: i32) -> crate::Env {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> \"$GH_CALLS\"\nprintf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2\nexit {code}\n"
    );
    std::fs::write(bin.join("gh"), script).unwrap();
    std::fs::set_permissions(bin.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = |name: &str| temp.path().join(name).display().to_string();
    crate::Env {
        vars: BTreeMap::from([
            ("HOME".to_owned(), path("home")),
            ("XDG_STATE_HOME".to_owned(), path("state")),
            ("PATH".to_owned(), path("bin")),
            ("GH_CALLS".to_owned(), path("calls")),
        ]),
        cwd: temp.path().to_path_buf(),
        terminal: None,
    }
}

fn calls(temp: &TempDir) -> Vec<String> {
    std::fs::read_to_string(temp.path().join("calls"))
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// A write of `commit` for `me/code`, which has no clone, checked as the command does: asked once in preflight.
fn github_write(env: &crate::Env, commit: &str) -> Result<Vec<Json>, CliError> {
    let model = candidate_model(commit);
    let candidates = BTreeSet::from([("item".to_owned(), "repo".to_owned(), commit.to_owned())]);
    let learned = Rc::default();
    let prepared = preflight(env, &learned, &candidates, &model);
    check(env, &learned, &BTreeSet::new(), &model, &prepared)
}

#[test]
fn a_repository_with_no_clone_is_checked_by_github_once_per_commit() {
    let behind = r#"{"status":"behind"}"#;
    let ahead = r#"{"status":"ahead"}"#;
    let cases = [
        (behind, "", 0, Ok(None)),
        (ahead, "", 0, Err("not-merged")),
        ("", "gh: No commit found for SHA: abcdef1 (HTTP 422)", 1, Err("unknown-commit")),
        ("", "gh: Not Found (HTTP 404)", 1, Ok(Some("can't see it"))),
        (
            "",
            "To get started with GitHub CLI, please run: gh auth login",
            4,
            Ok(Some("gh auth login")),
        ),
    ];
    for (stdout, stderr, code, expected) in cases {
        let temp = TempDir::new().unwrap();
        let env = gh_env(&temp, stdout, stderr, code);
        let result = github_write(&env, "abcdef1");
        match (result, expected) {
            (Ok(unchecked), Ok(None)) => assert!(unchecked.is_empty(), "{unchecked:?}"),
            (Ok(unchecked), Ok(Some(reason))) => {
                assert_eq!(unchecked.len(), 1, "{unchecked:?}");
                assert!(unchecked[0]["reason"].as_str().unwrap().contains(reason), "{unchecked:?}");
                assert_eq!(unchecked[0]["repository"], "me/code");
            },
            (Err(refused), Err(reason)) => {
                if reason == "unknown-commit" {
                    assert!(
                        refused.message.contains("GitHub has no commit abcdef1"),
                        "{}",
                        refused.message
                    );
                }
                assert_eq!(
                    (refused.code, refused.details["reason"].as_str()),
                    ("unmerged", Some(reason))
                );
            },
            (other, expected) => panic!("{other:?} for {expected:?}"),
        }
        assert_eq!(
            calls(&temp),
            ["api --hostname github.com repos/me/code/compare/main...abcdef1"],
            "{stdout}{stderr}"
        );
    }
}

#[test]
fn only_a_merge_github_reported_is_remembered_between_commands() {
    let temp = TempDir::new().unwrap();
    let env = gh_env(&temp, r#"{"status":"behind"}"#, "", 0);
    assert_eq!(github_write(&env, "abcdef1").unwrap(), Vec::<Json>::new());
    assert_eq!(calls(&temp).len(), 1);
    // The next command doesn't ask again, even with `gh` gone.
    let mut gone = env.clone();
    gone.vars.insert("PATH".to_owned(), String::new());
    assert_eq!(github_write(&gone, "abcdef1").unwrap(), Vec::<Json>::new());
    assert_eq!(calls(&temp).len(), 1);
    // An unmerged commit is asked about again each time.
    let temp = TempDir::new().unwrap();
    let env = gh_env(&temp, r#"{"status":"ahead"}"#, "", 0);
    for _ in 0..2 {
        assert!(github_write(&env, "abcdef1").is_err());
    }
    assert_eq!(calls(&temp).len(), 2);
}

#[test]
fn doctor_asks_github_once_per_commit_and_stops_once_it_cannot_be_reached() {
    let temp = TempDir::new().unwrap();
    let env = gh_env(&temp, "", "gh: authentication required", 4);
    let mut model = candidate_model("abcdef1");
    for (id, commit) in [("two", "abcdef2"), ("three", "abcdef3")] {
        let mut item = model.items["item"].clone();
        item.id = id.to_owned();
        item.evidence = Value::One(Evidence {
            repository: Some("repo".to_owned()),
            commit: Some(commit.to_owned()),
            text: None,
        });
        model.items.insert(id.to_owned(), item);
    }
    let audited = audit(&env, &model);
    assert_eq!((&audited["checked"], &audited["unchecked"]), (&json!(0), &json!(3)));
    assert_eq!(calls(&temp).len(), 1, "later commits aren't asked once GitHub failed");
    for row in audited["unchecked_commits"].as_array().unwrap() {
        assert!(row["reason"].as_str().unwrap().contains("authentication required"), "{row}");
    }

    // A reachable GitHub is asked once for each distinct commit, and its verdicts count as checked.
    let temp = TempDir::new().unwrap();
    let env = gh_env(&temp, r#"{"status":"ahead"}"#, "", 0);
    let audited = audit(&env, &model);
    assert_eq!(
        (&audited["checked"], audited["unmerged"].as_array().unwrap().len()),
        (&json!(3), 3)
    );
    assert_eq!(calls(&temp).len(), 3);
}
