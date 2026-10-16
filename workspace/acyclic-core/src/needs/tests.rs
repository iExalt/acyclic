use crate::model::SetField;
use serde_json::json;

use super::*;
use crate::model::{By, ItemState, LinkKind, add_item, item_key, link, set_parent, start_run};
use crate::ready::Ancestry;
use crate::schema::{GLOBAL, item as field, settings, thread};
use crate::scope::{ScopeEdit, USER, edit};
use crate::state::{Replica, ReplicaId};

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
fn each_level_lets_through_its_classes_and_blockers_always_pass() {
    use Class::{Blocker, Irreversible, Other, Scope};
    let matrix = [
        (Level::Everything, [true, true, true, true]),
        (Level::ScopeAndIrreversible, [true, true, true, false]),
        (Level::Blockers, [true, false, false, false]),
    ];
    for (level, expected) in matrix {
        let got = [Blocker, Scope, Irreversible, Other].map(|class| passes(level, class));
        assert_eq!(got, expected, "{level:?}");
    }
}

#[test]
fn a_threads_level_overrides_the_global_one_and_a_conflict_asks_most() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    let global = Key::new(kind::SETTINGS, GLOBAL).unwrap();
    let t = Key::new(kind::THREAD, "t").unwrap();
    assert_eq!(Model::read(&a.state).unwrap().level(Some("t")), Level::Blockers, "the default");
    model::set(&mut a, &global, settings::AUTONOMY, Some(&Level::ScopeAndIrreversible), user(1))
        .unwrap();
    assert_eq!(Model::read(&a.state).unwrap().level(Some("t")), Level::ScopeAndIrreversible);
    model::set(&mut a, &t, thread::AUTONOMY, Some(&Level::Blockers), user(2)).unwrap();
    model::set(&mut b, &t, thread::AUTONOMY, Some(&Level::Everything), user(2)).unwrap();
    let model = Model::read(&a.state).unwrap();
    assert_eq!(
        (model.level(Some("t")), model.level(None)),
        (Level::Blockers, Level::ScopeAndIrreversible)
    );
    a.join(&b.state).unwrap();
    assert_eq!(Model::read(&a.state).unwrap().level(Some("t")), Level::Everything);
}

#[test]
fn questions_reach_the_list_by_their_threads_level() {
    let mut r = replica("a");
    let t = Key::new(kind::THREAD, "t").unwrap();
    model::set(&mut r, &t, thread::TITLE, Some(&"t"), user(1)).unwrap();
    for (id, class) in [("q1", Class::Blocker), ("q2", Class::Scope), ("q3", Class::Other)] {
        let q = Key::new(kind::QUESTION, id).unwrap();
        model::set(&mut r, &q, question::THREAD, Some(&"t"), user(1)).unwrap();
        model::set(&mut r, &q, question::CLASS, Some(&class), user(1)).unwrap();
        model::set(&mut r, &q, question::TEXT, Some(&id), user(1)).unwrap();
        model::set(&mut r, &q, question::OPEN, Some(&true), user(1)).unwrap();
    }
    let asked = |r: &Replica| {
        list(r)
            .into_iter()
            .filter_map(|e| if let Entry::Question { id, .. } = e { Some(id) } else { None })
            .collect::<Vec<_>>()
    };
    assert_eq!(asked(&r), vec!["q1"]);
    model::set(&mut r, &t, thread::AUTONOMY, Some(&Level::ScopeAndIrreversible), user(2)).unwrap();
    assert_eq!(asked(&r), vec!["q1", "q2"]);
    model::set(
        &mut r,
        &Key::new(kind::QUESTION, "q1").unwrap(),
        question::OPEN,
        Some(&false),
        user(3),
    )
    .unwrap();
    assert_eq!(asked(&r), vec!["q2"]);
}

#[test]
fn the_state_puts_its_problems_on_the_list() {
    let (mut a, mut b) = (replica("a"), replica("b"));
    let by = user(1);
    for id in ["x", "y", "z", "p", "kid", "dep", "work", "gpu1", "gpu2", "orphan"] {
        add_item(&mut a, id, id, by).unwrap();
    }
    let ok = Key::new(kind::THREAD, "ok").unwrap();
    model::set(&mut a, &ok, thread::APPROVED, Some(&true), by).unwrap();
    edit(&mut a, &ScopeEdit::Member { item: "work".into(), thread: Some("ok".into()) }, by)
        .unwrap();
    for id in ["gpu1", "gpu2"] {
        model::set_element(&mut a, &item_key(id).unwrap(), SetField::Resource, "gpu", true, by)
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

    // Concurrent: a title conflict, a link cycle, a contest, a resource clash, two runs on one thread.
    model::set(&mut a, &item_key("x").unwrap(), field::TITLE, Some(&"one"), user(2)).unwrap();
    model::set(&mut b, &item_key("x").unwrap(), field::TITLE, Some(&"two"), user(2)).unwrap();
    link(&mut a, "x", "y", LinkKind::Blocks, user(2)).unwrap();
    link(&mut b, "y", "x", LinkKind::After, user(2)).unwrap();
    claim::claim(&mut a, &item_key("x").unwrap(), "session/a", 2, Some(NOW), NOW).unwrap();
    claim::claim(&mut b, &item_key("x").unwrap(), "session/b", 2, Some(NOW), NOW).unwrap();
    claim::claim(&mut a, &item_key("gpu1").unwrap(), "session/a", 2, Some(NOW), NOW).unwrap();
    claim::claim(&mut b, &item_key("gpu2").unwrap(), "session/b", 2, Some(NOW), NOW).unwrap();
    start_run(&mut a, "r1", "ok", user(2)).unwrap();
    start_run(&mut b, "r2", "ok", user(2)).unwrap();
    a.join(&b.state).unwrap();
    // After approval: a new blocker on approved work, an agent's proposal, a claim by an unknown run.
    link(&mut a, "work", "x", LinkKind::Blocks, user(3)).unwrap();
    edit(
        &mut a,
        &ScopeEdit::Member { item: "y".into(), thread: Some("ok".into()) },
        By { actor: "run/r1", at: 3 },
    )
    .unwrap();
    claim::claim(&mut a, &item_key("kid").unwrap(), "run/ghost", 3, None, NOW).unwrap();

    let entries = list(&a);
    let has = |want: &Entry| assert!(entries.contains(want), "missing {want:?} in {entries:#?}");
    has(&Entry::Conflict { kind: "item".into(), id: "x".into(), field: "title".into() });
    has(&Entry::LinkCycle { items: vec!["x".into(), "y".into()] });
    has(&Entry::Contest {
        item: "x".into(),
        holders: vec!["session/a".into(), "session/b".into()],
    });
    has(&Entry::ResourceClash {
        resource: "gpu".into(),
        holders: vec![("gpu1".into(), "session/a".into()), ("gpu2".into(), "session/b".into())],
    });
    has(&Entry::RunClash { thread: "ok".into(), runs: vec!["r1".into(), "r2".into()] });
    has(&Entry::NewBlocker { item: "work".into(), thread: "ok".into(), blocker: "x".into() });
    // Approving the thread again acknowledges it, though its members and approval are unchanged.
    let mut acknowledged = a.clone();
    assert!(scope::approve(&mut acknowledged, "ok", true, user(4)).unwrap());
    assert!(!list(&acknowledged).iter().any(|e| matches!(e, Entry::NewBlocker { .. })));
    assert!(
        !scope::approve(&mut acknowledged, "ok", true, user(5)).unwrap(),
        "a repeat changes nothing"
    );
    assert!(entries.iter().any(|e| matches!(e, Entry::Proposal { actor, threads, .. } if actor == "run/r1" && threads == &["ok"])));
    has(&Entry::UnknownRun { item: "kid".into(), holder: "run/ghost".into() });
    has(&Entry::DoneEarly { item: "p".into(), open: vec!["kid".into()] });
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
fn unapproved_threads_with_members_await_approval_with_their_deadlocks() {
    let mut r = replica("a");
    let by = user(1);
    for (item, thread) in [("x", "t"), ("x2", "t"), ("y", "u"), ("y2", "u")] {
        add_item(&mut r, item, item, by).unwrap();
        edit(&mut r, &ScopeEdit::Member { item: item.into(), thread: Some(thread.into()) }, by)
            .unwrap();
    }
    for (thread, title) in [("t", "T"), ("u", "U"), ("empty", "E")] {
        model::set(
            &mut r,
            &Key::new(kind::THREAD, thread).unwrap(),
            thread::TITLE,
            Some(&title),
            by,
        )
        .unwrap();
    }
    // `t` waits on `u` and `u` on `t`: neither can finish as they stand.
    link(&mut r, "x", "y", LinkKind::Blocks, by).unwrap();
    link(&mut r, "y2", "x2", LinkKind::Blocks, by).unwrap();
    let awaiting = |r: &Replica| -> Vec<Entry> {
        list(r).into_iter().filter(|e| matches!(e, Entry::AwaitingApproval { .. })).collect()
    };
    // Nothing within either thread orders its two members.
    let entry = |thread: &str, other: &str, heads: [&str; 2]| Entry::AwaitingApproval {
        thread: thread.into(),
        members: 2,
        waits_with: vec![other.into()],
        heads: heads.map(String::from).to_vec(),
    };
    model::set(&mut r, &Key::new(kind::THREAD, "finished").unwrap(), thread::TITLE, Some(&"F"), by)
        .unwrap();
    add_item(&mut r, "f", "f", by).unwrap();
    edit(&mut r, &ScopeEdit::Member { item: "f".into(), thread: Some("finished".into()) }, by)
        .unwrap();
    model::set_state(&mut r, "f", ItemState::Done, by).unwrap();
    assert_eq!(
        awaiting(&r),
        vec![entry("t", "u", ["x", "x2"]), entry("u", "t", ["y", "y2"])],
        "an empty thread, or one with only finished work, awaits nothing"
    );
    let t = Key::new(kind::THREAD, "t").unwrap();
    model::set(&mut r, &t, thread::APPROVED, Some(&true), user(2)).unwrap();
    assert_eq!(awaiting(&r), vec![entry("u", "t", ["y", "y2"])]);
}

#[test]
fn an_agents_member_the_approval_never_saw_is_the_users_to_keep_or_drop() {
    let agent = By { actor: "session/pi", at: 2 };
    let unapproved = |r: &Replica| -> Vec<String> {
        list(r)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Unapproved { item, .. } => Some(item),
                _ => None,
            })
            .collect()
    };
    let member = |item: &str| ScopeEdit::Member { item: item.into(), thread: Some("t".into()) };
    let (mut a, mut b) = (replica("a"), replica("b"));
    for id in ["seen", "late", "other"] {
        add_item(&mut a, id, id, user(1)).unwrap();
    }
    edit(&mut a, &member("seen"), user(1)).unwrap();
    b.join(&a.state).unwrap();
    // b's agent sees t unapproved, so its edit applies, while a's user approves t.
    scope::approve(&mut a, "t", true, user(2)).unwrap();
    edit(&mut b, &member("late"), agent).unwrap();
    a.join(&b.state).unwrap();
    let entry = Entry::Unapproved {
        item: "late".into(),
        thread: "t".into(),
        actor: "session/pi".into(),
        keep: "acyclic thread add late t".into(),
        drop: "acyclic thread remove late".into(),
    };
    assert!(list(&a).contains(&entry), "{:?}", list(&a));
    // The user putting it in the thread again keeps it: the user's write needs no approval.
    edit(&mut a, &member("late"), user(3)).unwrap();
    assert_eq!(unapproved(&a), Vec::<String>::new());
    // Approvals that raced with different members each approved theirs.
    edit(&mut b, &member("other"), agent).unwrap();
    let mut c = replica("c");
    c.join(&a.state).unwrap();
    c.join(&b.state).unwrap();
    scope::approve(&mut c, "t", true, user(4)).unwrap();
    edit(&mut a, &ScopeEdit::Member { item: "seen".into(), thread: None }, user(4)).unwrap();
    assert!(scope::approve(&mut a, "t", true, user(4)).unwrap());
    a.join(&c.state).unwrap();
    a.join(&b.state).unwrap();
    assert!(Model::read(&a.state).unwrap().threads["t"].approved_members.is_conflict());
    assert_eq!(unapproved(&a), Vec::<String>::new(), "other is in c's list, which raced a's");
    // A thread approved before approvals recorded their members has nothing to compare against.
    let old = Key::new(kind::THREAD, "old").unwrap();
    add_item(&mut b, "legacy", "legacy", user(5)).unwrap();
    edit(&mut b, &ScopeEdit::Member { item: "legacy".into(), thread: Some("old".into()) }, agent)
        .unwrap();
    model::set(&mut b, &old, thread::APPROVED, Some(&true), user(5)).unwrap();
    assert_eq!(unapproved(&b), Vec::<String>::new());
}
