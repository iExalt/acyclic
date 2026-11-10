use super::*;
use crate::model::Value;

/// Git in `dir` with no user or system settings and a fixed identity.
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.test")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn git_finds_a_commit_on_the_default_branch_of_a_clone_or_its_remote() {
    let dir = tempfile::TempDir::new().unwrap();
    let code = dir.path().join("code");
    git(dir.path(), &["init", "-q", "-b", "main", code.to_str().unwrap()]);
    git(&code, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(&code, &["rev-parse", "HEAD"]);
    git(&code, &["switch", "-q", "-c", "feature"]);
    git(&code, &["commit", "-q", "--allow-empty", "-m", "the work"]);
    let work = git(&code, &["rev-parse", "HEAD"]);
    git(&code, &["switch", "-q", "main"]);
    let record = |branch: &str| Repository {
        id: "code".to_owned(),
        owner: Value::None,
        name: Value::None,
        default_branch: Value::One(branch.to_owned()),
    };
    let main = record("main");
    // Each check looks the branch up afresh, as each command does.
    let merged = |clone: &std::path::Path, commit: &str| {
        GitAncestry::new(|_: &Repository| Some(clone.to_owned())).merged(&main, commit)
    };
    assert_eq!(merged(&code, &base[..7].to_uppercase()), Ok(true), "a prefix, in either case");
    assert_eq!(merged(&code, &work), Ok(false), "on another branch");
    assert!(merged(&code, &"ab".repeat(20)).is_err(), "a commit git doesn't know");
    let check =
        |commit: &str| GitAncestry::new(|_: &Repository| Some(code.clone())).check(&main, commit);
    assert_eq!(check(&work), Check::NotMerged);
    assert_eq!(check(&"ab".repeat(20)), Check::UnknownCommit);
    assert_eq!(check("zzzzzzz"), Check::UnknownCommit, "not a commit id");
    let ancestry = GitAncestry::new(|_: &Repository| Some(code.clone()));
    assert_eq!(
        ancestry.merged(&record("trunk"), &base),
        Err("the clone has neither an origin remote nor refs for default branch trunk".to_owned())
    );

    git(&code, &["merge", "-q", "--ff-only", "feature"]);
    assert_eq!(merged(&code, &work), Ok(true), "once merged");
    // A clone reads the default branch from its remote-tracking ref, here with no local branch to read instead.
    let clone = dir.path().join("clone");
    git(dir.path(), &["clone", "-q", "--bare", code.to_str().unwrap(), clone.to_str().unwrap()]);
    git(&clone, &["update-ref", "refs/remotes/origin/main", "refs/heads/main"]);
    git(&clone, &["update-ref", "-d", "refs/heads/main"]);
    assert_eq!(merged(&clone, &work), Ok(true));
}

#[test]
fn an_originless_clone_without_default_refs_is_unchecked_but_a_local_branch_still_checks() {
    let dir = tempfile::TempDir::new().unwrap();
    let origin = dir.path().join("origin");
    git(dir.path(), &["init", "-q", "-b", "main", origin.to_str().unwrap()]);
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(&origin, &["rev-parse", "HEAD"]);
    let clone = dir.path().join("clone");
    git(dir.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
    git(&clone, &["switch", "-q", "-c", "work"]);

    git(&clone, &["remote", "remove", "origin"]);
    let config_path = clone.join(".git/config");
    let initial_config = std::fs::read(&config_path).unwrap();
    std::fs::write(&config_path, "not a valid git config = [\n").unwrap();
    let record = Repository {
        id: "code".to_owned(),
        owner: Value::None,
        name: Value::None,
        default_branch: Value::One("main".to_owned()),
    };
    let ancestry = GitAncestry::new(|_: &Repository| Some(clone.clone()));
    assert_eq!(
        ancestry.check(&record, &base),
        Check::Merged,
        "a local default branch still counts"
    );

    std::fs::write(&config_path, initial_config).unwrap();
    git(&clone, &["branch", "-D", "main"]);
    std::fs::write(&config_path, "# comments and whitespace are harmless\n \n").unwrap();
    let without_refs = GitAncestry::new(|_: &Repository| Some(clone.clone()));
    assert_eq!(
        without_refs.check(&record, &base),
        Check::Unchecked(
            "the clone has neither an origin remote nor refs for default branch main".to_owned()
        )
    );
}

#[test]
fn noncommit_refs_configured_remotes_and_invalid_config_never_look_originless() {
    let dir = tempfile::TempDir::new().unwrap();
    let clone = dir.path().join("clone");
    git(dir.path(), &["init", "-q", "-b", "main", clone.to_str().unwrap()]);
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(&clone, &["switch", "-q", "-c", "work"]);
    git(&clone, &["branch", "-D", "main"]);
    let config_path = clone.join(".git/config");
    let initial_config = std::fs::read_to_string(&config_path).unwrap();
    let record = Repository {
        id: "code".to_owned(),
        owner: Value::None,
        name: Value::None,
        default_branch: Value::One("main".to_owned()),
    };
    let check =
        || GitAncestry::new(|_: &Repository| Some(clone.clone())).check(&record, &"ab".repeat(20));

    let tree = git(&clone, &["mktree"]);
    git(&clone, &["update-ref", "refs/remotes/origin/main", &tree]);
    assert!(matches!(check(), Check::NoBranch(_)), "a noncommit exact ref is not absent");
    git(&clone, &["update-ref", "-d", "refs/remotes/origin/main"]);

    git(&clone, &["config", "--add", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"]);
    assert!(matches!(check(), Check::NoBranch(_)), "an origin without a URL is not absent");

    std::fs::write(&config_path, initial_config).unwrap();
    std::fs::create_dir_all(clone.join(".git/refs/remotes/origin")).unwrap();
    std::fs::write(clone.join(".git/refs/remotes/origin/main"), "not-a-valid-object-id\n").unwrap();
    assert!(matches!(check(), Check::NoBranch(_)), "a corrupt exact ref is not absent");

    std::fs::remove_file(clone.join(".git/refs/remotes/origin/main")).unwrap();
    std::fs::write(&config_path, "not a valid git config = [\n").unwrap();
    assert!(matches!(check(), Check::NoBranch(_)), "invalid repository config is not absent");

    std::fs::write(&config_path, "[unterminated\n").unwrap();
    assert!(matches!(check(), Check::NoBranch(_)), "a malformed header is not absent");

    std::fs::remove_file(&config_path).unwrap();
    std::fs::create_dir(&config_path).unwrap();
    assert!(matches!(check(), Check::NoBranch(_)), "a config read failure is not absence");
}

#[test]
fn an_unmapped_repository_or_branch_is_unchecked() {
    let record = Repository {
        id: "code".to_owned(),
        owner: Value::One("me".to_owned()),
        name: Value::One("code".to_owned()),
        default_branch: Value::One("main".to_owned()),
    };
    let none = GitAncestry::new(|_: &Repository| None);
    assert_eq!(
        none.check(&record, &"ab".repeat(20)),
        Check::Unchecked(
            "no clone of me/code is known on this machine: run any acyclic command inside a clone of it whose origin is on \
             github.com, or map it in ~/.config/acyclic/clones.toml"
                .to_owned()
        )
    );
    let branchless = Repository { default_branch: Value::None, ..record };
    let mapped = GitAncestry::new(|_: &Repository| Some(std::path::PathBuf::from("/nonexistent")));
    assert!(matches!(mapped.check(&branchless, &"ab".repeat(20)), Check::Unchecked(_)));
}

#[test]
fn refresh_fetches_a_merge_made_since_the_clones_last_fetch() {
    let dir = tempfile::TempDir::new().unwrap();
    let origin = dir.path().join("origin");
    git(dir.path(), &["init", "-q", "-b", "main", origin.to_str().unwrap()]);
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let clone = dir.path().join("clone");
    git(dir.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
    git(&clone, &["switch", "-q", "-c", "work"]);
    // The work reaches the clone first, as a branch would, and merges on the remote later.
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "squashed"]);
    let squashed = git(&origin, &["rev-parse", "HEAD"]);
    let record = Repository {
        id: "code".to_owned(),
        owner: Value::None,
        name: Value::None,
        default_branch: Value::One("main".to_owned()),
    };
    let ancestry = GitAncestry::new(|_: &Repository| Some(clone.clone()));
    assert_eq!(ancestry.check(&record, &squashed), Check::UnknownCommit, "not fetched yet");
    ancestry.refresh(&record).unwrap();
    assert_eq!(ancestry.check(&record, &squashed), Check::Merged);
    assert_eq!(git(&clone, &["branch", "--show-current"]), "work", "the checkout is untouched");
    // A clone with no default branch to look in fetches it too, as before its first fetch.
    git(&clone, &["update-ref", "-d", "refs/remotes/origin/main"]);
    git(&clone, &["branch", "-q", "-D", "main"]);
    let fresh = GitAncestry::new(|_: &Repository| Some(clone.clone()));
    assert!(matches!(fresh.check(&record, &squashed), Check::NoBranch(_)));
    fresh.refresh(&record).unwrap();
    assert_eq!(fresh.check(&record, &squashed), Check::Merged);
    std::fs::remove_dir_all(&origin).unwrap();
    assert!(ancestry.refresh(&record).is_err(), "a fetch that fails says so");
}
