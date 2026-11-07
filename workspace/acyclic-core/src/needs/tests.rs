use crate::state::Key;
use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::model::{By, ItemState, add_item, item_key, link, set_parent, set_state};
use crate::ready::{Readiness, Wait};
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

/// The needs-you list but for the items made with no parent, which most of these tests make bare: those are
/// `every_entry`'s to show.
fn list(r: &Replica) -> Vec<Entry> {
    every_entry(r)
        .into_iter()
        .filter(|entry| !matches!(entry, Entry::ParentMissing { parent: None, .. }))
        .collect()
}

fn every_entry(r: &Replica) -> Vec<Entry> {
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    needs_you(&r.state, &model, &readiness, NOW).unwrap()
}

#[test]
fn inherited_leaf_cycles_are_needs_and_only_their_members_read_in_cycle() {
    let mut a = replica("a");
    for id in ["p", "d", "x", "upstream"] {
        add_item(&mut a, id, id, user(1)).unwrap();
    }
    set_parent(&mut a, "d", Some("p"), user(2)).unwrap();
    let mut b = replica("b");
    b.join(&a.state).unwrap();

    link(&mut a, "p", "x", user(3)).unwrap();
    link(&mut a, "upstream", "d", user(3)).unwrap();
    link(&mut b, "x", "d", user(3)).unwrap();
    a.join(&b.state).unwrap();

    let model = Model::read(&a.state).unwrap();
    let readiness = ReadinessEvaluator::new(&a.state, &model, NOW);
    assert!(
        needs_you(&a.state, &model, &readiness, NOW)
            .unwrap()
            .contains(&Entry::WaitsForCycle { items: vec!["d".into(), "x".into()] })
    );
    assert_eq!(readiness.of("d").unwrap(), Some(Readiness::InCycle));
    assert_eq!(readiness.of("x").unwrap(), Some(Readiness::InCycle));
    assert_eq!(
        readiness.of("upstream").unwrap(),
        Some(Readiness::Waiting { waiting: BTreeMap::from([("d".into(), Wait::Open)]) })
    );
    set_state(&mut a, "d", ItemState::Done, user(4)).unwrap();
    let model = Model::read(&a.state).unwrap();
    let readiness = ReadinessEvaluator::new(&a.state, &model, NOW);
    assert!(
        !needs_you(&a.state, &model, &readiness, NOW)
            .unwrap()
            .iter()
            .any(|entry| matches!(entry, Entry::WaitsForCycle { .. }))
    );
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

    // Concurrent: a title conflict, a waits-for cycle, a contest, a clock ahead.
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
    has(&Entry::WaitsForCycle { items: vec!["x".into(), "y".into()] });
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
    has(&Entry::ParentMissing { item: "orphan".into(), parent: Some("gone".into()) });
}

#[test]
fn an_item_with_no_parent_or_a_parent_that_names_nothing_is_in_no_initiative_and_never_notifies() {
    let mut r = replica("a");
    let by = user(1);
    crate::model::add_initiative(&mut r, "init", "Init", by).unwrap();
    for id in ["bare", "placed", "gone", "deleted", "under"] {
        add_item(&mut r, id, id, by).unwrap();
    }
    set_parent(&mut r, "placed", Some("init"), by).unwrap();
    set_parent(&mut r, "under", Some("bare"), by).unwrap();
    r.write(&item_key("gone").unwrap(), field::PARENT, USER, 1, Some(json!("nowhere"))).unwrap();
    let ended = Key::new(kind::INITIATIVE, "ended").unwrap();
    model::set(&mut r, &ended, "title", Some(&"Ended"), by).unwrap();
    model::set(&mut r, &ended, "deleted", Some(&true), by).unwrap();
    r.write(&item_key("deleted").unwrap(), field::PARENT, USER, 1, Some(json!("ended"))).unwrap();

    let entries: Vec<Entry> = every_entry(&r)
        .into_iter()
        .filter(|entry| matches!(entry, Entry::ParentMissing { .. } | Entry::Dangling { .. }))
        .collect();
    assert_eq!(
        entries,
        [
            Entry::ParentMissing { item: "bare".into(), parent: None },
            Entry::ParentMissing { item: "deleted".into(), parent: Some("ended".into()) },
            Entry::ParentMissing { item: "gone".into(), parent: Some("nowhere".into()) },
        ]
    );
    assert!(entries.iter().all(|entry| !entry.notifies()));
    assert_eq!(entries[0].key(), "parent-missing:bare");
    assert_eq!(entries[2].key(), "parent-missing:gone:nowhere");
    assert_eq!(
        entries[0].identity(),
        r#"{"entry":"parent-missing","item":"bare"}"#,
        "a missing parent is left out, not null"
    );
}

#[test]
fn a_leaf_in_a_thread_with_no_record_dangles_with_its_fix_and_a_leaf_in_none_lists_nothing() {
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
    let fix = |id: &str| (id.to_owned(), format!("acyclic item edit {id} --thread <THREAD>"));
    // A leaf in no thread is a thread of one, so nothing asks for a thread: only the reference that dangles is listed.
    assert!(entries.iter().all(|e| matches!(e, Entry::Dangling { .. })), "{entries:?}");
    // A leaf in a thread with no record is listed once, as the reference it holds, with a fix.
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
        Keep::Write(crate::write::ChangeSet(vec![crate::write::Change::Set(
            crate::write::Assign::Item("x".into(), assign),
        )]))
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
    assert_eq!(found[1], set(ItemAssign::Parent("p".into())));

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

#[test]
fn work_waiting_directly_on_canceled_work_goes_on_the_list_and_only_while_unfinished() {
    let mut r = replica("a");
    let by = user(1);
    for id in ["dep", "w", "done", "epic", "e1", "e2", "z"] {
        add_item(&mut r, id, id, by).unwrap();
    }
    for (kid, parent) in [("e1", "epic"), ("e2", "epic")] {
        set_parent(&mut r, kid, Some(parent), by).unwrap();
    }
    for (from, to) in [("w", "dep"), ("done", "dep"), ("z", "epic")] {
        link(&mut r, from, to, by).unwrap();
    }
    model::set_state(&mut r, "done", ItemState::Done, by).unwrap();
    let waits = |r: &Replica| {
        list(r).into_iter().filter(|e| e.kind() == "waits-on-canceled").collect::<Vec<_>>()
    };
    assert_eq!(waits(&r).len(), 0);
    // Waiting on canceled work is flagged; finished work that waited on it isn't.
    model::set_state(&mut r, "dep", ItemState::Canceled, by).unwrap();
    assert_eq!(waits(&r), [Entry::WaitsOnCanceled { item: "w".into(), waits_for: "dep".into() }]);
    assert!(list(&r).iter().all(|e| e.kind() != "done-waiting"));
    // One canceled leaf under an epic leaves its waiters waiting for the epic as a whole.
    model::set_state(&mut r, "e1", ItemState::Canceled, by).unwrap();
    assert_eq!(waits(&r).len(), 1);
    // The epic reads canceled once nothing else is under it, and then its waiter is flagged.
    model::set_state(&mut r, "e2", ItemState::Canceled, by).unwrap();
    assert_eq!(waits(&r).len(), 2);
    // Canceled work waiting on canceled work needs nothing.
    model::set_state(&mut r, "w", ItemState::Canceled, by).unwrap();
    assert_eq!(waits(&r), [Entry::WaitsOnCanceled { item: "z".into(), waits_for: "epic".into() }]);
    assert!(waits(&r)[0].notifies());
}

#[test]
fn canceled_written_against_done_on_two_replicas_conflicts_and_each_side_is_offered() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    add_item(&mut a, "x", "x", user(1)).unwrap();
    b.join(&a.state).unwrap();
    model::set_state(&mut a, "x", ItemState::Done, user(2)).unwrap();
    model::set_state(&mut b, "x", ItemState::Canceled, user(2)).unwrap();
    a.join(&b.state).unwrap();
    b.join(&a.state).unwrap();
    for r in [&a, &b] {
        assert!(list(r).iter().any(|e| e.key() == "conflict:item:x:state"));
        let mut offered = kept(r, kind::ITEM, "x", field::STATE);
        offered.sort_by_key(|(value, _)| value.to_string());
        let canceled = crate::write::Change::Set(crate::write::Assign::Item(
            "x".into(),
            crate::write::ItemAssign::State(ItemState::Canceled),
        ));
        assert_eq!(
            offered,
            [
                (json!("canceled"), Keep::Write(crate::write::ChangeSet(vec![canceled]))),
                (json!("done"), Keep::Not(Unkept::NeedsEvidence)),
            ]
        );
        // Neither value alone is finished, until the conflict is settled.
        assert!(!Model::read(&r.state).unwrap().items["x"].is_finished());
    }
}

fn list_at(r: &Replica, now: Millis) -> Vec<Entry> {
    let model = Model::read(&r.state).unwrap();
    let readiness = ReadinessEvaluator::new(&r.state, &model, now);
    needs_you(&r.state, &model, &readiness, now).unwrap()
}

#[test]
fn a_stalled_thread_reaches_needs_you_with_its_last_holder_until_it_is_claimed_released_or_finished()
 {
    let mut r = replica("a");
    let by = user(1);
    for id in ["x", "gated"] {
        add_item(&mut r, id, id, by).unwrap();
    }
    for (item, held) in [("x", "t"), ("gated", "u")] {
        model::set(&mut r, &Key::new(kind::THREAD, held).unwrap(), thread::TITLE, Some(&held), by)
            .unwrap();
        edit(&mut r, &ScopeEdit::Member { item: item.into(), thread: Some(held.into()) }, by)
            .unwrap();
    }
    // `u` holds only work waiting for `x`, so only `t` has ready work.
    link(&mut r, "gated", "x", by).unwrap();
    claim::claim(&mut r, "t", "session/a", 0, (1_000, 1_000), false).unwrap();
    claim::claim(&mut r, "u", "session/b", 0, (1_000, 1_000), false).unwrap();
    let stalled = |r: &Replica, now| -> Vec<Entry> {
        list_at(r, now).into_iter().filter(|e| e.kind() == "stalled").collect()
    };
    assert_eq!(stalled(&r, 999), [], "held until its lease ends");
    let entry = Entry::Stalled {
        thread: "t".into(),
        holder: "session/a".into(),
        expired_at: crate::time::rfc3339(1_000),
    };
    assert_eq!(stalled(&r, 1_000), std::slice::from_ref(&entry), "not u, whose work only waits");
    assert!(!entry.notifies(), "a busy agent can read as stalled: standing, not pushed");
    // Releasing, a fresh claim and finishing all end it.
    let mut released = r.clone();
    claim::release(&mut released, &claim::thread_key("t").unwrap(), "session/a");
    assert_eq!(stalled(&released, 1_000), []);
    let mut taken = r.clone();
    claim::claim(&mut taken, "t", "session/c", 1_000, (1_000, 1_000), true).unwrap();
    assert_eq!(stalled(&taken, 1_001), []);
    // Finishing `t`'s only item leaves it nothing; `u`'s work is ready now, so `u` is the one that stalled.
    model::set_state(&mut r, "x", ItemState::Done, user(2)).unwrap();
    let threads: Vec<String> = stalled(&r, 1_000)
        .into_iter()
        .filter_map(|e| if let Entry::Stalled { thread, .. } = e { Some(thread) } else { None })
        .collect();
    assert_eq!(threads, ["u"]);
}

/// One entry of each kind, with the key and identity it has always had. The identity is the serialized entry and
/// keys the notified needs on every user's devices, so a change here re-notifies them: it must be deliberate.
fn each_kind() -> Vec<(Entry, &'static str, &'static str, &'static str)> {
    let s = |text: &str| text.to_owned();
    let v = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
    vec![
        (
            Entry::Conflict { kind: s("thread"), id: s("t"), field: s("brief") },
            "conflict",
            "conflict:thread:t:brief",
            r#"{"entry":"conflict","kind":"thread","id":"t","field":"brief"}"#,
        ),
        (
            Entry::Unpushed { remote: s("origin") },
            "unpushed",
            "unpushed:origin",
            r#"{"entry":"unpushed","remote":"origin"}"#,
        ),
        (
            Entry::WaitsForCycle { items: v(&["a", "b"]) },
            "waits-for-cycle",
            "waits-for-cycle:a:b",
            r#"{"entry":"waits-for-cycle","items":["a","b"]}"#,
        ),
        (
            Entry::ParentCycle { items: v(&["a", "b"]) },
            "parent-cycle",
            "parent-cycle:a:b",
            r#"{"entry":"parent-cycle","items":["a","b"]}"#,
        ),
        (
            Entry::ThreadCycle { threads: v(&["t", "u"]) },
            "thread-cycle",
            "thread-cycle:t:u",
            r#"{"entry":"thread-cycle","threads":["t","u"]}"#,
        ),
        (
            Entry::Contest { thread: s("t"), holders: v(&["x", "y"]) },
            "contest",
            "contest:t",
            r#"{"entry":"contest","thread":"t","holders":["x","y"]}"#,
        ),
        (
            Entry::ClockAhead { thread: s("t"), holder: s("x") },
            "clock-ahead",
            "clock-ahead:t:x",
            r#"{"entry":"clock-ahead","thread":"t","holder":"x"}"#,
        ),
        (
            Entry::Stalled {
                thread: s("t"),
                holder: s("x"),
                expired_at: s("2026-10-21T00:00:00Z"),
            },
            "stalled",
            "stalled:t:x",
            r#"{"entry":"stalled","thread":"t","holder":"x","expired_at":"2026-10-21T00:00:00Z"}"#,
        ),
        (
            Entry::DoneWaiting { item: s("a"), waits_for: s("b") },
            "done-waiting",
            "done-waiting:a:b",
            r#"{"entry":"done-waiting","item":"a","waits_for":"b"}"#,
        ),
        (
            Entry::WaitsOnCanceled { item: s("a"), waits_for: s("b") },
            "waits-on-canceled",
            "waits-on-canceled:a:b",
            r#"{"entry":"waits-on-canceled","item":"a","waits_for":"b"}"#,
        ),
        (
            Entry::Dangling {
                kind: s("item"),
                id: s("a"),
                field: s("thread"),
                target: s("t"),
                fix: Some(s("fix")),
            },
            "dangling",
            "dangling:item:a:thread:t",
            r#"{"entry":"dangling","kind":"item","id":"a","field":"thread","target":"t","fix":"fix"}"#,
        ),
        (
            Entry::ThreadOnParent {
                item: s("a"),
                thread: s("t"),
                fix: s("acyclic item edit a --no-thread"),
            },
            "thread-on-parent",
            "thread-on-parent:a:t",
            r#"{"entry":"thread-on-parent","item":"a","thread":"t","fix":"acyclic item edit a --no-thread"}"#,
        ),
        (
            Entry::EmptyThread { thread: s("t"), fix: s("acyclic thread delete t") },
            "empty-thread",
            "empty-thread:t",
            r#"{"entry":"empty-thread","thread":"t","fix":"acyclic thread delete t"}"#,
        ),
    ]
}

#[test]
fn every_kind_keeps_the_tag_key_and_identity_devices_already_know_it_by() {
    for (entry, kind, key, identity) in each_kind() {
        assert_eq!(entry.kind(), kind);
        assert_eq!(entry.key(), key);
        assert_eq!(entry.identity(), identity);
        assert_eq!(serde_json::to_value(&entry).unwrap()["entry"], kind);
    }
}

#[test]
fn every_kind_says_what_it_is_in_one_line_without_stored_names() {
    let name = |id: &str| id.to_uppercase();
    let said: Vec<String> = each_kind().iter().map(|(entry, ..)| entry.summary(&name)).collect();
    assert_eq!(
        said,
        [
            "T: its notes was written differently on two machines",
            "This clone holds writes origin lacks",
            "Items wait for each other in a cycle: A, B",
            "Parents form a cycle: A, B",
            "Threads wait for each other in a cycle: T, U",
            "T has a claim to sort out",
            "T has a claim by x that starts in the future",
            "T has stalled: x's claim ran out at 2026-10-21T00:00:00Z",
            "A is done but waits for B",
            "A waits for B, which was canceled",
            "A: its thread names t, which doesn't exist",
            "A has children, so its thread T is ignored",
            "T has no items",
        ]
    );
    let waits = Entry::Dangling {
        kind: "item".into(),
        id: "a".into(),
        field: "waits-for".into(),
        target: "q".into(),
        fix: None,
    };
    assert_eq!(waits.summary(&name), "A waits for q, which doesn't exist");
    // No wording leaks the stored link name.
    assert!(each_kind().iter().all(|(entry, ..)| !entry.summary(&name).contains("blocks")));
}

/// Whether a kind of entry notifies, by kind.
fn notifies(kind: &str) -> bool {
    each_kind().into_iter().find(|(entry, ..)| entry.kind() == kind).unwrap().0.notifies()
}

#[test]
fn a_thread_on_a_parent_and_an_empty_thread_are_listed_and_only_the_first_notifies() {
    assert!(notifies("thread-on-parent"));
    assert!(!notifies("empty-thread"));
}

#[test]
fn a_thread_stored_on_a_parent_is_ignored_by_every_reader_and_listed() {
    let mut r = replica("a");
    let by = user(1);
    for id in ["p", "kid", "solo"] {
        add_item(&mut r, id, id, by).unwrap();
    }
    for held in ["t", "u", "empty"] {
        model::set(&mut r, &Key::new(kind::THREAD, held).unwrap(), thread::TITLE, Some(&held), by)
            .unwrap();
    }
    set_parent(&mut r, "kid", Some("p"), by).unwrap();
    for (item, held) in [("kid", "t"), ("solo", "t")] {
        edit(&mut r, &ScopeEdit::Member { item: item.into(), thread: Some(held.into()) }, by)
            .unwrap();
    }
    // Written under the judge, `p` could not hold a thread once it had a child: a sync can leave it so.
    r.write(&item_key("p").unwrap(), field::THREAD, USER, 2, Some(json!("u"))).unwrap();
    // Listed, with the thread it is on and the thread it leaves empty.
    let entries = list(&r);
    let found: Vec<&str> = entries
        .iter()
        .map(Entry::kind)
        .filter(|k| k.contains("thread-on") || k.contains("empty"))
        .collect();
    assert_eq!(found, ["thread-on-parent", "empty-thread", "empty-thread"], "{entries:?}");
    assert!(entries.contains(&Entry::ThreadOnParent {
        item: "p".into(),
        thread: "u".into(),
        fix: "acyclic item edit p --no-thread".into()
    }));
    let model = Model::read(&r.state).unwrap();
    // Not a member, not in a thread, not readiness's, and no reason to keep the thread from being deleted.
    assert_eq!(model.thread_of("p"), None);
    assert_eq!(model.members("u"), Vec::<&str>::new());
    assert_eq!(model.members("t"), ["kid", "solo"]);
    let readiness = ReadinessEvaluator::new(&r.state, &model, NOW);
    assert_eq!(readiness.of("p").unwrap(), Some(crate::ready::Readiness::Parent));
    assert_eq!(readiness.thread("u").unwrap(), crate::ready::ThreadReadiness::NothingReady);
    assert_eq!(claim::covering(&r.state, &model, "p", NOW).unwrap(), Vec::new());
    assert!(crate::delete::delete_thread(&mut r.clone(), "u", by).unwrap());
}

#[test]
fn an_emptied_thread_of_one_is_on_the_list_only_while_someone_holds_it() {
    let mut r = replica("a");
    let by = user(1);
    add_item(&mut r, "x", "x", by).unwrap();
    add_item(&mut r, "kid", "kid", by).unwrap();
    let one = crate::ids::one("x");
    claim::claim(&mut r, &one, "session/s", 2, (20_000, 20_000), true).unwrap();
    model::put(&mut r, &item_key("kid").unwrap(), field::THREAD, Some(&one), by).unwrap();
    // Two members, and no title: valid to read, and nothing is wrong with it.
    assert_eq!(list(&r), []);
    // Both leave: held, the holder lost its work, so the user hears of it; its lease then runs out.
    for id in ["x", "kid"] {
        model::put(&mut r, &item_key(id).unwrap(), field::THREAD, None::<&String>, by).unwrap();
    }
    assert_eq!(
        list(&r),
        [Entry::EmptyThread { thread: one.clone(), fix: format!("acyclic thread release {one}") }]
    );
    claim::release(&mut r, &claim::thread_key(&one).unwrap(), "session/s");
    assert_eq!(list(&r), []);
    // Entering the list never costs the unclaimed task its own row in the ready threads.
    let model = Model::read(&r.state).unwrap();
    assert!(model.thread_exists(&one));
}

#[test]
fn replicas_moving_an_item_under_an_item_and_an_initiative_apart_conflict_and_each_value_is_kept_by_a_move()
 {
    let mut a = replica("a");
    let by = user(1);
    crate::model::add_initiative(&mut a, "init", "Init", by).unwrap();
    for id in ["epic", "x"] {
        add_item(&mut a, id, id, by).unwrap();
    }
    set_parent(&mut a, "epic", Some("init"), by).unwrap();
    set_parent(&mut a, "x", Some("init"), by).unwrap();
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // Apart: one replica files x under the epic, the other leaves it directly under the initiative and says so again.
    set_parent(&mut a, "x", Some("epic"), user(2)).unwrap();
    set_parent(&mut b, "x", Some("init"), user(3)).unwrap();
    let (mut one, mut other) = (a.clone(), b.clone());
    one.join(&b.state).unwrap();
    other.join(&a.state).unwrap();
    for joined in [&one, &other] {
        let model = Model::read(&joined.state).unwrap();
        // Both values stand until chosen, and the item counts under each: the epic's tree and the initiative alike.
        assert_eq!(
            model.items["x"].parent,
            Value::Conflict(vec![Some("epic".to_owned()), Some("init".to_owned())])
        );
        assert_eq!(model.initiatives_of("x"), ["init"]);
        assert_eq!(model.role("x"), crate::model::Role::Task);
        let entries = list(joined);
        assert!(entries.contains(&Entry::Conflict {
            kind: "item".into(),
            id: "x".into(),
            field: "parent".into(),
        }));
        // Each value is kept by moving the item there, and no value keeps it parentless.
        let kept = keeps(&joined.state, &model, ("item", "x", "parent"), user(4)).unwrap();
        let moves: Vec<&Keep> = kept.iter().map(|k| &k.keep).collect();
        let set = |parent: &str| {
            Keep::Write(crate::write::ChangeSet(vec![crate::write::Change::Set(
                crate::write::Assign::Item(
                    "x".into(),
                    crate::write::ItemAssign::Parent(parent.into()),
                ),
            )]))
        };
        assert_eq!(moves, [&set("epic"), &set("init")]);
    }
    assert_eq!(one.state.to_files(), other.state.to_files());
}

#[test]
fn a_clear_raced_against_a_move_is_no_value_to_keep() {
    let mut a = replica("a");
    let by = user(1);
    crate::model::add_initiative(&mut a, "init", "Init", by).unwrap();
    add_item(&mut a, "x", "x", by).unwrap();
    set_parent(&mut a, "x", Some("init"), by).unwrap();
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // An older replica could still clear a parent.
    set_parent(&mut b, "x", None, user(2)).unwrap();
    set_parent(&mut a, "x", Some("init"), user(3)).unwrap();
    a.join(&b.state).unwrap();
    let model = Model::read(&a.state).unwrap();
    let kept = keeps(&a.state, &model, ("item", "x", "parent"), user(4)).unwrap();
    assert!(
        kept.iter().any(|k| k.value.is_null() && k.keep == Keep::Not(Unkept::NoParent)),
        "{kept:?}"
    );
}

#[test]
fn two_replicas_making_one_title_apart_leave_a_duplicate_on_the_list_that_never_notifies() {
    let mut a = replica("a");
    crate::model::add_initiative(&mut a, "init", "Init", user(1)).unwrap();
    crate::model::add_initiative(&mut a, "other", "Other", user(1)).unwrap();
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // Each made "Sow" without seeing the other's; one more in another initiative is no twin.
    add_item(&mut a, "x", "Sow", user(2)).unwrap();
    set_parent(&mut a, "x", Some("init"), user(2)).unwrap();
    for (id, parent) in [("y", "init"), ("z", "other")] {
        add_item(&mut b, id, "Sow", user(2)).unwrap();
        set_parent(&mut b, id, Some(parent), user(2)).unwrap();
    }
    assert!(!list(&a).iter().any(|entry| matches!(entry, Entry::DuplicateTitle { .. })));
    a.join(&b.state).unwrap();

    let twins: Vec<Entry> = list(&a)
        .into_iter()
        .filter(|entry| matches!(entry, Entry::DuplicateTitle { .. }))
        .collect();
    assert_eq!(
        twins,
        [Entry::DuplicateTitle {
            items: vec!["x".into(), "y".into()],
            initiative: Some("init".into())
        }]
    );
    let entry = &twins[0];
    assert_eq!(entry.kind(), "duplicate-title");
    assert_eq!(entry.key(), "duplicate-title:x:y");
    assert_eq!(
        entry.identity(),
        r#"{"entry":"duplicate-title","items":["x","y"],"initiative":"init"}"#
    );
    assert!(!entry.notifies());
    assert_eq!(
        entry.summary(&|id| match id {
            "x" => "Sow".into(),
            "init" => "Init".into(),
            other => other.into(),
        }),
        "2 items share the title Sow in Init"
    );
    // In no initiative, it says so, and leaves the initiative out of its identity.
    let bare = Entry::DuplicateTitle { items: vec!["p".into(), "q".into()], initiative: None };
    assert_eq!(bare.summary(&|_| "Sow".into()), "2 items share the title Sow in no initiative");
    assert_eq!(bare.identity(), r#"{"entry":"duplicate-title","items":["p","q"]}"#);

    // Finishing one leaf frees the title, and the entry goes.
    model::set_state(&mut a, "y", ItemState::Done, user(3)).unwrap();
    assert!(!list(&a).iter().any(|entry| matches!(entry, Entry::DuplicateTitle { .. })));
}

#[test]
fn twins_in_no_initiative_are_listed_with_no_initiative() {
    let mut r = replica("a");
    for id in ["p", "q"] {
        add_item(&mut r, id, "Bare", user(1)).unwrap();
    }
    let twins: Vec<Entry> =
        list(&r).into_iter().filter(|e| matches!(e, Entry::DuplicateTitle { .. })).collect();
    assert_eq!(
        twins,
        [Entry::DuplicateTitle { items: vec!["p".into(), "q".into()], initiative: None }]
    );
}
