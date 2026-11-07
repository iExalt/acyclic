use crate::claim;
use crate::model::{By, ItemState, add_item, item_key, link, set_parent, set_state};
use crate::needs::{Entry, needs_you};
use crate::ready::ReadinessEvaluator;
use crate::schema::kind;
use crate::scope::USER;
use crate::scope::{ScopeEdit, edit};
use crate::state::Key;
use crate::state::{Millis, ReplicaId, State};

use super::*;

const NOW: Millis = 10_000;

fn user(at: Millis) -> By<'static> {
    By { actor: USER, at }
}

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

fn with_items(ids: &[&str]) -> Replica {
    let mut r = replica("mac");
    for id in ids {
        add_item(&mut r, id, id, user(1)).unwrap();
    }
    r
}

fn refusal(r: &mut Replica, id: &str, by: By<'_>) -> Vec<String> {
    match delete_item(r, id, by) {
        Err(WriteError::InUse { reasons, .. }) => reasons
            .into_iter()
            .map(|reason| serde_json::to_value(reason).unwrap()["why"].as_str().unwrap().to_owned())
            .collect(),
        other => vec![format!("{other:?}")],
    }
}

#[test]
fn only_an_item_nothing_refers_to_is_deleted() {
    let mut r = with_items(&["parent", "kid", "a", "b", "done", "claimed", "mine", "free"]);
    set_parent(&mut r, "kid", Some("parent"), user(2)).unwrap();
    link(&mut r, "a", "b", user(2)).unwrap();
    set_state(&mut r, "done", ItemState::Done, user(2)).unwrap();
    let held = Key::new(kind::THREAD, "held").unwrap();
    crate::model::set(&mut r, &held, crate::schema::thread::TITLE, Some(&"Held"), user(2)).unwrap();
    for id in ["claimed", "mine"] {
        crate::model::put(
            &mut r,
            &item_key(id).unwrap(),
            crate::schema::item::THREAD,
            Some(&"held"),
            user(2),
        )
        .unwrap();
    }
    claim::claim(&mut r, "held", "session/pi", NOW, (60_000, 480 * 60_000), false).unwrap();
    assert_eq!(refusal(&mut r, "parent", user(NOW)), ["has-children"]);
    // A link either way, and a link of any kind.
    assert_eq!(refusal(&mut r, "a", user(NOW)), ["linked"]);
    assert_eq!(refusal(&mut r, "b", user(NOW)), ["linked"]);
    assert_eq!(refusal(&mut r, "done", user(NOW)), ["finished"]);
    // Someone else's claim on its thread holds it; the deleter's own doesn't.
    assert_eq!(refusal(&mut r, "claimed", user(NOW)), ["claimed"]);
    assert_eq!(delete_item(&mut r, "mine", By { actor: "session/pi", at: NOW }), Ok(true));
    // The refusal says why in one line.
    let message = delete_item(&mut r, "parent", user(NOW)).unwrap_err().to_string();
    assert_eq!(message, "parent can't be deleted: it has children (kid)");
    let message = delete_item(&mut r, "a", user(NOW)).unwrap_err().to_string();
    assert_eq!(message, "a can't be deleted: a waits-for b");
    // The details name a link's kind as the command line does.
    let Err(WriteError::InUse { reasons, .. }) = delete_item(&mut r, "a", user(NOW)) else {
        panic!("in use")
    };
    assert_eq!(serde_json::to_value(&reasons).unwrap()[0]["kind"], "waits-for");
    assert!(matches!(delete_item(&mut r, "nope", user(NOW)), Err(WriteError::NoItem(_))));
    // A claim that has lapsed no longer holds it.
    assert_eq!(delete_item(&mut r, "claimed", user(NOW + 61_000)), Ok(true));
    assert_eq!(delete_item(&mut r, "free", user(NOW)), Ok(true));
    let before = r.changes();
    assert_eq!(delete_item(&mut r, "free", user(NOW)), Ok(false), "a repeat changes nothing");
    assert_eq!(r.changes(), before);
}

#[test]
fn an_agent_deletes_a_threads_member() {
    let mut r = with_items(&["a"]);
    edit(&mut r, &ScopeEdit::Member { item: "a".into(), thread: Some("t".into()) }, user(2))
        .unwrap();
    assert_eq!(delete_item(&mut r, "a", By { actor: "session/x", at: NOW }), Ok(true));
}

#[test]
fn a_deleted_item_is_missing_to_every_checked_write_and_its_id_is_retired() {
    let mut r = with_items(&["gone", "other"]);
    delete_item(&mut r, "gone", user(2)).unwrap();
    let key = item_key("gone").unwrap();
    assert!(r.state().entity(&key).is_some(), "the tombstone is state, so it joins");
    assert!(!Model::read(r.state()).unwrap().items.contains_key("gone"));
    let missing = |result: Result<(), WriteError>| matches!(result, Err(WriteError::NoItem(_)));
    assert!(missing(link(&mut r, "gone", "other", user(3)).map(|_| ())));
    assert!(missing(link(&mut r, "other", "gone", user(3)).map(|_| ())));
    assert!(missing(set_parent(&mut r, "gone", Some("other"), user(3))));
    assert!(missing(set_parent(&mut r, "other", Some("gone"), user(3))));
    assert!(missing(set_state(&mut r, "gone", ItemState::Parked, user(3))));
    let member = ScopeEdit::Member { item: "gone".into(), thread: Some("t".into()) };
    assert!(matches!(edit(&mut r, &member, user(3)), Err(WriteError::NoItem(_))));
    assert!(matches!(add_item(&mut r, "gone", "Again", user(3)), Err(WriteError::Deleted(_))));
}

/// What a reader sees of `r`: its needs, with every other reader run on it to show none of them fails.
fn needs_of(r: &Replica) -> Vec<Entry> {
    let model = Model::read(r.state()).unwrap();
    let readiness = ReadinessEvaluator::new(r.state(), &model, NOW);
    let _ = readiness.ready("session/mac").unwrap();
    for id in model.items.keys() {
        let _ = readiness.of(id).unwrap();
        let _ = (model.role(id), model.ancestors(id));
    }
    let (lenient, unreadable) = Model::read_lenient(r.state());
    assert_eq!((lenient.items.len(), unreadable.len()), (model.items.len(), 0));
    needs_you(r.state(), &model, &readiness, NOW).unwrap()
}

#[test]
fn delete_wins_a_join_and_what_was_attached_meanwhile_reads_as_dangling() {
    let mut mac = with_items(&["gone", "other"]);
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    // The two replicas act without seeing each other: mac deletes the item while pi puts a child under it and links
    // another item to it.
    assert_eq!(delete_item(&mut mac, "gone", user(2)), Ok(true));
    add_item(&mut pi, "kid", "Kid", user(2)).unwrap();
    set_parent(&mut pi, "kid", Some("gone"), user(2)).unwrap();
    link(&mut pi, "other", "gone", user(2)).unwrap();
    let mut one = Replica::open(ReplicaId::new("a").unwrap(), mac.state().clone());
    one.join(pi.state()).unwrap();
    let mut other = Replica::open(ReplicaId::new("b").unwrap(), pi.state().clone());
    other.join(mac.state()).unwrap();
    for joined in [&one, &other] {
        // Gone from every reader...
        let model = Model::read(joined.state()).unwrap();
        let ids: Vec<&str> = model.items.keys().map(String::as_str).collect();
        assert_eq!(ids, ["kid", "other"]);
        // ...and what refers to it surfaces for the user.
        let mut dangling: Vec<(String, String, Option<String>)> = needs_of(joined)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Dangling { id, field, target, fix, .. } if target == "gone" => {
                    Some((id, field, fix))
                },
                Entry::ParentMissing { item, parent: Some(target) } if target == "gone" => {
                    Some((item, "parent".to_owned(), None))
                },
                _ => None,
            })
            .collect();
        dangling.sort();
        let fix = "acyclic item edit other --no-waits-for gone".to_owned();
        assert_eq!(
            dangling,
            [
                ("kid".to_owned(), "parent".to_owned(), None),
                ("other".to_owned(), "waits-for".to_owned(), Some(fix)),
            ],
            "the child's parent, and what the other item waits for with the fix that lets go of it"
        );
    }
    // The same state in either order.
    assert_eq!(one.state().to_files(), other.state().to_files());
    assert_eq!(needs_of(&one), needs_of(&other));
}

#[test]
fn a_thread_is_deleted_only_with_no_members_and_no_live_claim() {
    let mut r = with_items(&["a"]);
    for id in ["t", "u"] {
        let key = Key::new(kind::THREAD, id).unwrap();
        crate::model::set(&mut r, &key, crate::schema::thread::TITLE, Some(&id), user(2)).unwrap();
    }
    edit(&mut r, &ScopeEdit::Member { item: "a".into(), thread: Some("t".into()) }, user(2))
        .unwrap();
    claim::claim(&mut r, "u", USER, NOW, (60_000, 480 * 60_000), false).unwrap();
    let why = |r: &mut Replica, id: &str| match delete_thread(r, id, user(NOW)) {
        Err(WriteError::ThreadInUse { reasons, .. }) => reasons
            .into_iter()
            .map(|reason| serde_json::to_value(reason).unwrap()["why"].as_str().unwrap().to_owned())
            .collect(),
        other => vec![format!("{other:?}")],
    };
    assert_eq!(why(&mut r, "t"), ["has-members"]);
    // Even the deleter's own claim keeps it.
    assert_eq!(why(&mut r, "u"), ["claimed"]);
    claim::release(&mut r, &claim::thread_key("u").unwrap(), USER);
    assert_eq!(delete_thread(&mut r, "u", user(NOW)), Ok(true));
    assert!(!Model::read(&r.state).unwrap().threads.contains_key("u"));
    assert_eq!(delete_thread(&mut r, "u", user(NOW)), Ok(false), "deleting again writes nothing");
    assert_eq!(delete_thread(&mut r, "v", user(NOW)), Err(WriteError::NoThread("v".into())));
}

#[test]
fn a_tasks_thread_of_one_is_never_deleted_but_deleting_the_task_lets_its_own_claim_go() {
    let mut r = with_items(&["x", "y"]);
    let one = crate::ids::one("x");
    claim::claim(&mut r, &one, "actor/user", 5, (10_000, 10_000), true).unwrap();
    // Not while the task is in it, nor once it is empty: a tombstone would make the task unclaimable.
    let refused =
        Err(WriteError::ThreadInUse { thread: one.clone(), reasons: vec![Reason::TaskThread] });
    assert_eq!(delete_thread(&mut r, &one, user(6)), refused);
    assert!(delete_item(&mut r, "x", user(6)).unwrap());
    assert_eq!(delete_thread(&mut r, &one, user(6)), refused);
    // Deleting the task releases the actor's own claim in the same write, so nothing holds an emptied thread.
    let model = Model::read(&r.state).unwrap();
    assert!(!model.items.contains_key("x"));
    assert_eq!(claim::live(&r.state, &claim::thread_key(&one).unwrap(), 7).unwrap().len(), 0);
    assert!(model.is_emptied_one(&one));
    let readiness = ReadinessEvaluator::new(&r.state, &model, 7);
    assert!(
        needs_you(&r.state, &model, &readiness, 7)
            .unwrap()
            .iter()
            .all(|entry| matches!(entry, Entry::ParentMissing { .. })),
        "emptied and unheld is hidden"
    );

    // Someone else's claim still refuses the delete, as on any thread.
    let one_y = crate::ids::one("y");
    claim::claim(&mut r, &one_y, "session/other", 8, (10_000, 10_000), true).unwrap();
    assert_eq!(refusal(&mut r, "y", user(9)), ["claimed"]);
}

/// An initiative `id` with a title, as the judge writes it.
fn with_initiative(r: &mut Replica, id: &str, at: Millis) {
    crate::model::add_initiative(r, id, id, user(at)).unwrap();
}

#[test]
fn an_initiative_is_deleted_only_while_no_item_is_under_it_and_its_tombstone_is_permanent() {
    let mut r = with_items(&["a"]);
    with_initiative(&mut r, "init", 1);
    set_parent(&mut r, "a", Some("init"), user(2)).unwrap();
    let refused = delete_initiative(&mut r, "init", user(3));
    assert_eq!(
        refused,
        Err(WriteError::InitiativeInUse {
            initiative: "init".into(),
            reasons: vec![Reason::HoldsItems { items: vec!["a".into()] }],
        })
    );
    assert_eq!(refused.unwrap_err().to_string(), "init can't be deleted: items are under it (a)");
    // Moving the item out frees it; a second delete changes nothing.
    with_initiative(&mut r, "other", 4);
    set_parent(&mut r, "a", Some("other"), user(4)).unwrap();
    assert_eq!(delete_initiative(&mut r, "init", user(5)), Ok(true));
    assert_eq!(delete_initiative(&mut r, "init", user(5)), Ok(false));
    assert_eq!(
        delete_initiative(&mut r, "nowhere", user(5)),
        Err(WriteError::NoInitiative("nowhere".into()))
    );
    let model = Model::read(&r.state).unwrap();
    assert!(!model.initiatives.contains_key("init") && model.initiatives.contains_key("other"));
    // The id is retired for good: not for another initiative, and not for an item either.
    assert!(matches!(
        crate::model::add_initiative(&mut r, "init", "Again", user(6)),
        Err(WriteError::Deleted(_))
    ));
    let at = user(6);
    let taken = crate::write::commit(&mut r, at, false, |_| {
        Ok::<_, ()>(crate::write::ChangeSet(vec![crate::write::Change::Add {
            record: crate::write::Record::Item("init".into()),
            minted: false,
            on_repeat: crate::write::Repeat::Refuse,
            with: vec![crate::write::Assign::Item(
                "init".into(),
                crate::write::ItemAssign::Title("T".into()),
            )],
        }]))
    });
    assert!(
        matches!(taken, Err(crate::write::CommitError::Refused(ref v, _))
            if v.iter().any(|v| matches!(&v.rule, crate::write::Rule::IdTaken(_)))),
        "{taken:?}"
    );
}

#[test]
fn an_item_added_under_an_initiative_deleted_apart_reads_as_a_missing_parent_in_either_join_order()
{
    let mut mac = replica("mac");
    with_initiative(&mut mac, "init", 1);
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    // Apart: mac deletes the empty initiative while pi adds an item under it.
    assert_eq!(delete_initiative(&mut mac, "init", user(2)), Ok(true));
    add_item(&mut pi, "kid", "Kid", user(2)).unwrap();
    set_parent(&mut pi, "kid", Some("init"), user(2)).unwrap();
    let mut one = Replica::open(ReplicaId::new("a").unwrap(), mac.state().clone());
    one.join(pi.state()).unwrap();
    let mut other = Replica::open(ReplicaId::new("b").unwrap(), pi.state().clone());
    other.join(mac.state()).unwrap();
    for joined in [&one, &other] {
        let model = Model::read(joined.state()).unwrap();
        // Delete wins: the initiative is gone from every reader, and the item reads as in no initiative.
        assert!(model.initiatives.is_empty());
        assert_eq!(model.initiative_of("kid"), None);
        assert_eq!(
            needs_of(joined),
            [Entry::ParentMissing { item: "kid".into(), parent: Some("init".into()) }]
        );
    }
    assert_eq!(one.state().to_files(), other.state().to_files());
}
