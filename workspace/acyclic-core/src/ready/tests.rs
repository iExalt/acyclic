use std::cell::Cell;

use super::*;
use crate::model::{self, By, add_item, item_key, link, set, set_element, set_parent, set_state};
use crate::schema::{item as field, kind, repository, set as sets};
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

fn items(r: &mut Replica, ids: &[&str]) {
    for id in ids {
        add_item(r, id, id, by(1)).unwrap();
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

fn thread(r: &mut Replica, id: &str, thread: &str) {
    model::put(r, &item_key(id).unwrap(), field::THREAD, Some(&thread), by(2)).unwrap();
}

fn status(r: &Replica, ancestry: &dyn Ancestry, id: &str) -> Status {
    let model = Model::read(&r.state).unwrap();
    Readiness::new(&r.state, &model, NOW, ancestry).of(id).unwrap().unwrap()
}

fn blocked(waits: &[(&str, Wait)]) -> Status {
    Status::Blocked { waiting: waits.iter().map(|(id, w)| ((*id).to_owned(), w.clone())).collect() }
}

#[test]
fn a_hard_dependency_blocks_until_done_and_a_soft_one_only_ranks() {
    let mut r = replica("a");
    items(&mut r, &["dep", "soft", "x"]);
    link(&mut r, "x", "dep", LinkKind::Blocks, by(2)).unwrap();
    link(&mut r, "x", "soft", LinkKind::After, by(2)).unwrap();
    let none = Fake::default();
    assert_eq!(status(&r, &none, "x"), blocked(&[("dep", Wait::Open)]));
    // Work with no repository is met when done.
    done_with(&mut r, "dep", None);
    assert_eq!(status(&r, &none, "x"), Status::Ready { rank: 1 });
    done_with(&mut r, "soft", None);
    assert_eq!(status(&r, &none, "x"), Status::Ready { rank: 0 });
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
    let unmerged = Fake::with(&[("c1", Ok(false))]);
    assert_eq!(status(&r, &unmerged, "same"), Status::Ready { rank: 0 });
    assert_eq!(status(&r, &unmerged, "other"), blocked(&[("dep", Wait::InReview)]));
    // An item in no thread needs the merge too.
    assert_eq!(status(&r, &unmerged, "loose"), blocked(&[("dep", Wait::InReview)]));
    assert_eq!(status(&r, &unmerged, "dep"), Status::InReview);
    let merged = Fake::with(&[("c1", Ok(true))]);
    assert_eq!(status(&r, &merged, "other"), Status::Ready { rank: 0 });
    assert_eq!(status(&r, &merged, "dep"), Status::Done);
}

#[test]
fn an_unverifiable_merge_says_why() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "dep2", "y"]);
    link(&mut r, "x", "dep", LinkKind::Blocks, by(2)).unwrap();
    done_with(&mut r, "dep", Some("c1"));
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
    set(&mut r, &item_key("goal").unwrap(), field::ARCHIVED, Some(&true), by(3)).unwrap();
    let unmerged = Fake::with(&[("c1", Ok(false))]);
    assert_eq!(status(&r, &unmerged, "x"), Status::Ready { rank: 0 });
    assert_eq!(status(&r, &unmerged, "dep"), Status::Archived);
}

#[test]
fn a_parents_dependencies_hold_back_its_children_and_a_parent_is_met_by_its_own_state() {
    let mut r = replica("a");
    items(&mut r, &["goal", "child", "dep", "after-goal"]);
    set_parent(&mut r, "child", Some("goal"), by(2)).unwrap();
    link(&mut r, "goal", "dep", LinkKind::Blocks, by(2)).unwrap();
    link(&mut r, "after-goal", "goal", LinkKind::Blocks, by(2)).unwrap();
    let none = Fake::default();
    assert_eq!(status(&r, &none, "child"), blocked(&[("dep", Wait::Open)]));
    assert_eq!(status(&r, &none, "goal"), Status::Parent);
    assert_eq!(status(&r, &none, "after-goal"), blocked(&[("goal", Wait::Open)]));
    done_with(&mut r, "goal", None);
    assert_eq!(status(&r, &none, "after-goal"), Status::Ready { rank: 0 });
}

#[test]
fn cycles_conflicts_and_parked_items_are_not_ready() {
    let mut a = replica("a");
    items(&mut a, &["x", "y", "p", "c"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    link(&mut a, "x", "y", LinkKind::After, by(2)).unwrap();
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
fn claims_show_and_a_contest_names_its_winner_first() {
    let mut a = replica("a");
    items(&mut a, &["x"]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    let key = item_key("x").unwrap();
    claim::claim(&mut a, &key, "session/one", 900, Some(500), 1_000).unwrap();
    let none = Fake::default();
    assert_eq!(status(&a, &none, "x"), Status::Claimed { holder: "session/one".into() });
    claim::claim(&mut b, &key, "session/two", 950, Some(500), 1_000).unwrap();
    a.join(&b.state).unwrap();
    assert_eq!(
        status(&a, &none, "x"),
        Status::Contested { holders: vec!["session/one".into(), "session/two".into()] }
    );
}

#[test]
fn ready_ranks_leaves_and_checks_each_needed_commit_once() {
    let mut r = replica("a");
    items(&mut r, &["dep", "x", "y", "z", "unrelated", "goal", "archived-leaf"]);
    for id in ["x", "y"] {
        link(&mut r, id, "dep", LinkKind::Blocks, by(2)).unwrap();
    }
    link(&mut r, "z", "x", LinkKind::After, by(2)).unwrap();
    done_with(&mut r, "dep", Some("c1"));
    done_with(&mut r, "unrelated", Some("c2"));
    set_parent(&mut r, "archived-leaf", Some("goal"), by(2)).unwrap();
    set(&mut r, &item_key("goal").unwrap(), field::ARCHIVED, Some(&true), by(2)).unwrap();
    set_element(&mut r, &item_key("x").unwrap(), sets::AREA, "ui", true, by(2)).unwrap();
    let ancestry = Fake::with(&[("c1", Ok(true)), ("c2", Ok(true))]);
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, NOW, &ancestry);
    let ready = readiness.ready(false).unwrap();
    assert_eq!(ready, vec![("x".into(), 0), ("y".into(), 0), ("z".into(), 1)]);
    assert_eq!(
        ancestry.calls.get(),
        1,
        "c1 once for two dependents, and never c2, which nothing needs"
    );
    let with_archived = readiness.ready(true).unwrap();
    assert!(with_archived.contains(&("archived-leaf".into(), 0)));
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
fn a_held_resource_keeps_an_item_waiting_and_a_dead_runs_claim_frees_it() {
    let mut r = replica("a");
    items(&mut r, &["x", "y"]);
    for id in ["x", "y"] {
        set_element(&mut r, &item_key(id).unwrap(), sets::RESOURCE, "gpu", true, by(2)).unwrap();
    }
    model::start_run(&mut r, "r1", "t1", by(2)).unwrap();
    claim::claim(&mut r, &item_key("x").unwrap(), "run/r1", 10, None, 1_000).unwrap();
    let none = Fake::default();
    assert_eq!(
        status(&r, &none, "y"),
        Status::ResourceBusy { resource: "gpu".into(), item: "x".into(), holder: "run/r1".into() }
    );
    model::set_run_state(&mut r, "r1", model::RunState::Done, by(20)).unwrap();
    assert_eq!(status(&r, &none, "x"), Status::Ready { rank: 0 });
    assert_eq!(status(&r, &none, "y"), Status::Ready { rank: 0 });
}
