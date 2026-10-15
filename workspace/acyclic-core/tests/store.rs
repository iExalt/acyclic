//! The store against real Git repositories: atomic batches, races, crashes, squashes and replica identity.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use acyclic_core::state::{Key, Millis, State};
use acyclic_core::store::{self, Hooks, Meta, Outcome, StoreError, Tracker};
use serde_json::json;
use tempfile::TempDir;

/// 2026-10-15T00:00:00Z.
const NOW: Millis = 1_792_022_400_000;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn tracker(dir: &TempDir, replica: &str) -> PathBuf {
    let path = dir.path().join(replica);
    git(dir.path(), &["init", "-q", path.to_str().unwrap()]);
    git(&path, &["config", store::REPLICA_CONFIG, replica]);
    // Durability isn't under test, and fsync makes each write about ten times slower on macOS.
    git(&path, &["config", "core.fsync", "none"]);
    path
}

fn meta() -> Meta {
    Meta { actor: "actor/user".to_owned(), run: None }
}

fn no_project(_: &State, _: &Key) -> Option<String> {
    None
}

fn item(id: &str) -> Key {
    Key::new("item", id).unwrap()
}

fn write(tracker: &Tracker, id: &str, field: &str, value: serde_json::Value) -> Outcome {
    let key = item(id);
    tracker
        .batch(&meta(), NOW, false, &no_project, |replica, at| {
            replica.write(&key, field, "actor/user", at, Some(value.clone())).map(|_| ())
        })
        .unwrap()
}

fn log_lines(path: &Path, reference: &str, file: &str) -> Vec<serde_json::Value> {
    git(path, &["show", &format!("{reference}:{file}")])
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_batch_moves_state_and_log_together() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, renamed) = Tracker::open(&path).unwrap();
    assert_eq!(renamed, None);
    write(&tracker, "a1", "title", json!("first"));
    write(&tracker, "a1", "title", json!("second"));

    let (tip, state) = tracker.load().unwrap();
    assert!(tip.is_some());
    let values: Vec<_> =
        state.field(&item("a1"), "title").unwrap().values().map(|v| v.value.clone()).collect();
    assert_eq!(values, vec![json!("second")]);
    assert_eq!(git(&path, &["rev-list", "--count", "refs/heads/state/mac"]), "2");

    let segment = "refs/heads/log/mac/2026-10/0001";
    let events = log_lines(&path, segment, "_.jsonl");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["actor"], "actor/user");
    assert_eq!(events[1]["writes"][0]["field"], "title");
    assert_eq!(events[1]["writes"][0]["values"][0]["value"], "second");
    // Nothing was checked out.
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1, "only .git");
}

#[test]
fn an_unchanged_batch_commits_nothing() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    let outcome = tracker.batch(&meta(), NOW, false, &no_project, |_, _| Ok(())).unwrap();
    assert_eq!(outcome, Outcome::Unchanged);
    assert!(tracker.load().unwrap().0.is_none());
}

#[test]
fn events_go_to_their_project_roots_file_with_run_and_actor() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    let root_of = |_: &State, key: &Key| (key.id.starts_with('p')).then(|| "p0".to_owned());
    let actor = Key::new("run", "r1").unwrap();
    let meta = Meta { actor: "run/r1".to_owned(), run: Some(("r1".to_owned(), 7)) };
    tracker
        .batch(&meta, NOW, false, &root_of, |replica, at| {
            replica.write(&actor, "agent", "actor/user", at, Some(json!("claude")))?;
            replica.write(&item("p1"), "title", "run/r1", at, Some(json!("in a project")))?;
            Ok(())
        })
        .unwrap();
    let segment = "refs/heads/log/mac/2026-10/0001";
    let events = log_lines(&path, segment, "p0.jsonl");
    assert_eq!(events.len(), 1);
    assert_eq!((events[0]["run"].clone(), events[0]["seq"].clone()), (json!("r1"), json!(7)));
    assert_eq!(events[0]["actor_record"]["agent"], json!(["claude"]));
    assert_eq!(log_lines(&path, segment, "_.jsonl")[0]["writes"][0]["kind"], "run");
}

#[test]
fn a_new_month_starts_a_new_segment() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "f", json!(1));
    let november = 1_793_491_200_000; // 2026-11-01
    tracker
        .batch(&meta(), november, false, &no_project, |replica, at| {
            replica.write(&item("a1"), "f", "actor/user", at, Some(json!(2))).map(|_| ())
        })
        .unwrap();
    let refs = git(&path, &["for-each-ref", "--format=%(refname)", "refs/heads/log/"]);
    assert_eq!(refs, "refs/heads/log/mac/2026-10/0001\nrefs/heads/log/mac/2026-11/0001");
}

#[test]
fn writers_in_separate_worktrees_lose_and_duplicate_nothing() {
    const WRITERS: usize = 4;
    const BATCHES: usize = 10;
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (first, _) = Tracker::open(&path).unwrap();
    write(&first, "seed", "f", json!(0));
    let mut handles = Vec::new();
    for writer in 0..WRITERS {
        let worktree = dir.path().join(format!("wt{writer}"));
        git(
            &path,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                worktree.to_str().unwrap(),
                "refs/heads/state/mac",
            ],
        );
        handles.push(std::thread::spawn(move || {
            let (tracker, _) = Tracker::open(&worktree).unwrap();
            for batch in 0..BATCHES {
                let key = item(&format!("w{writer}"));
                let field = format!("f{batch}");
                tracker
                    .batch(&meta(), NOW, false, &no_project, |replica, at| {
                        replica
                            .write(&key, &field, "actor/user", at, Some(json!(batch)))
                            .map(|_| ())
                    })
                    .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().expect("no batch failed");
    }
    let (_, state) = first.load().unwrap();
    let mut dots = std::collections::BTreeSet::new();
    for writer in 0..WRITERS {
        let entity = state.entity(&item(&format!("w{writer}"))).unwrap();
        assert_eq!(entity.len(), BATCHES, "writer {writer} lost writes");
        for register in entity.values() {
            assert_eq!(register.len(), 1);
            assert!(dots.insert(register.keys().next().unwrap().clone()), "a duplicated dot");
        }
    }
    let events = log_lines(&path, "refs/heads/log/mac/2026-10/0001", "_.jsonl");
    assert_eq!(events.len(), 1 + WRITERS * BATCHES);
}

/// The child half of the crash tests: runs one batch that aborts at the failpoint named in the environment.
#[test]
fn crash_child() {
    let (Ok(repo), Ok(point)) =
        (std::env::var("ACYCLIC_CRASH_REPO"), std::env::var("ACYCLIC_CRASH_AT"))
    else {
        return;
    };
    let (tracker, _) = Tracker::open(Path::new(&repo)).unwrap();
    let abort: Box<dyn Fn()> = Box::new(|| std::process::abort());
    let hooks = match point.as_str() {
        "after-objects" => Hooks { after_objects: Some(abort), after_prepare: None },
        _ => Hooks { after_objects: None, after_prepare: Some(abort) },
    };
    let key = item("a1");
    let _ = tracker.batch_with(
        &meta(),
        NOW,
        false,
        &no_project,
        |replica, at| {
            replica.write(&key, "f", "actor/user", at, Some(json!("crashed"))).map(|_| ())
        },
        &hooks,
    );
    unreachable!("the failpoint aborts");
}

fn crash_at(point: &str) {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "f", json!("before"));
    let refs = || git(&path, &["for-each-ref", "--format=%(refname) %(objectname)"]);
    let before = refs();

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["crash_child", "--exact", "--nocapture"])
        .env("ACYCLIC_CRASH_REPO", &path)
        .env("ACYCLIC_CRASH_AT", point)
        .status()
        .unwrap();
    assert!(!status.success(), "the child aborted");
    assert_eq!(refs(), before, "neither the state ref nor the log ref moved");
    // git aborts an unfinished transaction when its input closes, so no lock is left and the next batch succeeds.
    write(&tracker, "a1", "f", json!("after"));
    let events = log_lines(&path, "refs/heads/log/mac/2026-10/0001", "_.jsonl");
    assert_eq!(events.len(), 2, "the crashed batch left no event");
}

#[test]
fn a_crash_before_the_transaction_moves_nothing() {
    crash_at("after-objects");
}

#[test]
fn a_crash_after_prepare_moves_nothing() {
    crash_at("after-prepare");
}

#[test]
fn a_held_lock_fails_clearly_and_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "f", json!(1));
    let lock = path.join(".git/refs/heads/state/mac.lock");
    std::fs::write(&lock, "").unwrap();
    let key = item("a1");
    let result = tracker.batch(&meta(), NOW, false, &no_project, |replica, at| {
        replica.write(&key, "f", "actor/user", at, Some(json!(2))).map(|_| ())
    });
    match result {
        Err(StoreError::Locked { path: locked, age_seconds }) => {
            assert!(locked.ends_with("refs/heads/state/mac.lock"), "{}", locked.display());
            assert!(age_seconds.is_some());
        },
        other => panic!("expected a lock error, got {other:?}"),
    }
    assert!(lock.exists(), "never removed automatically");
}

#[test]
fn a_squashed_ref_holds_the_same_state() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    for n in 0..3 {
        write(&tracker, "a1", &format!("f{n}"), json!(n));
    }
    let (_, before) = tracker.load().unwrap();
    let tree = git(&path, &["rev-parse", "refs/heads/state/mac^{tree}"]);
    tracker.squash().unwrap().unwrap();
    assert_eq!(git(&path, &["rev-list", "--count", "refs/heads/state/mac"]), "1");
    assert_eq!(git(&path, &["rev-parse", "refs/heads/state/mac^{tree}"]), tree);

    // Another replica joining the squashed tip gets what it would have from the history.
    let (_, after) = tracker.load().unwrap();
    assert_eq!(after, before);
    let mut joined = State::new();
    joined.join(&after);
    assert_eq!(joined, before);
    write(&tracker, "a1", "f9", json!(9));
}

#[test]
fn a_replica_that_lost_its_state_takes_a_new_id() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "a1", "f", json!(1));
    // The server's replica has joined the Mac's state.
    git(&path, &["config", store::REPLICA_CONFIG, "server"]);
    let (server, _) = Tracker::open(&path).unwrap();
    server.join_ref("refs/heads/state/mac").unwrap();
    // The Mac's own ref is lost.
    git(&path, &["update-ref", "-d", "refs/heads/state/mac"]);
    git(&path, &["config", store::REPLICA_CONFIG, "mac"]);
    let (reopened, renamed) = Tracker::open(&path).unwrap();
    assert_eq!(renamed.as_ref().map(|r| r.as_str()), Some("mac-2"));
    assert_eq!(reopened.replica().as_str(), "mac-2");
    assert_eq!(git(&path, &["config", store::REPLICA_CONFIG]), "mac-2");
}

#[test]
fn two_clones_writing_as_one_replica_are_caught_at_sync() {
    let dir = TempDir::new().unwrap();
    let one = tracker(&dir, "mac");
    let two = tracker(&dir, "mac-copy");
    git(&two, &["config", store::REPLICA_CONFIG, "mac"]);
    let (first, _) = Tracker::open(&one).unwrap();
    let (second, _) = Tracker::open(&two).unwrap();
    write(&first, "a1", "f", json!(1));
    write(&first, "a1", "g", json!(2));
    write(&second, "b1", "f", json!(3));
    // The second clone fetches the first's state ref and joins it.
    git(
        &two,
        &["fetch", "-q", one.to_str().unwrap(), "refs/heads/state/mac:refs/remotes/one/state/mac"],
    );
    let err = second.join_ref("refs/remotes/one/state/mac").unwrap_err();
    assert!(matches!(err, StoreError::State(acyclic_core::state::Error::IdReused { .. })), "{err}");
}

#[test]
fn a_backward_clock_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "f", json!(1));
    let key = item("a1");
    let shared = Arc::new(key);
    let result = tracker.batch(&meta(), NOW - 3_600_000, false, &no_project, |replica, at| {
        replica.write(&shared, "f", "actor/user", at, Some(json!(2))).map(|_| ())
    });
    assert!(matches!(result, Err(StoreError::State(acyclic_core::state::Error::ClockJump { .. }))));
}
