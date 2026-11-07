//! Two replicas that diverge offline and exchange through a bare remote: after a full
//! exchange both hold the same state, each conflict is listed on both, and resolving it on one side clears it on both.
//! Everything goes through `Tracker::sync`, so real refs, join commits and the parsed-state cache are under test; the
//! join's algebra itself is the state layer's properties'.
//!
//! Left out on purpose:
//! - what each kind of problem lists, which needs' own tests cover on replicas joined in memory: the rows here are
//!   the ones a sync could get wrong, such as values held in conflict, a cleared value and rewritten tips;
//! - a property over random histories: each takes seconds here, as every step spawns git, while the state layer's
//!   properties already cover the join and each row below checks the cached load against a full read.

use std::path::{Path, PathBuf};
use std::process::Command;

use acyclic_core::model::{self, By, Model};
use acyclic_core::needs::{self, Entry};
use acyclic_core::ready::ReadinessEvaluator;
use acyclic_core::schema::kind;

use acyclic_core::state::{Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use acyclic_core::write::{self, Assign, Change, ChangeSet, ItemAssign, Outcome, Record, Repeat};
use tempfile::TempDir;

/// 2026-10-15T00:00:00Z.
const NOW: Millis = 1_792_022_400_000;
const USER: &str = "actor/user";

type Edit<'a> = &'a dyn Fn(&mut Replica, By<'_>) -> Result<(), Box<dyn std::error::Error>>;

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
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Two clones, `mac` and `pi`, and the bare remote they sync through.
struct Pair {
    _dir: TempDir,
    remote: String,
    mac: Tracker,
    pi: Tracker,
    pi_path: PathBuf,
}

impl Pair {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let remote = dir.path().join("remote.git");
        git(dir.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
        // The ruleset's hook is the store tests'; this suite is about joining. The remote's own repacking is
        // GitHub's business, not this suite's time.
        git(&remote, &["config", "receive.autogc", "false"]);
        let clone = |name: &str| {
            let path = dir.path().join(name);
            git(dir.path(), &["init", "-q", path.to_str().unwrap()]);
            store::init_replica(&path, &ReplicaId::new(name).unwrap()).unwrap();
            git(&path, &["config", "core.fsync", "none"]);
            path
        };
        let (mac, pi_path) = (clone("mac"), clone("pi"));
        let (mac, _) = Tracker::open(&mac).unwrap();
        let (pi, _) = Tracker::open(&pi_path).unwrap();
        Self { remote: remote.to_str().unwrap().to_owned(), _dir: dir, mac, pi, pi_path }
    }

    /// One batch on `tracker` as `actor`, at `now` or the replica's latest time if later.
    fn edit(tracker: &Tracker, actor: &str, now: Millis, edit: Edit<'_>) {
        let meta = Meta { actor: actor.to_owned() };
        tracker
            .batch(&meta, now, false, &|_, _| None, |replica, at| edit(replica, By { actor, at }))
            .unwrap();
    }

    fn user(&self, tracker: &Tracker, edit: Edit<'_>) {
        Self::edit(tracker, USER, NOW, edit);
    }

    /// A full exchange: after it, each replica holds what both wrote.
    fn exchange(&self) {
        for tracker in [&self.mac, &self.pi, &self.mac] {
            tracker.sync(&self.remote).unwrap();
        }
    }

    /// What one side alone wrote reaches the other: two syncs rather than a full exchange.
    fn spread(&self, from: &Tracker) {
        let to = if std::ptr::eq(from, &self.mac) { &self.pi } else { &self.mac };
        for tracker in [from, to] {
            tracker.sync(&self.remote).unwrap();
        }
    }

    /// Asserts the pair has settled: equal states, contexts included, and each clone's cached state equal to Git's.
    fn assert_settled(&self) -> State {
        let [mac, pi] = [&self.mac, &self.pi].map(|tracker| {
            let strict = tracker.load_strict().unwrap().1;
            assert_eq!(tracker.load().unwrap().1, strict, "the cache drifted");
            strict
        });
        assert_eq!(mac, pi, "the replicas differ");
        mac
    }

    /// Asserts nothing is left to exchange either way, once a scenario ends. A join of what a replica already holds
    /// changes nothing, which the state layer's properties cover, so only the rows where a sync might keep finding
    /// something new check it: values held in conflict, and tips rewritten by a squash.
    fn assert_quiescent(&self) {
        for tracker in [&self.pi, &self.mac] {
            let synced = tracker.sync(&self.remote).unwrap();
            assert_eq!((synced.joined, synced.pushed), (false, 0), "not quiescent");
        }
    }

    /// The needs-you list on each replica, which must agree.
    fn needs(&self, now: Millis) -> Vec<Entry> {
        let list = |tracker: &Tracker| {
            let (_, state) = tracker.load().unwrap();
            let model = Model::read(&state).unwrap();
            let readiness = ReadinessEvaluator::new(&state, &model, now);
            needs::needs_you(&state, &model, &readiness, now).unwrap()
        };
        let mac = list(&self.mac);
        assert_eq!(mac, list(&self.pi), "the replicas list different needs");
        mac
    }
}

/// Judges and writes `changes` as the CLI does.
fn commit(
    replica: &mut Replica,
    by: By<'_>,
    changes: Vec<Change>,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    write::commit(replica, by, false, |_| Ok::<_, ()>(ChangeSet(changes)))
        .map_err(|error| format!("{error:?}").into())
}

fn add(
    replica: &mut Replica,
    by: By<'_>,
    id: &str,
    parent: Option<&str>,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let mut with = vec![Assign::Item(id.into(), ItemAssign::Title(id.into()))];
    with.extend(parent.map(|parent| Assign::Item(id.into(), ItemAssign::Parent(parent.into()))));
    let add = Change::Add {
        record: Record::Item(id.into()),
        minted: false,
        on_repeat: Repeat::Refuse,
        with,
    };
    commit(replica, by, vec![add])
}

fn set_parent(
    replica: &mut Replica,
    by: By<'_>,
    id: &str,
    parent: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = ItemAssign::Parent(parent.to_owned());
    commit(replica, by, vec![Change::Set(Assign::Item(id.into(), parent))]).map(drop)
}

fn set_title(
    title: &'static str,
) -> impl Fn(&mut Replica, By<'_>) -> Result<(), Box<dyn std::error::Error>> {
    move |replica, by| {
        let title = ItemAssign::Title(title.into());
        commit(replica, by, vec![Change::Set(Assign::Item("x".into(), title))]).map(drop)
    }
}

fn conflict(id: &str, field: &str) -> Entry {
    Entry::Conflict { kind: kind::ITEM.into(), id: id.into(), field: field.into() }
}

#[test]
fn concurrent_edits_conflict_and_a_resolution_clears_them_unless_it_races_another_edit() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None).map(drop));
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &set_title("one"));
    pair.user(&pair.pi, &set_title("two"));
    pair.exchange();
    pair.assert_settled();
    assert!(pair.needs(NOW).contains(&conflict("x", "title")));
    // Resolved on one side, it clears on both.
    pair.user(&pair.pi, &set_title("both"));
    pair.spread(&pair.pi);
    pair.assert_settled();
    assert!(!pair.needs(NOW).contains(&conflict("x", "title")));
    // A resolution racing a fresh edit is a new conflict that keeps both, not a lost write.
    pair.user(&pair.mac, &set_title("resolved"));
    pair.user(&pair.pi, &set_title("edited"));
    pair.exchange();
    let state = pair.assert_settled();
    assert!(pair.needs(NOW).contains(&conflict("x", "title")));
    let title = &Model::read(&state).unwrap().items["x"].title;
    let mut values: Vec<&String> = title.all();
    values.sort();
    assert_eq!(values, ["edited", "resolved"]);
    pair.assert_quiescent();
}

#[test]
fn moves_racing_conflict_until_one_is_chosen() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "p", None)?;
        add(r, by, "q", None)?;
        add(r, by, "s", None)?;
        add(r, by, "x", Some("p")).map(drop)
    });
    pair.spread(&pair.mac);
    // The user moves x under s while an agent moves it under q.
    pair.user(&pair.mac, &|r, by| set_parent(r, by, "x", "s"));
    Pair::edit(&pair.pi, "session/pi", NOW, &|r, by| set_parent(r, by, "x", "q"));
    pair.exchange();
    let state = pair.assert_settled();
    let parent = &Model::read(&state).unwrap().items["x"].parent;
    assert_eq!(*parent, model::Value::Conflict(vec![Some("s".to_owned()), Some("q".to_owned())]));
    assert!(pair.needs(NOW).contains(&conflict("x", "parent")));
    pair.user(&pair.pi, &|r, by| set_parent(r, by, "x", "q"));
    pair.spread(&pair.pi);
    let state = pair.assert_settled();
    assert_eq!(Model::read(&state).unwrap().items["x"].parent, model::Value::One("q".to_owned()));
    assert!(!pair.needs(NOW).contains(&conflict("x", "parent")));
}

#[test]
fn parents_joined_into_a_cycle_are_listed_every_walk_ends_and_a_move_clears_it() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "a", None)?;
        add(r, by, "b", None)?;
        add(r, by, "c", Some("a"))?;
        add(r, by, "z", None).map(drop)
    });
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &|r, by| set_parent(r, by, "a", "b"));
    pair.user(&pair.pi, &|r, by| set_parent(r, by, "b", "a"));
    pair.exchange();
    let state = pair.assert_settled();
    assert!(pair.needs(NOW).iter().any(|e| matches!(e, Entry::ParentCycle { .. })));
    // Every walk up the tree ends: readiness and placing new work under the cycle.
    let model = Model::read(&state).unwrap();
    let readiness = ReadinessEvaluator::new(&state, &model, NOW);
    assert_eq!(
        readiness.thread_cycles().unwrap(),
        Vec::<std::collections::BTreeSet<String>>::new()
    );
    readiness.ready("session/mac").unwrap();
    Pair::edit(&pair.mac, "session/mac", NOW, &|r, by| {
        let added = add(r, by, "d", Some("c"))?;
        assert_eq!(added.placed(), Vec::new(), "nothing to inherit");
        Ok(())
    });
    pair.user(&pair.pi, &|r, by| set_parent(r, by, "b", "z"));
    pair.exchange();
    pair.assert_settled();
    assert!(!pair.needs(NOW).iter().any(|e| matches!(e, Entry::ParentCycle { .. })));
}

#[test]
fn squashes_on_both_sides_join_through_tips_the_cache_never_saw() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None).map(drop));
    pair.user(&pair.mac, &set_title("base"));
    pair.spread(&pair.mac);
    // pi holds mac's tip; both squash and write, so pi's next fetch is a forced update to a parentless commit.
    assert!(pair.mac.squash().unwrap().is_some());
    pair.user(&pair.mac, &|r, by| add(r, by, "m", None).map(drop));
    pair.user(&pair.pi, &set_title("peer"));
    assert!(pair.pi.squash().unwrap().is_some());
    pair.user(&pair.pi, &|r, by| add(r, by, "p", None).map(drop));
    pair.exchange();
    let state = pair.assert_settled();
    let model = Model::read(&state).unwrap();
    assert!(["x", "m", "p"].iter().all(|id| model.items.contains_key(*id)));
    pair.assert_quiescent();
}

#[test]
fn a_replica_holds_the_whole_state_without_another_replicas_logs() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None).map(drop));
    pair.user(&pair.pi, &|r, by| add(r, by, "y", None).map(drop));
    pair.exchange();
    pair.assert_settled();
    assert_eq!(git(&pair.pi_path, &["for-each-ref", "refs/heads/log/mac/"]), "", "state refs only");
    let (_, state) = pair.pi.load_strict().unwrap();
    let (model, unreadable) = Model::read_lenient(&state);
    assert!(unreadable.is_empty() && model.items.contains_key("x"));
}
