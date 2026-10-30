use hegel::TestCase;
use hegel::generators as gs;
use serde_json::json;

use super::*;
use crate::delete::delete_item;
use crate::model::{By, add_item};
use crate::scope::USER;
use crate::state::{Key, Replica};

fn user() -> By<'static> {
    By { actor: USER, at: 1, seen: None }
}

fn replica(name: &str) -> Replica {
    Replica::open(ReplicaId::new(name).unwrap(), State::new())
}

/// A replica holding these items, and these threads by their titles.
fn holding(items: &[&str], threads: &[&str]) -> Replica {
    let mut r = replica("mac");
    for id in items {
        add_item(&mut r, id, id, user()).unwrap();
    }
    for id in threads {
        let key = Key::new(kind::THREAD, id).unwrap();
        r.write(&key, "title", USER, 1, Some(json!(id))).unwrap();
    }
    r
}

#[test]
fn numbers_end_the_trackers_own_ids_only() {
    let cases = [
        ("thread-27", Some(27)),
        ("thread-pi-40", Some(40)),
        ("thread-mac-3fa9c1-32", Some(32)),
        ("thread-", None),
        ("thread-mac", None),
        ("thread-mac-", None),
        ("thread-x_1", None),
        ("threads-1", None),
        ("side-5", None),
        ("thread-99999999999999999999999", None),
    ];
    for (id, expected) in cases {
        assert_eq!(number("thread", id), expected, "{id}");
    }
    assert!(reserved("item", "item-x") && reserved("thread", "thread-27"));
    assert!(
        !reserved("item", "items-x")
            && !reserved("item", "item")
            && !reserved("item", "spike-item-1")
    );
}

#[test]
fn the_next_id_follows_the_highest_number_of_its_kind_wherever_it_came_from() {
    let mac = ReplicaId::new("mac").unwrap();
    assert_eq!(next(&State::new(), "item", &mac), "item-mac-1");
    // Older ids, other replicas' and deleted ones count; slugs and other kinds don't.
    let mut r =
        holding(&["side-5", "spike-2026", "item-pi-7", "item-mac-9", "thread-80"], &["thread-27"]);
    assert_eq!(next(r.state(), "item", &mac), "item-mac-10");
    delete_item(&mut r, "item-mac-9", user()).unwrap();
    assert_eq!(next(r.state(), "item", &mac), "item-mac-10", "a deleted id is never reused");
    assert_eq!(next(r.state(), "thread", &mac), "thread-mac-28");
    // A thread only an item's membership names counts too.
    let member = Key::new(kind::ITEM, "side-5").unwrap();
    r.write(&member, item::THREAD, USER, 1, Some(json!("thread-mac-3fa9c1-40"))).unwrap();
    assert_eq!(next(r.state(), "thread", &mac), "thread-mac-41");
}

/// An assigned id is never one the state already names, whatever ids it holds.
#[hegel::test]
fn an_assigned_id_is_new(tc: TestCase) {
    let ids: Vec<String> = tc.draw(
        gs::vecs(gs::from_regex("item-(mac-|pi-)?[0-9]{1,3}|[a-z]{1,3}-[0-9]{1,2}")).max_size(6),
    );
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let r = holding(&ids, &[]);
    let assigned = next(r.state(), "item", &ReplicaId::new("mac").unwrap());
    assert!(!ids.contains(&assigned.as_str()), "{assigned} in {ids:?}");
}
