use crate::model::SetField;
use crate::state::Key;
use serde_json::json;

use super::*;
use crate::model::{By, ItemState, LinkKind, add_item, item_key, link, set_parent};
use crate::ready::Ancestry;
use crate::schema::{item as field, thread};
use crate::scope::{ScopeEdit, USER, edit};
use crate::state::{CLOCK_SKEW_MS, Replica, ReplicaId};

const NOW: Millis = 10_000;

struct Unmapped;

impl Ancestry for Unmapped {
    fn merged(&self, _: &model::Repository, _: &str) -> Result<bool, String> {
        Err("no clone".into())
    }
}

fn user(at: Millis) -> By<'static> {
    By { actor: USER, at }
}

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn list(r: &Replica) -> Vec<Entry> {
    let model = Model::read(&r.state).unwrap();
    let readiness = Readiness::new(&r.state, &model, NOW, &Unmapped);
    needs_you(&r.state, &model, &readiness, NOW).unwrap()
}

#[test]
fn the_state_puts_its_problems_on_the_list() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    let by = user(1);
    for id in ["x", "y", "z", "p", "kid", "dep", "gpu1", "gpu2", "orphan"] {
        add_item(&mut a, id, id, by).unwrap();
    }
    for id in ["gpu1", "gpu2"] {
        model::set_element(&mut a, &item_key(id).unwrap(), SetField::Resource, "gpu", true, by)
            .unwrap();
    }
    // Threads to claim: one each for x, the two gpu items and kid.
    for (item, held) in [("x", "tx"), ("gpu1", "t1"), ("gpu2", "t2"), ("kid", "tk")] {
        model::set(&mut a, &Key::new(kind::THREAD, held).unwrap(), thread::TITLE, Some(&held), by)
            .unwrap();
        edit(&mut a, &ScopeEdit::Member { item: item.into(), thread: Some(held.into()) }, by)
            .unwrap();
    }
    set_parent(&mut a, "kid", Some("p"), by).unwrap();
    model::set_state(&mut a, "p", ItemState::Done, by).unwrap();
    // A dangling parent and repository, written as if by an older or newer replica.
    a.write(&item_key("orphan").unwrap(), field::PARENT, USER, 1, Some(json!("gone"))).unwrap();
    edit(
        &mut a,
        &ScopeEdit::ItemRepository {
            item: "orphan".into(),
            repository: "nowhere".into(),
            present: true,
        },
        by,
    )
    .unwrap();
    // An unverifiable dependency.
    model::set_state(&mut a, "dep", ItemState::Done, by).unwrap();
    let evidence = model::Evidence {
        repository: Some("nowhere".into()),
        commit: Some("abcdef1".into()),
        note: None,
    };
    model::set(&mut a, &item_key("dep").unwrap(), field::EVIDENCE, Some(&evidence), by).unwrap();
    link(&mut a, "z", "dep", LinkKind::Blocks, by).unwrap();
    b.join(&a.state).unwrap();

    // Concurrent: a title conflict, a link cycle, a contest, a resource clash, a clock ahead.
    model::set(&mut a, &item_key("x").unwrap(), field::TITLE, Some(&"one"), user(2)).unwrap();
    model::set(&mut b, &item_key("x").unwrap(), field::TITLE, Some(&"two"), user(2)).unwrap();
    link(&mut a, "x", "y", LinkKind::Blocks, user(2)).unwrap();
    link(&mut b, "y", "x", LinkKind::Blocks, user(2)).unwrap();
    claim::claim(&mut a, "tx", "session/a", 2, NOW, NOW).unwrap();
    claim::claim(&mut b, "tx", "session/b", 2, NOW, NOW).unwrap();
    claim::claim(&mut a, "t1", "session/a", 2, NOW, NOW).unwrap();
    claim::claim(&mut b, "t2", "session/b", 2, NOW, NOW).unwrap();
    // A claim from a clock running ahead of the reader's by more than the skew.
    claim::claim(&mut b, "tk", "session/b", NOW + CLOCK_SKEW_MS + 1, NOW, NOW).unwrap();
    a.join(&b.state).unwrap();

    let entries = list(&a);
    let has = |want: &Entry| assert!(entries.contains(want), "missing {want:?} in {entries:#?}");
    has(&Entry::Conflict { kind: "item".into(), id: "x".into(), field: "title".into() });
    has(&Entry::LinkCycle { items: vec!["x".into(), "y".into()] });
    has(&Entry::Contest {
        thread: "tx".into(),
        holders: vec!["session/a".into(), "session/b".into()],
    });
    has(&Entry::ResourceClash {
        resource: "gpu".into(),
        holders: vec![("t1".into(), "session/a".into()), ("t2".into(), "session/b".into())],
    });
    has(&Entry::DoneEarly { item: "p".into(), open: vec!["kid".into()] });
    has(&Entry::ClockAhead { thread: "tk".into(), holder: "session/b".into() });
    has(&Entry::Dangling {
        kind: "item".into(),
        id: "orphan".into(),
        field: "parent".into(),
        target: "gone".into(),
    });
    has(&Entry::Dangling {
        kind: "item".into(),
        id: "orphan".into(),
        field: "repository".into(),
        target: "nowhere".into(),
    });
    assert!(entries.iter().any(|e| matches!(e, Entry::Unverified { item, repository, .. } if item == "dep" && repository == "nowhere")));
}

#[test]
fn every_open_leaf_no_thread_holds_is_listed_with_its_fix() {
    let mut r = replica("a");
    let by = user(1);
    for id in ["loose", "ghostly", "waiting", "parked", "done", "member", "goal", "kid"] {
        add_item(&mut r, id, id, by).unwrap();
    }
    model::set(&mut r, &Key::new(kind::THREAD, "t").unwrap(), thread::TITLE, Some(&"T"), by)
        .unwrap();
    for (item, held) in [("member", "t"), ("kid", "t"), ("ghostly", "ghost")] {
        edit(&mut r, &ScopeEdit::Member { item: item.into(), thread: Some(held.into()) }, by)
            .unwrap();
    }
    link(&mut r, "waiting", "loose", LinkKind::Blocks, by).unwrap();
    model::set_state(&mut r, "parked", ItemState::Parked, by).unwrap();
    model::set_state(&mut r, "done", ItemState::Done, by).unwrap();
    set_parent(&mut r, "kid", Some("goal"), by).unwrap();
    let listed: Vec<(String, String)> = list(&r)
        .into_iter()
        .filter_map(|e| match e {
            Entry::Unthreaded { item, fix } => Some((item, fix)),
            _ => None,
        })
        .collect();
    let fix = |id: &str| (id.to_owned(), format!("acyclic item edit {id} --thread <THREAD>"));
    // Whatever else holds it back; never a parked or done item, a member or a parent.
    assert_eq!(listed, [fix("ghostly"), fix("loose"), fix("waiting")]);
}

#[test]
fn threads_that_wait_on_each_other_are_one_thread_cycle() {
    let mut r = replica("a");
    let by = user(1);
    for (item, thread) in [("x", "t"), ("x2", "t"), ("y", "u"), ("y2", "u"), ("z", "v")] {
        add_item(&mut r, item, item, by).unwrap();
        edit(&mut r, &ScopeEdit::Member { item: item.into(), thread: Some(thread.into()) }, by)
            .unwrap();
    }
    for thread in ["t", "u", "v"] {
        let key = Key::new(kind::THREAD, thread).unwrap();
        model::set(&mut r, &key, thread::TITLE, Some(&thread), by).unwrap();
    }
    // `t` waits on `u` and `u` on `t`; `v` only waits on `t`, so it is on no cycle.
    link(&mut r, "x", "y", LinkKind::Blocks, by).unwrap();
    link(&mut r, "y2", "x2", LinkKind::Blocks, by).unwrap();
    link(&mut r, "z", "x", LinkKind::Blocks, by).unwrap();
    let cycles: Vec<Entry> =
        list(&r).into_iter().filter(|e| matches!(e, Entry::ThreadCycle { .. })).collect();
    assert_eq!(cycles, [Entry::ThreadCycle { threads: vec!["t".into(), "u".into()] }]);
    assert!(cycles[0].notifies());
}

/// Trackers written by earlier versions hold kinds and fields no reader gives meaning to any more: proposals,
/// questions, global settings, and a thread's approval and autonomy level. Every reader skips them.
#[test]
fn kinds_and_fields_no_reader_knows_are_skipped() {
    let mut r = replica("a");
    add_item(&mut r, "x", "x", user(1)).unwrap();
    edit(&mut r, &ScopeEdit::Member { item: "x".into(), thread: Some("t".into()) }, user(1))
        .unwrap();
    let thread = Key::new(kind::THREAD, "t").unwrap();
    model::set(&mut r, &thread, thread::TITLE, Some(&"T"), user(1)).unwrap();
    let old = [
        (thread.clone(), "approved", json!(true)),
        (thread.clone(), "approved-members", json!([])),
        (thread, "autonomy", json!("everything")),
        (Key::new("settings", "global").unwrap(), "autonomy", json!("everything")),
        (Key::new("proposal", "a-1").unwrap(), "edit", json!({"edit": "member", "item": "x"})),
        (Key::new("question", "q").unwrap(), "open", json!(true)),
        (Key::new("question", "q").unwrap(), "class", json!("blocker")),
    ];
    for (key, name, value) in old {
        r.write(&key, name, "session/old", 2, Some(value)).unwrap();
    }
    let model = Model::read(&r.state).unwrap();
    assert_eq!(model.thread_of("x"), Some("t"));
    assert_eq!(list(&r), [], "nothing about them reaches the list");
}
