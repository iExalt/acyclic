use crate::model::SetField;
use hegel::TestCase;
use hegel::generators as gs;
use std::cell::Cell;

use super::*;
use crate::model::{self, By, add_item, item_key, link, set, set_element, set_parent, set_state};
use crate::schema::{item as field, kind, repository, thread as thread_field};
use crate::state::{Key, Replica, ReplicaId};

const NOW: Millis = 1_000;

/// Ancestry from a table of commits, counting the checks it answers.
#[derive(Default)]
struct Fake {
    commits: BTreeMap<String, Result<bool, String>>,
    calls: Cell<usize>,
}

impl Fake {
    fn with(commits: &[(&str, Result<bool, String>)]) -> Self {
        Self {
            commits: commits.iter().map(|(c, r)| ((*c).to_owned(), r.clone())).collect(),
            calls: Cell::new(0),
        }
    }
}

impl Ancestry for Fake {
    fn merged(&self, _: &Repository, commit: &str) -> Result<bool, String> {
        self.calls.set(self.calls.get() + 1);
        self.commits.get(commit).cloned().unwrap_or_else(|| Err("unknown commit".to_owned()))
    }
}

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
        note: None,
    };
    set(r, &item_key(id).unwrap(), field::EVIDENCE, Some(&evidence), by(2)).unwrap();
}

/// Puts an item in a thread, giving the thread a record.
fn thread(r: &mut Replica, id: &str, thread: &str) {
    set(r, &claim::thread_key(thread).unwrap(), thread_field::TITLE, Some(&thread), by(1)).unwrap();
    model::put(r, &item_key(id).unwrap(), field::THREAD, Some(&thread), by(2)).unwrap();
}

fn status(r: &Replica, ancestry: &dyn Ancestry, id: &str) -> Status {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, NOW, ancestry).of(id).unwrap().unwrap()
}

/// The ready items, thread by thread, in the order `ready` lists them.
fn flat(threads: &[ReadyThread]) -> Vec<String> {
    threads.iter().flat_map(|t| t.items.iter().cloned()).collect()
}

fn blocked(waits: &[(&str, Wait)]) -> Status {
    Status::Blocked { waiting: waits.iter().map(|(id, w)| ((*id).to_owned(), w.clone())).collect() }
}

#[test]
fn a_hard_dependency_blocks_until_done_and_a_stored_after_link_never_does() {
    let mut r = replica("a");
    items(&mut r, &["dep", "soft", "x"]);
    link(&mut r, "x", "dep", LinkKind::Blocks, by(2)).unwrap();
    model::store_after_link(&mut r, "x", "soft", by(2));
    let none = Fake::default();
    assert_eq!(status(&r, &none, "x"), blocked(&[("dep", Wait::Open)]));
    // Work with no repository is met when done.
    done_with(&mut r, "dep", None);
    assert_eq!(status(&r, &none, "x"), Status::Ready);
    done_with(&mut r, "soft", None);
    assert_eq!(status(&r, &none, "x"), Status::Ready);
}

#[test]
fn done_inside_a_thread_merged_across() {
    let mut r = replica("a");
    items(&mut r, &["dep", "same", "other", "loose"]);
    for id in ["same", "other", "loose"] {
        link(&mut r, id, "dep", LinkKind::Blocks, by(2)).unwrap();
    }
    done_with(&mut r, "dep", Some("c1"));
    thread(&mut r, "dep", "t1");
    thread(&mut r, "same", "t1");
    thread(&mut r, "other", "t2");
    model::put(&mut r, &item_key("loose").unwrap(), field::THREAD, None::<&String>, by(2)).unwrap();
    let unmerged = Fake::with(&[("c1", Ok(false))]);
    assert_eq!(status(&r, &unmerged, "same"), Status::Ready);
    assert_eq!(status(&r, &unmerged, "other"), blocked(&[("dep", Wait::InReview)]));
    // An item in no thread needs the merge too.
    assert_eq!(status(&r, &unmerged, "loose"), blocked(&[("dep", Wait::InReview)]));
    assert_eq!(status(&r, &unmerged, "dep"), Status::InReview);
    let merged = Fake::with(&[("c1", Ok(true))]);
    assert_eq!(status(&r, &merged, "other"), Status::Ready);
    assert_eq!(status(&r, &merged, "dep"), Status::Done);
}

#[test]
fn an_unverifiable_merge_says_why() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "dep2", "y"]);
    link(&mut r, "x", "dep", LinkKind::Blocks, by(2)).unwrap();
    done_with(&mut r, "dep", Some("c1"));
    // Across threads, so the merge matters.
    for id in ["dep", "dep2"] {
        thread(&mut r, id, "deps");
    }
    let broken = Fake::with(&[("c1", Err("no clone of the repository is mapped".to_owned()))]);
    let unverified = Wait::Unverified {
        repository: "code".to_owned(),
        reason: "no clone of the repository is mapped".to_owned(),
    };
    assert_eq!(status(&r, &broken, "x"), blocked(&[("dep", unverified)]));
    // Evidence naming a repository with no record.
    link(&mut r, "y", "dep2", LinkKind::Blocks, by(2)).unwrap();
    set_state(&mut r, "dep2", ItemState::Done, by(3)).unwrap();
    let evidence =
        Evidence { repository: Some("gone".into()), commit: Some("c2".into()), note: None };
    set(&mut r, &item_key("dep2").unwrap(), field::EVIDENCE, Some(&evidence), by(3)).unwrap();
    assert!(matches!(
        status(&r, &broken, "y"),
        Status::Blocked { waiting } if matches!(&waiting["dep2"], Wait::Unverified { repository, .. } if repository == "gone")
    ));
}

#[test]
fn an_archived_dependency_is_met() {
    let mut r = replica("a");
    items(&mut r, &["goal", "dep", "x"]);
    set_parent(&mut r, "dep", Some("goal"), by(2)).unwrap();
    link(&mut r, "x", "dep", LinkKind::Blocks, by(2)).unwrap();
    done_with(&mut r, "dep", Some("c1"));
    model::put(&mut r, &item_key("goal").unwrap(), field::ARCHIVED, Some(&true), by(3)).unwrap();
    let unmerged = Fake::with(&[("c1", Ok(false))]);
    assert_eq!(status(&r, &unmerged, "x"), Status::Ready);
    assert_eq!(status(&r, &unmerged, "dep"), Status::Archived);
}

#[test]
fn a_parents_dependencies_hold_back_its_children_and_a_parent_done_by_hand_is_met() {
    let mut r = replica("a");
    items(&mut r, &["goal", "child", "dep", "after-goal"]);
    set_parent(&mut r, "child", Some("goal"), by(2)).unwrap();
    link(&mut r, "goal", "dep", LinkKind::Blocks, by(2)).unwrap();
    link(&mut r, "after-goal", "goal", LinkKind::Blocks, by(2)).unwrap();
    let none = Fake::default();
    assert_eq!(status(&r, &none, "child"), blocked(&[("dep", Wait::Open)]));
    assert_eq!(status(&r, &none, "goal"), Status::Parent);
    assert_eq!(status(&r, &none, "after-goal"), blocked(&[("goal", Wait::Open)]));
    // Finishing the parent waits on its child, and on its own dependency once the child is done.
    let unfinished = |r: &Replica| {
        let model = Model::read(&r.state).unwrap();
        Readiness::new(&r.state, &model, 5, &none).unfinished("goal")
    };
    assert_eq!(unfinished(&r), (vec!["child".to_owned()], Some(blocked(&[("dep", Wait::Open)]))));
    done_with(&mut r, "child", None);
    assert_eq!(unfinished(&r), (vec![], Some(blocked(&[("dep", Wait::Open)]))));
    done_with(&mut r, "dep", None);
    assert_eq!(unfinished(&r), (vec![], None));
    set_state(&mut r, "goal", ItemState::Parked, by(4)).unwrap();
    assert_eq!(unfinished(&r).1, Some(Status::Parked));
    set_state(&mut r, "goal", ItemState::Backlog, by(4)).unwrap();
    done_with(&mut r, "goal", None);
    assert_eq!(status(&r, &none, "after-goal"), Status::Ready);
}

#[test]
fn cycles_conflicts_and_parked_items_are_not_ready() {
    let mut a = replica("a");
    items(&mut a, &["x", "y", "p", "c"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    link(&mut a, "x", "y", LinkKind::Blocks, by(2)).unwrap();
    link(&mut b, "y", "x", LinkKind::Blocks, by(2)).unwrap();
    set_state(&mut a, "c", ItemState::InProgress, by(2)).unwrap();
    set_state(&mut b, "c", ItemState::Done, by(2)).unwrap();
    thread(&mut a, "p", "t1");
    thread(&mut b, "p", "t2");
    a.join(&b.state).unwrap();
    set_state(&mut a, "p", ItemState::Parked, by(3)).unwrap();
    let none = Fake::default();
    assert_eq!(status(&a, &none, "x"), Status::InCycle);
    assert_eq!(status(&a, &none, "c"), Status::Conflicted { fields: vec!["state".into()] });
    assert_eq!(status(&a, &none, "p"), Status::Conflicted { fields: vec!["thread".into()] });
    thread(&mut a, "p", "t1");
    assert_eq!(status(&a, &none, "p"), Status::Parked);
}

#[test]
fn a_threads_claim_shows_on_its_members_and_a_contest_names_its_winner_first() {
    let mut a = replica("a");
    items(&mut a, &["x"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    claim::claim(&mut a, "main", "session/one", 900, 500, 1_000).unwrap();
    let none = Fake::default();
    assert_eq!(status(&a, &none, "x"), Status::Claimed { holder: "session/one".into() });
    claim::claim(&mut b, "main", "session/two", 950, 500, 1_000).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(
        status(&a, &none, "x"),
        Status::Contested { holders: vec!["session/one".into(), "session/two".into()] }
    );
}

#[test]
fn an_unthreaded_leaf_is_never_ready_but_what_holds_it_back_shows_first() {
    let mut r = replica("a");
    for id in ["loose", "ghostly", "dep", "waiting"] {
        add_item(&mut r, id, id, by(1)).unwrap();
    }
    // A thread named but with no record holds nothing.
    model::put(&mut r, &item_key("ghostly").unwrap(), field::THREAD, Some(&"ghost"), by(2))
        .unwrap();
    link(&mut r, "waiting", "dep", LinkKind::Blocks, by(2)).unwrap();
    let none = Fake::default();
    for id in ["loose", "ghostly"] {
        assert_eq!(status(&r, &none, id), Status::Unthreaded, "{id}");
    }
    assert_eq!(status(&r, &none, "waiting"), blocked(&[("dep", Wait::Open)]));
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, NOW, &none);
    assert_eq!(readiness.ready(false, "session/s").unwrap(), []);
    assert_eq!(readiness.thread("ghost").unwrap(), ThreadStatus::Missing);
}

#[test]
fn a_thread_is_claimable_by_anyone_while_ready_and_by_its_holder_while_held() {
    let mut a = replica("a");
    items(&mut a, &["x", "blocked", "dep"]);
    link(&mut a, "blocked", "dep", LinkKind::Blocks, by(2)).unwrap();
    for (id, held) in [("y", "other"), ("idle", "quiet")] {
        add_item(&mut a, id, id, by(1)).unwrap();
        thread(&mut a, id, held);
    }
    set_state(&mut a, "idle", ItemState::Parked, by(2)).unwrap();
    let none = Fake::default();
    let at = |r: &Replica, thread: &str| {
        let model = Model::read(&r.state).unwrap();
        Readiness::new(&r.state, &model, NOW, &none).thread(thread).unwrap()
    };
    let ready = |r: &Replica, actor: &str| {
        let model = Model::read(&r.state).unwrap();
        flat(&Readiness::new(&r.state, &model, NOW, &none).ready(false, actor).unwrap())
    };
    let workable = vec!["dep".to_owned(), "x".to_owned()];
    assert_eq!(at(&a, "main"), ThreadStatus::Ready { items: workable.clone() });
    assert_eq!(at(&a, "quiet"), ThreadStatus::NothingReady);
    assert!(!at(&a, "quiet").claimable_by("session/one"));
    assert_eq!(ready(&a, "session/one"), ["dep", "x", "y"]);
    let unclaimed = a.state.clone();
    claim::claim(&mut a, "main", "session/one", 900, 500, 1_000).unwrap();
    // Its members stay workable for the holder alone.
    let held = at(&a, "main");
    assert_eq!(held, ThreadStatus::Claimed { holder: "session/one".into(), items: workable });
    assert!(held.claimable_by("session/one") && !held.claimable_by("session/two"));
    assert_eq!(ready(&a, "session/one"), ["dep", "x", "y"]);
    assert_eq!(ready(&a, "session/two"), ["y"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // A contest's winner may go on; its loser may not.
    let mut c = replica("c");
    c.join(&unclaimed).unwrap();
    claim::claim(&mut c, "main", "session/two", 950, 500, 1_000).unwrap();
    b.join(&c.state).unwrap();
    let contested = at(&b, "main");
    assert!(
        matches!(&contested, ThreadStatus::Contested { holders, .. } if holders[0] == "session/one")
    );
    assert!(contested.claimable_by("session/one") && !contested.claimable_by("session/two"));
}

#[test]
fn ready_lists_leaves_by_id_and_checks_each_needed_commit_once() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "y", "z", "unrelated", "goal", "archived-leaf"]);
    for id in ["x", "y"] {
        link(&mut r, id, "dep", LinkKind::Blocks, by(2)).unwrap();
    }
    model::store_after_link(&mut r, "z", "x", by(2));
    done_with(&mut r, "dep", Some("c1"));
    thread(&mut r, "dep", "deps");
    done_with(&mut r, "unrelated", Some("c2"));
    set_parent(&mut r, "archived-leaf", Some("goal"), by(2)).unwrap();
    model::put(&mut r, &item_key("goal").unwrap(), field::ARCHIVED, Some(&true), by(2)).unwrap();
    set_element(&mut r, &item_key("x").unwrap(), SetField::Area, "ui", true, by(2)).unwrap();
    let ancestry = Fake::with(&[("c1", Ok(true)), ("c2", Ok(true))]);
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, NOW, &ancestry);
    let ready = flat(&readiness.ready(false, "session/s").unwrap());
    assert_eq!(ready, ["x", "y", "z"], "an open after link holds nothing back");
    assert_eq!(
        ancestry.calls.get(),
        1,
        "c1 once for two dependents, and never c2, which nothing needs"
    );
    let with_archived = flat(&readiness.ready(true, "session/s").unwrap());
    assert!(with_archived.contains(&"archived-leaf".to_owned()));
}

#[test]
fn git_ancestry_takes_only_commit_ids() {
    let ancestry = GitAncestry::new(|_: &Repository| Some(PathBuf::from("/nonexistent")));
    let model = Model::read(&replica("a").state).unwrap();
    let record = &model.repositories["code"];
    for bad in ["--foo", "abc", "zzzzzzz", &"a".repeat(65)] {
        assert_eq!(ancestry.merged(record, bad), Err("not a commit id".to_owned()), "{bad}");
    }
}

#[test]
fn a_held_resource_keeps_another_threads_work_waiting_until_its_claim_is_released() {
    let mut r = replica("a");
    items(&mut r, &["x"]);
    add_item(&mut r, "y", "y", by(1)).unwrap();
    thread(&mut r, "y", "other");
    for id in ["x", "y"] {
        set_element(&mut r, &item_key(id).unwrap(), SetField::Resource, "gpu", true, by(2))
            .unwrap();
    }
    claim::claim(&mut r, "main", "session/one", 900, 500, 1_000).unwrap();
    let none = Fake::default();
    let busy = Status::ResourceBusy {
        resource: "gpu".into(),
        thread: "main".into(),
        holder: "session/one".into(),
    };
    assert_eq!(status(&r, &none, "y"), busy);
    let model = Model::read(&r.state).unwrap();
    assert_eq!(
        Readiness::new(&r.state, &model, NOW, &none).thread("other").unwrap(),
        ThreadStatus::ResourceBusy {
            resource: "gpu".into(),
            thread: "main".into(),
            holder: "session/one".into()
        }
    );
    claim::release(&mut r, &claim::thread_key("main").unwrap(), "session/one");
    assert_eq!(status(&r, &none, "x"), Status::Ready);
    assert_eq!(status(&r, &none, "y"), Status::Ready);
}

#[test]
fn ready_lists_the_actors_thread_first_then_approved_ones_by_natural_title_and_leaves_out_others_held()
 {
    let mut r = replica("a");
    // Thread ids run against their titles, so only the titles can order them.
    for (item, held, title) in [
        ("i1", "a", "Step 10"),
        ("i2", "b", "Step 2"),
        ("i3", "c", "Step 1"),
        ("i4", "d", "Mine"),
        ("i5", "e", "Theirs"),
        ("i6", "f", "Step 2"),
    ] {
        add_item(&mut r, item, item, by(1)).unwrap();
        thread(&mut r, item, held);
        set(&mut r, &claim::thread_key(held).unwrap(), thread_field::TITLE, Some(&title), by(1))
            .unwrap();
    }
    set(&mut r, &claim::thread_key("c").unwrap(), thread_field::APPROVED, Some(&true), by(1))
        .unwrap();
    // Items within a thread sort by title in natural order too, then by id.
    for (item, title) in [("m10", "Part 10"), ("m2", "Part 2"), ("m1", "Part 2")] {
        add_item(&mut r, item, title, by(1)).unwrap();
        thread(&mut r, item, "d");
    }
    claim::claim(&mut r, "d", "session/me", 900, 500, 1_000).unwrap();
    claim::claim(&mut r, "e", "session/other", 900, 500, 1_000).unwrap();
    let model = Model::read(&r.state).unwrap();
    let ready =
        Readiness::new(&r.state, &model, NOW, &Fake::default()).ready(false, "session/me").unwrap();
    let order: Vec<(&str, Vec<&str>)> = ready
        .iter()
        .map(|t| (t.thread.as_str(), t.items.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(
        order,
        [
            ("d", vec!["i4", "m1", "m2", "m10"]),
            ("c", vec!["i3"]),
            ("b", vec!["i2"]),
            ("f", vec!["i6"]),
            ("a", vec!["i1"]),
        ]
    );
}

#[test]
fn an_item_is_busy_whenever_its_thread_is_even_through_another_members_resource() {
    let mut r = replica("a");
    items(&mut r, &["free", "parked"]);
    set_state(&mut r, "parked", ItemState::Parked, by(2)).unwrap();
    add_item(&mut r, "theirs", "theirs", by(1)).unwrap();
    thread(&mut r, "theirs", "other");
    for id in ["parked", "theirs"] {
        set_element(&mut r, &item_key(id).unwrap(), SetField::Resource, "gpu", true, by(2))
            .unwrap();
    }
    claim::claim(&mut r, "other", "session/other", 900, 500, 1_000).unwrap();
    let none = Fake::default();
    let busy = Status::ResourceBusy {
        resource: "gpu".into(),
        thread: "other".into(),
        holder: "session/other".into(),
    };
    assert_eq!(status(&r, &none, "free"), busy, "show agrees with the thread");
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, NOW, &none);
    assert!(matches!(readiness.thread("main").unwrap(), ThreadStatus::ResourceBusy { .. }));
    assert_eq!(readiness.ready(false, "session/me").unwrap(), []);
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
        link(&mut r, id, "goal", LinkKind::Blocks, by(2)).unwrap();
    }
    // An archived leaf is met already; an open one, however deep, holds the dependency.
    model::put(&mut r, &item_key("old").unwrap(), field::ARCHIVED, Some(&true), by(2)).unwrap();
    done_with(&mut r, "b", Some("c1"));
    let unmerged = Fake::with(&[("c1", Ok(false)), ("c2", Ok(false))]);
    assert_eq!(status(&r, &unmerged, "same"), blocked(&[("goal", Wait::Open)]));
    // Done in the leaves' thread is enough there; across threads each leaf's commit must be merged.
    done_with(&mut r, "a", Some("c2"));
    assert_eq!(status(&r, &unmerged, "same"), Status::Ready);
    assert_eq!(status(&r, &unmerged, "other"), blocked(&[("goal", Wait::InReview)]));
    let merged = Fake::with(&[("c1", Ok(true)), ("c2", Ok(true))]);
    assert_eq!(status(&r, &merged, "other"), Status::Ready);
    // A part closed by hand counts as closed, whatever is open under it.
    set_state(&mut r, "a", ItemState::InProgress, by(3)).unwrap();
    assert_eq!(status(&r, &merged, "same"), blocked(&[("goal", Wait::Open)]));
    set_state(&mut r, "mid", ItemState::Done, by(3)).unwrap();
    assert_eq!(status(&r, &merged, "same"), Status::Ready);
    // A parked parent meets nothing, whatever its leaves.
    set_state(&mut r, "goal", ItemState::Parked, by(4)).unwrap();
    assert_eq!(status(&r, &merged, "other"), blocked(&[("goal", Wait::Open)]));
}

#[test]
fn a_parent_reads_in_progress_while_its_open_leaves_thread_is_claimed_and_never_otherwise() {
    let mut r = replica("a");
    items(&mut r, &["goal", "x", "y"]);
    for leaf in ["x", "y"] {
        set_parent(&mut r, leaf, Some("goal"), by(2)).unwrap();
    }
    thread(&mut r, "y", "other");
    let none = Fake::default();
    let state = |r: &Replica, now: Millis| {
        let model = Model::read(&r.state).unwrap();
        Readiness::new(&r.state, &model, now, &none)
            .progress("goal")
            .unwrap()
            .unwrap()
            .progress
            .state
    };
    assert_eq!(state(&r, NOW), ItemState::Backlog);
    // A claim lasts its lease: the parent reads in progress while it lives, and backlog once it lapses.
    claim::claim(&mut r, "main", "session/one", 900, 500, 1_000).unwrap();
    assert_eq!(state(&r, NOW), ItemState::InProgress);
    assert_eq!(state(&r, 2_000), ItemState::Backlog);
    // Parked work isn't being worked, though its thread is claimed.
    set_state(&mut r, "x", ItemState::Parked, by(3)).unwrap();
    assert_eq!(state(&r, NOW), ItemState::Backlog);
    // A state set by hand holds.
    set_state(&mut r, "goal", ItemState::Parked, by(3)).unwrap();
    assert_eq!(state(&r, NOW), ItemState::Parked);
}

fn stage(r: &Replica, id: &str) -> Option<Stage> {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, NOW, &Fake::default()).stage(id).unwrap()
}

fn rollup(r: &Replica, id: &str) -> Rollup {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, NOW, &Fake::default()).progress(id).unwrap().unwrap()
}

#[test]
fn a_leaf_started_by_hand_reads_in_progress_whatever_holds_it_and_a_claim_starts_the_rest() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "y", "parent", "kid"]);
    set_parent(&mut r, "kid", Some("parent"), by(2)).unwrap();
    assert_eq!(stage(&r, "x"), Some(Stage::Ready));
    assert_eq!(stage(&r, "parent"), None);
    // Started with nothing holding it, and still so once a link is added after it started.
    set_state(&mut r, "x", ItemState::InProgress, by(3)).unwrap();
    assert_eq!(stage(&r, "x"), Some(Stage::InProgress));
    link(&mut r, "x", "dep", LinkKind::Blocks, by(4)).unwrap();
    assert_eq!(stage(&r, "x"), Some(Stage::InProgress));
    // A backlog leaf in a claimed thread is in progress; one that waits is still blocked.
    link(&mut r, "y", "dep", LinkKind::Blocks, by(4)).unwrap();
    claim::claim(&mut r, "main", "session/one", 900, 500, 1_000).unwrap();
    assert_eq!(stage(&r, "dep"), Some(Stage::InProgress));
    assert_eq!(stage(&r, "y"), Some(Stage::Blocked));
    set_state(&mut r, "y", ItemState::Parked, by(5)).unwrap();
    assert_eq!(stage(&r, "y"), Some(Stage::Parked));
    done_with(&mut r, "dep", Some("c1"));
    assert_eq!(stage(&r, "dep"), Some(Stage::InReview));
    // Out of every thread, started work is still in progress, and work not started is in no thread.
    model::put::<String>(&mut r, &item_key("x").unwrap(), field::THREAD, None, by(6)).unwrap();
    assert_eq!(stage(&r, "x"), Some(Stage::InProgress));
    add_item(&mut r, "loose", "loose", by(6)).unwrap();
    assert_eq!(stage(&r, "loose"), Some(Stage::Unthreaded));
}

#[test]
fn a_parent_counts_its_leaves_by_stage_closed_and_parked_parts_counting_whole() {
    let mut r = replica("a");
    items(
        &mut r,
        &["goal", "closed", "c1", "c2", "parked", "p1", "p2", "p3", "gone", "g1", "ready"],
    );
    for (kid, parent) in [
        ("closed", "goal"),
        ("c1", "closed"),
        ("c2", "closed"),
        ("parked", "goal"),
        ("p1", "parked"),
        ("p2", "parked"),
        ("p3", "parked"),
        ("gone", "goal"),
        ("g1", "gone"),
        ("ready", "goal"),
    ] {
        set_parent(&mut r, kid, Some(parent), by(2)).unwrap();
    }
    // A part closed by hand counts every leaf as done; under a part parked by hand only the open ones park, so a done
    // leaf stays done and a started one neither counts as in progress nor moves the parent.
    set_state(&mut r, "closed", ItemState::Done, by(3)).unwrap();
    set_state(&mut r, "parked", ItemState::Parked, by(3)).unwrap();
    done_with(&mut r, "p1", None);
    set_state(&mut r, "p2", ItemState::InProgress, by(3)).unwrap();
    model::put(&mut r, &item_key("gone").unwrap(), field::ARCHIVED, Some(&true), by(3)).unwrap();
    let rolled = rollup(&r, "goal");
    assert_eq!(
        rolled.stages,
        Stages { done: 3, parked: 2, ready: 1, ..Stages::default() },
        "the archived part counts for nothing"
    );
    assert_eq!((rolled.progress.done, rolled.progress.total), (3, 6));
    assert_eq!(rolled.progress.state, ItemState::InProgress, "done work under it moves it");
    let rolled = rollup(&r, "parked");
    assert_eq!(rolled.stages, Stages { done: 1, parked: 2, ..Stages::default() });
    assert_eq!(rolled.progress.state, ItemState::Parked);
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
    assert_eq!(
        status(&a, &Fake::default(), "d"),
        Status::Conflicted { fields: vec!["thread".into()] }
    );
    assert_eq!(stage(&a, "d"), Some(Stage::Done));
    let rolled = rollup(&a, "goal");
    assert_eq!(rolled.stages, Stages { done: 1, ..Stages::default() });
}

#[test]
fn a_parent_reads_in_progress_while_its_threads_claim_holds_even_blocked_leaves() {
    let mut r = replica("a");
    items(&mut r, &["first", "box", "x", "y"]);
    for leaf in ["x", "y"] {
        set_parent(&mut r, leaf, Some("box"), by(2)).unwrap();
        link(&mut r, leaf, "first", LinkKind::Blocks, by(2)).unwrap();
    }
    assert_eq!(rollup(&r, "box").progress.state, ItemState::Backlog);
    claim::claim(&mut r, "main", "session/one", 900, 500, 1_000).unwrap();
    let rolled = rollup(&r, "box");
    assert_eq!(rolled.progress.state, ItemState::InProgress);
    assert_eq!(rolled.stages, Stages { blocked: 2, ..Stages::default() }, "the counts stay honest");
}

/// A random plan of up to eight items: parents, states, threads, blocks links, a claim, archives and commits.
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
            0 => set_state(&mut r, id, ItemState::InProgress, by(3)).unwrap(),
            1 => done_with(&mut r, id, None),
            2 => done_with(&mut r, id, Some(["merged", "open"][below(2)])),
            3 if parks => set_state(&mut r, id, ItemState::Parked, by(3)).unwrap(),
            _ => {},
        }
        // A link the model refuses, such as one closing a cycle, is just left out.
        if below(5) == 0 {
            link(&mut r, id, &ids[below(n)], LinkKind::Blocks, by(3)).ok();
        }
        if below(8) == 0 {
            model::put(&mut r, &item_key(id).unwrap(), field::ARCHIVED, Some(&true), by(3))
                .unwrap();
        }
    }
    if claims && below(2) == 0 && Model::read(&r.state).unwrap().threads.contains_key("t0") {
        claim::claim(&mut r, "t0", "session/one", 900, 500, 1_000).unwrap();
    }
    r
}

/// A parent's leaves by stage add up to its leaves, the done and in-review ones to its done, and its done and leaves
/// are the model's own; with no claims and no parked parts, its state is the model's too.
#[hegel::test]
fn stages_partition_a_parents_leaves_as_the_model_counts_them(tc: TestCase) {
    let (claims, parks) = (tc.draw(gs::booleans()), tc.draw(gs::booleans()));
    let r = random_plan(&tc, claims, parks);
    let model = Model::read(&r.state).unwrap();
    let ancestry = Fake::with(&[("merged", Ok(true)), ("open", Ok(false))]);
    let readiness = Readiness::new(&r.state, &model, NOW, &ancestry);
    for id in model.items.keys() {
        let (Some(rolled), Some(counted)) = (readiness.progress(id).unwrap(), model.progress(id))
        else {
            assert!(readiness.progress(id).unwrap().is_none() && model.progress(id).is_none());
            continue;
        };
        let stages = rolled.stages;
        assert_eq!(stages.total(), counted.total, "{id}: {stages:?}");
        assert_eq!(stages.done + stages.in_review, counted.done, "{id}: {stages:?}");
        assert_eq!((rolled.progress.done, rolled.progress.total), (counted.done, counted.total));
        if !claims && !parks {
            assert_eq!(rolled.progress.state, counted.state, "{id}: {stages:?}");
        }
    }
}

fn waves(r: &Replica, ancestry: &dyn Ancestry) -> Waves {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, NOW, ancestry).waves().unwrap()
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
    link(r, leaf.0, on.0, LinkKind::Blocks, by(2)).unwrap();
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
    let found = waves(&r, &Fake::default());
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
    let found = waves(&r, &Fake::default());
    assert_eq!(found.waves, ids(&[&["t2", "t10"], &["t3"]]));
    assert!(found.anytime.is_empty() && !found.waits_for.contains_key("t1"));
}

#[test]
fn work_in_review_holds_its_waiters_back_and_a_cycle_shares_a_wave() {
    let mut r = replica("a");
    waits(&mut r, ("b", "t2"), ("a", "t1"));
    done_with(&mut r, "a", Some("open"));
    let review = Fake::with(&[("open", Ok(false))]);
    assert_eq!(status(&r, &review, "b"), blocked(&[("a", Wait::InReview)]));
    assert_eq!(waves(&r, &review).waves, ids(&[&["t1"], &["t2"]]), "as its leaf reads blocked");
    let merged = Fake::with(&[("open", Ok(true))]);
    assert_eq!(waves(&r, &merged).anytime, ["t2"], "merged, t1 is finished and t2 is free");
    let mut r = replica("a");
    waits(&mut r, ("x", "t1"), ("y", "t2"));
    waits(&mut r, ("y2", "t2"), ("x2", "t1"));
    waits(&mut r, ("z", "t3"), ("y", "t2"));
    assert_eq!(waves(&r, &Fake::default()).waves, ids(&[&["t1", "t2"], &["t3"]]));
}

#[test]
fn threads_wait_on_each_other_in_a_cycle_while_work_in_review_holds_one_back() {
    let mut r = replica("a");
    waits(&mut r, ("x", "t1"), ("y", "t2"));
    waits(&mut r, ("y2", "t2"), ("x2", "t1"));
    let cycles = |r: &Replica, ancestry: &dyn Ancestry| {
        let model = Model::read(&r.state).unwrap();
        Readiness::new(&r.state, &model, NOW, ancestry).thread_cycles().unwrap()
    };
    let both = vec![BTreeSet::from(["t1".to_owned(), "t2".to_owned()])];
    assert_eq!(cycles(&r, &Fake::default()), both);
    done_with(&mut r, "x2", Some("open"));
    assert_eq!(
        cycles(&r, &Fake::with(&[("open", Ok(false))])),
        both,
        "unmerged, x2 still holds y2 back"
    );
    assert!(cycles(&r, &Fake::with(&[("open", Ok(true))])).is_empty(), "merged, it holds nothing");
    let mut r = replica("a");
    items(&mut r, &["x", "x2", "z", "z2"]);
    thread(&mut r, "x", "t1");
    thread(&mut r, "x2", "t1");
    for id in ["z", "z2"] {
        model::put(&mut r, &item_key(id).unwrap(), field::THREAD, Some(&"loose"), by(3)).unwrap();
    }
    link(&mut r, "x", "z", LinkKind::Blocks, by(3)).unwrap();
    link(&mut r, "z2", "x2", LinkKind::Blocks, by(3)).unwrap();
    assert!(cycles(&r, &Fake::default()).is_empty(), "a thread with no record is in no cycle");
}

#[test]
fn parked_work_holds_its_waiters_back_but_waits_on_nothing() {
    let mut r = replica("a");
    waits(&mut r, ("b", "t2"), ("a", "t1"));
    set_state(&mut r, "b", ItemState::Parked, by(3)).unwrap();
    let found = waves(&r, &Fake::default());
    assert_eq!(found.anytime, ["t1", "t2"], "a parked leaf orders nothing");
    waits(&mut r, ("c", "t3"), ("b", "t2"));
    assert_eq!(
        waves(&r, &Fake::default()).waits_for["t3"],
        ["t2"],
        "but holds back what waits on it"
    );
}

#[test]
fn a_link_on_a_group_counts_its_open_work_and_archived_work_holds_nothing_back() {
    let mut r = replica("a");
    items(&mut r, &["box", "in1", "in2"]);
    thread(&mut r, "in1", "t1");
    thread(&mut r, "in2", "t2");
    for leaf in ["in1", "in2"] {
        set_parent(&mut r, leaf, Some("box"), by(2)).unwrap();
    }
    waits(&mut r, ("w", "t3"), ("box", "main"));
    let found = waves(&r, &Fake::default());
    assert_eq!(found.waits_for["t3"], ["t1", "t2"]);
    model::put(&mut r, &item_key("in2").unwrap(), field::ARCHIVED, Some(&true), by(3)).unwrap();
    let found = waves(&r, &Fake::default());
    assert_eq!(found.waits_for["t3"], ["t1"]);
    assert!(!found.waits_for.contains_key("t2"), "archived work leaves its thread nothing to do");
}

/// Every thread with a leaf left is in one wave or apart, and no other; each thread waited on comes in an earlier
/// wave, or the same when they wait on each other; and a leaf blocked on work in another thread makes its thread wait
/// on that one.
#[hegel::test]
fn waves_order_threads_as_their_leaves_wait(tc: TestCase) {
    let r = random_plan(&tc, tc.draw(gs::booleans()), tc.draw(gs::booleans()));
    let model = Model::read(&r.state).unwrap();
    let ancestry = Fake::with(&[("merged", Ok(true)), ("open", Ok(false))]);
    let readiness = Readiness::new(&r.state, &model, NOW, &ancestry);
    let found = readiness.waves().unwrap();
    let wave: BTreeMap<&str, usize> = found
        .waves
        .iter()
        .enumerate()
        .flat_map(|(i, wave)| wave.iter().map(move |id| (id.as_str(), i)))
        .collect();
    let placed: Vec<&str> =
        found.waves.iter().flatten().chain(&found.anytime).map(String::as_str).collect();
    let left: BTreeSet<&str> = model
        .items
        .keys()
        .filter(|id| readiness.stage(id).unwrap().is_some_and(|stage| stage != Stage::Done))
        .filter_map(|id| model.thread_of(id))
        .filter(|thread| model.threads.contains_key(*thread))
        .collect();
    assert_eq!(placed.len(), left.len(), "each once: {found:?}");
    assert_eq!(placed.iter().copied().collect::<BTreeSet<_>>(), left);
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
        let Some(Status::Blocked { waiting }) = readiness.of(id).unwrap() else { continue };
        let Some(thread) = model.thread_of(id).filter(|t| model.threads.contains_key(*t)) else {
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
                if let Some(other) = model.thread_of(at).filter(|t| left.contains(t))
                    && other != thread
                    && readiness.stage(at).unwrap().is_some_and(|stage| stage != Stage::Done)
                {
                    assert!(
                        found.waits_for[thread].iter().any(|t| t == other),
                        "{id} waits on {at}: {found:?}"
                    );
                }
                stack.extend(model.children(at));
            }
        }
    }
}
