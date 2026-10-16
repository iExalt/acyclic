//! Two replicas that diverge offline and exchange through a bare remote: after a full
//! exchange both hold the same state, each conflict is listed on both, and resolving it on one side clears it on both.
//! Everything goes through `Tracker::sync`, so real refs, join commits and the parsed-state cache are under test; the
//! join's algebra itself is the state layer's properties'.
//!
//! Left out on purpose:
//! - a run's claim and the run's record travel in one replica's state, so no sync can separate them, and an unknown
//!   run is a single-replica case that needs' own tests cover;
//! - a property over random histories: each takes seconds here, as every step spawns git, while the state layer's
//!   properties already cover the join and each row below checks the cached load against a full read.

use std::path::{Path, PathBuf};
use std::process::Command;

use acyclic_core::claim;
use acyclic_core::export::{self, Scope};
use acyclic_core::model::{self, By, ItemState, LinkKind, Model, RunState, SetField};
use acyclic_core::needs::{self, Entry};
use acyclic_core::place;
use acyclic_core::ready::{Ancestry, Readiness};
use acyclic_core::schema::{item as field, kind, thread};
use acyclic_core::scope::{self, ScopeEdit};
use acyclic_core::state::{Key, Millis, Replica, ReplicaId, State};
use acyclic_core::store::{self, Meta, Tracker};
use tempfile::TempDir;

/// 2026-10-15T00:00:00Z.
const NOW: Millis = 1_792_022_400_000;
const MINUTE: Millis = 60_000;
const USER: &str = "actor/user";

type Edit<'a> = &'a dyn Fn(&mut Replica, By<'_>) -> Result<(), Box<dyn std::error::Error>>;

struct Unmapped;

impl Ancestry for Unmapped {
    fn merged(&self, _: &model::Repository, _: &str) -> Result<bool, String> {
        Err("no clone".into())
    }
}

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
        let meta = Meta { actor: actor.to_owned(), run: None };
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
            let readiness = Readiness::new(&state, &model, now, &Unmapped);
            needs::needs_you(&state, &model, &readiness, now).unwrap()
        };
        let mac = list(&self.mac);
        assert_eq!(mac, list(&self.pi), "the replicas list different needs");
        mac
    }
}

fn item(id: &str) -> Key {
    model::item_key(id).unwrap()
}

/// A thread record holding `item`.
fn threaded(
    replica: &mut Replica,
    by: By<'_>,
    thread: &str,
    item: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    model::set(replica, &Key::new(kind::THREAD, thread)?, thread::TITLE, Some(&thread), by)?;
    let member =
        acyclic_core::scope::ScopeEdit::Member { item: item.into(), thread: Some(thread.into()) };
    acyclic_core::scope::edit(replica, &member, by)?;
    Ok(())
}

fn add(
    replica: &mut Replica,
    by: By<'_>,
    id: &str,
    parent: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    model::add_item(replica, id, id, by)?;
    if parent.is_some() {
        model::set_parent(replica, id, parent, by)?;
    }
    Ok(())
}

fn set_title(
    title: &'static str,
) -> impl Fn(&mut Replica, By<'_>) -> Result<(), Box<dyn std::error::Error>> {
    move |replica, by| Ok(model::set(replica, &item("x"), field::TITLE, Some(&title), by)?)
}

fn conflict(id: &str, field: &str) -> Entry {
    Entry::Conflict { kind: kind::ITEM.into(), id: id.into(), field: field.into() }
}

#[test]
fn concurrent_edits_conflict_and_a_resolution_clears_them_unless_it_races_another_edit() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None));
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
fn a_move_to_the_top_racing_another_move_conflicts_until_one_is_chosen() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "p", None)?;
        add(r, by, "q", None)?;
        add(r, by, "x", Some("p"))
    });
    pair.spread(&pair.mac);
    // The user moves x to the top level, a clear of its parent, while an agent moves it under q.
    pair.user(&pair.mac, &|r, by| Ok(model::set_parent(r, "x", None, by)?));
    Pair::edit(&pair.pi, "session/pi", NOW, &|r, by| Ok(model::set_parent(r, "x", Some("q"), by)?));
    pair.exchange();
    let state = pair.assert_settled();
    let parent = &Model::read(&state).unwrap().items["x"].parent;
    assert_eq!(*parent, model::Value::Conflict(vec![None, Some("q".to_owned())]));
    assert!(pair.needs(NOW).contains(&conflict("x", "parent")));
    pair.user(&pair.pi, &|r, by| Ok(model::set_parent(r, "x", None, by)?));
    pair.spread(&pair.pi);
    let state = pair.assert_settled();
    assert_eq!(Model::read(&state).unwrap().items["x"].parent, model::Value::None);
    assert!(!pair.needs(NOW).contains(&conflict("x", "parent")));
}

#[test]
fn links_joined_into_a_cycle_are_listed_and_unlinking_clears_it() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "x", None)?;
        add(r, by, "y", None)
    });
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &|r, by| Ok(model::link(r, "x", "y", LinkKind::Blocks, by).map(|_| ())?));
    pair.user(&pair.pi, &|r, by| Ok(model::link(r, "y", "x", LinkKind::Blocks, by).map(|_| ())?));
    pair.exchange();
    pair.assert_settled();
    let cycle = Entry::LinkCycle { items: vec!["x".into(), "y".into()] };
    assert!(pair.needs(NOW).contains(&cycle));
    pair.user(&pair.pi, &|r, by| Ok(model::unlink(r, "y", "x", LinkKind::Blocks, by).map(|_| ())?));
    pair.spread(&pair.pi);
    pair.assert_settled();
    assert!(!pair.needs(NOW).contains(&cycle));
}

#[test]
fn parents_joined_into_a_cycle_are_listed_every_walk_ends_and_a_move_clears_it() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "a", None)?;
        add(r, by, "b", None)?;
        add(r, by, "c", Some("a"))
    });
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &|r, by| Ok(model::set_parent(r, "a", Some("b"), by)?));
    pair.user(&pair.pi, &|r, by| Ok(model::set_parent(r, "b", Some("a"), by)?));
    pair.exchange();
    let state = pair.assert_settled();
    assert!(pair.needs(NOW).iter().any(|e| matches!(e, Entry::ParentCycle { .. })));
    // Every walk up the tree ends: inheritance, readiness, the graph and placing new work under the cycle.
    let model = Model::read(&state).unwrap();
    assert_eq!(place::inherited_thread(&model, "c"), None);
    assert_eq!(place::thread_cycles(&model), Vec::<std::collections::BTreeSet<String>>::new());
    Readiness::new(&state, &model, NOW, &Unmapped).ready(false, "session/mac").unwrap();
    let view = export::view(&model, &Scope::default()).unwrap();
    assert!(!export::dot(&model, &view).is_empty());
    Pair::edit(&pair.mac, "session/mac", NOW, &|r, by| {
        add(r, by, "d", Some("c"))?;
        assert_eq!(place::place(r, &["d".to_owned()], by)?, Vec::new(), "nothing to inherit");
        Ok(())
    });
    pair.user(&pair.pi, &|r, by| Ok(model::set_parent(r, "b", None, by)?));
    pair.exchange();
    pair.assert_settled();
    assert!(!pair.needs(NOW).iter().any(|e| matches!(e, Entry::ParentCycle { .. })));
}

#[test]
fn claims_that_never_saw_each_other_contest_alike_on_both_until_the_loser_steps_aside() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "x", None)?;
        threaded(r, by, "t", "x")
    });
    pair.spread(&pair.mac);
    for (tracker, holder) in [(&pair.mac, "session/mac"), (&pair.pi, "session/pi")] {
        Pair::edit(tracker, holder, NOW, &|r, by| {
            Ok(claim::claim(r, "t", by.actor, by.at, Some(60 * MINUTE), 480 * MINUTE)
                .map(|_| ())?)
        });
    }
    pair.exchange();
    pair.assert_settled();
    let contests: Vec<Entry> =
        pair.needs(NOW).into_iter().filter(|e| matches!(e, Entry::Contest { .. })).collect();
    let [Entry::Contest { thread: contested, holders }] = contests.as_slice() else {
        panic!("{contests:?}")
    };
    assert_eq!((contested.as_str(), holders.len()), ("t", 2));
    let loser = holders[1].clone();
    let (tracker, winner) =
        if loser == "session/mac" { (&pair.mac, "session/pi") } else { (&pair.pi, "session/mac") };
    Pair::edit(tracker, &loser, NOW + MINUTE, &|r, by| {
        assert!(claim::release(r, &claim::thread_key("t")?, by.actor));
        Ok(())
    });
    pair.exchange();
    let state = pair.assert_settled();
    assert!(!pair.needs(NOW + MINUTE).iter().any(|e| matches!(e, Entry::Contest { .. })));
    let held =
        claim::winner(&state, &claim::thread_key("t").unwrap(), NOW + MINUTE).unwrap().unwrap();
    assert_eq!(held.holder, winner, "the winner's claim stands");
}

#[test]
fn claims_on_one_resource_clash_on_both() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        for id in ["gpu1", "gpu2"] {
            add(r, by, id, None)?;
            model::set_element(r, &item(id), SetField::Resource, "gpu", true, by)?;
            threaded(r, by, id, id)?;
        }
        Ok(())
    });
    pair.spread(&pair.mac);
    for (tracker, holder, id) in
        [(&pair.mac, "session/mac", "gpu1"), (&pair.pi, "session/pi", "gpu2")]
    {
        Pair::edit(tracker, holder, NOW, &|r, by| {
            Ok(claim::claim(r, id, by.actor, by.at, Some(60 * MINUTE), 480 * MINUTE).map(|_| ())?)
        });
    }
    pair.exchange();
    pair.assert_settled();
    assert!(
        pair.needs(NOW)
            .iter()
            .any(|e| matches!(e, Entry::ResourceClash { resource, .. } if resource == "gpu"))
    );
    assert!(
        !pair.needs(NOW + 61 * MINUTE).iter().any(|e| matches!(e, Entry::ResourceClash { .. }))
    );
}

#[test]
fn two_runs_on_one_thread_clash_until_one_stops() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        Ok(model::set(r, &Key::new(kind::THREAD, "t")?, thread::TITLE, Some(&"T"), by)?)
    });
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &|r, by| Ok(model::start_run(r, "r1", "t", by).map(|_| ())?));
    pair.user(&pair.pi, &|r, by| Ok(model::start_run(r, "r2", "t", by).map(|_| ())?));
    pair.exchange();
    pair.assert_settled();
    let clash = Entry::RunClash { thread: "t".into(), runs: vec!["r1".into(), "r2".into()] };
    assert!(pair.needs(NOW).contains(&clash));
    pair.user(&pair.pi, &|r, by| Ok(model::set_run_state(r, "r2", RunState::Stopped, by)?));
    pair.spread(&pair.pi);
    pair.assert_settled();
    assert!(!pair.needs(NOW).contains(&clash));
}

#[test]
fn a_claim_from_a_clock_running_ahead_is_listed_until_time_catches_up() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "x", None)?;
        threaded(r, by, "t", "x")
    });
    pair.spread(&pair.mac);
    let ahead = NOW + 30 * MINUTE;
    Pair::edit(&pair.pi, "session/pi", ahead, &|r, by| {
        Ok(claim::claim(r, "t", by.actor, by.at, Some(60 * MINUTE), 480 * MINUTE).map(|_| ())?)
    });
    pair.exchange();
    pair.assert_settled();
    let entry = Entry::ClockAhead { thread: "t".into(), holder: "session/pi".into() };
    assert!(pair.needs(NOW).contains(&entry));
    assert!(!pair.needs(ahead).contains(&entry));
}

#[test]
fn a_parent_finished_while_a_child_arrives_is_done_early_until_reopened() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "p", None)?;
        add(r, by, "a", Some("p"))?;
        Ok(model::set_state(r, "a", ItemState::Done, by)?)
    });
    pair.spread(&pair.mac);
    pair.user(&pair.mac, &|r, by| Ok(model::set_state(r, "p", ItemState::Done, by)?));
    pair.user(&pair.pi, &|r, by| add(r, by, "late", Some("p")));
    pair.exchange();
    pair.assert_settled();
    let early = Entry::DoneEarly { item: "p".into(), open: vec!["late".into()] };
    assert!(pair.needs(NOW).contains(&early));
    pair.user(&pair.pi, &|r, by| Ok(model::set_state(r, "p", ItemState::Backlog, by)?));
    pair.spread(&pair.pi);
    pair.assert_settled();
    assert!(!pair.needs(NOW).contains(&early));
}

#[test]
fn an_agents_member_racing_an_approval_is_listed_until_the_user_keeps_it() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| {
        add(r, by, "x", None)?;
        add(r, by, "y", None)?;
        scope::edit(r, &ScopeEdit::Member { item: "x".into(), thread: Some("t".into()) }, by)?;
        Ok(())
    });
    pair.spread(&pair.mac);
    // pi's agent sees t unapproved, so its edit applies; mac's user approves t at the same time.
    Pair::edit(&pair.pi, "session/pi", NOW, &|r, by| {
        let change = ScopeEdit::Member { item: "y".into(), thread: Some("t".into()) };
        assert_eq!(scope::edit(r, &change, by)?, scope::Outcome::Applied);
        Ok(())
    });
    pair.user(&pair.mac, &|r, by| Ok(scope::approve(r, "t", true, by).map(|_| ())?));
    pair.exchange();
    pair.assert_settled();
    let listed = |entries: Vec<Entry>| {
        entries.iter().any(|e| matches!(e, Entry::Unapproved { item, .. } if item == "y"))
    };
    assert!(listed(pair.needs(NOW)));
    pair.user(&pair.pi, &|r, by| {
        Ok(scope::edit(r, &ScopeEdit::Member { item: "y".into(), thread: Some("t".into()) }, by)
            .map(|_| ())?)
    });
    pair.spread(&pair.pi);
    pair.assert_settled();
    assert!(!listed(pair.needs(NOW)));
}

#[test]
fn squashes_on_both_sides_join_through_tips_the_cache_never_saw() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None));
    pair.spread(&pair.mac);
    // pi holds mac's tip; both squash and write, so pi's next fetch is a forced update to a parentless commit.
    pair.mac.squash().unwrap();
    pair.user(&pair.mac, &|r, by| add(r, by, "m", None));
    pair.pi.squash().unwrap();
    pair.user(&pair.pi, &|r, by| add(r, by, "p", None));
    pair.exchange();
    let state = pair.assert_settled();
    let model = Model::read(&state).unwrap();
    assert!(["x", "m", "p"].iter().all(|id| model.items.contains_key(*id)));
    pair.assert_quiescent();
}

#[test]
fn a_replica_holds_the_whole_state_without_another_replicas_logs() {
    let pair = Pair::new();
    pair.user(&pair.mac, &|r, by| add(r, by, "x", None));
    pair.user(&pair.pi, &|r, by| add(r, by, "y", None));
    pair.exchange();
    pair.assert_settled();
    assert_eq!(git(&pair.pi_path, &["for-each-ref", "refs/heads/log/mac/"]), "", "state refs only");
    let (_, state) = pair.pi.load_strict().unwrap();
    let (model, unreadable) = Model::read_lenient(&state);
    assert!(unreadable.is_empty() && model.items.contains_key("x"));
}
