//! Done inside a thread, merged across: readiness over stored state, against a scratch code repository.

use std::path::Path;
use std::process::Command;

use acyclic_core::model::{self, By, Evidence, ItemState, LinkKind, Model};
use acyclic_core::ready::{GitAncestry, Readiness, Status, Wait};
use acyclic_core::schema::{item, kind, repository};
use acyclic_core::scope::{self, ScopeEdit};
use acyclic_core::state::{Error, Key, ReplicaId};
use acyclic_core::store::{self, Meta, Tracker};
use tempfile::TempDir;

const NOW: i64 = 1_792_022_400_000;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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
fn a_dependency_in_another_thread_is_met_once_its_commit_merges() {
    let dir = TempDir::new().unwrap();
    let code = dir.path().join("code");
    git(dir.path(), &["init", "-q", "-b", "main", code.to_str().unwrap()]);
    git(&code, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(&code, &["switch", "-q", "-c", "feature"]);
    git(&code, &["commit", "-q", "--allow-empty", "-m", "the work"]);
    let commit = git(&code, &["rev-parse", "HEAD"]);
    git(&code, &["switch", "-q", "main"]);

    let path = dir.path().join("tracker");
    git(dir.path(), &["init", "-q", path.to_str().unwrap()]);
    store::init_replica(&path, &ReplicaId::new("mac").unwrap()).unwrap();
    let (tracker, _) = Tracker::open(&path).unwrap();
    let meta = Meta { actor: "actor/user".to_owned(), run: None };
    tracker
        .batch(&meta, NOW, false, &|_, _| None, |replica, at| {
            let by = By { actor: "actor/user", at };
            let record = Key::new(kind::REPOSITORY, "code").unwrap();
            model::set(replica, &record, repository::DEFAULT_BRANCH, Some(&"main"), by)
                .map_err(state_error)?;
            for (id, thread) in [("dep", "t1"), ("same", "t1"), ("other", "t2")] {
                model::add_item(replica, id, id, by).map_err(state_error)?;
                let member =
                    ScopeEdit::Member { item: id.to_owned(), thread: Some(thread.to_owned()) };
                scope::edit(replica, &member, by).map_err(|e| Error::Format(e.to_string()))?;
            }
            for id in ["same", "other"] {
                model::link(replica, id, "dep", LinkKind::Blocks, by).map_err(state_error)?;
            }
            model::set_state(replica, "dep", ItemState::Done, by).map_err(state_error)?;
            let evidence = Evidence {
                repository: Some("code".into()),
                commit: Some(commit.clone()),
                note: None,
            };
            let dep = model::item_key("dep").map_err(state_error)?;
            model::set(replica, &dep, item::EVIDENCE, Some(&evidence), by).map_err(state_error)?;
            Ok::<(), Error>(())
        })
        .unwrap();

    let ancestry = GitAncestry::new(|_: &model::Repository| Some(code.clone()));
    let check = |id: &str| {
        let (_, state) = tracker.load().unwrap();
        let model = Model::read(&state).unwrap();
        Readiness::new(&state, &model, NOW, &ancestry).of(id).unwrap().unwrap()
    };
    assert_eq!(check("same"), Status::Ready, "met at done inside its thread");
    assert!(
        matches!(check("other"), Status::Blocked { waiting } if waiting["dep"] == Wait::InReview)
    );
    assert_eq!(check("dep"), Status::InReview);

    git(&code, &["merge", "-q", "--ff-only", "feature"]);
    assert_eq!(check("other"), Status::Ready, "met once merged");
    assert_eq!(check("dep"), Status::Done);

    // A clone of the repository reads the default branch from its remote-tracking ref.
    let clone = dir.path().join("clone");
    git(dir.path(), &["clone", "-q", code.to_str().unwrap(), clone.to_str().unwrap()]);
    let cloned = GitAncestry::new(|_: &model::Repository| Some(clone.clone()));
    let (_, state) = tracker.load().unwrap();
    let model = Model::read(&state).unwrap();
    assert_eq!(
        Readiness::new(&state, &model, NOW, &cloned).of("other").unwrap(),
        Some(Status::Ready)
    );
}

fn state_error(err: model::WriteError) -> acyclic_core::state::Error {
    match err {
        model::WriteError::State(err) => err,
        other => acyclic_core::state::Error::Format(other.to_string()),
    }
}
