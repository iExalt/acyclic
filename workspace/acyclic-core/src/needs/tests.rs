use crate::state::Key;
use serde_json::json;

use super::*;
use crate::model::{By, ItemState, add_item, item_key, link, set_parent};
use crate::schema::{item as field, repository, thread};
use crate::scope::{ScopeEdit, USER, edit};
use crate::state::{CLOCK_SKEW_MS, Replica, ReplicaId};
use serde_json::Value as Json;

const NOW: Millis = 10_000;

fn user(at: Millis) -> By<'static> {
    By { actor: USER, at }
}

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn list(r: &Replica) -> Vec<Entry> {
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    needs_you(&r.state, &model, &readiness, NOW).unwrap()
}

#[test]
fn need_identity_includes_the_core_payload_even_when_its_stable_key_does_not() {
    let first = Entry::Contest { thread: "t".into(), holders: vec!["a".into(), "b".into()] };
    let changed = Entry::Contest { thread: "t".into(), holders: vec!["a".into(), "c".into()] };
    assert_eq!(first.key(), changed.key());
    assert_ne!(first.identity(), changed.identity());
    assert_eq!(first.identity(), r#"{"entry":"contest","thread":"t","holders":["a","b"]}"#);
}

#[test]
fn the_state_puts_its_problems_on_the_list() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    let by = user(1);
    for id in ["x", "y", "z", "p", "kid", "dep", "orphan"] {
        add_item(&mut a, id, id, by).unwrap();
    }
    // Threads to claim: one each for x and kid.
    for (item, held) in [("x", "tx"), ("kid", "tk")] {
        model::set(&mut a, &Key::new(kind::THREAD, held).unwrap(), thread::TITLE, Some(&held), by)
            .unwrap();
        edit(&mut a, &ScopeEdit::Member { item: item.into(), thread: Some(held.into()) }, by)
            .unwrap();
    }
    set_parent(&mut a, "kid", Some("p"), by).unwrap();
    model::set_state(&mut a, "p", ItemState::Done, by).unwrap();
    // A dangling parent, written as if by an older or newer replica.
    a.write(&item_key("orphan").unwrap(), field::PARENT, USER, 1, Some(json!("gone"))).unwrap();
    // An unverifiable dependency.
    model::set_state(&mut a, "dep", ItemState::Done, by).unwrap();
    let evidence = model::Evidence {
        repository: Some("nowhere".into()),
        commit: Some("abcdef1".into()),
        text: None,
    };
    model::set(&mut a, &item_key("dep").unwrap(), field::EVIDENCE, Some(&evidence), by).unwrap();
    link(&mut a, "z", "dep", by).unwrap();
    link(&mut a, "dep", "kid", by).unwrap();
    b.join(&a.state).unwrap();

    // Concurrent: a title conflict, a link cycle, a contest, a clock ahead.
    model::set(&mut a, &item_key("x").unwrap(), field::TITLE, Some(&"one"), user(2)).unwrap();
    model::set(&mut b, &item_key("x").unwrap(), field::TITLE, Some(&"two"), user(2)).unwrap();
    link(&mut a, "x", "y", user(2)).unwrap();
    link(&mut b, "y", "x", user(2)).unwrap();
    claim::claim(&mut a, "tx", "session/a", 2, (NOW, NOW), false).unwrap();
    claim::claim(&mut b, "tx", "session/b", 2, (NOW, NOW), false).unwrap();
    // A claim from a clock running ahead of the reader's by more than the skew.
    claim::claim(&mut b, "tk", "session/b", NOW + CLOCK_SKEW_MS + 1, (NOW, NOW), false).unwrap();
    a.join(&b.state).unwrap();

    let entries = list(&a);
    let has = |want: &Entry| assert!(entries.contains(want), "missing {want:?} in {entries:#?}");
    has(&Entry::Conflict { kind: "item".into(), id: "x".into(), field: "title".into() });
    has(&Entry::LinkCycle { items: vec!["x".into(), "y".into()] });
    has(&Entry::Contest {
        thread: "tx".into(),
        holders: vec!["session/a".into(), "session/b".into()],
    });
    // Done work waiting on an open leaf; a parent stored done over open work is no longer flagged, since it reads open.
    has(&Entry::DoneWaiting { item: "dep".into(), waits_for: "kid".into() });
    assert!(
        !entries.iter().any(|e| matches!(e, Entry::DoneWaiting { item, .. } if item == "p")),
        "{entries:#?}"
    );
    has(&Entry::ClockAhead { thread: "tk".into(), holder: "session/b".into() });
    has(&Entry::Dangling {
        kind: "item".into(),
        id: "orphan".into(),
        field: "parent".into(),
        target: "gone".into(),
        fix: None,
    });
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
    link(&mut r, "waiting", "loose", by).unwrap();
    model::set_state(&mut r, "parked", ItemState::Parked, by).unwrap();
    model::set_state(&mut r, "done", ItemState::Done, by).unwrap();
    set_parent(&mut r, "kid", Some("goal"), by).unwrap();
    // A done item in the missing thread dangles too, but needs no thread, so it gets no fix.
    edit(&mut r, &ScopeEdit::Member { item: "done".into(), thread: Some("ghost".into()) }, by)
        .unwrap();
    let entries = list(&r);
    let listed: Vec<(String, String)> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Unthreaded { item, fix } => Some((item.clone(), fix.clone())),
            _ => None,
        })
        .collect();
    let fix = |id: &str| (id.to_owned(), format!("acyclic item edit {id} --thread <THREAD>"));
    // Whatever else holds it back; never a parked or done item, a member or a parent.
    assert_eq!(listed, [fix("loose"), fix("waiting")]);
    // A leaf in a thread with no record is listed once, as the reference it holds, with the same fix.
    let ghosts: Vec<(&str, Option<&str>)> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Dangling { id, field, target, fix, .. }
                if field == "thread" && target == "ghost" =>
            {
                Some((id.as_str(), fix.as_deref()))
            },
            _ => None,
        })
        .collect();
    assert_eq!(ghosts, [("done", None), ("ghostly", Some(fix("ghostly").1.as_str()))]);
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
    link(&mut r, "x", "y", by).unwrap();
    link(&mut r, "y2", "x2", by).unwrap();
    link(&mut r, "z", "x", by).unwrap();
    let cycles: Vec<Entry> =
        list(&r).into_iter().filter(|e| matches!(e, Entry::ThreadCycle { .. })).collect();
    assert_eq!(cycles, [Entry::ThreadCycle { threads: vec!["t".into(), "u".into()] }]);
    assert!(cycles[0].notifies());
}

/// Trackers written by earlier versions hold kinds and fields no reader gives meaning to any more: proposals,
/// questions, global settings, a thread's approval and autonomy level, and the retired fields. Every reader skips
/// them.
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
    // Retired fields two replicas wrote apart conflict in the state, but lie dormant: no conflict, no need.
    let x = item_key("x").unwrap();
    let repo = Key::new(kind::REPOSITORY, "r").unwrap();
    let retired = [
        (x.clone(), "size", json!("m"), json!("xl")),
        (x.clone(), "archived", json!(true), json!(false)),
        (x.clone(), "reopened", json!(true), json!(null)),
        (x.clone(), "repository/r", json!(true), json!(false)),
        (Key::new(kind::THREAD, "t").unwrap(), "repository/r", json!(true), json!(false)),
        (repo.clone(), "installation", json!("1"), json!("2")),
        (x.clone(), "gate", json!("review"), json!("ship")),
        (x.clone(), "area/app", json!(true), json!(false)),
        (x.clone(), "resource/gpu", json!(true), json!(false)),
    ];
    for (key, name, mine, theirs) in &retired {
        apart(&mut r, key, name, mine, theirs);
    }
    let dormant = |r: &Replica| {
        retired.iter().map(|(key, name, ..)| r.state.field(key, name).cloned()).collect::<Vec<_>>()
    };
    let before = dormant(&r);
    assert!(before.iter().all(|register| register.as_ref().is_some_and(|r| r.len() == 2)));
    let model = Model::read(&r.state).unwrap();
    assert_eq!(model.thread_of("x"), Some("t"));
    assert_eq!(model.conflicts().count(), 0);
    assert_eq!(list(&r), [], "nothing about them reaches the list");
    // A new write to the same records leaves them as they were.
    model::set(&mut r, &x, field::TITLE, Some(&"new"), user(6)).unwrap();
    model::set(&mut r, &repo, repository::DEFAULT_BRANCH, Some(&"main"), user(6)).unwrap();
    assert_eq!(dormant(&r), before);
    assert_eq!(Model::read(&r.state).unwrap().conflicts().count(), 0);
}

#[test]
fn a_need_is_known_by_what_it_is_about_not_by_what_changes_while_it_stands() {
    let waiting = Entry::DoneWaiting { item: "a".into(), waits_for: "b".into() };
    assert_eq!(waiting.key(), "done-waiting:a:b");
    let contest = |holders: &[&str]| Entry::Contest {
        thread: "t".into(),
        holders: holders.iter().map(|&h| h.to_owned()).collect(),
    };
    assert_eq!(contest(&["x", "y"]).key(), contest(&["y", "z"]).key());
    let conflict = Entry::Conflict { kind: "item".into(), id: "a".into(), field: "title".into() };
    assert_eq!(conflict.key(), "conflict:item:a:title");
    // Two references from one item differ only in what they point at.
    let dangling = |target: &str| Entry::Dangling {
        kind: "item".into(),
        id: "a".into(),
        field: "thread".into(),
        target: target.into(),
        fix: None,
    };
    assert_eq!(dangling("r").key(), "dangling:item:a:thread:r");
    assert_ne!(dangling("r").key(), dangling("s").key());
    let cycle = Entry::ThreadCycle { threads: vec!["t".into(), "u".into()] };
    assert_eq!(cycle.key(), "thread-cycle:t:u");
}

/// Writes `field` of `key` as `mine` here and as `theirs` on a replica that hasn't seen it, then joins them.
fn apart(r: &mut Replica, key: &Key, field: &str, mine: &Json, theirs: &Json) {
    // Each call writes from a replica of its own, as a replica's ids are its own.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut other = replica(&format!("other{n}"));
    other.join(&r.state).unwrap();
    r.write(key, field, USER, 5, Some(mine.clone())).unwrap();
    other.write(key, field, USER, 5, Some(theirs.clone())).unwrap();
    r.join(&other.state).unwrap();
}

fn kept(r: &Replica, entity: &str, id: &str, field: &str) -> Vec<(Json, Keep)> {
    let model = Model::read(&r.state).unwrap();
    keeps(&r.state, &model, (entity, id, field), user(NOW))
        .unwrap()
        .into_iter()
        .map(|kept| (kept.value, kept.keep))
        .collect()
}

#[test]
fn each_value_of_a_conflict_is_kept_by_a_change_the_judge_accepts_or_says_why_not() {
    use crate::write::{Assign, Change, ChangeSet, ItemAssign, RepositoryAssign, Rule};
    let mut r = replica("a");
    for id in ["x", "p", "kid"] {
        add_item(&mut r, id, id, user(1)).unwrap();
    }
    set_parent(&mut r, "kid", Some("x"), user(1)).unwrap();
    let x = item_key("x").unwrap();
    let set = |assign: ItemAssign| {
        Keep::Write(ChangeSet(vec![Change::Set(Assign::Item("x".into(), assign))]))
    };
    let values = |found: &[(Json, Keep)]| found.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
    let keeps_in = |r: &Replica, field: &str| {
        let mut found: Vec<(Json, Keep)> = kept(r, kind::ITEM, "x", field);
        found.sort_by_key(|(value, _)| value.to_string());
        found.into_iter().map(|(_, keep)| keep).collect::<Vec<_>>()
    };

    apart(&mut r, &x, field::TITLE, &json!("one"), &json!(null));
    assert_eq!(values(&kept(&r, kind::ITEM, "x", field::TITLE)).len(), 2);
    assert_eq!(
        keeps_in(&r, field::TITLE),
        [set(ItemAssign::Title("one".into())), Keep::Not(Unkept::NoClear)]
    );

    // Keeping done is left to a terminal, where evidence comes with it.
    apart(&mut r, &x, field::STATE, &json!("done"), &json!("parked"));
    assert_eq!(
        keeps_in(&r, field::STATE),
        [Keep::Not(Unkept::NeedsEvidence), set(ItemAssign::State(ItemState::Parked))]
    );

    // A clear is kept where the field can be cleared.
    apart(&mut r, &x, field::NOTES, &json!("n"), &json!(null));
    assert_eq!(
        keeps_in(&r, field::NOTES),
        [set(ItemAssign::Notes(Some("n".into()))), set(ItemAssign::Notes(None))]
    );

    // Putting x under its own child is refused by the judge, so no command is offered for it.
    apart(&mut r, &x, field::PARENT, &json!("kid"), &json!("p"));
    let found = keeps_in(&r, field::PARENT);
    assert!(
        matches!(&found[0], Keep::Not(Unkept::Refused(v)) if matches!(v[0].rule, Rule::ParentCycle(_))),
        "{found:?}"
    );
    assert_eq!(found[1], set(ItemAssign::Parent(Some("p".into()))));

    // A repository's owner is kept with its name as it stands, and not while the name is in conflict too.
    let repo = Key::new(kind::REPOSITORY, "r").unwrap();
    for (field, value) in [(repository::OWNER, "o"), (repository::NAME, "n")] {
        r.write(&repo, field, USER, 1, Some(json!(value))).unwrap();
    }
    apart(&mut r, &repo, repository::OWNER, &json!("o1"), &json!("o2"));
    let with = |assign| Change::Set(Assign::Repository("r".into(), assign));
    let owner = |owner: &str| {
        Keep::Write(ChangeSet(vec![
            with(RepositoryAssign::Owner(owner.into())),
            with(RepositoryAssign::Name("n".into())),
        ]))
    };
    let mut found = kept(&r, kind::REPOSITORY, "r", repository::OWNER);
    found.sort_by_key(|(value, _)| value.to_string());
    assert_eq!(
        found.into_iter().map(|(_, keep)| keep).collect::<Vec<_>>(),
        [owner("o1"), owner("o2")]
    );
    apart(&mut r, &repo, repository::NAME, &json!("n1"), &json!("n2"));
    assert!(
        kept(&r, kind::REPOSITORY, "r", repository::OWNER)
            .iter()
            .all(|(_, keep)| *keep == Keep::Not(Unkept::NameAlsoConflicted))
    );
}
