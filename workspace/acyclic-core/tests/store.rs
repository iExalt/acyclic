//! The store against real Git repositories: atomic batches, races, crashes, squashes and replica identity.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use acyclic_core::state::{Error as StateError, Key, Millis, ReplicaId, State};
use acyclic_core::store::{
    self, BatchError, Hooks, Maintenance, Meta, Outcome, StoreError, Tracker,
};
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
        // Without an identity, a command that writes a reflog looks the host name up to make an email address, which
        // stalls each call by seconds on a machine with a slow resolver.
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_NAME", "acyclic test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
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
    Meta { actor: "actor/user".to_owned() }
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
fn a_state_whose_tree_names_a_missing_object_fails_to_load() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    write(&tracker, "a1", "title", json!("kept"));
    // The state as a damaged object store would hold it: its tree names a2's file, whose object isn't there.
    let index = dir.path().join("index");
    let indexed = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(args)
            .env("GIT_INDEX_FILE", &index)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    indexed(&["read-tree", "state/mac"]);
    // An object id, of whatever hash the repository uses, for content never written.
    let id = {
        let mut hashing = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["hash-object", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut hashing.stdin.take().unwrap(), b"never written").unwrap();
        String::from_utf8(hashing.wait_with_output().unwrap().stdout).unwrap().trim().to_owned()
    };
    let absent = format!("100644,{id},{}", item("a2").path());
    indexed(&["update-index", "--add", "--cacheinfo", &absent]);
    let tree = indexed(&["write-tree", "--missing-ok"]);
    let commit = git(&path, &["commit-tree", &tree, "-p", "state/mac", "-m", "damaged"]);
    git(&path, &["update-ref", "refs/heads/state/mac", &commit]);
    let Err(StoreError::Git(message)) = tracker.load_strict() else {
        panic!("a state missing a record loaded")
    };
    assert!(message.contains(&item("a2").path()) && message.contains("missing"), "{message}");
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
fn events_go_to_their_project_roots_file_with_their_actor() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    let root_of = |_: &State, key: &Key| (key.id.starts_with('p')).then(|| "p0".to_owned());
    let actor = Key::new("session", "mac").unwrap();
    let meta = Meta { actor: "session/mac".to_owned() };
    tracker
        .batch(&meta, NOW, false, &root_of, |replica, at| {
            replica.write(&actor, "agent", "actor/user", at, Some(json!("claude")))?;
            replica.write(&item("p1"), "title", "session/mac", at, Some(json!("in a project")))?;
            Ok::<(), StateError>(())
        })
        .unwrap();
    let segment = "refs/heads/log/mac/2026-10/0001";
    let events = log_lines(&path, segment, "p0/15.jsonl");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["actor"], json!("session/mac"));
    assert_eq!(events[0]["actor_record"]["agent"], json!(["claude"]));
    assert_eq!(log_lines(&path, segment, "_/15.jsonl")[0]["writes"][0]["kind"], "session");
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

/// The child half of the crash tests: runs one batch that aborts at the failpoint named in the environment.
#[test]
fn crash_child() {
    let (Ok(repo), Ok(point)) =
        (std::env::var("ACYCLIC_CRASH_REPO"), std::env::var("ACYCLIC_CRASH_AT"))
    else {
        return;
    };
    let (tracker, _) = Tracker::open(Path::new(&repo)).unwrap();
    assert_eq!(point, "after-prepare");
    let hooks = Hooks { after_prepare: Some(Box::new(|| std::process::abort())) };
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
    // The transaction ran in the dead process, as one of git does when it is killed, so its locks stay until they are
    // deleted. The next batch says so, and moves nothing.
    let key = item("a1");
    let result = tracker.batch(&meta(), NOW, false, &no_project, |replica, at| {
        replica.write(&key, "f", "actor/user", at, Some(json!("after"))).map(|_| ())
    });
    let Err(BatchError::Store(StoreError::Locked { path: lock, .. })) = result else {
        panic!("expected the dead process's lock to be reported, got {result:?}");
    };
    assert!(lock.starts_with(path.join(".git/refs/heads")), "{}", lock.display());
    for lock in ["state/mac.lock", "log/mac/2026-10/0001.lock"] {
        let _ = std::fs::remove_file(path.join(".git/refs/heads").join(lock));
    }
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

/// The repack runs beside the command, not inside it: the batch that crosses the threshold returns with the packs
/// still piled up, and they are folded soon after. A second repack isn't started while a run began lately.
#[test]
fn a_detached_repack_leaves_the_command_alone_and_follows_soon() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (tracker, _) = Tracker::open(&path).unwrap();
    let tracker = tracker.with_maintenance(Maintenance::Thread);
    let packs = || -> u32 {
        let counts = git(&path, &["count-objects", "-v"]);
        counts.lines().find_map(|l| l.strip_prefix("packs:")).unwrap().trim().parse().unwrap()
    };
    // The batch that makes the 51st pack, the first past git's limit of 50, starts the repack and returns.
    for n in 0..51 {
        write(&tracker, "a1", "f", json!(n));
    }
    let marker = path.join(".git/acyclic/maintenance");
    assert!(marker.exists() || packs() <= 3, "a run began, and is under way or done");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while packs() > 3 {
        assert!(std::time::Instant::now() < deadline, "the packs were folded: {}", packs());
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    while marker.exists() {
        assert!(std::time::Instant::now() < deadline, "the finished run cleared its marker");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    git(&path, &["fsck", "--strict", "--full"]);
}

#[test]
fn a_crash_after_prepare_moves_nothing() {
    crash_at("after-prepare");
}

/// A linked worktree of the clone is the same tracker: its configuration, refs, cache and writer lock live in the common
/// directory, so a batch from either sees the other's.
#[test]
fn a_linked_worktree_is_the_same_tracker() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    git(&path, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let linked = dir.path().join("linked");
    git(&path, &["worktree", "add", "-q", linked.to_str().unwrap()]);
    let (main, _) = Tracker::open(&path).unwrap();
    let (other, renamed) = Tracker::open(&linked).unwrap();
    assert_eq!(renamed, None, "the replica id is in the shared configuration");
    assert_eq!(other.replica(), main.replica());
    write(&main, "a1", "f", json!(1));
    write(&other, "a1", "g", json!(2));
    assert_eq!(
        other.local_dir().unwrap().canonicalize().unwrap_or_default(),
        path.join(".git").canonicalize().unwrap().join("acyclic"),
        "local files live in the common directory"
    );
    let (_, state) = main.load().unwrap();
    assert!(
        format!("{state:?}").contains('2') && format!("{state:?}").contains('1'),
        "both worktrees' writes are in one state"
    );
    assert_eq!(git(&path, &["rev-list", "--count", "refs/heads/state/mac"]), "2");
    assert_eq!(
        git(&linked, &["rev-list", "--count", "refs/heads/state/mac"]),
        "2",
        "git sees the same refs"
    );
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
    let remote = bare_remote(&dir);
    let url = remote.to_str().unwrap();
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
    phone.sync(url).unwrap();
    assert!(tracker.sync(url).unwrap().joined);
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
fn a_peer_state_this_build_cant_read_whole_still_lets_a_clone_open() {
    let dir = TempDir::new().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "a1", "f", json!(1));
    // A peer's state holding an entity file this build can't read, beside a header it can that also names an id this
    // build can't parse.
    let mut header: serde_json::Value =
        serde_json::from_str(&git(&path, &["show", "refs/heads/state/mac:state.json"])).unwrap();
    header["context"]["seen"]["Not An Id"] = json!(1);
    std::fs::write(path.join("header"), header.to_string()).unwrap();
    std::fs::write(path.join("junk"), "{ \"not\": canonical").unwrap();
    git(&path, &["read-tree", "refs/heads/state/mac"]);
    for (file, at) in [("junk", "item/junk.json"), ("header", "state.json")] {
        let blob = git(&path, &["hash-object", "-w", file]);
        git(&path, &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{at}")]);
    }
    let tree = git(&path, &["write-tree"]);
    let peer = git(&path, &["commit-tree", &tree, "-m", "peer"]);
    git(&path, &["update-ref", "refs/heads/state/pi", &peer]);
    git(&path, &["update-ref", "-d", "refs/heads/state/mac"]);
    // Opening reads only the peers' headers, which remember mac, so it takes a fresh id.
    let (_, renamed) = Tracker::open(&path).unwrap();
    assert_fresh(renamed.as_ref(), "mac");
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
fn joining_a_state_that_saw_more_of_our_writes_is_refused() {
    let dir = TempDir::new().unwrap();
    let remote = bare_remote(&dir);
    let url = remote.to_str().unwrap();
    let one = tracker(&dir, "mac");
    // A copied clone, config and all, before either has written.
    let two = tracker(&dir, "mac-copy");
    store::init_replica(&two, &ReplicaId::new("mac").unwrap()).unwrap();
    let (first, _) = Tracker::open(&one).unwrap();
    let (second, _) = Tracker::open(&two).unwrap();
    let (pi, _) = Tracker::open(&tracker(&dir, "pi")).unwrap();
    write(&first, "a1", "f", json!(1));
    write(&first, "a1", "g", json!(2));
    write(&second, "b1", "f", json!(3));
    // A peer joins the first clone's writes, and the remote then loses the first's own ref, so only the join can
    // tell the second clone that another writes as it.
    first.sync(url).unwrap();
    pi.sync(url).unwrap();
    git(&remote, &["update-ref", "-d", "refs/heads/state/mac"]);
    let err = second.sync(url).unwrap_err();
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

/// A bare remote.
fn bare_remote(dir: &TempDir) -> PathBuf {
    let remote = dir.path().join("remote.git");
    git(dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
    remote
}

/// A bare remote whose pre-receive hook refuses forced updates to log segments, like the tracker's ruleset.
fn remote_with_ruleset(dir: &TempDir) -> PathBuf {
    let remote = bare_remote(dir);
    let hook = remote.join("hooks/pre-receive");
    std::fs::write(
        &hook,
        r#"#!/bin/sh
zero=$(git hash-object --stdin </dev/null | tr '0-9a-f' '0')
while read -r old new ref; do
  case "$ref" in
    refs/heads/log/*)
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
    assert_eq!(tracker.push(remote_url).unwrap(), 3, "state and two log segments");
    assert_eq!(tracker.push(remote_url).unwrap(), 0, "nothing new");

    // A squash rewrites the state ref; the lease lets it through.
    tracker.squash().unwrap();
    write(&tracker, "x1", "f", json!(3));
    assert_eq!(tracker.push(remote_url).unwrap(), 2);

    // Pruning deletes one generation and creates the next: both pass the ruleset, and a sync sends them though the
    // state ref hasn't moved.
    let pruned = tracker.prune(NOW, &|_| Some(1), &|_, _| true).unwrap();
    assert!(
        pruned.iter().any(|dropped| dropped.starts_with("refs/heads/log/mac/2026-07/")),
        "{pruned:?}"
    );
    tracker.sync(remote_url).unwrap();
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
        // Without an identity, a command that writes a reflog looks the host name up to make an email address, which
        // stalls each call by seconds on a machine with a slow resolver.
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_NAME", "acyclic test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
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

#[test]
fn sync_joins_every_replica_through_the_remote_and_a_squash_passes_the_ruleset() {
    let dir = TempDir::new().unwrap();
    let remote = remote_with_ruleset(&dir);
    let url = remote.to_str().unwrap();
    let (mac, _) = Tracker::open(&tracker(&dir, "mac")).unwrap();
    let pi_path = tracker(&dir, "pi");
    let (pi, _) = Tracker::open(&pi_path).unwrap();
    write(&mac, "x1", "f", json!(1));
    write(&pi, "x1", "g", json!(2));
    let first = mac.sync(url).unwrap();
    assert_eq!((first.fetched.len(), first.joined), (0, false), "an empty remote");
    let second = pi.sync(url).unwrap();
    assert_eq!((second.fetched, second.joined), (vec!["mac".to_owned()], true));
    mac.sync(url).unwrap();
    let entities = |t: &Tracker| {
        t.load().unwrap().1.entities().map(|(k, e)| (k.clone(), e.clone())).collect::<Vec<_>>()
    };
    assert_eq!(entities(&mac), entities(&pi), "converged");
    let again = mac.sync(url).unwrap();
    assert_eq!((again.joined, again.pushed), (false, 0), "nothing new either way");
    // A clone whose state ref went, here along with the remote's copy, takes a fresh id; that id has joined none
    // of the others yet, whatever the old one had.
    pi.sync(url).unwrap();
    git(&pi_path, &["update-ref", "-d", "refs/heads/state/pi"]);
    git(&remote, &["update-ref", "-d", "refs/heads/state/pi"]);
    let (fresh, renamed) = Tracker::open(&pi_path).unwrap();
    assert!(renamed.is_some());
    assert!(fresh.sync(url).unwrap().joined);
    assert_eq!(entities(&fresh), entities(&mac), "it holds the others' work again");
    // A squash rewrites the state ref; the lease lets it through, and segments still only fast-forward.
    mac.squash().unwrap();
    write(&mac, "x1", "f", json!(3));
    assert!(mac.sync(url).unwrap().pushed > 0);
    // A replica that joined the old history joins the squashed one, and one that only ever saw the squash agrees.
    assert!(fresh.sync(url).unwrap().joined);
    assert_eq!(entities(&fresh), entities(&mac), "converged across the squash");
    let (late, _) = Tracker::open(&tracker(&dir, "late")).unwrap();
    late.sync(url).unwrap();
    assert_eq!(entities(&late), entities(&fresh), "a squashed tip joins like its history");
}

/// The state refs a clone fetched, with their tips.
fn fetched_states(path: &Path) -> String {
    let listing =
        git(path, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/acyclic/remotes/"]);
    listing.lines().filter(|line| line.contains("/state/")).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_copied_clone_writing_as_the_same_replica_is_refused_at_sync_and_moves_nothing() {
    let dir = TempDir::new().unwrap();
    let remote = remote_with_ruleset(&dir);
    let url = remote.to_str().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "x1", "f", json!(1));
    mac.sync(url).unwrap();
    let copy = dir.path().join("copy");
    let copied = Command::new("cp").arg("-R").arg(&path).arg(&copy).status().unwrap();
    assert!(copied.success());
    let (twin, renamed) = Tracker::open(&copy).unwrap();
    assert_eq!(renamed, None, "a copy carries the id and its state ref");
    // Each writes as many operations, so a join alone can't tell their writes apart.
    write(&mac, "x1", "f", json!(2));
    write(&twin, "x1", "f", json!(3));
    mac.sync(url).unwrap();
    let before = remote_refs(&remote);
    let seen = fetched_states(&copy);
    assert!(matches!(twin.sync(url), Err(StoreError::Duplicate(_))));
    assert_eq!(remote_refs(&remote), before, "nothing pushed");
    assert_eq!(fetched_states(&copy), seen, "and the copy still leases against what it last saw");
}

#[test]
fn a_lost_tracking_update_or_a_deleted_remote_ref_is_not_a_duplicate() {
    let dir = TempDir::new().unwrap();
    let remote = bare_remote(&dir);
    let url = remote.to_str().unwrap();
    let path = tracker(&dir, "mac");
    let (mac, _) = Tracker::open(&path).unwrap();
    write(&mac, "x1", "f", json!(1));
    mac.sync(url).unwrap();
    let tracking = fetched_states(&path).split(' ').next().unwrap().to_owned();
    // A push the remote applied, whose local tracking update never ran.
    write(&mac, "x1", "f", json!(2));
    let pushed = git(&path, &["rev-parse", "refs/heads/state/mac"]);
    mac.sync(url).unwrap();
    git(&path, &["update-ref", &tracking, "refs/heads/state/mac~1"]);
    write(&mac, "x1", "f", json!(3));
    mac.sync(url).unwrap();
    assert_ne!(git(&remote, &["rev-parse", "refs/heads/state/mac"]), pushed, "pushed again");
    // A remote recreated without this replica's ref.
    git(&remote, &["update-ref", "-d", "refs/heads/state/mac"]);
    mac.sync(url).unwrap();
    assert_eq!(
        git(&remote, &["rev-parse", "refs/heads/state/mac"]),
        git(&path, &["rev-parse", "refs/heads/state/mac"])
    );
}
