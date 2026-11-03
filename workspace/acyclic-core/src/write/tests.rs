use hegel::TestCase;
use hegel::generators as gs;

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
    commit(replica, by(at), |_| Ok::<_, ()>(ChangeSet(changes))).unwrap()
}

/// The rules a refused change set breaks.
fn refused(replica: &Replica, changes: Vec<Change>) -> Vec<Rule> {
    let violations = judge(&read(replica), &ChangeSet(changes), by(99)).unwrap_err();
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
            add("mid", vec![ItemAssign::Parent(Some("root".into()))]),
            add(
                "leaf",
                vec![ItemAssign::Parent(Some("mid".into())), ItemAssign::Thread(Some("t".into()))],
            ),
        ],
    );
    let done = ItemAssign::State(ItemState::Done);
    commit_ok(
        &mut r,
        2,
        vec![item("root", done.clone()), item("mid", done.clone()), item("leaf", done)],
    );
    r
}

#[test]
fn a_change_that_holds_already_writes_nothing_but_a_conflict_takes_it() {
    let mut a = done_tree();
    let again =
        judge(&read(&a), &ChangeSet(vec![item("leaf", ItemAssign::Title("LEAF".into()))]), by(3));
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
fn done_work_set_back_is_reopened_and_leaves_the_archive_keeping_its_evidence() {
    let mut r = done_tree();
    let evidence = Evidence { text: Some("shipped".into()), ..Evidence::default() };
    commit_ok(
        &mut r,
        3,
        vec![item("leaf", ItemAssign::Evidence(Some(evidence))), item("root", ItemAssign::Archive)],
    );
    let outcome = commit_ok(&mut r, 4, vec![item("leaf", ItemAssign::State(ItemState::Backlog))]);
    let m = read(&r);
    assert_eq!(m.items["leaf"].reopened, Value::One(Reopened { archived: true }));
    assert_eq!(m.items["root"].archived, Value::None);
    assert!(
        outcome
            .steps
            .iter()
            .any(|step| step.write == Written::Unarchive("root".into()) && !step.asked)
    );
    // Finished again, the flag clears.
    commit_ok(&mut r, 5, vec![item("leaf", ItemAssign::State(ItemState::Done))]);
    assert_eq!(read(&r).items["leaf"].reopened, Value::None);
}

#[test]
fn archiving_a_root_while_reopening_under_it_is_refused_rather_than_undone() {
    let r = done_tree();
    let rules = refused(
        &r,
        vec![
            item("root", ItemAssign::Archive),
            item("leaf", ItemAssign::State(ItemState::Backlog)),
        ],
    );
    assert!(rules.contains(&Rule::ArchivedOpen("root".into())), "{rules:?}");
}

#[test]
fn settling_a_state_conflict_that_held_done_is_no_reopen() {
    // One replica finishes the leaf while another sets it back to the backlog without seeing that.
    let mut a = done_tree();
    commit_ok(&mut a, 3, vec![item("leaf", ItemAssign::State(ItemState::Parked))]);
    let mut c = replica("c");
    c.join(&a.state).unwrap();
    commit_ok(&mut a, 4, vec![item("leaf", ItemAssign::State(ItemState::Done))]);
    commit_ok(&mut c, 4, vec![item("leaf", ItemAssign::State(ItemState::Backlog))]);
    a.join(&c.state).unwrap();
    assert!(read(&a).items["leaf"].state.is_conflict());
    commit_ok(&mut a, 5, vec![item("leaf", ItemAssign::State(ItemState::Backlog))]);
    assert_eq!(read(&a).items["leaf"].reopened, Value::None);
}

#[test]
fn new_or_moved_work_joins_the_thread_passed_down_unless_given_one() {
    let mut r = done_tree();
    // The leaf open again, its thread is the one `mid`'s open children share.
    commit_ok(&mut r, 3, vec![item("leaf", ItemAssign::State(ItemState::Backlog))]);
    let outcome = commit_ok(
        &mut r,
        4,
        vec![
            add("x", vec![ItemAssign::Parent(Some("mid".into()))]),
            add("y", vec![]),
            item("y", ItemAssign::Parent(Some("mid".into()))),
            add("z", vec![ItemAssign::Parent(Some("mid".into())), ItemAssign::Thread(None)]),
        ],
    );
    let placed: Vec<(String, String)> =
        outcome.placed().into_iter().map(|placed| (placed.item, placed.thread)).collect();
    assert_eq!(placed, [("x".into(), "t".into()), ("y".into(), "t".into())]);
    // An item given a thread, even none, keeps what it was given.
    assert_eq!(read(&r).items["z"].thread, Value::None);
    assert_eq!(read(&r).items["x"].thread, Value::One("t".into()));
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
    assert_eq!(read(&a).link_cycles().len(), 1);
    // An unrelated write still goes through; a new cycle doesn't.
    commit_ok(&mut a, 5, vec![item("p", ItemAssign::Title("P2".into()))]);
    commit_ok(&mut a, 6, vec![add("s", vec![ItemAssign::WaitsFor("p".into(), true)])]);
    let rules = refused(&a, vec![item("p", ItemAssign::WaitsFor("s".into(), true))]);
    assert!(matches!(rules.as_slice(), [Rule::LinkCycle(_)]), "{rules:?}");
}

#[test]
fn every_violation_is_named_at_once() {
    let mut r = done_tree();
    commit_ok(&mut r, 3, vec![add("gone", vec![])]);
    crate::delete::delete_item(&mut r, "gone", by(4)).unwrap();
    let rules = refused(
        &r,
        vec![
            item("leaf", ItemAssign::Parent(Some("nowhere".into()))),
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
    assert!(!judge(&read(&r), &ChangeSet(vec![again("OPEN")]), by(4)).unwrap().changes());
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
    assert!(judge(&read(&a), &ChangeSet(vec![minted("twin", "LEAF")]), by(4)).is_ok());
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
    let assign = match tc.draw(gs::integers::<u8>().max_value(9)) {
        0 => ItemAssign::Title(pick(tc)),
        1 => ItemAssign::Notes(maybe(tc)),
        2 => ItemAssign::State(
            [ItemState::Backlog, ItemState::Parked, ItemState::Done]
                [tc.draw(gs::integers::<usize>().max_value(2))],
        ),
        3 => ItemAssign::Parent(maybe(tc)),
        4 => ItemAssign::Size(tc.draw(gs::booleans()).then_some(Size::M)),
        5 => ItemAssign::Thread(tc.draw(gs::booleans()).then(|| "t".to_owned())),
        6 => ItemAssign::Repository("r".into(), tc.draw(gs::booleans())),
        7 => ItemAssign::WaitsFor(pick(tc), tc.draw(gs::booleans())),
        8 => ItemAssign::Archive,
        _ => return add(&id, vec![]),
    };
    Change::Set(Assign::Item(id, assign))
}

/// What the judge projects is what reading the written state gives, across replicas and conflicts.
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
        eprintln!("PROBE conflicts {}", before.conflicts().count());
        // And settle a conflict, which needs a write to a field two replicas wrote apart.
        if let Some((_, id, field)) = before.conflicts().next()
            && tc.draw(gs::booleans())
        {
            let settle = match field {
                "title" => ItemAssign::Title("settled".into()),
                "state" => ItemAssign::State(ItemState::Done),
                "notes" => ItemAssign::Notes(None),
                "size" => ItemAssign::Size(None),
                "thread" => ItemAssign::Thread(None),
                _ => ItemAssign::Parent(None),
            };
            changes.0.push(item(id, settle));
        }
        if let Ok(outcome) = judge(&before, &changes, by(at)) {
            write(replica, &outcome, by(at)).unwrap();
            assert_eq!(read(replica), outcome.after, "after {changes:?}");
        }
    }
}
