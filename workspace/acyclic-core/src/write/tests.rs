use hegel::TestCase;
use hegel::generators as gs;
use serde_json::json;

use super::*;
use crate::state::{ReplicaId, State};

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn by(at: Millis) -> By<'static> {
    By { actor: "actor/user", at }
}

fn read(replica: &Replica) -> Model {
    Model::read(&replica.state).unwrap()
}

#[test]
fn expected_fields_and_one_wait_edge_match_the_model_values() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            add("a", vec![ItemAssign::Notes(None), ItemAssign::WaitsFor("b".into(), true)]),
            add("b", vec![]),
            add("c", vec![]),
        ],
    );
    let expected = [
        Expectation::ItemField {
            item: "a".into(),
            field: ExpectedItemField::Title,
            value: ExpectedValue::One("A".into()),
        },
        Expectation::ItemField {
            item: "a".into(),
            field: ExpectedItemField::Notes,
            value: ExpectedValue::Unset(()),
        },
        Expectation::WaitsFor { item: "a".into(), target: "b".into(), present: true },
    ];
    assert_eq!(check_expected(&r.state, &read(&r), &expected, 2).unwrap(), Vec::new());

    // Another edge from the same item does not invalidate the one this Undo depends on.
    commit_ok(&mut r, 3, vec![item("a", ItemAssign::WaitsFor("c".into(), true))]);
    assert_eq!(check_expected(&r.state, &read(&r), &expected, 4).unwrap(), Vec::new());

    let mismatch = check_expected(
        &r.state,
        &read(&r),
        &[Expectation::WaitsFor { item: "a".into(), target: "b".into(), present: false }],
        4,
    )
    .unwrap();
    assert_eq!(mismatch.len(), 1);
    assert_eq!(mismatch[0].actual, Json::Bool(true));

    for (item, target) in [("gone", "b"), ("a", "gone")] {
        let mismatch = check_expected(
            &r.state,
            &read(&r),
            &[Expectation::WaitsFor { item: item.into(), target: target.into(), present: false }],
            4,
        )
        .unwrap();
        assert_eq!(mismatch.len(), 1);
        assert_eq!(mismatch[0].actual["missing_items"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn expected_thread_title_and_notes_match_and_mismatch() {
    let mut r = replica("a");
    commit_ok(&mut r, 1, vec![thread("t")]);
    let expected = [
        Expectation::ThreadField {
            thread: "t".into(),
            field: ExpectedThreadField::Title,
            value: ExpectedValue::One("T".into()),
        },
        Expectation::ThreadField {
            thread: "t".into(),
            field: ExpectedThreadField::Notes,
            value: ExpectedValue::Unset(()),
        },
    ];
    assert_eq!(check_expected(&r.state, &read(&r), &expected, 2).unwrap(), Vec::new());
    let mismatch = check_expected(
        &r.state,
        &read(&r),
        &[Expectation::ThreadField {
            thread: "t".into(),
            field: ExpectedThreadField::Title,
            value: ExpectedValue::One("stale".into()),
        }],
        2,
    )
    .unwrap();
    assert_eq!(mismatch.len(), 1);
    assert_eq!(mismatch[0].actual, json!("T"));
}

#[test]
fn expected_conflict_values_keep_their_order_and_malformed_conditions_are_rejected() {
    let mut a = replica("a");
    commit_ok(&mut a, 1, vec![add("a", vec![])]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    commit_ok(&mut a, 2, vec![item("a", ItemAssign::Title("first".into()))]);
    commit_ok(&mut b, 2, vec![item("a", ItemAssign::Title("second".into()))]);
    a.join(&b.state).unwrap();

    let current = serde_json::to_value(&read(&a).items["a"].title).unwrap();
    let expected: ExpectedValue = serde_json::from_value(current.clone()).unwrap();
    let condition = Expectation::ItemField {
        item: "a".into(),
        field: ExpectedItemField::Title,
        value: expected,
    };
    assert_eq!(
        check_expected(&a.state, &read(&a), std::slice::from_ref(&condition), 3).unwrap(),
        Vec::new()
    );

    let reversed = match current {
        Json::Object(mut object) => {
            let Json::Array(mut values) = object.remove("conflict").unwrap() else {
                panic!("conflict sides are an array");
            };
            values.reverse();
            json!({"conflict": values})
        },
        other => panic!("expected a conflict, got {other}"),
    };
    let condition: Expectation = serde_json::from_value(json!({
        "kind": "item-field", "item": "a", "field": "title", "value": reversed
    }))
    .unwrap();
    assert_eq!(check_expected(&a.state, &read(&a), &[condition], 3).unwrap().len(), 1);

    for invalid in [
        json!({"kind": "item-field", "item": "a", "field": "unknown", "value": null}),
        json!({"kind": "waits-for", "item": "a", "target": "b", "present": true, "ignore": true}),
        json!({"kind": "item-field", "item": "a", "field": "title", "value": {"other": "x"}}),
        json!({"kind": "item-field", "item": "a", "field": "title", "value": {"conflict": []}}),
        json!({"kind": "item-field", "item": "a", "field": "title"}),
    ] {
        assert!(serde_json::from_value::<Expectation>(invalid).is_err());
    }
}

#[test]
fn waiter_guard_covers_new_direct_and_ancestor_waiters_and_keeps_original_waiters() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("parent", vec![]),
            add("done", vec![ItemAssign::Parent("parent".into())]),
            add("direct", vec![ItemAssign::Thread(Some("t".into()))]),
            add("ancestor", vec![ItemAssign::Thread(Some("t".into()))]),
        ],
    );
    commit_ok(&mut r, 2, vec![item("done", ItemAssign::State(ItemState::Done))]);
    let guard = |baselines| Expectation::WaitersStartedAtMost {
        item: "done".into(),
        baselines: StartedBaselines(baselines),
    };
    let empty = guard(vec![]);
    assert_eq!(check_expected(&r.state, &read(&r), &[empty], 3).unwrap(), Vec::new());

    commit_ok(
        &mut r,
        3,
        vec![
            item("direct", ItemAssign::WaitsFor("done".into(), true)),
            item("ancestor", ItemAssign::WaitsFor("parent".into(), true)),
        ],
    );
    assert_eq!(check_expected(&r.state, &read(&r), &[guard(vec![])], 4).unwrap(), Vec::new());
    // A deleted original waiter has a current started count of zero.
    let deleted = guard(vec![StartedBaseline { item: "gone".into(), count: 0 }]);
    assert_eq!(check_expected(&r.state, &read(&r), &[deleted], 4).unwrap(), Vec::new());

    crate::claim::claim(&mut r, "t", "session/test", 5, (60_000, 60_000), false).unwrap();
    let mismatch = check_expected(&r.state, &read(&r), &[guard(vec![])], 6).unwrap();
    assert_eq!(mismatch.len(), 1);
    assert_eq!(mismatch[0].actual["started"].as_array().unwrap().len(), 2);

    // Removing an original edge does not erase the conservative baseline captured by Undo.
    let captured = guard(vec![StartedBaseline { item: "direct".into(), count: 0 }]);
    commit_ok(&mut r, 7, vec![item("direct", ItemAssign::WaitsFor("done".into(), false))]);
    assert_eq!(check_expected(&r.state, &read(&r), &[captured], 8).unwrap().len(), 1);

    let duplicate = json!({
        "kind": "waiters-started-at-most", "item": "done", "baselines": [
            {"item": "direct", "count": 0}, {"item": "direct", "count": 1}
        ]
    });
    assert!(serde_json::from_value::<Expectation>(duplicate).is_err());
}

#[test]
fn started_at_most_includes_live_claims_on_leaf_and_parent() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("root", vec![]),
            add(
                "leaf",
                vec![ItemAssign::Parent("root".into()), ItemAssign::Thread(Some("t".into()))],
            ),
        ],
    );
    let no_started = [
        Expectation::StartedAtMost { item: "leaf".into(), count: 0 },
        Expectation::StartedAtMost { item: "root".into(), count: 0 },
    ];
    assert_eq!(check_expected(&r.state, &read(&r), &no_started, 2).unwrap(), Vec::new());

    crate::claim::claim(&mut r, "t", "session/test", 3, (60_000, 60_000), false).unwrap();
    let mismatch = check_expected(&r.state, &read(&r), &no_started, 4).unwrap();
    assert_eq!(mismatch.len(), 2);
    assert!(mismatch.iter().all(|one| one.actual.as_u64() == Some(1)));
    let allowed = [
        Expectation::StartedAtMost { item: "leaf".into(), count: 1 },
        Expectation::StartedAtMost { item: "root".into(), count: 1 },
    ];
    assert_eq!(check_expected(&r.state, &read(&r), &allowed, 4).unwrap(), Vec::new());
    let missing = [Expectation::StartedAtMost { item: "gone".into(), count: 0 }];
    assert_eq!(check_expected(&r.state, &read(&r), &missing, 4).unwrap()[0].actual, Json::Null);
}

fn item(id: &str, assign: ItemAssign) -> Change {
    Change::Set(Assign::Item(id.to_owned(), assign))
}

fn add(id: &str, more: Vec<ItemAssign>) -> Change {
    let mut with = vec![Assign::Item(id.to_owned(), ItemAssign::Title(id.to_uppercase()))];
    with.extend(more.into_iter().map(|assign| Assign::Item(id.to_owned(), assign)));
    Change::Add {
        record: Record::Item(id.to_owned()),
        minted: false,
        on_repeat: Repeat::Update,
        with,
    }
}

fn thread(id: &str) -> Change {
    Change::Add {
        record: Record::Thread(id.to_owned()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![Assign::Thread(id.to_owned(), ThreadAssign::Title(id.to_uppercase()))],
    }
}

/// Commits `changes` at `at`, panicking on a refusal.
fn commit_ok(replica: &mut Replica, at: Millis, changes: Vec<Change>) -> Outcome {
    commit(replica, by(at), false, |_| Ok::<_, ()>(ChangeSet(changes))).unwrap()
}

/// The rules a refused change set breaks.
fn refused(replica: &Replica, changes: Vec<Change>) -> Vec<Rule> {
    let violations = judge(&read(replica), &ChangeSet(changes), by(99), None).unwrap_err();
    violations.into_iter().map(|violation| violation.rule).collect()
}

/// A tree: `root` over `mid` over `leaf`, all done, with `leaf` in thread `t`.
fn done_tree() -> Replica {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("root", vec![]),
            add("mid", vec![ItemAssign::Parent("root".into())]),
            add(
                "leaf",
                vec![ItemAssign::Parent("mid".into()), ItemAssign::Thread(Some("t".into()))],
            ),
        ],
    );
    commit_ok(&mut r, 2, vec![item("leaf", ItemAssign::State(ItemState::Done))]);
    r
}

#[test]
fn a_change_that_holds_already_writes_nothing_but_a_conflict_takes_it() {
    let mut a = done_tree();
    let again = judge(
        &read(&a),
        &ChangeSet(vec![item("leaf", ItemAssign::Title("LEAF".into()))]),
        by(3),
        None,
    );
    assert!(!again.unwrap().changes());
    // Two replicas retitle it apart: the same title again now settles the conflict.
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    commit_ok(&mut a, 3, vec![item("leaf", ItemAssign::Title("A".into()))]);
    commit_ok(&mut b, 3, vec![item("leaf", ItemAssign::Title("B".into()))]);
    a.join(&b.state).unwrap();
    let settle = commit_ok(&mut a, 4, vec![item("leaf", ItemAssign::Title("A".into()))]);
    assert!(settle.changes());
    assert_eq!(read(&a).items["leaf"].title, Value::One("A".into()));
    assert_eq!(read(&a).conflicts().count(), 0);
}

#[test]
fn a_created_record_needs_a_field_write_and_the_model_matches_rereading() {
    let mut r = replica("empty");
    for record in [Record::Thread("t".into()), Record::Repository("r".into())] {
        let empty = Change::Add {
            record: record.clone(),
            minted: false,
            on_repeat: Repeat::Update,
            with: vec![],
        };
        assert_eq!(refused(&r, vec![empty.clone()]), [Rule::EmptyRecord(record.clone())]);

        let before = r.state.clone();
        let error =
            commit(&mut r, by(1), false, |_| Ok::<_, ()>(ChangeSet(vec![empty]))).unwrap_err();
        let CommitError::Refused(violations, _) = error else {
            panic!("an empty record add should be refused");
        };
        assert_eq!(
            violations.into_iter().map(|violation| violation.rule).collect::<Vec<_>>(),
            [Rule::EmptyRecord(record)]
        );
        assert_eq!(r.state, before, "a refused empty add must leave the replica unchanged");
    }

    let thread = Change::Add {
        record: Record::Thread("t".into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![],
    };
    let repository = Change::Add {
        record: Record::Repository("r".into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![],
    };
    let item = Change::Add {
        record: Record::Item("i".into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![],
    };
    let outcome = commit_ok(
        &mut r,
        1,
        vec![
            thread,
            Change::Set(Assign::Thread("t".into(), ThreadAssign::Notes(None))),
            repository,
            Change::Set(Assign::Repository("r".into(), RepositoryAssign::Owner("example".into()))),
            item,
        ],
    );
    assert_eq!(outcome.after, read(&r), "null fields and the item's implicit open state persist");
    assert!(read(&r).threads.contains_key("t"));
    assert!(read(&r).repositories.contains_key("r"));
    assert!(read(&r).items.contains_key("i"));
}

#[test]
fn waiting_between_an_item_and_one_above_or_below_it_is_refused_however_it_comes() {
    let mut r = done_tree();
    commit_ok(&mut r, 3, vec![add("other", vec![])]);
    let lineage = |from: &str, to: &str| Rule::LineageLink { from: from.into(), to: to.into() };
    let waits = |from: &str, to: &str| item(from, ItemAssign::WaitsFor(to.into(), true));
    assert!(refused(&r, vec![waits("leaf", "root")]).contains(&lineage("leaf", "root")));
    assert!(refused(&r, vec![waits("root", "leaf")]).contains(&lineage("root", "leaf")));
    // A move that puts two linked items in one line is refused too.
    commit_ok(&mut r, 4, vec![waits("other", "mid")]);
    let moved = refused(&r, vec![item("mid", ItemAssign::Parent("other".into()))]);
    assert!(moved.contains(&lineage("other", "mid")), "{moved:?}");
    // One a sync brought holds no other write back.
    let mut synced = done_tree();
    commit_ok(&mut synced, 3, vec![add("x", vec![])]);
    let mut apart = replica("b");
    apart.join(&synced.state).unwrap();
    commit_ok(&mut synced, 4, vec![waits("x", "leaf")]);
    commit_ok(&mut apart, 4, vec![item("leaf", ItemAssign::Parent("x".into()))]);
    synced.join(&apart.state).unwrap();
    assert!(read(&synced).ancestors("leaf").contains(&"x"));
    commit_ok(&mut synced, 5, vec![item("mid", ItemAssign::Title("Mid".into()))]);
}

fn thread_of(r: &Replica, id: &str) -> Option<String> {
    read(r).items[id].thread.one().cloned()
}

fn under(parent: &str) -> ItemAssign {
    ItemAssign::Parent(parent.into())
}

#[test]
fn new_work_joins_its_siblings_thread_unless_given_one_and_a_move_changes_none() {
    let mut r = done_tree();
    // `leaf` is done and the only child of `mid`, and it is in `t`: finished siblings still place new work.
    let outcome = commit_ok(
        &mut r,
        3,
        vec![
            add("x", vec![under("mid")]),
            add("y", vec![]),
            item("y", under("mid")),
            add("z", vec![under("mid"), ItemAssign::Thread(None)]),
        ],
    );
    let placed: Vec<(String, String)> =
        outcome.placed().into_iter().map(|placed| (placed.item, placed.thread)).collect();
    assert_eq!(placed, [("x".into(), "t".into()), ("y".into(), "t".into())]);
    // An item given a thread, even none, keeps what it was given.
    assert_eq!(thread_of(&r, "z"), None);
    assert_eq!(thread_of(&r, "x"), Some("t".into()));
    // A move never changes a thread: not given one, and not taking the siblings' either.
    commit_ok(&mut r, 4, vec![add("loose", vec![]), add("other", vec![ItemAssign::Thread(None)])]);
    let moved =
        commit_ok(&mut r, 5, vec![item("loose", under("mid")), item("other", under("mid"))]);
    assert_eq!(moved.placed(), Vec::new());
    assert_eq!((thread_of(&r, "loose"), thread_of(&r, "other")), (None, None));
    commit_ok(&mut r, 6, vec![thread("u"), item("x", ItemAssign::Thread(Some("u".into())))]);
    commit_ok(&mut r, 7, vec![add("top", vec![]), item("x", under("top"))]);
    assert_eq!(thread_of(&r, "x"), Some("u".into()));
}

#[test]
fn siblings_that_disagree_or_part_in_none_refuse_a_new_child_unless_it_says_where_it_goes() {
    let mut r = done_tree();
    commit_ok(&mut r, 3, vec![thread("u"), add("w", vec![under("mid"), ItemAssign::Thread(None)])]);
    // `leaf` is in `t`, `w` in none.
    let unclear = Rule::ThreadUnclear { parent: "mid".into() };
    assert_eq!(refused(&r, vec![add("x", vec![under("mid")])]), std::slice::from_ref(&unclear));
    commit_ok(&mut r, 4, vec![item("w", ItemAssign::Thread(Some("u".into())))]);
    assert_eq!(refused(&r, vec![add("x", vec![under("mid")])]), [unclear]);
    // Each of the ways to say where it goes is judged as given.
    let explicit = [ItemAssign::Thread(Some("u".into())), ItemAssign::Thread(None)];
    for (id, assign) in ["ok1", "ok2"].into_iter().zip(explicit) {
        commit_ok(&mut r, 5, vec![add(id, vec![under("mid"), assign])]);
    }
    // A moved item is never refused for it, and finished new work needs no thread.
    commit_ok(&mut r, 6, vec![add("m", vec![])]);
    commit_ok(&mut r, 7, vec![item("m", under("mid"))]);
    commit_ok(&mut r, 8, vec![add("d", vec![under("mid"), ItemAssign::State(ItemState::Done)])]);
}

#[test]
fn a_first_child_takes_its_parents_thread_or_a_move_or_an_explicit_thread_drops_it_in_one_write() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            thread("u"),
            add("p", vec![ItemAssign::Thread(Some("t".into()))]),
            add("q", vec![ItemAssign::Thread(Some("t".into()))]),
            add("s", vec![ItemAssign::Thread(Some("t".into()))]),
            add("mv", vec![ItemAssign::Thread(Some("u".into()))]),
            add("only", vec![ItemAssign::Thread(Some("t".into()))]),
        ],
    );
    // A plain add: the thread moves from the parent to the child.
    let outcome = commit_ok(&mut r, 2, vec![add("p1", vec![under("p")])]);
    assert_eq!((thread_of(&r, "p"), thread_of(&r, "p1")), (None, Some("t".into())));
    assert_eq!(outcome.placed().len(), 1);
    // An explicit thread, even none, takes the child's own way and drops the parent's.
    commit_ok(&mut r, 3, vec![add("q1", vec![under("q"), ItemAssign::Thread(Some("u".into()))])]);
    assert_eq!((thread_of(&r, "q"), thread_of(&r, "q1")), (None, Some("u".into())));
    commit_ok(&mut r, 4, vec![add("s1", vec![under("s"), ItemAssign::Thread(None)])]);
    assert_eq!((thread_of(&r, "s"), thread_of(&r, "s1")), (None, None));
    // A move drops it and leaves the moved item's own thread alone.
    commit_ok(&mut r, 5, vec![item("mv", under("only"))]);
    assert_eq!((thread_of(&r, "only"), thread_of(&r, "mv")), (None, Some("u".into())));
    // Only the first child: a later one joins the siblings' thread and the parent holds nothing to drop.
    let later = commit_ok(&mut r, 6, vec![add("p2", vec![under("p")])]);
    assert_eq!(later.placed().len(), 1);
    assert_eq!(thread_of(&r, "p2"), Some("t".into()));
    // One write: a set refused for another reason leaves the parent's thread where it was.
    commit_ok(&mut r, 7, vec![add("leafy", vec![ItemAssign::Thread(Some("t".into()))])]);
    let before = r.state.clone();
    let broken = ChangeSet(vec![
        add("leafy1", vec![under("leafy")]),
        item("leafy", ItemAssign::WaitsFor("leafy".into(), true)),
    ]);
    assert!(matches!(
        commit(&mut r, by(8), false, |_| Ok::<_, ()>(broken)),
        Err(CommitError::Refused(..))
    ));
    assert_eq!(r.state, before);
    assert_eq!(thread_of(&r, "leafy"), Some("t".into()));
}

#[test]
fn only_violations_the_change_set_introduces_are_refused() {
    let mut a = done_tree();
    commit_ok(&mut a, 3, vec![add("p", vec![]), add("q", vec![])]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // Two replicas close a cycle between them.
    commit_ok(&mut a, 4, vec![item("p", ItemAssign::WaitsFor("q".into(), true))]);
    commit_ok(&mut b, 4, vec![item("q", ItemAssign::WaitsFor("p".into(), true))]);
    a.join(&b.state).unwrap();
    assert_eq!(read(&a).waits_for_cycles().len(), 1);
    // An unrelated write still goes through; a new cycle doesn't.
    commit_ok(&mut a, 5, vec![item("p", ItemAssign::Title("P2".into()))]);
    commit_ok(&mut a, 6, vec![add("s", vec![ItemAssign::WaitsFor("p".into(), true)])]);
    let rules = refused(&a, vec![item("p", ItemAssign::WaitsFor("s".into(), true))]);
    assert!(matches!(rules.as_slice(), [Rule::WaitsForCycle(_)]), "{rules:?}");
}

#[test]
fn an_add_assignment_must_name_the_record_being_added() {
    let mut r = replica("a");
    let before = r.state.clone();
    let changes = ChangeSet(vec![Change::Add {
        record: Record::Item("new".into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![Assign::Item("other".into(), ItemAssign::Title("Wrong".into()))],
    }]);
    let Err(CommitError::Refused(violations, _)) =
        commit(&mut r, by(1), false, |_| Ok::<_, ()>(changes))
    else {
        panic!("a misdirected assignment must be refused");
    };
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, Rule::WrongRecord(Record::Item("new".into())));
    assert_eq!(
        violations[0].to_string(),
        "item other's title: the assignment must name the item new being added"
    );
    assert_eq!(r.state, before);
}

#[test]
fn every_violation_is_named_at_once() {
    let mut r = done_tree();
    commit_ok(&mut r, 3, vec![add("gone", vec![])]);
    crate::delete::delete_item(&mut r, "gone", by(4)).unwrap();
    let rules = refused(
        &r,
        vec![
            item("leaf", ItemAssign::Parent("nowhere".into())),
            item("leaf", ItemAssign::WaitsFor("leaf".into(), true)),
            item("mid", ItemAssign::Title("A".into())),
            item("mid", ItemAssign::Title("B".into())),
            item("gone", ItemAssign::Title("back".into())),
        ],
    );
    // In the order of the changes that break them.
    assert_eq!(
        rules,
        [
            Rule::NoRecord(Record::Item("nowhere".into())),
            Rule::SelfLink,
            Rule::Contradicts,
            Rule::Deleted(Record::Item("gone".into())),
        ]
    );
}

#[test]
fn a_deleted_thread_refusal_names_the_thread() {
    let mut r = replica("a");
    commit_ok(&mut r, 1, vec![thread("gone")]);
    crate::delete::delete_thread(&mut r, "gone", by(2)).unwrap();
    let violations = judge(&read(&r), &ChangeSet(vec![thread("gone")]), by(3), None).unwrap_err();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].rule, Rule::Deleted(Record::Thread("gone".into())));
    assert_eq!(violations[0].to_string(), "thread gone was deleted, and its id isn't used again");
}

#[test]
fn evidence_needs_done_work_and_an_add_may_refuse_to_repeat_differently() {
    let mut r = done_tree();
    commit_ok(&mut r, 3, vec![add("open", vec![])]);
    let rules = refused(&r, vec![item("open", ItemAssign::Evidence(Some(Evidence::default())))]);
    assert_eq!(rules, [Rule::EvidenceNotDone]);
    let again = |title: &str| Change::Add {
        record: Record::Item("open".into()),
        minted: false,
        on_repeat: Repeat::Refuse,
        with: vec![Assign::Item("open".into(), ItemAssign::Title(title.into()))],
    };
    assert!(!judge(&read(&r), &ChangeSet(vec![again("OPEN")]), by(4), None).unwrap().changes());
    assert_eq!(refused(&r, vec![again("other")]), [Rule::RepeatDiffers]);
}

#[test]
fn a_minted_add_may_not_take_open_works_title_and_a_name_is_taken_by_any_value_a_record_could_hold()
{
    let mut a = done_tree();
    commit_ok(&mut a, 3, vec![add("open", vec![])]);
    let minted = |id: &str, title: &str| Change::Add {
        record: Record::Item(id.into()),
        minted: true,
        on_repeat: Repeat::Refuse,
        with: vec![Assign::Item(id.into(), ItemAssign::Title(title.into()))],
    };
    assert_eq!(
        refused(&a, vec![minted("twin", "OPEN")]),
        [Rule::TitleTaken(Record::Item("open".into()))]
    );
    // Done work's title is free again.
    assert!(judge(&read(&a), &ChangeSet(vec![minted("twin", "LEAF")]), by(4), None).is_ok());
    let repository = |id: &str, owner: &str| Change::Add {
        record: Record::Repository(id.into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![
            Assign::Repository(id.into(), RepositoryAssign::Owner(owner.into())),
            Assign::Repository(id.into(), RepositoryAssign::Name("code".into())),
            Assign::Repository(id.into(), RepositoryAssign::DefaultBranch("main".into())),
        ],
    };
    commit_ok(&mut a, 4, vec![repository("r1", "me")]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    commit_ok(
        &mut b,
        5,
        vec![Change::Set(Assign::Repository("r1".into(), RepositoryAssign::Owner("you".into())))],
    );
    a.join(&b.state).unwrap();
    // r1's owner is me or you until settled, so both names are taken.
    let rules = refused(&a, vec![repository("r2", "You")]);
    assert_eq!(rules, [Rule::NameTaken { name: "you/code".into(), other: "r1".into() }]);
}

/// A random change to a small fixed set of records.
fn change(tc: &TestCase) -> Change {
    // `d` never exists, so naming it is refused.
    let ids = ["a", "b", "c", "a", "b", "c", "d"];
    let pick = |tc: &TestCase| ids[tc.draw(gs::integers::<usize>().max_value(6))].to_owned();
    let maybe = |tc: &TestCase| tc.draw(gs::booleans()).then(|| pick(tc));
    let id = pick(tc);
    let assign = match tc.draw(gs::integers::<u8>().max_value(7)) {
        0 => ItemAssign::Title(pick(tc)),
        1 => ItemAssign::Notes(maybe(tc)),
        2 => ItemAssign::State(
            [ItemState::Backlog, ItemState::Parked, ItemState::Done]
                [tc.draw(gs::integers::<usize>().max_value(2))],
        ),
        3 => ItemAssign::Parent(pick(tc)),
        4 => ItemAssign::Thread(tc.draw(gs::booleans()).then(|| "t".to_owned())),
        5 => ItemAssign::WaitsFor(pick(tc), tc.draw(gs::booleans())),
        _ => return add(&id, vec![]),
    };
    Change::Set(Assign::Item(id, assign))
}

/// What the judge projects is what reading the written state gives, across replicas and conflicts.
#[test]
fn settling_a_conflicted_parent_on_the_value_that_closes_a_cycle_is_refused() {
    let mut a = replica("a");
    let kid = add("kid", vec![ItemAssign::Parent("x".into())]);
    commit_ok(&mut a, 1, vec![add("x", vec![]), add("p", vec![]), kid]);
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    // One replica, as an older version might, wrote x under its own child while another moved it under p.
    let x = crate::model::item_key("x").unwrap();
    a.write(&x, field::PARENT, "actor/user", 2, Some(Json::from("kid"))).unwrap();
    commit_ok(&mut b, 2, vec![item("x", ItemAssign::Parent("p".into()))]);
    a.join(&b.state).unwrap();
    let rules = refused(&a, vec![item("x", ItemAssign::Parent("kid".into()))]);
    assert!(matches!(rules.as_slice(), [Rule::ParentCycle(_)]), "{rules:?}");
    assert_eq!(refused_or_none(&a, item("x", ItemAssign::Parent("p".into()))), []);
}

/// The rules one change breaks, or none when it is accepted.
fn refused_or_none(replica: &Replica, change: Change) -> Vec<Rule> {
    judge(&read(replica), &ChangeSet(vec![change]), by(99), None)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|violation| violation.rule)
        .collect()
}

/// The rules finishing `id` breaks, judged with readiness or without it, or none when it is accepted.
fn finishing(
    r: &Replica,
    id: &str,
    evidence: Option<Evidence>,
    force: bool,
    ready: bool,
) -> Vec<Rule> {
    let before = read(r);
    let readiness = ready.then(|| ReadinessEvaluator::new(&r.state, &before, 99));
    let finish = Change::Finish { item: id.into(), evidence, force };
    judge(&before, &ChangeSet(vec![finish]), by(99), readiness.as_ref())
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|violation| violation.rule)
        .collect()
}

#[test]
fn finishing_takes_work_that_is_ready_or_the_finishers_own_unless_forced() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("free", vec![ItemAssign::Thread(Some("t".into()))]),
            add("parent", vec![ItemAssign::Thread(Some("t".into()))]),
            add("child", vec![ItemAssign::Parent("parent".into())]),
            add("lone", vec![]),
        ],
    );
    assert_eq!(finishing(&r, "free", None, false, true), []);
    // Without readiness to judge by, only a forced finish passes.
    assert_eq!(finishing(&r, "free", None, false, false), [Rule::NotReady(None)]);
    assert_eq!(finishing(&r, "free", None, true, false), []);
    // A task in no thread is a thread of one: ready, so finishing it needs no claim.
    assert_eq!(finishing(&r, "lone", None, false, true), []);
    // A parent reads done once its leaves are, so storing done on one is refused, forced or not.
    assert_eq!(finishing(&r, "parent", None, false, true), [Rule::ParentDone]);
    assert_eq!(finishing(&r, "parent", None, true, false), [Rule::ParentDone]);
    assert_eq!(finishing(&r, "lone", None, true, true), []);
    // Someone else's claim holds the thread's work back from the finisher.
    crate::claim::claim(&mut r, "t", "session/other", 2, (60_000, 60_000), false).unwrap();
    let held = Readiness::Claimed { holder: "session/other".into() };
    assert_eq!(finishing(&r, "free", None, false, true), [Rule::NotReady(Some(held))]);
    commit_ok(&mut r, 3, vec![item("child", ItemAssign::State(ItemState::Done))]);
    assert_eq!(finishing(&r, "parent", None, false, true), [Rule::ParentDone]);
    let ghost = Change::Finish { item: "ghost".into(), evidence: None, force: true };
    assert_eq!(refused(&r, vec![ghost]), [Rule::NoRecord(Record::Item("ghost".into()))]);
}

/// The rules canceling `id` breaks, judged with readiness as `by(99)`, or none when it is accepted.
fn canceling(r: &Replica, id: &str, force: bool) -> Vec<Rule> {
    let before = read(r);
    let readiness = ReadinessEvaluator::new(&r.state, &before, 99);
    let cancel = Change::Cancel { item: id.into(), force };
    judge(&before, &ChangeSet(vec![cancel]), by(99), Some(&readiness))
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|violation| violation.rule)
        .collect()
}

#[test]
fn canceling_refuses_done_work_and_another_holders_claim_unless_forced_and_frees_the_title() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("free", vec![ItemAssign::Thread(Some("t".into()))]),
            add("done", vec![]),
            add("parent", vec![]),
            add("child", vec![ItemAssign::Parent("parent".into())]),
        ],
    );
    assert_eq!(canceling(&r, "free", false), []);
    // Done work, and a parent that reads done, are reopened first, forced or not.
    commit_ok(&mut r, 2, vec![item("done", ItemAssign::State(ItemState::Done))]);
    assert_eq!(canceling(&r, "done", false), [Rule::CancelDone]);
    assert_eq!(canceling(&r, "done", true), [Rule::CancelDone]);
    assert_eq!(canceling(&r, "parent", false), []);
    commit_ok(&mut r, 3, vec![item("child", ItemAssign::State(ItemState::Done))]);
    assert_eq!(canceling(&r, "parent", true), [Rule::CancelDone]);
    // Someone else's claim refuses, on the item or on a parent over it; the actor's own, or force, doesn't.
    crate::claim::claim(&mut r, "t", "actor/user", 3, (60_000, 60_000), false).unwrap();
    assert_eq!(canceling(&r, "free", false), []);
    assert!(crate::claim::release(&mut r, &crate::claim::thread_key("t").unwrap(), "actor/user"));
    crate::claim::claim(&mut r, "t", "session/other", 4, (60_000, 60_000), false).unwrap();
    let held = Rule::CancelHeld { holder: "session/other".into() };
    assert_eq!(canceling(&r, "free", false), [held]);
    assert_eq!(canceling(&r, "free", true), []);
    // Canceled, the state is stored as `canceled`, and what it frees is its title.
    commit_ok(&mut r, 5, vec![Change::Cancel { item: "free".into(), force: true }]);
    assert_eq!(read(&r).items["free"].state, Value::One(ItemState::Canceled));
    let stored = |r: &Replica| {
        let key = crate::model::item_key("free").unwrap();
        r.state.field(&key, "state").unwrap().values().map(|v| v.value.clone()).collect::<Vec<_>>()
    };
    assert_eq!(stored(&r), [json!("canceled")]);
    // Opened again, it is stored as `backlog`, as open always is.
    commit_ok(&mut r, 5, vec![item("free", ItemAssign::State(ItemState::Backlog))]);
    assert_eq!(stored(&r), [json!("backlog")]);
    commit_ok(&mut r, 5, vec![Change::Cancel { item: "free".into(), force: true }]);
    let minted = Change::Add {
        record: Record::Item("twin".into()),
        minted: true,
        on_repeat: Repeat::Refuse,
        with: vec![Assign::Item("twin".into(), ItemAssign::Title("FREE".into()))],
    };
    assert!(judge(&read(&r), &ChangeSet(vec![minted]), by(6), None).is_ok());
}

#[test]
fn canceling_work_that_waits_in_a_thread_someone_else_holds_is_refused_unless_forced() {
    let mut r = replica("a");
    commit_ok(
        &mut r,
        1,
        vec![
            thread("t"),
            add("x", vec![ItemAssign::Thread(Some("t".into()))]),
            add(
                "y",
                vec![ItemAssign::Thread(Some("t".into())), ItemAssign::WaitsFor("x".into(), true)],
            ),
        ],
    );
    crate::claim::claim(&mut r, "t", "session/other", 2, (60_000, 60_000), false).unwrap();
    assert_eq!(read(&r).items["y"].state, Value::One(ItemState::Backlog));
    let held = Rule::CancelHeld { holder: "session/other".into() };
    assert_eq!(canceling(&r, "y", false), [held]);
    assert_eq!(canceling(&r, "y", true), []);
}

#[test]
fn evidence_must_be_able_to_verify_and_done_work_keeps_what_it_was_finished_with() {
    let mut r = replica("a");
    let repository = |id: &str, more: Vec<RepositoryAssign>| Change::Add {
        record: Record::Repository(id.into()),
        minted: false,
        on_repeat: Repeat::Update,
        with: std::iter::once(RepositoryAssign::Owner("o".into()))
            .chain(more)
            .map(|assign| Assign::Repository(id.into(), assign))
            .collect(),
    };
    commit_ok(&mut r, 1, vec![add("a", vec![]), repository("half", vec![])]);
    let evidence = |repository: &str, commit: &str| Evidence {
        repository: Some(repository.into()),
        commit: Some(commit.into()),
        text: None,
    };
    let forced = |r: &Replica, evidence: Evidence| finishing(r, "a", Some(evidence), true, false);
    let ghost = Rule::NoRecord(Record::Repository("ghost".into()));
    assert_eq!(forced(&r, evidence("ghost", "abc1234")), [ghost]);
    let missing = vec!["name", "default-branch"];
    assert_eq!(
        forced(&r, evidence("half", "abc1234")),
        [Rule::IncompleteRepository { repository: "half".into(), missing }]
    );
    // A repository the same change set completes is complete.
    let complete = repository(
        "half",
        vec![RepositoryAssign::Name("n".into()), RepositoryAssign::DefaultBranch("main".into())],
    );
    let finish_a =
        |evidence| Change::Finish { item: "a".into(), evidence: Some(evidence), force: true };
    let both = ChangeSet(vec![complete.clone(), finish_a(evidence("half", "abc1234"))]);
    assert!(judge(&read(&r), &both, by(2), None).is_ok());
    commit_ok(&mut r, 2, vec![complete]);
    for bad in ["123", "abc123", "abc123g", &"a".repeat(65), ""] {
        assert_eq!(forced(&r, evidence("half", bad)), [Rule::InvalidCommit], "{bad:?}");
    }
    for good in ["abc1234", "ABC1234", &"a".repeat(64)] {
        assert_eq!(forced(&r, evidence("half", good)), [], "{good}");
    }
    // Done work keeps the evidence it was finished with, unless forced; the same evidence again changes nothing.
    commit_ok(&mut r, 3, vec![finish_a(evidence("half", "abc1234"))]);
    let other = evidence("half", "def5678");
    assert_eq!(finishing(&r, "a", Some(other.clone()), false, true), [Rule::EvidenceDiffers]);
    let text = Evidence { text: Some("n".into()), ..Evidence::default() };
    assert_eq!(finishing(&r, "a", Some(text), false, true), [Rule::EvidenceDiffers]);
    assert_eq!(forced(&r, other), []);
    let again = ChangeSet(vec![finish_a(evidence("half", "abc1234"))]);
    assert!(!judge(&read(&r), &again, by(4), None).unwrap().changes());
}

#[hegel::test]
fn the_projection_is_what_reading_the_writes_gives(tc: TestCase) {
    let mut replicas = [replica("a"), replica("b")];
    let base = vec![
        thread("t"),
        Change::Add {
            record: Record::Repository("r".into()),
            minted: false,
            on_repeat: Repeat::Update,
            with: vec![
                Assign::Repository("r".into(), RepositoryAssign::Owner("me".into())),
                Assign::Repository("r".into(), RepositoryAssign::Name("code".into())),
                Assign::Repository("r".into(), RepositoryAssign::DefaultBranch("main".into())),
            ],
        },
        add("a", vec![]),
        add("b", vec![]),
        add("c", vec![]),
    ];
    commit_ok(&mut replicas[0], 1, base);
    let first = replicas[0].state.clone();
    replicas[1].join(&first).unwrap();
    let rounds = tc.draw(gs::integers::<u8>().max_value(12));
    for round in 0..rounds {
        let at = Millis::from(round) + 2;
        let which = tc.draw(gs::integers::<usize>().max_value(1));
        if tc.draw(gs::integers::<u8>().max_value(3)) == 0 {
            // The other replica changes a field this one changes too, unseen, so joining it makes a conflict.
            if tc.draw(gs::booleans()) {
                let id = ["a", "b", "c"][tc.draw(gs::integers::<usize>().max_value(2))];
                for (n, replica) in replicas.iter_mut().enumerate() {
                    let state = if n == which { ItemState::Parked } else { ItemState::Done };
                    let _ = judge(
                        &read(replica),
                        &ChangeSet(vec![item(id, ItemAssign::State(state))]),
                        by(at),
                        None,
                    )
                    .map(|outcome| write(replica, &outcome, by(at)).unwrap());
                }
            }
            let other = replicas[1 - which].state.clone();
            replicas[which].join(&other).unwrap();
        }
        let count = tc.draw(gs::integers::<usize>().max_value(5));
        let replica = &mut replicas[which];
        let before = read(replica);
        let mut changes = ChangeSet((0..count).map(|_| change(&tc)).collect());
        // Now and then take back a link that is there, which a random pick seldom names.
        if let Some(link) = before.links.first()
            && tc.draw(gs::booleans())
        {
            changes.0.push(item(&link.from, ItemAssign::WaitsFor(link.to.clone(), false)));
        }
        // And settle a conflict, which needs a write to a field two replicas wrote apart.
        if let Some((_, id, field)) = before.conflicts().next()
            && tc.draw(gs::booleans())
        {
            let settle = match field {
                "title" => ItemAssign::Title("settled".into()),
                "state" => ItemAssign::State(ItemState::Done),
                "notes" => ItemAssign::Notes(None),
                "thread" => ItemAssign::Thread(None),
                _ => ItemAssign::Parent("a".into()),
            };
            changes.0.push(item(id, settle));
        }
        if let Ok(outcome) = judge(&before, &changes, by(at), None) {
            write(replica, &outcome, by(at)).unwrap();
            assert_eq!(read(replica), outcome.after, "after {changes:?}");
        }
    }
}

#[test]
fn a_tasks_thread_of_one_takes_its_title_when_a_second_item_joins_and_frees_it_once_emptied() {
    let mut r = replica("a");
    commit_ok(&mut r, 1, vec![add("x", vec![])]);
    let one = crate::ids::one("x");
    crate::claim::claim(&mut r, &one, "session/s", 2, (60_000, 60_000), true).unwrap();
    assert_eq!(
        read(&r).threads[&one].title,
        Value::None,
        "a lone task's thread has no title of its own"
    );
    // Breaking the task down: the first child takes the thread, and the thread takes the task's title with it.
    commit_ok(&mut r, 3, vec![add("kid", vec![ItemAssign::Parent("x".into())])]);
    let model = read(&r);
    assert_eq!(model.items["kid"].thread, Value::One(one.clone()));
    assert_eq!(model.items["x"].thread, Value::None);
    assert_eq!(model.threads[&one].title, Value::One("X".into()));
    assert_eq!(model.shown_as_task(&one), None, "it is an ordinary thread now");
    let minted = |title: &str| Change::Add {
        record: Record::Thread("fresh".into()),
        minted: true,
        on_repeat: Repeat::Refuse,
        with: vec![Assign::Thread("fresh".into(), ThreadAssign::Title(title.into()))],
    };
    assert_eq!(refused(&r, vec![minted("X")]), [Rule::TitleTaken(Record::Thread(one.clone()))]);
    // Emptied, it is hidden and its title is free for a new thread, though the record keeps it.
    commit_ok(&mut r, 4, vec![item("kid", ItemAssign::Thread(None))]);
    let model = read(&r);
    assert!(model.is_emptied_one(&one));
    assert_eq!(model.threads[&one].title, Value::One("X".into()));
    assert!(judge(&model, &ChangeSet(vec![minted("X")]), by(5), None).is_ok());
}

fn initiative_add(id: &str) -> Change {
    Change::Add {
        record: Record::Initiative(id.to_owned()),
        minted: false,
        on_repeat: Repeat::Update,
        with: vec![Assign::Initiative(id.to_owned(), InitiativeAssign::Title(id.to_uppercase()))],
    }
}

#[test]
fn an_initiative_is_added_and_edited_like_a_record_and_an_item_may_be_put_under_it() {
    let mut r = replica("a");
    let outcome = commit_ok(
        &mut r,
        1,
        vec![
            initiative_add("init"),
            Change::Set(Assign::Initiative(
                "init".into(),
                InitiativeAssign::Notes(Some("n".into())),
            )),
            add("epic", vec![under("init")]),
        ],
    );
    // What the judge projects is what reading the written state gives.
    assert_eq!(read(&r), outcome.after);
    let m = read(&r);
    assert_eq!(m.initiatives["init"].title, Value::One("INIT".into()));
    assert_eq!(m.initiatives["init"].notes, Value::One("n".into()));
    assert_eq!(m.children("init"), ["epic"]);
    // Saying it again, or settling its notes to the same value, writes nothing; clearing them writes.
    let again = commit_ok(&mut r, 2, vec![initiative_add("init")]);
    assert!(!again.changes());
    commit_ok(
        &mut r,
        3,
        vec![Change::Set(Assign::Initiative("init".into(), InitiativeAssign::Notes(None)))],
    );
    assert_eq!(read(&r).initiatives["init"].notes, Value::None);
    // A parent that is neither an item nor an initiative is refused.
    assert_eq!(
        refused(&r, vec![add("lost", vec![under("nowhere")])]),
        [Rule::NoRecord(Record::Item("nowhere".into()))]
    );
    // Editing an initiative that doesn't exist is refused as any record is.
    assert_eq!(
        refused(
            &r,
            vec![Change::Set(Assign::Initiative(
                "nowhere".into(),
                InitiativeAssign::Title("T".into())
            ))]
        ),
        [Rule::NoRecord(Record::Initiative("nowhere".into()))]
    );
}

#[test]
fn no_id_names_both_an_initiative_and_an_item_live_or_deleted() {
    let mut r = replica("a");
    commit_ok(&mut r, 1, vec![initiative_add("init"), add("work", vec![under("init")])]);
    // An item can't take an initiative's id, nor an initiative an item's.
    assert_eq!(
        refused(&r, vec![add("init", vec![])]),
        [Rule::IdTaken(Record::Initiative("init".into()))]
    );
    assert_eq!(
        refused(&r, vec![initiative_add("work")]),
        [Rule::IdTaken(Record::Item("work".into()))]
    );
    // Nor one that was deleted, since another replica may still name it.
    let at = By { actor: "actor/user", at: 2 };
    r.write(
        &Key::new(kind::INITIATIVE, "ended").unwrap(),
        "deleted",
        "actor/user",
        at.at,
        Some(json!(true)),
    )
    .unwrap();
    assert_eq!(
        refused(&r, vec![add("ended", vec![under("init")])]),
        [Rule::IdTaken(Record::Initiative("ended".into()))]
    );
    // A kind's own id is no clash: saying an existing record again is an update.
    assert!(
        !commit_ok(&mut r, 3, vec![initiative_add("init"), add("work", vec![under("init")])])
            .changes()
    );
}

#[test]
fn work_directly_under_an_initiative_takes_no_thread_from_its_neighbours() {
    let mut r = replica("a");
    commit_ok(&mut r, 1, vec![initiative_add("init"), thread("t")]);
    commit_ok(
        &mut r,
        2,
        vec![add("one", vec![under("init"), ItemAssign::Thread(Some("t".into()))])],
    );
    // A sibling in thread `t` places no new child of an epic or task directly under the initiative.
    let outcome = commit_ok(
        &mut r,
        3,
        vec![add("two", vec![under("init")]), add("three", vec![under("init")])],
    );
    assert_eq!(outcome.placed(), Vec::new());
    assert_eq!((thread_of(&r, "two"), thread_of(&r, "three")), (None, None));
    // Under an item the rule stands: siblings place.
    commit_ok(&mut r, 4, vec![add("kid", vec![under("one")])]);
    commit_ok(&mut r, 5, vec![add("kid2", vec![under("one")])]);
    assert_eq!(thread_of(&r, "kid2"), thread_of(&r, "kid"));
}

/// An item with this title under `parent`, or under none.
fn titled(id: &str, title: &str, parent: Option<&str>) -> Change {
    let mut with = vec![Assign::Item(id.to_owned(), ItemAssign::Title(title.to_owned()))];
    if let Some(parent) = parent {
        with.push(Assign::Item(id.to_owned(), ItemAssign::Parent(parent.to_owned())));
    }
    Change::Add {
        record: Record::Item(id.to_owned()),
        minted: false,
        on_repeat: Repeat::Update,
        with,
    }
}

/// Two initiatives and a few titles in them: `s1` and `s2` are both "Sow", one in each; `f1` is done and `r1` open, both
/// "Reap"; `c1` is canceled and `p1` open, both "Plow"; `done-epic` is a caught-up parent of one done leaf; `e2` is an
/// epic in `i2` over `k2`, which is "Reap" too.
fn titled_world() -> Replica {
    let mut r = replica("a");
    for id in ["i1", "i2"] {
        crate::model::add_initiative(&mut r, id, id, by(1)).unwrap();
    }
    commit_ok(
        &mut r,
        2,
        vec![
            titled("s1", "Sow", Some("i1")),
            titled("s2", "Sow", Some("i2")),
            titled("f1", "Reap", Some("i1")),
            titled("c1", "Plow", Some("i1")),
            titled("w2", "Water", Some("i2")),
            titled("done-epic", "Roadmap", Some("i1")),
            titled("done-kid", "Stage", Some("done-epic")),
            titled("e2", "Epic", Some("i2")),
            titled("k2", "Reap", Some("e2")),
        ],
    );
    commit_ok(
        &mut r,
        3,
        vec![
            item("f1", ItemAssign::State(ItemState::Done)),
            item("c1", ItemAssign::State(ItemState::Canceled)),
            item("done-kid", ItemAssign::State(ItemState::Done)),
        ],
    );
    // A title a finished leaf held is free to take, and so twins appear that reopening would make.
    commit_ok(&mut r, 4, vec![titled("r1", "Reap", Some("i1")), titled("p1", "Plow", Some("i1"))]);
    r
}

#[test]
fn a_title_is_unique_among_unfinished_items_in_an_initiative_on_every_way_to_take_one() {
    let r = titled_world();
    let taken = |other: &str| vec![Rule::TitleTaken(Record::Item(other.into()))];
    let minted = |id: &str, title: &str, parent: &str| match titled(id, title, Some(parent)) {
        Change::Add { record, on_repeat, with, .. } => {
            Change::Add { record, minted: true, on_repeat, with }
        },
        other => other,
    };
    let state = |id: &str, to: ItemState| item(id, ItemAssign::State(to));
    let table: Vec<(&str, Change, Vec<Rule>)> = vec![
        ("add, given id", titled("n", "Sow", Some("i1")), taken("s1")),
        ("add, minted id", minted("n", "Sow", "i1"), taken("s1")),
        ("add under an epic of the initiative", titled("n", "Sow", Some("e2")), taken("s2")),
        ("add, same title in another initiative", titled("n", "Water", Some("i1")), vec![]),
        ("add, a finished leaf's title", titled("n", "Reap", Some("i1")), taken("r1")),
        ("add, a caught-up epic's title", titled("n", "Roadmap", Some("i1")), taken("done-epic")),
        ("add, a free title", titled("n", "Weed", Some("i1")), vec![]),
        ("rename onto a twin", item("w2", ItemAssign::Title("Sow".into())), taken("s2")),
        ("rename to a free title", item("s1", ItemAssign::Title("Water".into())), vec![]),
        ("move onto a twin", item("s2", ItemAssign::Parent("i1".into())), taken("s1")),
        ("move without a twin", item("w2", ItemAssign::Parent("i1".into())), vec![]),
        (
            "move an epic whose child has a twin",
            item("e2", ItemAssign::Parent("i1".into())),
            taken("r1"),
        ),
        (
            "move under an item of the initiative",
            item("s2", ItemAssign::Parent("done-epic".into())),
            taken("s1"),
        ),
        ("reopen a finished twin", state("f1", ItemState::Backlog), taken("r1")),
        ("un-cancel a twin", state("c1", ItemState::Backlog), taken("p1")),
        ("park a twin", state("c1", ItemState::Parked), taken("p1")),
        ("reopen a finished leaf with no twin", state("done-kid", ItemState::Backlog), vec![]),
    ];
    for (name, change, expected) in table {
        assert_eq!(refused_or_none(&r, change), expected, "{name}");
    }
}

#[test]
fn only_a_leaf_frees_its_title_by_finishing_so_a_caught_up_epic_keeps_its_name() {
    let mut r = titled_world();
    let model = read(&r);
    assert!(model.items["done-kid"].is_done() && !model.holds_title("done-kid"));
    assert!(
        model.holds_title("done-epic"),
        "its leaves are done, so it reads done, and it keeps its name"
    );
    assert_eq!(
        refused(&r, vec![titled("n", "Roadmap", Some("i1"))]),
        [Rule::TitleTaken(Record::Item("done-epic".into()))]
    );
    // A finished leaf that is given a child is a parent again, and takes its name back from the twin that has it.
    commit_ok(&mut r, 6, vec![titled("again", "Stage", Some("i1"))]);
    assert_eq!(
        refused(&r, vec![titled("late", "Late", Some("done-kid"))]),
        [Rule::TitleTaken(Record::Item("again".into()))]
    );
}

#[test]
fn items_in_no_initiative_are_one_scope() {
    let mut r = titled_world();
    commit_ok(&mut r, 5, vec![titled("n1", "Loose", None)]);
    commit_ok(&mut r, 5, vec![titled("n2", "Other", None)]);
    assert_eq!(
        refused(&r, vec![titled("n3", "Loose", None)]),
        [Rule::TitleTaken(Record::Item("n1".into()))]
    );
    assert_eq!(
        refused(&r, vec![item("n2", ItemAssign::Title("Loose".into()))]),
        [Rule::TitleTaken(Record::Item("n1".into()))]
    );
    // An initiative's own items are another scope.
    assert_eq!(refused_or_none(&r, titled("n4", "Loose", Some("i1"))), vec![]);
}

#[test]
fn a_clash_the_tracker_already_has_is_not_refused_again_by_an_edit_that_leaves_it_be() {
    let mut a = titled_world();
    let mut b = replica("b");
    b.join(&a.state).unwrap();
    commit_ok(&mut a, 5, vec![titled("twin-a", "Fresh", Some("i1"))]);
    commit_ok(&mut b, 5, vec![titled("twin-b", "Fresh", Some("i1"))]);
    a.join(&b.state).unwrap();
    assert_eq!(read(&a).duplicate_titles(), [vec!["twin-a", "twin-b"]]);
    // Moving the epic that holds one of them within its scope, or parking it, makes nothing worse.
    commit_ok(&mut a, 6, vec![item("twin-a", ItemAssign::Parent("i1".into()))]);
    commit_ok(&mut a, 7, vec![item("twin-b", ItemAssign::State(ItemState::Parked))]);
    // Renaming one ends it.
    commit_ok(&mut a, 8, vec![item("twin-b", ItemAssign::Title("Other".into()))]);
    assert_eq!(read(&a).duplicate_titles(), Vec::<Vec<&str>>::new());
}
