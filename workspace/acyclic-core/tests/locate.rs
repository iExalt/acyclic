//! Finding a code repository's tracker from its remotes, against a real Git repository.

use std::path::Path;
use std::process::Command;

use acyclic_core::locate::{CONFIG_PATH, LocateError, Trackers};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git").arg("-C").arg(dir).args(args).status().unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn a_repository_finds_its_tracker_by_origin_first() {
    let home = TempDir::new().unwrap();
    let config = home.path().join(CONFIG_PATH);
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        "[prefixes]\n\"github.com/upstream-org\" = \"~/t/upstream\"\n\"github.com/my-org\" = \"~/t/mine\"\n",
    )
    .unwrap();
    let trackers = Trackers::load(home.path()).unwrap();

    let repo = home.path().join("code");
    git(home.path(), &["init", "-q", repo.to_str().unwrap()]);
    assert_eq!(trackers.for_repo(&repo, home.path()), Err(LocateError::NoRemote));
    git(&repo, &["remote", "add", "a-fork", "https://github.com/upstream-org/code.git"]);
    git(&repo, &["remote", "add", "origin", "git@github.com:My-Org/code.git"]);
    assert_eq!(trackers.for_repo(&repo, home.path()), Ok(home.path().join("t/mine")));

    git(&repo, &["remote", "set-url", "origin", "https://gitlab.test/my-org/code"]);
    assert_eq!(trackers.for_repo(&repo, home.path()), Ok(home.path().join("t/upstream")));
    git(&repo, &["remote", "remove", "a-fork"]);
    assert_eq!(
        trackers.for_repo(&repo, home.path()),
        Err(LocateError::NotMapped(vec!["gitlab.test/my-org/code".to_owned()]))
    );
}
