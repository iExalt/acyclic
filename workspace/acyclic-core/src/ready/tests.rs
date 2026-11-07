use hegel::TestCase;
use hegel::generators as gs;

use super::*;
use crate::ids;
use crate::model::{self, By, Evidence, add_item, item_key, link, set, set_parent, set_state};
use crate::schema::{item as field, kind, repository, thread as thread_field};
use crate::state::{Key, Replica, ReplicaId};

const NOW: Millis = 1_000;

fn by(at: Millis) -> By<'static> {
    By { actor: "actor/user", at }
}

fn replica(name: &str) -> Replica {
    let mut r = Replica::open(ReplicaId::new(name).unwrap(), State::new());
    let code = Key::new(kind::REPOSITORY, "code").unwrap();
    set(&mut r, &code, repository::DEFAULT_BRANCH, Some(&"main"), by(1)).unwrap();
    r
}

/// New items, each in the thread `main`: threads are the unit of work, and an item in none is never ready.
fn items(r: &mut Replica, ids: &[&str]) {
    for id in ids {
        add_item(r, id, id, by(1)).unwrap();
        thread(r, id, "main");
    }
}

fn done_with(r: &mut Replica, id: &str, commit: Option<&str>) {
    set_state(r, id, ItemState::Done, by(2)).unwrap();
    let evidence = Evidence {
        repository: commit.map(|_| "code".to_owned()),
        commit: commit.map(str::to_owned),
        text: None,
    };
    set(r, &item_key(id).unwrap(), field::EVIDENCE, Some(&evidence), by(2)).unwrap();
}

/// Puts an item in a thread, giving the thread a record.
fn thread(r: &mut Replica, id: &str, thread: &str) {
    set(r, &claim::thread_key(thread).unwrap(), thread_field::TITLE, Some(&thread), by(1)).unwrap();
    model::put(r, &item_key(id).unwrap(), field::THREAD, Some(&thread), by(2)).unwrap();
}

fn status(r: &Replica, id: &str) -> Readiness {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, NOW).of(id).unwrap().unwrap()
}

/// The ready items, thread by thread, in the order `ready` lists them.
fn flat(threads: &[ReadyThread]) -> Vec<String> {
    threads.iter().flat_map(|t| t.items.iter().cloned()).collect()
}

fn waiting_on(waits: &[(&str, Wait)]) -> Readiness {
    Readiness::Waiting {
        waiting: waits.iter().map(|(id, w)| ((*id).to_owned(), w.clone())).collect(),
    }
}

#[test]
fn a_hard_dependency_blocks_until_done() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x"]);
    link(&mut r, "x", "dep", by(2)).unwrap();
    assert_eq!(status(&r, "x"), waiting_on(&[("dep", Wait::Open)]));
    // Work with no repository is met when done.
    done_with(&mut r, "dep", None);
    assert_eq!(status(&r, "x"), Readiness::Ready);
}

#[test]
fn a_done_dependency_is_met_in_any_thread() {
    let mut r = replica("a");
    items(&mut r, &["dep", "same", "other", "loose"]);
    for id in ["same", "other", "loose"] {
        link(&mut r, id, "dep", by(2)).unwrap();
    }
    done_with(&mut r, "dep", Some("c1"));
    thread(&mut r, "dep", "t1");
    thread(&mut r, "same", "t1");
    thread(&mut r, "other", "t2");
    model::put(&mut r, &item_key("loose").unwrap(), field::THREAD, None::<&String>, by(2)).unwrap();
    // No git runs: a done dependency is met wherever its dependent is, and the commit was checked when written.
    assert_eq!(status(&r, "same"), Readiness::Ready);
    assert_eq!(status(&r, "other"), Readiness::Ready);
    assert_eq!(status(&r, "loose"), Readiness::Ready);
    assert_eq!(status(&r, "dep"), Readiness::Done);
}

#[test]
fn evidence_naming_no_repository_record_still_meets_its_dependents() {
    let mut r = replica("a");
    items(&mut r, &["dep", "y"]);
    link(&mut r, "y", "dep", by(2)).unwrap();
    set_state(&mut r, "dep", ItemState::Done, by(3)).unwrap();
    let evidence =
        Evidence { repository: Some("gone".into()), commit: Some("c2".into()), text: None };
    set(&mut r, &item_key("dep").unwrap(), field::EVIDENCE, Some(&evidence), by(3)).unwrap();
    assert_eq!(status(&r, "y"), Readiness::Ready);
    assert_eq!(status(&r, "dep"), Readiness::Done);
}

#[test]
fn a_parents_dependencies_hold_back_its_children_and_a_parent_is_met_by_its_leaves() {
    let mut r = replica("a");
    items(&mut r, &["goal", "child", "dep", "after-goal"]);
    set_parent(&mut r, "child", Some("goal"), by(2)).unwrap();
    link(&mut r, "goal", "dep", by(2)).unwrap();
    link(&mut r, "after-goal", "goal", by(2)).unwrap();
    assert_eq!(status(&r, "child"), waiting_on(&[("dep", Wait::Open)]));
    assert_eq!(status(&r, "goal"), Readiness::Parent);
    assert_eq!(status(&r, "after-goal"), waiting_on(&[("goal", Wait::Open)]));
    // A done stored on the parent, as an older version wrote, is ignored: its leaves decide.
    done_with(&mut r, "goal", None);
    assert_eq!(status(&r, "goal"), Readiness::Parent);
    assert_eq!(status(&r, "after-goal"), waiting_on(&[("goal", Wait::Open)]));
    done_with(&mut r, "dep", None);
    done_with(&mut r, "child", None);
    assert_eq!(status(&r, "after-goal"), Readiness::Ready);
    // A parent parked by hand meets nothing.
    set_state(&mut r, "goal", ItemState::Parked, by(4)).unwrap();
    assert_eq!(status(&r, "after-goal"), waiting_on(&[("goal", Wait::Open)]));
}

#[test]
fn a_canceled_dependency_never_unblocks_but_an_epic_waits_on_its_other_work() {
    let mut r = replica("a");
    items(&mut r, &["dep", "y", "epic", "e1", "e2", "z", "dead", "d1", "w"]);
    for (child, parent) in [("e1", "epic"), ("e2", "epic"), ("d1", "dead")] {
        set_parent(&mut r, child, Some(parent), by(2)).unwrap();
    }
    for (from, to) in [("y", "dep"), ("z", "epic"), ("w", "dead")] {
        link(&mut r, from, to, by(2)).unwrap();
    }
    set_state(&mut r, "dep", ItemState::Canceled, by(3)).unwrap();
    assert_eq!(status(&r, "dep"), Readiness::Canceled);
    assert_eq!(status(&r, "y"), waiting_on(&[("dep", Wait::Canceled)]));
    // One canceled leaf among others: the epic is waited for as a whole.
    set_state(&mut r, "e1", ItemState::Canceled, by(3)).unwrap();
    assert_eq!(status(&r, "z"), waiting_on(&[("epic", Wait::Open)]));
    done_with(&mut r, "e2", None);
    assert_eq!(status(&r, "z"), Readiness::Ready);
    // Nothing but canceled work under it, or canceled by hand, reads canceled.
    set_state(&mut r, "d1", ItemState::Canceled, by(3)).unwrap();
    assert_eq!(status(&r, "w"), waiting_on(&[("dead", Wait::Canceled)]));
    set_state(&mut r, "d1", ItemState::Backlog, by(4)).unwrap();
    set_state(&mut r, "dead", ItemState::Canceled, by(4)).unwrap();
    assert_eq!(status(&r, "w"), waiting_on(&[("dead", Wait::Canceled)]));
    // Canceled work is never ready, and an open leaf under a part canceled by hand counts as canceled.
    let model = Model::read(&r.state).unwrap();
    let evaluator = ReadinessEvaluator::new(&r.state, &model, NOW);
    assert_eq!(evaluator.stage("dep").unwrap(), Some(Stage::Canceled));
    let dead = evaluator.progress("dead").unwrap().unwrap();
    assert_eq!(
        (dead.stage, dead.stages.canceled, dead.stages.total()),
        (Some(Stage::Canceled), 1, 0)
    );
    let epic = evaluator.progress("epic").unwrap().unwrap();
    assert_eq!((epic.stage, epic.stages.canceled, epic.stages.done), (Some(Stage::Done), 1, 1));
    assert!(!flat(&evaluator.ready("actor/user").unwrap()).contains(&"dep".to_owned()));
}

#[test]
fn open_leaves_under_a_part_canceled_by_hand_read_canceled_until_it_is_opened_again() {
    let mut r = replica("a");
    items(&mut r, &["part", "leaf", "done", "w"]);
    for kid in ["leaf", "done"] {
        set_parent(&mut r, kid, Some("part"), by(2)).unwrap();
    }
    link(&mut r, "w", "leaf", by(2)).unwrap();
    done_with(&mut r, "done", None);
    let ready = |r: &Replica| {
        let model = Model::read(&r.state).unwrap();
        flat(&ReadinessEvaluator::new(&r.state, &model, NOW).ready("actor/user").unwrap())
    };
    assert_eq!(ready(&r), ["leaf"]);
    set_state(&mut r, "part", ItemState::Canceled, by(3)).unwrap();
    // The open leaf reads canceled and isn't offered; done work stays done; its dependent waits on canceled work.
    assert_eq!(status(&r, "leaf"), Readiness::Canceled);
    assert_eq!(status(&r, "done"), Readiness::Done);
    assert_eq!(ready(&r).len(), 0);
    assert_eq!(status(&r, "w"), waiting_on(&[("leaf", Wait::Canceled)]));
    let model = Model::read(&r.state).unwrap();
    let evaluator = ReadinessEvaluator::new(&r.state, &model, NOW);
    assert_eq!(evaluator.stage("leaf").unwrap(), Some(Stage::Canceled));
    assert!(model.reads_finished("leaf") && !model.reads_canceled("done"));
    let needs = crate::needs::needs_you(&r.state, &model, &evaluator, NOW).unwrap();
    let flagged =
        crate::needs::Entry::WaitsOnCanceled { item: "w".into(), waits_for: "leaf".into() };
    assert!(needs.contains(&flagged), "{needs:?}");
    // Opened again, all of it comes back.
    set_state(&mut r, "part", ItemState::Backlog, by(4)).unwrap();
    assert_eq!(status(&r, "leaf"), Readiness::Ready);
    assert_eq!(status(&r, "w"), waiting_on(&[("leaf", Wait::Open)]));
    assert_eq!(ready(&r), ["leaf"]);
    let model = Model::read(&r.state).unwrap();
    let evaluator = ReadinessEvaluator::new(&r.state, &model, NOW);
    let needs = crate::needs::needs_you(&r.state, &model, &evaluator, NOW).unwrap();
    assert!(!needs.contains(&flagged));
}

#[test]
fn the_evaluators_memoized_canceled_reading_is_the_models_for_every_item() {
    let mut r = replica("a");
    items(
        &mut r,
        &[
            "top", "cut", "c1", "c2", "mix", "m1", "m2", "all", "a1", "a2", "dn", "d1", "d2", "g",
            "gc", "gd", "go",
        ],
    );
    for (kid, parent) in [
        ("cut", "top"),
        ("c1", "cut"),
        ("c2", "cut"),
        ("mix", "top"),
        ("m1", "mix"),
        ("m2", "mix"),
        ("all", "top"),
        ("a1", "all"),
        ("a2", "all"),
        ("dn", "cut"),
        ("d1", "dn"),
        ("d2", "dn"),
        // A grandparent whose only child is canceled by hand, over a done leaf and an open one, reads done.
        ("gc", "g"),
        ("gd", "gc"),
        ("go", "gc"),
    ] {
        set_parent(&mut r, kid, Some(parent), by(2)).unwrap();
    }
    for (id, state) in [
        ("cut", ItemState::Canceled),
        ("m1", ItemState::Canceled),
        ("m2", ItemState::Done),
        ("a1", ItemState::Canceled),
        ("a2", ItemState::Canceled),
        ("d1", ItemState::Done),
        ("d2", ItemState::Done),
        ("gc", ItemState::Canceled),
        ("gd", ItemState::Done),
    ] {
        set_state(&mut r, id, state, by(3)).unwrap();
    }
    let model = Model::read(&r.state).unwrap();
    let evaluator = ReadinessEvaluator::new(&r.state, &model, NOW);
    let mut reading = Vec::new();
    for id in model.items.keys() {
        assert_eq!(evaluator.canceled(id), model.reads_canceled(id), "{id}");
        reading.push((id.as_str(), model.reads_canceled(id)));
    }
    // Pinned: a part under one canceled by hand reads canceled unless it reads done; an all-canceled part reads so.
    let canceled: Vec<&str> = reading.iter().filter(|(_, c)| *c).map(|(id, _)| *id).collect();
    assert_eq!(canceled, ["a1", "a2", "all", "c1", "c2", "cut", "gc", "go", "m1"]);
}

#[test]
fn a_thread_is_finished_when_every_leaf_is_done_or_canceled_and_an_empty_one_is_not() {
    let mut r = replica("a");
    items(&mut r, &["x", "y"]);
    let finished = |r: &Replica| {
        let model = Model::read(&r.state).unwrap();
        let evaluator = ReadinessEvaluator::new(&r.state, &model, NOW);
        Stages::of_leaves(&evaluator, ["x", "y"]).unwrap().is_finished()
    };
    assert!(!Stages::default().is_finished());
    assert!(!finished(&r));
    set_state(&mut r, "x", ItemState::Canceled, by(3)).unwrap();
    assert!(!finished(&r));
    set_state(&mut r, "y", ItemState::Done, by(3)).unwrap();
    assert!(finished(&r));
    set_state(&mut r, "y", ItemState::Canceled, by(4)).unwrap();
    assert!(finished(&r));
}

#[test]
fn cycles_conflicts_and_parked_items_are_not_ready() {
    let mut a = replica("a");
    items(&mut a, &["x", "y", "p", "c"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    link(&mut a, "x", "y", by(2)).unwrap();
    link(&mut b, "y", "x", by(2)).unwrap();
    set_state(&mut a, "c", ItemState::Parked, by(2)).unwrap();
    set_state(&mut b, "c", ItemState::Done, by(2)).unwrap();
    thread(&mut a, "p", "t1");
    thread(&mut b, "p", "t2");
    a.join(&b.state).unwrap();
    set_state(&mut a, "p", ItemState::Parked, by(3)).unwrap();
    assert_eq!(status(&a, "x"), Readiness::InCycle);
    assert_eq!(status(&a, "c"), Readiness::Conflicted { fields: vec!["state".into()] });
    assert_eq!(status(&a, "p"), Readiness::Conflicted { fields: vec!["thread".into()] });
    thread(&mut a, "p", "t1");
    assert_eq!(status(&a, "p"), Readiness::Parked);
}

#[test]
fn a_threads_claim_shows_on_its_members_and_a_contest_names_its_winner_first() {
    let mut a = replica("a");
    items(&mut a, &["x"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    claim::claim(&mut a, "main", "session/one", 900, (500, 1_000), false).unwrap();
    assert_eq!(status(&a, "x"), Readiness::Claimed { holder: "session/one".into() });
    claim::claim(&mut b, "main", "session/two", 950, (500, 1_000), false).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(
        status(&a, "x"),
        Readiness::Contested { holders: vec!["session/one".into(), "session/two".into()] }
    );
}

#[test]
fn a_leaf_in_no_thread_is_ready_as_a_thread_of_one_unless_something_holds_it_back() {
    let mut r = replica("a");
    for id in ["loose", "ghostly", "dep", "waiting"] {
        add_item(&mut r, id, id, by(1)).unwrap();
    }
    // A thread named but with no record holds nothing.
    model::put(&mut r, &item_key("ghostly").unwrap(), field::THREAD, Some(&"ghost"), by(2))
        .unwrap();
    link(&mut r, "waiting", "dep", by(2)).unwrap();
    for id in ["loose", "ghostly", "dep"] {
        assert_eq!(status(&r, id), Readiness::Ready, "{id}");
    }
    assert_eq!(status(&r, "waiting"), waiting_on(&[("dep", Wait::Open)]));
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    // Each ready one is its own thread, named for it; a thread with no record that isn't derived stays missing.
    let ready = readiness.ready("session/s").unwrap();
    let ones: Vec<(String, Vec<String>)> = ready.into_iter().map(|t| (t.thread, t.items)).collect();
    let one = |id: &str| (ids::one(id), vec![id.to_owned()]);
    assert_eq!(ones, [one("dep"), one("ghostly"), one("loose")]);
    assert_eq!(readiness.thread("ghost").unwrap(), ThreadReadiness::Missing);
    assert_eq!(readiness.thread(&ids::one("waiting")).unwrap(), ThreadReadiness::NothingReady);
}

#[test]
fn a_thread_is_claimable_by_anyone_while_ready_and_by_its_holder_while_held() {
    let mut a = replica("a");
    items(&mut a, &["x", "waiter", "dep"]);
    link(&mut a, "waiter", "dep", by(2)).unwrap();
    for (id, held) in [("y", "other"), ("idle", "quiet")] {
        add_item(&mut a, id, id, by(1)).unwrap();
        thread(&mut a, id, held);
    }
    set_state(&mut a, "idle", ItemState::Parked, by(2)).unwrap();
    let at = |r: &Replica, thread: &str| {
        let model = Model::read(&r.state).unwrap();
        ReadinessEvaluator::new(&r.state, &model, NOW).thread(thread).unwrap()
    };
    let ready = |r: &Replica, actor: &str| {
        let model = Model::read(&r.state).unwrap();
        flat(&ReadinessEvaluator::new(&r.state, &model, NOW).ready(actor).unwrap())
    };
    let workable = vec!["dep".to_owned(), "x".to_owned()];
    assert_eq!(at(&a, "main"), ThreadReadiness::Ready { items: workable.clone() });
    assert_eq!(at(&a, "quiet"), ThreadReadiness::NothingReady);
    assert!(!at(&a, "quiet").claimable_by("session/one"));
    assert_eq!(ready(&a, "session/one"), ["dep", "x", "y"]);
    let unclaimed = a.state.clone();
    claim::claim(&mut a, "main", "session/one", 900, (500, 1_000), false).unwrap();
    // Its members stay workable for the holder alone.
    let held = at(&a, "main");
    assert_eq!(held, ThreadReadiness::Claimed { holder: "session/one".into(), items: workable });
    assert!(held.claimable_by("session/one") && !held.claimable_by("session/two"));
    assert_eq!(ready(&a, "session/one"), ["dep", "x", "y"]);
    assert_eq!(ready(&a, "session/two"), ["y"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // A contest's winner may go on; its loser may not.
    let mut c = replica("c");
    c.join(&unclaimed).unwrap();
    claim::claim(&mut c, "main", "session/two", 950, (500, 1_000), false).unwrap();
    b.join(&c.state).unwrap();
    let contested = at(&b, "main");
    assert!(
        matches!(&contested, ThreadReadiness::Contested { holders, .. } if holders[0] == "session/one")
    );
    assert!(contested.claimable_by("session/one") && !contested.claimable_by("session/two"));
}

#[test]
fn ready_lists_leaves_by_id_and_checks_each_needed_commit_once() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "y", "z", "unrelated"]);
    for id in ["x", "y"] {
        link(&mut r, id, "dep", by(2)).unwrap();
    }
    done_with(&mut r, "dep", Some("c1"));
    thread(&mut r, "dep", "deps");
    done_with(&mut r, "unrelated", Some("c2"));
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    let ready = flat(&readiness.ready("session/s").unwrap());
    assert_eq!(ready, ["x", "y", "z"]);
}

#[test]
fn two_threads_whose_items_once_named_one_resource_are_both_ready() {
    let mut r = replica("a");
    items(&mut r, &["x"]);
    add_item(&mut r, "y", "y", by(1)).unwrap();
    thread(&mut r, "y", "other");
    // Resources older versions stored are ignored, so a claim on one thread holds nothing the other needs.
    for id in ["x", "y"] {
        let gpu = Some(serde_json::json!(true));
        r.write(&item_key(id).unwrap(), "resource/gpu", "actor/user", 2, gpu).unwrap();
    }
    claim::claim(&mut r, "main", "session/one", 900, (500, 1_000), false).unwrap();
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    assert_eq!(readiness.of("y").unwrap(), Some(Readiness::Ready));
    assert_eq!(
        readiness.thread("other").unwrap(),
        ThreadReadiness::Ready { items: vec!["y".into()] }
    );
    assert_eq!(flat(&readiness.ready("session/two").unwrap()), ["y"]);
}

#[test]
fn ready_lists_the_actors_thread_first_then_the_rest_by_natural_id_and_leaves_out_others_held() {
    let mut r = replica("a");
    // Titles run against ids, so only the ids can order what is listed. Digits compare as numbers (t2 before t10),
    // and a bare prefix sorts before the same prefix and a dash (t10 before t-1), unlike plain byte order.
    for (item, held, title) in [
        ("i1", "t10", "A"),
        ("i2", "t2", "B"),
        ("i3", "t-1", "C"),
        ("i4", "mine", "D"),
        ("i5", "theirs", "E"),
        ("i6", "t3", "F"),
    ] {
        add_item(&mut r, item, item, by(1)).unwrap();
        thread(&mut r, item, held);
        set(&mut r, &claim::thread_key(held).unwrap(), thread_field::TITLE, Some(&title), by(1))
            .unwrap();
    }
    // Items within a thread sort by id in natural order too, whatever their titles say.
    for (item, title) in [("m10", "Part 1"), ("m2", "Part 2"), ("m-1", "Part 3")] {
        add_item(&mut r, item, title, by(1)).unwrap();
        thread(&mut r, item, "mine");
    }
    claim::claim(&mut r, "mine", "session/me", 900, (500, 1_000), false).unwrap();
    claim::claim(&mut r, "theirs", "session/other", 900, (500, 1_000), false).unwrap();
    let model = Model::read(&r.state).unwrap();
    let ready = ReadinessEvaluator::new(&r.state, &model, NOW).ready("session/me").unwrap();
    let order: Vec<(&str, Vec<&str>)> = ready
        .iter()
        .map(|t| (t.thread.as_str(), t.items.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(
        order,
        [
            ("mine", vec!["i4", "m2", "m10", "m-1"]),
            ("t2", vec!["i2"]),
            ("t3", vec!["i6"]),
            ("t10", vec!["i1"]),
            ("t-1", vec!["i3"]),
        ]
    );
}

#[test]
fn a_dependency_on_a_parent_waits_on_each_leaf_under_it_as_its_own_dependency() {
    let mut r = replica("a");
    items(&mut r, &["goal", "mid", "a", "b", "old", "same", "other"]);
    for (child, parent) in [("mid", "goal"), ("a", "mid"), ("b", "goal"), ("old", "goal")] {
        set_parent(&mut r, child, Some(parent), by(2)).unwrap();
    }
    for id in ["a", "b", "old", "same"] {
        thread(&mut r, id, "t1");
    }
    thread(&mut r, "other", "t2");
    for id in ["same", "other"] {
        link(&mut r, id, "goal", by(2)).unwrap();
    }
    // A done leaf is met already; an open one, however deep, holds the dependency.
    done_with(&mut r, "old", None);
    done_with(&mut r, "b", Some("c1"));
    assert_eq!(status(&r, "same"), waiting_on(&[("goal", Wait::Open)]));
    // Done is enough, in the leaves' thread or another.
    done_with(&mut r, "a", Some("c2"));
    assert_eq!(status(&r, "same"), Readiness::Ready);
    assert_eq!(status(&r, "other"), Readiness::Ready);
    // A done stored on a part, as older versions wrote, counts for nothing: its open leaf holds the dependency.
    set_state(&mut r, "a", ItemState::Backlog, by(3)).unwrap();
    set_state(&mut r, "mid", ItemState::Done, by(3)).unwrap();
    assert_eq!(status(&r, "same"), waiting_on(&[("goal", Wait::Open)]));
    // A parked parent meets nothing, whatever its leaves.
    set_state(&mut r, "goal", ItemState::Parked, by(4)).unwrap();
    assert_eq!(status(&r, "other"), waiting_on(&[("goal", Wait::Open)]));
}

#[test]
fn a_parent_reads_in_progress_while_its_open_leaves_thread_is_claimed_and_never_otherwise() {
    let mut r = replica("a");
    items(&mut r, &["goal", "x", "y"]);
    for leaf in ["x", "y"] {
        set_parent(&mut r, leaf, Some("goal"), by(2)).unwrap();
    }
    thread(&mut r, "y", "other");
    let state = |r: &Replica, now: Millis| {
        let model = Model::read(&r.state).unwrap();
        ReadinessEvaluator::new(&r.state, &model, now).progress("goal").unwrap().unwrap().stage
    };
    assert_eq!(state(&r, NOW), Some(Stage::Ready));
    // A claim lasts its lease: the parent reads in progress while it lives, and stalled once it lapses with work still ready.
    claim::claim(&mut r, "main", "session/one", 900, (500, 1_000), false).unwrap();
    assert_eq!(state(&r, NOW), Some(Stage::InProgress));
    assert_eq!(state(&r, 2_000), Some(Stage::Stalled));
    // Parked work isn't being worked, though its thread is claimed.
    set_state(&mut r, "x", ItemState::Parked, by(3)).unwrap();
    assert_eq!(state(&r, NOW), Some(Stage::Ready));
    // A state set by hand holds.
    set_state(&mut r, "x", ItemState::Backlog, by(3)).unwrap();
    set_state(&mut r, "goal", ItemState::Parked, by(3)).unwrap();
    assert_eq!(state(&r, NOW), Some(Stage::Parked));
}

#[test]
fn a_done_stored_on_a_part_hides_none_of_its_claimed_work_from_above() {
    let mut r = replica("a");
    items(&mut r, &["first", "goal", "part", "x"]);
    set_parent(&mut r, "part", Some("goal"), by(2)).unwrap();
    set_parent(&mut r, "x", Some("part"), by(2)).unwrap();
    // Waiting, so only the claim on its thread says it is under way.
    link(&mut r, "x", "first", by(2)).unwrap();
    claim::claim(&mut r, "main", "session/one", 900, (500, 1_000), false).unwrap();
    // As an older version could write it.
    set_state(&mut r, "part", ItemState::Done, by(3)).unwrap();
    let rolled = rollup(&r, "goal");
    assert_eq!(rolled.stage, Some(Stage::InProgress));
}

#[test]
fn readiness_cycles_only_include_unfinished_leaves_and_keep_canceled_waits_distinct() {
    let mut r = replica("a");
    items(&mut r, &["p", "d", "u", "x"]);
    set_parent(&mut r, "d", Some("p"), by(2)).unwrap();
    set_parent(&mut r, "u", Some("p"), by(2)).unwrap();
    link(&mut r, "p", "x", by(3)).unwrap();
    link(&mut r, "x", "d", by(3)).unwrap();
    assert_eq!(status(&r, "d"), Readiness::InCycle);
    assert_eq!(status(&r, "x"), Readiness::InCycle);

    set_state(&mut r, "d", ItemState::Done, by(4)).unwrap();
    assert_eq!(status(&r, "d"), Readiness::Done);
    assert_eq!(status(&r, "x"), Readiness::Ready);
    set_state(&mut r, "d", ItemState::Canceled, by(5)).unwrap();
    assert_eq!(status(&r, "x"), waiting_on(&[("d", Wait::Canceled)]));
    set_state(&mut r, "d", ItemState::Backlog, by(6)).unwrap();
    assert_eq!(status(&r, "d"), Readiness::InCycle);
    assert_eq!(status(&r, "x"), Readiness::InCycle);

    set_state(&mut r, "p", ItemState::Canceled, by(7)).unwrap();
    set_state(&mut r, "d", ItemState::Done, by(8)).unwrap();
    let model = Model::read(&r.state).unwrap();
    assert!(model.reads_finished("d") && !model.reads_canceled("d"));
    assert!(model.reads_finished("u") && model.reads_canceled("u"));
    assert_eq!(model.read_wait_cycles(), Vec::<Vec<String>>::new());
    assert_eq!(status(&r, "d"), Readiness::Done);
    assert_eq!(status(&r, "u"), Readiness::Canceled);
    assert_eq!(status(&r, "x"), Readiness::Ready);
}

#[test]
fn a_parked_parent_with_done_leaves_stays_parked_until_cleared() {
    let mut r = replica("a");
    items(&mut r, &["goal", "leaf", "dependent"]);
    set_parent(&mut r, "leaf", Some("goal"), by(2)).unwrap();
    link(&mut r, "dependent", "goal", by(2)).unwrap();
    done_with(&mut r, "leaf", None);
    set_state(&mut r, "goal", ItemState::Parked, by(3)).unwrap();

    let parked = Model::read(&r.state).unwrap();
    assert_eq!(parked.progress("goal"), Some(model::Progress { done: 1, canceled: 0, total: 1 }));
    assert!(!parked.reads_done("goal"));
    assert_eq!(rollup(&r, "goal").stage, Some(Stage::Parked));
    assert_eq!(status(&r, "dependent"), waiting_on(&[("goal", Wait::Open)]));

    set_state(&mut r, "goal", ItemState::Backlog, by(4)).unwrap();
    let cleared = Model::read(&r.state).unwrap();
    assert!(cleared.reads_done("goal"));
    assert_eq!(rollup(&r, "goal").stage, Some(Stage::Done));
    assert_eq!(status(&r, "dependent"), Readiness::Ready);

    set_state(&mut r, "goal", ItemState::Canceled, by(5)).unwrap();
    let canceled = Model::read(&r.state).unwrap();
    assert!(canceled.reads_canceled("goal") && !canceled.reads_done("goal"));
    assert_eq!(rollup(&r, "goal").stage, Some(Stage::Canceled));
    assert_eq!(canceled.progress("goal"), Some(model::Progress { done: 1, canceled: 0, total: 1 }));
}

#[test]
fn a_parked_parent_keeps_canceled_rollups_canceled_and_done_rollups_parked() {
    let mut r = replica("a");
    items(&mut r, &["p", "d"]);
    set_parent(&mut r, "d", Some("p"), by(2)).unwrap();
    set_state(&mut r, "p", ItemState::Parked, by(3)).unwrap();
    set_state(&mut r, "d", ItemState::Canceled, by(4)).unwrap();
    let model = Model::read(&r.state).unwrap();
    assert!(model.reads_canceled("p"));
    assert_eq!(
        ReadinessEvaluator::new(&r.state, &model, NOW).progress("p").unwrap().unwrap().stage,
        Some(Stage::Canceled)
    );

    set_state(&mut r, "d", ItemState::Done, by(5)).unwrap();
    let model = Model::read(&r.state).unwrap();
    assert!(!model.reads_done("p"));
    assert_eq!(
        ReadinessEvaluator::new(&r.state, &model, NOW).progress("p").unwrap().unwrap().stage,
        Some(Stage::Parked)
    );
}

fn stage(r: &Replica, id: &str) -> Option<Stage> {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, NOW).stage(id).unwrap()
}

fn rollup(r: &Replica, id: &str) -> Rollup {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, NOW).progress(id).unwrap().unwrap()
}

#[test]
fn a_leaf_in_a_claimed_thread_reads_in_progress_unless_it_waits() {
    let mut r = replica("a");
    items(&mut r, &["dep", "y", "parent", "kid"]);
    set_parent(&mut r, "kid", Some("parent"), by(2)).unwrap();
    assert_eq!(stage(&r, "y"), Some(Stage::Ready));
    assert_eq!(stage(&r, "parent"), None);
    // An open leaf in a claimed thread is in progress; one that waits is still waiting.
    link(&mut r, "y", "dep", by(4)).unwrap();
    claim::claim(&mut r, "main", "session/one", 900, (500, 1_000), false).unwrap();
    assert_eq!(stage(&r, "dep"), Some(Stage::InProgress));
    assert_eq!(stage(&r, "y"), Some(Stage::Waiting));
    set_state(&mut r, "y", ItemState::Parked, by(5)).unwrap();
    assert_eq!(stage(&r, "y"), Some(Stage::Parked));
    done_with(&mut r, "dep", Some("c1"));
    assert_eq!(stage(&r, "dep"), Some(Stage::Done));
    // Work in no thread is a thread of one, ready to start.
    add_item(&mut r, "loose", "loose", by(6)).unwrap();
    assert_eq!(stage(&r, "loose"), Some(Stage::Ready));
}

#[test]
fn a_parent_counts_its_leaves_by_stage_a_parked_part_counting_whole() {
    let mut r = replica("a");
    items(&mut r, &["goal", "closed", "c1", "c2", "parked", "p1", "p2", "p3", "ready"]);
    for (kid, parent) in [
        ("closed", "goal"),
        ("c1", "closed"),
        ("c2", "closed"),
        ("parked", "goal"),
        ("p1", "parked"),
        ("p2", "parked"),
        ("p3", "parked"),
        ("ready", "goal"),
    ] {
        set_parent(&mut r, kid, Some(parent), by(2)).unwrap();
    }
    // Under a part parked by hand only the open leaves park, so a done leaf stays done.
    done_with(&mut r, "c1", None);
    done_with(&mut r, "c2", None);
    set_state(&mut r, "parked", ItemState::Parked, by(3)).unwrap();
    done_with(&mut r, "p1", None);
    let rolled = rollup(&r, "goal");
    assert_eq!(rolled.stages, Stages { done: 3, parked: 2, ready: 1, ..Stages::default() });
    assert_eq!((rolled.stages.done, rolled.stages.total()), (3, 6));
    assert_eq!(rolled.stage, Some(Stage::InProgress), "done work under it moves it");
    let rolled = rollup(&r, "parked");
    assert_eq!(rolled.stages, Stages { done: 1, parked: 2, ..Stages::default() });
    assert_eq!(
        rolled.stage,
        Some(Stage::Parked),
        "a part parked by hand reads parked though work under it is done"
    );
}

#[test]
fn done_work_counts_done_though_two_clones_put_it_in_different_threads() {
    let mut a = replica("a");
    items(&mut a, &["goal", "d"]);
    set_parent(&mut a, "d", Some("goal"), by(2)).unwrap();
    done_with(&mut a, "d", None);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    thread(&mut a, "d", "t1");
    thread(&mut b, "d", "t2");
    a.join(&b.state).unwrap();
    assert_eq!(status(&a, "d"), Readiness::Conflicted { fields: vec!["thread".into()] });
    assert_eq!(stage(&a, "d"), Some(Stage::Done));
    let rolled = rollup(&a, "goal");
    assert_eq!(rolled.stages, Stages { done: 1, ..Stages::default() });
}

#[test]
fn a_parent_reads_in_progress_while_its_threads_claim_holds_even_waiting_leaves() {
    let mut r = replica("a");
    items(&mut r, &["first", "box", "x", "y"]);
    for leaf in ["x", "y"] {
        set_parent(&mut r, leaf, Some("box"), by(2)).unwrap();
        link(&mut r, leaf, "first", by(2)).unwrap();
    }
    assert_eq!(rollup(&r, "box").stage, Some(Stage::Waiting));
    claim::claim(&mut r, "main", "session/one", 900, (500, 1_000), false).unwrap();
    let rolled = rollup(&r, "box");
    assert_eq!(rolled.stage, Some(Stage::InProgress));
    assert_eq!(rolled.stages, Stages { waiting: 2, ..Stages::default() }, "the counts stay honest");
}

/// A random plan of up to eight items: parents, states, threads, blocks links, a claim and commits.
fn random_plan(tc: &TestCase, claims: bool, parks: bool) -> Replica {
    let below = |n: usize| tc.draw(gs::integers::<usize>().max_value(n - 1));
    let mut r = replica("a");
    let n = 1 + below(8);
    let ids: Vec<String> = (0..n).map(|i| format!("i{i}")).collect();
    for (i, id) in ids.iter().enumerate() {
        add_item(&mut r, id, id, by(1)).unwrap();
        if below(4) != 0 {
            thread(&mut r, id, ["t0", "t1", "t2"][below(3)]);
        }
        if i > 0 && below(3) != 0 {
            set_parent(&mut r, id, Some(&ids[below(i)]), by(2)).unwrap();
        }
    }
    for id in &ids {
        match below(5) {
            1 => done_with(&mut r, id, None),
            2 => done_with(&mut r, id, Some(["merged", "open"][below(2)])),
            3 if parks => set_state(&mut r, id, ItemState::Parked, by(3)).unwrap(),
            _ => {},
        }
        // A link the model refuses, such as one closing a cycle, is just left out.
        if below(5) == 0 {
            link(&mut r, id, &ids[below(n)], by(3)).ok();
        }
    }
    if claims && below(2) == 0 && Model::read(&r.state).unwrap().threads.contains_key("t0") {
        claim::claim(&mut r, "t0", "session/one", 900, (500, 1_000), false).unwrap();
    }
    r
}

/// A parent's leaves by stage add up to its leaves, and the done ones to its done, as the model counts them.
#[hegel::test]
fn stages_partition_a_parents_leaves_as_the_model_counts_them(tc: TestCase) {
    let (claims, parks) = (tc.draw(gs::booleans()), tc.draw(gs::booleans()));
    let r = random_plan(&tc, claims, parks);
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    for id in model.items.keys() {
        let (Some(rolled), Some(counted)) = (readiness.progress(id).unwrap(), model.progress(id))
        else {
            assert!(readiness.progress(id).unwrap().is_none() && model.progress(id).is_none());
            continue;
        };
        let stages = rolled.stages;
        assert_eq!(stages.total(), counted.total, "{id}: {stages:?}");
        assert_eq!(stages.done, counted.done, "{id}: {stages:?}");
    }
}

fn waves(r: &Replica) -> Waves {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, NOW).waves().unwrap()
}

fn ids(lists: &[&[&str]]) -> Vec<Vec<String>> {
    lists.iter().map(|list| list.iter().map(|id| (*id).to_owned()).collect()).collect()
}

/// A leaf in thread `t` waiting on one in thread `u`.
fn waits(r: &mut Replica, leaf: (&str, &str), on: (&str, &str)) {
    for (id, in_thread) in [leaf, on] {
        if !Model::read(&r.state).unwrap().items.contains_key(id) {
            add_item(r, id, id, by(1)).unwrap();
        }
        thread(r, id, in_thread);
    }
    link(r, leaf.0, on.0, by(2)).unwrap();
}

#[test]
fn threads_come_in_waves_after_the_threads_they_wait_on() {
    let mut r = replica("a");
    // t9 and t10 start; t2 waits on t9, t3 on t2 and t10; t1 waits on nothing and nothing on it.
    waits(&mut r, ("b", "t2"), ("a", "t9"));
    waits(&mut r, ("c", "t3"), ("b", "t2"));
    waits(&mut r, ("d", "t3"), ("e", "t10"));
    items(&mut r, &["lone"]);
    thread(&mut r, "lone", "t1");
    let found = waves(&r);
    assert_eq!(
        found.waves,
        ids(&[&["t9", "t10"], &["t2"], &["t3"]]),
        "natural order within a wave"
    );
    assert_eq!(found.anytime, ["t1"]);
    assert_eq!(found.waits_for["t3"], ["t2", "t10"]);
    assert_eq!(found.waits_for["t9"], Vec::<String>::new());
    // A thread whose work is all done is in none, and waits on and holds back nothing.
    done_with(&mut r, "lone", None);
    done_with(&mut r, "a", None);
    let found = waves(&r);
    assert_eq!(found.waves, ids(&[&["t2", "t10"], &["t3"]]));
    assert!(found.anytime.is_empty() && !found.waits_for.contains_key("t1"));
}

#[test]
fn a_loose_task_is_a_thread_of_one_in_the_waves_and_a_thread_waiting_on_it_comes_after() {
    let mut r = replica("a");
    items(&mut r, &["w", "w2"]);
    thread(&mut r, "w", "t");
    thread(&mut r, "w2", "t");
    add_item(&mut r, "l", "l", by(1)).unwrap();
    link(&mut r, "w", "l", by(2)).unwrap();
    let one = ids::one("l");
    let found = waves(&r);
    assert_eq!(found.waves, ids(&[&[one.as_str()], &["t"]]));
    assert_eq!(found.waits_for["t"], std::slice::from_ref(&one));
    assert_eq!(found.waits_for[&one], Vec::<String>::new());
    // A loose task nothing orders stands apart, and a done one is no thread at all.
    add_item(&mut r, "free", "free", by(1)).unwrap();
    assert_eq!(waves(&r).anytime, [ids::one("free")]);
    done_with(&mut r, "l", None);
    done_with(&mut r, "free", None);
    let found = waves(&r);
    assert_eq!(found.waves, Vec::<Vec<String>>::new());
    assert_eq!(found.anytime, ["t"]);
}

#[test]
fn finished_work_frees_its_waiters_and_a_cycle_shares_a_wave() {
    let mut r = replica("a");
    waits(&mut r, ("b", "t2"), ("a", "t1"));
    assert_eq!(waves(&r).waves, ids(&[&["t1"], &["t2"]]), "as its leaf reads waiting");
    done_with(&mut r, "a", Some("c1"));
    assert_eq!(status(&r, "b"), Readiness::Ready);
    assert_eq!(waves(&r).anytime, ["t2"], "done, t1 is finished and t2 is free");
    let mut r = replica("a");
    waits(&mut r, ("x", "t1"), ("y", "t2"));
    waits(&mut r, ("y2", "t2"), ("x2", "t1"));
    waits(&mut r, ("z", "t3"), ("y", "t2"));
    assert_eq!(waves(&r).waves, ids(&[&["t1", "t2"], &["t3"]]));
}

#[test]
fn threads_wait_on_each_other_in_a_cycle_until_the_work_holding_one_back_is_done() {
    let mut r = replica("a");
    waits(&mut r, ("x", "t1"), ("y", "t2"));
    waits(&mut r, ("y2", "t2"), ("x2", "t1"));
    let cycles = |r: &Replica| {
        let model = Model::read(&r.state).unwrap();
        ReadinessEvaluator::new(&r.state, &model, NOW).thread_cycles().unwrap()
    };
    let both = vec![BTreeSet::from(["t1".to_owned(), "t2".to_owned()])];
    assert_eq!(cycles(&r), both);
    done_with(&mut r, "x2", Some("c1"));
    assert!(cycles(&r).is_empty(), "done, x2 holds nothing");
    let mut r = replica("a");
    items(&mut r, &["x", "x2", "z", "z2"]);
    thread(&mut r, "x", "t1");
    thread(&mut r, "x2", "t1");
    for id in ["z", "z2"] {
        model::put(&mut r, &item_key(id).unwrap(), field::THREAD, Some(&"loose"), by(3)).unwrap();
    }
    link(&mut r, "x", "z", by(3)).unwrap();
    link(&mut r, "z2", "x2", by(3)).unwrap();
    assert!(cycles(&r).is_empty(), "a thread with no record is in no cycle");
}

#[test]
fn parked_work_holds_its_waiters_back_but_waits_on_nothing() {
    let mut r = replica("a");
    waits(&mut r, ("b", "t2"), ("a", "t1"));
    set_state(&mut r, "b", ItemState::Parked, by(3)).unwrap();
    let found = waves(&r);
    assert_eq!(found.anytime, ["t1", "t2"], "a parked leaf orders nothing");
    waits(&mut r, ("c", "t3"), ("b", "t2"));
    assert_eq!(waves(&r).waits_for["t3"], ["t2"], "but holds back what waits on it");
}

#[test]
fn a_link_on_a_group_counts_its_open_work_and_done_work_holds_nothing_back() {
    let mut r = replica("a");
    items(&mut r, &["box", "in1", "in2"]);
    thread(&mut r, "in1", "t1");
    thread(&mut r, "in2", "t2");
    for leaf in ["in1", "in2"] {
        set_parent(&mut r, leaf, Some("box"), by(2)).unwrap();
    }
    waits(&mut r, ("w", "t3"), ("box", "main"));
    let found = waves(&r);
    assert_eq!(found.waits_for["t3"], ["t1", "t2"]);
    done_with(&mut r, "in2", None);
    let found = waves(&r);
    assert_eq!(found.waits_for["t3"], ["t1"]);
    assert!(!found.waits_for.contains_key("t2"), "done work leaves its thread nothing to do");
}

/// Every thread with a leaf left is in one wave or apart, and no other; each thread waited on comes in an earlier
/// wave, or the same when they wait on each other; and a leaf waiting on work in another thread makes its thread wait
/// on that one.
#[hegel::test]
fn waves_order_threads_as_their_leaves_wait(tc: TestCase) {
    let r = random_plan(&tc, tc.draw(gs::booleans()), tc.draw(gs::booleans()));
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    let found = readiness.waves().unwrap();
    let wave: BTreeMap<&str, usize> = found
        .waves
        .iter()
        .enumerate()
        .flat_map(|(i, wave)| wave.iter().map(move |id| (id.as_str(), i)))
        .collect();
    let placed: Vec<&str> =
        found.waves.iter().flatten().chain(&found.anytime).map(String::as_str).collect();
    let left: BTreeSet<String> = model
        .items
        .keys()
        .filter(|id| readiness.stage(id).unwrap().is_some_and(|stage| stage != Stage::Done))
        .filter_map(|id| model.held_thread(id).map(std::borrow::Cow::into_owned))
        .collect();
    assert_eq!(placed.len(), left.len(), "each once: {found:?}");
    assert_eq!(placed.iter().map(|id| (*id).to_owned()).collect::<BTreeSet<_>>(), left);
    let reaches = |from: &str, to: &str| {
        let mut stack = vec![from];
        let mut seen = BTreeSet::new();
        while let Some(at) = stack.pop() {
            if at == to {
                return true;
            }
            if seen.insert(at) {
                stack.extend(found.waits_for[at].iter().map(String::as_str));
            }
        }
        false
    };
    for cycle in readiness.thread_cycles().unwrap() {
        let shared: BTreeSet<usize> = cycle.iter().map(|thread| wave[thread.as_str()]).collect();
        assert_eq!(shared.len(), 1, "{cycle:?} share a wave: {found:?}");
    }
    for (thread, on) in &found.waits_for {
        for first in on {
            let (a, b) = (wave[first.as_str()], wave[thread.as_str()]);
            assert!(
                a < b || (a == b && reaches(first, thread)),
                "{first} before {thread}: {found:?}"
            );
        }
    }
    for id in model.items.keys() {
        let Some(Readiness::Waiting { waiting }) = readiness.of(id).unwrap() else { continue };
        let Some(thread) = model.held_thread(id) else {
            continue;
        };
        if model.items[id].is_done() || readiness.stage(id).unwrap().is_none() {
            continue;
        }
        for dependency in waiting.keys() {
            let mut stack = vec![dependency.as_str()];
            let mut seen = BTreeSet::new();
            while let Some(at) = stack.pop() {
                if !seen.insert(at) {
                    continue;
                }
                if let Some(other) = model.held_thread(at).filter(|t| left.contains(t.as_ref()))
                    && other != thread
                    && readiness.stage(at).unwrap().is_some_and(|stage| stage != Stage::Done)
                {
                    assert!(
                        found.waits_for[thread.as_ref()].iter().any(|t| *t == other),
                        "{id} waits on {at}: {found:?}"
                    );
                }
                stack.extend(model.children(at));
            }
        }
    }
}

#[test]
fn an_actor_may_finish_what_is_ready_its_own_or_done_already() {
    let me = "session/mac";
    let claimed = |holder: &str| Readiness::Claimed { holder: holder.into() };
    let contested = |first: &str, second: &str| Readiness::Contested {
        holders: vec![first.into(), second.into()],
    };
    for (status, may) in [
        (Readiness::Ready, true),
        (claimed(me), true),
        (claimed("session/r1"), false),
        (contested(me, "session/r1"), true),
        (contested("session/r1", me), false),
        // Finishing again changes nothing, so it needs no claim.
        (Readiness::Done, true),
        (Readiness::Parent, false),
        (waiting_on(&[]), false),
    ] {
        assert_eq!(status.finishable_by(me), may, "{status:?}");
    }
}

#[test]
fn a_parent_rolls_up_its_leaves_as_started_once_work_below_has_begun_unless_what_is_left_is_parked()
{
    let of = |done, in_progress, ready, waiting, parked| Stages {
        done,
        in_progress,
        ready,
        waiting,
        parked,
        canceled: 0,
        stalled: 0,
    };
    for (stages, claimed, stage) in [
        (of(0, 0, 0, 0, 0), false, None),
        (of(3, 0, 0, 0, 0), false, Some(Stage::Done)),
        // Started wins over what is still ready.
        (of(1, 0, 2, 0, 0), false, Some(Stage::InProgress)),
        (of(0, 1, 0, 3, 0), false, Some(Stage::InProgress)),
        // A live claim below starts it, even with only waiting work.
        (of(0, 0, 0, 2, 0), true, Some(Stage::InProgress)),
        (of(0, 0, 2, 1, 1), false, Some(Stage::Ready)),
        // Stalled work reads before ready work, and once started counts as started.
        (Stages { stalled: 1, ..of(0, 0, 2, 0, 0) }, false, Some(Stage::Stalled)),
        (Stages { stalled: 1, ..of(1, 0, 0, 0, 0) }, false, Some(Stage::InProgress)),
        (of(0, 0, 0, 1, 1), false, Some(Stage::Waiting)),
        // What is left all parked, by hand or under a parked part, reads parked however much is done.
        (of(2, 0, 0, 0, 3), false, Some(Stage::Parked)),
        // Canceled leaves are left out; with nothing else the parent reads canceled.
        (Stages { canceled: 2, ..of(0, 0, 0, 0, 0) }, false, Some(Stage::Canceled)),
        (Stages { canceled: 2, ..of(1, 0, 0, 0, 0) }, false, Some(Stage::Done)),
        (Stages { canceled: 1, ..of(0, 0, 1, 0, 0) }, false, Some(Stage::Ready)),
    ] {
        assert_eq!(stages.rolled_up(claimed), stage, "{stages:?} claimed {claimed}");
    }
}

/// A replica with items `x` and `y` in `main`, claimed by `session/a` from 10 for 100: the lease ends at 110.
fn lapsing() -> Replica {
    let mut r = replica("a");
    items(&mut r, &["x", "y"]);
    claim::claim(&mut r, "main", "session/a", 10, (100, 100), false).unwrap();
    r
}

fn main_at(r: &Replica, at: Millis) -> ThreadReadiness {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, at).thread("main").unwrap()
}

fn stage_at(r: &Replica, id: &str, at: Millis) -> Option<Stage> {
    let model = Model::read(&r.state).unwrap();
    ReadinessEvaluator::new(&r.state, &model, at).stage(id).unwrap()
}

#[test]
fn a_thread_holding_only_expired_claims_with_ready_work_is_stalled_and_its_items_read_stalled() {
    let mut r = lapsing();
    let both = vec!["x".to_owned(), "y".to_owned()];
    let held = ThreadReadiness::Claimed { holder: "session/a".into(), items: both.clone() };
    assert_eq!(main_at(&r, 109), held, "live until the lease ends");
    let stalled = ThreadReadiness::Stalled {
        holder: "session/a".into(),
        expired_at: crate::time::rfc3339(110),
        items: both,
    };
    assert_eq!(main_at(&r, 110), stalled);
    assert!(stalled.claimable_by("session/b"), "anyone may take it");
    assert_eq!(stage_at(&r, "x", 109), Some(Stage::InProgress));
    assert_eq!(stage_at(&r, "x", 110), Some(Stage::Stalled));
    // The work is still ready: only the stage reads stalled.
    let model = Model::read(&r.state).unwrap();
    assert_eq!(
        ReadinessEvaluator::new(&r.state, &model, 110).of("x").unwrap(),
        Some(Readiness::Ready)
    );
    // A parent counts its stalled leaves, and reads stalled when nothing else is going on.
    add_item(&mut r, "p", "p", by(1)).unwrap();
    set_parent(&mut r, "x", Some("p"), by(1)).unwrap();
    set_parent(&mut r, "y", Some("p"), by(1)).unwrap();
    let model = Model::read(&r.state).unwrap();
    let rollup = ReadinessEvaluator::new(&r.state, &model, 110).progress("p").unwrap().unwrap();
    assert_eq!(
        (rollup.stages.stalled, rollup.stages.total(), rollup.stage),
        (2, 2, Some(Stage::Stalled))
    );
}

#[test]
fn a_thread_whose_remaining_work_all_waits_is_waiting_not_stalled() {
    let mut r = lapsing();
    add_item(&mut r, "gate", "gate", by(1)).unwrap();
    link(&mut r, "x", "gate", by(1)).unwrap();
    link(&mut r, "y", "gate", by(1)).unwrap();
    assert_eq!(main_at(&r, 500), ThreadReadiness::NothingReady);
    assert_eq!(stage_at(&r, "x", 500), Some(Stage::Waiting));
}

#[test]
fn releasing_finishing_everything_or_a_fresh_claim_ends_a_stall() {
    let r = lapsing();
    assert!(matches!(main_at(&r, 500), ThreadReadiness::Stalled { .. }));
    let mut released = r.clone();
    assert!(claim::release(&mut released, &claim::thread_key("main").unwrap(), "session/a"));
    assert!(matches!(main_at(&released, 500), ThreadReadiness::Ready { .. }));
    let mut done = r.clone();
    for id in ["x", "y"] {
        done_with(&mut done, id, None);
    }
    assert_eq!(main_at(&done, 500), ThreadReadiness::NothingReady);
    // A fresh claim, with the check on, overwrites the expired value rather than adding to it.
    let mut taken = r.clone();
    claim::claim(&mut taken, "main", "session/b", 500, (100, 100), true).unwrap();
    let items = vec!["x".to_owned(), "y".to_owned()];
    assert_eq!(
        main_at(&taken, 501),
        ThreadReadiness::Claimed { holder: "session/b".into(), items }
    );
    let key = claim::thread_key("main").unwrap();
    assert_eq!(
        taken.state.field(&key, claim::FIELD).unwrap().len(),
        1,
        "the expired value is gone"
    );
    // The old holder resumes the same way.
    claim::claim(&mut r.clone(), "main", "session/a", 500, (100, 100), true).unwrap();
}

#[test]
fn ready_lists_a_stalled_thread_in_id_order_among_the_unclaimed_marked_stalled() {
    let mut r = replica("a");
    for (item, thread_id) in [("w", "t1"), ("x", "t2"), ("y", "t3"), ("z", "t0")] {
        add_item(&mut r, item, item, by(1)).unwrap();
        thread(&mut r, item, thread_id);
    }
    claim::claim(&mut r, "t2", "session/old", 10, (100, 100), false).unwrap();
    claim::claim(&mut r, "t3", "session/me", 400, (200, 200), false).unwrap();
    let model = Model::read(&r.state).unwrap();
    let listed = ReadinessEvaluator::new(&r.state, &model, 500).ready("session/me").unwrap();
    let rows: Vec<(&str, bool)> = listed.iter().map(|t| (t.thread.as_str(), t.stalled)).collect();
    // The actor's own claimed thread first, then the unclaimed and stalled by id.
    assert_eq!(rows, [("t3", false), ("t0", false), ("t1", false), ("t2", true)]);
}

#[test]
fn a_loose_task_is_one_ready_thread_until_claimed_and_is_counted_once_at_each_stage() {
    let mut r = replica("a");
    add_item(&mut r, "x", "x", by(1)).unwrap();
    let one = ids::one("x");
    let listed = |r: &Replica, actor: &str| -> Vec<String> {
        let model = Model::read(&r.state).unwrap();
        let ready = ReadinessEvaluator::new(&r.state, &model, NOW).ready(actor).unwrap();
        ready.into_iter().map(|t| t.thread).collect()
    };
    let counted = |r: &Replica| -> Stages {
        let model = Model::read(&r.state).unwrap();
        let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
        Stages::of_leaves(&readiness, ["x"]).unwrap()
    };
    assert_eq!(listed(&r, "session/a"), std::slice::from_ref(&one));
    assert_eq!(counted(&r), Stages { ready: 1, ..Stages::default() });
    claim::claim(&mut r, &one, "session/a", 900, (500, 1_000), true).unwrap();
    assert_eq!(listed(&r, "session/a"), std::slice::from_ref(&one), "its holder still sees it");
    assert_eq!(listed(&r, "session/b"), Vec::<String>::new());
    assert_eq!(counted(&r), Stages { in_progress: 1, ..Stages::default() });
    assert_eq!(status(&r, "x"), Readiness::Claimed { holder: "session/a".into() });
}
