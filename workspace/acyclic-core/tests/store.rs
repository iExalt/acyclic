//! The store against real Git repositories: atomic batches, races, crashes, squashes and replica identity.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use acyclic_core::state::{Error as StateError, Key, Millis, ReplicaId, State};
use acyclic_core::store::{self, BatchError, Hooks, Meta, Outcome, StoreError, Tracker};
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
    store::init_replica(&path, &ReplicaId::new(replica).unwrap()).unwrap();
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
    let events = log_lines(&path, segment, "_/15.jsonl");
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
    let outcome =
        tracker.batch(&meta(), NOW, false, &no_project, |_, _| Ok::<(), StateError>(())).unwrap();
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
            Ok::<(), StateError>(())
        })
        .unwrap();
    let segment = "refs/heads/log/mac/2026-10/0001";
    let events = log_lines(&path, segment, "p0/15.jsonl");
    assert_eq!(events.len(), 1);
    assert_eq!((events[0]["run"].clone(), events[0]["seq"].clone()), (json!("r1"), json!(7)));
    assert_eq!(events[0]["actor_record"]["agent"], json!(["claude"]));
    assert_eq!(log_lines(&path, segment, "_/15.jsonl")[0]["writes"][0]["kind"], "run");
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

const WRITERS: usize = 4;
const BATCHES: usize = 5;

/// The child half of the concurrency test: one writer process running its batches.
#[test]
fn writer_child() {
    let (Ok(repo), Ok(writer)) =
        (std::env::var("ACYCLIC_WRITER_REPO"), std::env::var("ACYCLIC_WRITER"))
    else {
        return;
    };
    let (tracker, _) = Tracker::open(Path::new(&repo)).unwrap();
    let key = item(&format!("w{writer}"));
    for batch in 0..BATCHES {
        let field = format!("f{batch}");
        tracker
            .batch(&meta(), NOW, false, &no_project, |replica, at| {
                replica.write(&key, &field, "actor/user", at, Some(json!(batch))).map(|_| ())
            })
            .unwrap();
    }
}

#[test]
fn writer_processes_in_separate_worktrees_lose_and_duplicate_nothing() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (first, _) = Tracker::open(&path).unwrap();
    write(&first, "seed", "f", json!(0));
    let children: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let worktree = dir.path().join(format!("wt{writer}"));
            let target = worktree.to_str().unwrap();
            git(&path, &["worktree", "add", "-q", "--detach", target, "refs/heads/state/mac"]);
            Command::new(std::env::current_exe().unwrap())
                .args(["writer_child", "--exact", "--nocapture"])
                .env("ACYCLIC_WRITER_REPO", &worktree)
                .env("ACYCLIC_WRITER", writer.to_string())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success(), "no batch failed");
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
    let events = log_lines(&path, "refs/heads/log/mac/2026-10/0001", "_/15.jsonl");
    assert_eq!(events.len(), 1 + WRITERS * BATCHES);
}

#[test]
fn a_batch_that_loses_the_race_retries_on_the_new_tip() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "f", json!(1));
    let refs = ["refs/heads/state/mac", "refs/heads/log/mac/2026-10/0001"];
    let before: Vec<_> = refs.iter().map(|r| git(&path, &["rev-parse", r])).collect();
    // Another writer's batch, rewound so the hook can replay it mid-batch.
    write(&tracker, "b1", "f", json!(2));
    let theirs: Vec<_> = refs.iter().map(|r| git(&path, &["rev-parse", r])).collect();
    for (reference, tip) in refs.iter().zip(&before) {
        git(&path, &["update-ref", reference, tip]);
    }
    let moved = std::cell::Cell::new(false);
    let replay = {
        let path = path.clone();
        let theirs = theirs.clone();
        move || {
            if !moved.replace(true) {
                for (reference, tip) in refs.iter().zip(&theirs) {
                    git(&path, &["update-ref", reference, tip]);
                }
            }
        }
    };
    let hooks = Hooks { after_objects: Some(Box::new(replay)), after_prepare: None };
    let outcome = tracker
        .batch_with(
            &meta(),
            NOW,
            false,
            &no_project,
            |replica, at| {
                replica.write(&item("c1"), "f", "actor/user", at, Some(json!(3))).map(|_| ())
            },
            &hooks,
        )
        .unwrap();
    assert!(matches!(outcome, Outcome::Committed { retries, .. } if retries >= 1), "{outcome:?}");
    let (_, state) = tracker.load().unwrap();
    for id in ["a1", "b1", "c1"] {
        assert!(state.field(&item(id), "f").is_some(), "{id} is present");
    }
    assert_eq!(log_lines(&path, refs[1], "_/15.jsonl").len(), 3);
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
    // The batch's scratch refs may be left behind; only the real refs count.
    let refs = || git(&path, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads/"]);
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
    let events = log_lines(&path, "refs/heads/log/mac/2026-10/0001", "_/15.jsonl");
    assert_eq!(events.len(), 2, "the crashed batch left no event");
}

#[test]
fn a_large_batch_writes_a_pack_not_loose_objects() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    tracker
        .batch(&meta(), NOW, false, &no_project, |replica, at| {
            (0..500).try_for_each(|n| {
                replica
                    .write(&item(&format!("i{n}")), "f", "actor/user", at, Some(json!(n)))
                    .map(|_| ())
            })
        })
        .unwrap();
    let counts = git(&path, &["count-objects", "-v"]);
    let field = |name: &str| -> u32 {
        counts.lines().find_map(|l| l.strip_prefix(name)).unwrap().trim().parse().unwrap()
    };
    // git unpacks an import of under 100 objects (fastimport.unpackLimit); this one is several hundred.
    assert!(field("count:") < 100, "{counts}");
    assert_eq!(field("packs:"), 1, "{counts}");
}

#[test]
fn the_store_repacks_once_packs_pile_up() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    // In the foreground, so the test sees the result, and after a few packs rather than git's 50.
    git(&path, &["config", "gc.autoDetach", "false"]);
    git(&path, &["config", "gc.autoPackLimit", "3"]);
    let (tracker, _) = Tracker::open(&path).unwrap();
    for n in 0..5 {
        write(&tracker, "a1", "f", json!(n));
    }
    let counts = git(&path, &["count-objects", "-v"]);
    let packs: u32 =
        counts.lines().find_map(|l| l.strip_prefix("packs:")).unwrap().trim().parse().unwrap();
    assert!(packs <= 3, "each batch writes a pack, and gc folds them: {counts}");
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
        Err(BatchError::Store(StoreError::Locked { path: locked, age_seconds })) => {
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
fn the_cached_state_matches_git_after_batches_squashes_and_joins() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let other = tracker(&dir, "phone");
    let (tracker, _) = Tracker::open(&path).unwrap();
    let (phone, _) = Tracker::open(&other).unwrap();
    let agree = |tracker: &Tracker| {
        let (tip, cached) = tracker.load().unwrap();
        assert_eq!((tip, cached), tracker.load_strict().unwrap());
    };
    // Enough state that a small batch's changes make a delta smaller than a sixteenth of the base.
    tracker
        .batch(&meta(), NOW, false, &no_project, |replica, at| {
            (0..40).try_for_each(|n| {
                let value = json!("x".repeat(100));
                replica
                    .write(&item(&format!("b{n}")), "f", "actor/user", at, Some(value))
                    .map(|_| ())
            })
        })
        .unwrap();
    agree(&tracker);
    // Later batches extend the cache with a delta rather than rewriting it.
    write(&tracker, "a2", "f", json!(2));
    write(&tracker, "a1", "f", json!(3));
    assert!(path.join(".git/acyclic/cache/mac.delta.json").exists());
    agree(&tracker);
    tracker.squash().unwrap().unwrap();
    agree(&tracker);
    write(&phone, "b1", "f", json!(2));
    git(
        &path,
        &["fetch", "-q", other.to_str().unwrap(), "refs/heads/state/phone:refs/remotes/phone"],
    );
    tracker.join_ref("refs/remotes/phone").unwrap();
    agree(&tracker);

    // A damaged cache is a miss: the read goes to Git and rewrites it.
    let cache = path.join(".git/acyclic/cache/mac.json");
    std::fs::write(&cache, "{not json\n").unwrap();
    agree(&tracker);
    assert_ne!(std::fs::read_to_string(&cache).unwrap(), "{not json\n");
}

fn assert_fresh(renamed: Option<&ReplicaId>, base: &str) -> String {
    let renamed = renamed.expect("a fresh id").as_str().to_owned();
    let suffix = renamed.strip_prefix(&format!("{base}-")).expect("the base id, then a suffix");
    assert!(!suffix.is_empty());
    renamed
}

#[test]
fn a_replica_that_lost_its_state_takes_a_new_id() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "a1", "f", json!(1));
    // The Mac's own ref is lost.
    git(&path, &["update-ref", "-d", "refs/heads/state/mac"]);
    let (reopened, renamed) = Tracker::open(&path).unwrap();
    let renamed = assert_fresh(renamed.as_ref(), "mac");
    assert_eq!(reopened.replica().as_str(), renamed);
    assert_eq!(git(&path, &["config", store::REPLICA_CONFIG]), renamed);
    // The new id is used until it writes, then kept.
    write(&reopened, "a1", "f", json!(2));
    let (again, none) = Tracker::open(&path).unwrap();
    assert_eq!((again.replica().as_str(), none), (renamed.as_str(), None));
}

#[test]
fn fresh_clones_of_a_replica_take_different_ids() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "a1", "f", json!(1));
    // Clones that fetched no state refs carry the Mac's id over, with nothing to remember it by.
    let ids: Vec<_> = (0..2)
        .map(|n| {
            let clone = dir.path().join(format!("clone{n}"));
            git(dir.path(), &["init", "-q", clone.to_str().unwrap()]);
            git(&clone, &["config", store::REPLICA_CONFIG, "mac"]);
            let (tracker, renamed) = Tracker::open(&clone).unwrap();
            assert_eq!(tracker.replica(), renamed.as_ref().unwrap());
            assert_fresh(renamed.as_ref(), "mac")
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn two_clones_writing_as_one_replica_are_caught_at_sync() {
    let dir = TempDir::new().unwrap();
    let one = tracker(&dir, "mac");
    // A copied clone, config and all, before either has written.
    let two = tracker(&dir, "mac-copy");
    store::init_replica(&two, &ReplicaId::new("mac").unwrap()).unwrap();
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
    assert!(matches!(
        result,
        Err(BatchError::Store(StoreError::State(acyclic_core::state::Error::ClockJump { .. })))
    ));
}

/// A batch at `at` writing one field of an item in project `p0` and one in no project.
fn write_month(tracker: &Tracker, at: Millis, value: i64) {
    let root_of = |_: &State, key: &Key| (key.id.starts_with('p')).then(|| "p0".to_owned());
    tracker
        .batch(&meta(), at, false, &root_of, |replica, at| {
            replica.write(&item("p1"), "f", "actor/user", at, Some(json!(value)))?;
            replica.write(&item("x1"), "f", "actor/user", at, Some(json!(value)))?;
            Ok::<(), StateError>(())
        })
        .unwrap();
}

const JULY: Millis = 1_783_296_000_000; // 2026-07-06
const AUGUST: Millis = 1_786_060_800_000; // 2026-08-07
const SEPTEMBER: Millis = 1_788_739_200_000; // 2026-09-07

fn log_refs(path: &Path) -> String {
    git(path, &["for-each-ref", "--format=%(refname)", "refs/heads/log/"])
}

#[test]
fn pruning_replaces_expired_segments_with_a_new_generation() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write_month(&tracker, JULY, 1);
    write_month(&tracker, AUGUST, 2);
    // September holds only the project's file.
    let root_of = |_: &State, _: &Key| Some("p0".to_owned());
    tracker
        .batch(&meta(), SEPTEMBER, false, &root_of, |replica, at| {
            replica.write(&item("p1"), "f", "actor/user", at, Some(json!(3))).map(|_| ())
        })
        .unwrap();
    write(&tracker, "x1", "f", json!(4)); // October, the current month

    let retention = |root: &str| (root == "p0").then_some(1);
    // August's archived subtrees have no records yet, so it stays whole.
    let archived_ok = |_: &str, month: &str| month != "2026-08";
    let dropped = tracker.prune(NOW, &retention, &archived_ok).unwrap();
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(
        git(&path, &["for-each-ref", "refs/acyclic/scratch/log"]),
        "",
        "no scratch ref keeps pruned events reachable"
    );
    // September's events are under a month old in the worst case, so they stay until November.
    assert_eq!(
        log_refs(&path),
        "refs/heads/log/mac/2026-07/0002\nrefs/heads/log/mac/2026-08/0001\nrefs/heads/log/mac/2026-09/0001\nrefs/heads/log/mac/2026-10/0001"
    );
    let july = "refs/heads/log/mac/2026-07/0002";
    assert_eq!(git(&path, &["ls-tree", "-r", "--name-only", july]), "_/06.jsonl");
    assert_eq!(
        git(&path, &["rev-list", "--count", july]),
        "1",
        "the new generation has no history"
    );
    assert_eq!(log_lines(&path, july, "_/06.jsonl").len(), 1);
    assert!(
        tracker.prune(NOW, &retention, &archived_ok).unwrap().is_empty(),
        "pruning again changes nothing"
    );
    let november = 1_793_491_200_000; // 2026-11-01
    assert_eq!(tracker.prune(november, &retention, &archived_ok).unwrap().len(), 1);
    assert!(
        !log_refs(&path).contains("2026-09"),
        "September's only file went, and its segment with it"
    );
}

/// A bare remote whose pre-receive hook refuses forced updates to log and usage segments, like the tracker's ruleset.
fn remote_with_ruleset(dir: &TempDir) -> PathBuf {
    let remote = dir.path().join("remote.git");
    git(dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
    let hook = remote.join("hooks/pre-receive");
    std::fs::write(
        &hook,
        r#"#!/bin/sh
zero=$(git hash-object --stdin </dev/null | tr '0-9a-f' '0')
while read -r old new ref; do
  case "$ref" in
    refs/heads/log/*|refs/heads/usage/*)
      if [ "$old" != "$zero" ] && [ "$new" != "$zero" ] && ! git merge-base --is-ancestor "$old" "$new"; then
        echo "ruleset: $ref may not be force-pushed" >&2
        exit 1
      fi ;;
  esac
done
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    remote
}

fn remote_refs(remote: &Path) -> String {
    git(remote, &["for-each-ref", "--format=%(refname) %(objectname)"])
}

#[test]
fn pushes_are_atomic_and_pass_the_ruleset() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let remote = remote_with_ruleset(&dir);
    let remote_url = remote.to_str().unwrap();
    let (tracker, _) = Tracker::open(&path).unwrap();
    write_month(&tracker, JULY, 1);
    write(&tracker, "x1", "f", json!(2));
    assert_eq!(tracker.push(remote_url).unwrap(), 3, "state and two segments");
    assert_eq!(tracker.push(remote_url).unwrap(), 0, "nothing new");

    // A squash rewrites the state ref; the lease lets it through.
    tracker.squash().unwrap();
    write(&tracker, "x1", "f", json!(3));
    assert_eq!(tracker.push(remote_url).unwrap(), 2);

    // Pruning deletes one generation and creates the next: both pass the ruleset.
    let pruned = tracker.prune(NOW, &|_| Some(1), &|_, _| true).unwrap();
    assert!(!pruned.is_empty());
    tracker.push(remote_url).unwrap();
    let local = git(&path, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads/"]);
    assert_eq!(remote_refs(&remote), local);
}

#[test]
fn a_forced_rewrite_of_a_segment_is_refused_and_moves_nothing() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let remote = remote_with_ruleset(&dir);
    let remote_url = remote.to_str().unwrap();
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "x1", "f", json!(1));
    write(&tracker, "x1", "f", json!(2));
    tracker.push(remote_url).unwrap();
    let before = remote_refs(&remote);

    // Rewind the log segment locally and write again: the segment no longer fast-forwards.
    let segment = "refs/heads/log/mac/2026-10/0001";
    git(&path, &["update-ref", segment, &format!("{segment}~1")]);
    write(&tracker, "x1", "f", json!(3));
    // The store never forces a segment, so git refuses it before sending, and the atomic push moves nothing.
    match tracker.push(remote_url) {
        Err(StoreError::Rejected { reference, .. }) => assert_eq!(reference, segment),
        other => panic!("expected a rejection, got {other:?}"),
    }
    assert_eq!(remote_refs(&remote), before, "the state ref didn't move either");
    // Forced by hand, the remote's ruleset refuses it.
    let forced = Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["push", "--force", remote_url, &format!("{segment}:{segment}")])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(!forced.status.success());
    assert!(String::from_utf8_lossy(&forced.stderr).contains("may not be force-pushed"));
    assert_eq!(remote_refs(&remote), before);
}

#[test]
fn a_push_leaves_segments_this_clone_never_had() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let remote = remote_with_ruleset(&dir);
    let remote_url = remote.to_str().unwrap();
    let (tracker, _) = Tracker::open(&path).unwrap();
    write_month(&tracker, JULY, 1);
    write(&tracker, "x1", "f", json!(2));
    tracker.push(remote_url).unwrap();
    let before = remote_refs(&remote);
    // Like a clone that fetched only state refs.
    for segment in log_refs(&path).lines() {
        git(&path, &["update-ref", "-d", segment]);
    }
    assert_eq!(tracker.push(remote_url).unwrap(), 0);
    assert_eq!(remote_refs(&remote), before, "the audit history survives");
}

#[test]
fn a_push_after_the_remote_moved_is_refused_until_fetched() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let remote = remote_with_ruleset(&dir);
    let remote_url = remote.to_str().unwrap();
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "x1", "f", json!(1));
    write(&tracker, "x1", "f", json!(2));
    tracker.push(remote_url).unwrap();
    // Something else moves the remote's state ref behind this clone's back.
    git(&remote, &["update-ref", "refs/heads/state/mac", "refs/heads/state/mac~1"]);
    let before = remote_refs(&remote);
    write(&tracker, "x1", "f", json!(3));
    assert!(matches!(tracker.push(remote_url), Err(StoreError::RemoteMoved)));
    assert_eq!(remote_refs(&remote), before, "nothing moved");
    // Once fetched (and joined, which here adds nothing new), the push goes through.
    tracker.fetch(remote_url).unwrap();
    tracker.push(remote_url).unwrap();
    assert_eq!(
        git(&remote, &["rev-parse", "refs/heads/state/mac"]),
        git(&path, &["rev-parse", "refs/heads/state/mac"])
    );
}

#[test]
fn a_plain_fetch_does_not_vouch_for_a_push() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let remote = remote_with_ruleset(&dir);
    git(&path, &["remote", "add", "origin", remote.to_str().unwrap()]);
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "x1", "f", json!(1));
    write(&tracker, "x1", "f", json!(2));
    tracker.push("origin").unwrap();
    git(&remote, &["update-ref", "refs/heads/state/mac", "refs/heads/state/mac~1"]);
    // The user, an editor or another tool fetches with the default refspec, joining nothing.
    git(&path, &["fetch", "-q", "origin"]);
    write(&tracker, "x1", "f", json!(3));
    assert!(matches!(tracker.push("origin"), Err(StoreError::RemoteMoved)));
}
