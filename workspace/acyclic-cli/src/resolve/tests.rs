use acyclic_core::delete::delete_item;
use acyclic_core::model::{self, By, ItemState, add_item, set_state};
use acyclic_core::schema::{kind, repository};
use acyclic_core::scope::USER;
use acyclic_core::state::{Key, Replica, ReplicaId, State};

use super::*;

const BY: By<'static> = By { actor: USER, at: 1 };

/// A fixed `UUIDv7` ending in `tail`, a hex string.
fn id(tail: &str) -> String {
    format!("01900000-0000-7000-8000-{tail:0>12}")
}

/// Items: "Plant" twice (one done), "Water", "Weed", whose id ends in Water's id's last 6 characters, and a deleted
/// "Gone"; a repository iExalt/acyclic and one other/acyclic.
fn tracker() -> Replica {
    let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    for (tail, title) in [
        ("a1", "Plant"),
        ("a2", "Plant"),
        ("abcdef", "Water"),
        ("12abcdef", "Weed"),
        ("c0ffee", "Gone"),
        ("b1", "c0ffee"),
    ] {
        add_item(&mut r, &id(tail), title, BY).unwrap();
    }
    set_state(&mut r, &id("a2"), ItemState::Done, BY).unwrap();
    delete_item(&mut r, &id("c0ffee"), BY).unwrap();
    for (tail, owner) in [("e1", "iExalt"), ("e2", "other")] {
        let key = Key::new(kind::REPOSITORY, &id(tail)).unwrap();
        model::set(&mut r, &key, repository::OWNER, Some(&owner), BY).unwrap();
        model::set(&mut r, &key, repository::NAME, Some(&"acyclic"), BY).unwrap();
    }
    r
}

fn with<T>(check: impl FnOnce(&Records<'_>) -> T) -> T {
    let r = tracker();
    let model = Model::read(r.state()).unwrap();
    check(&Records::of(&model))
}

fn candidates(error: &CliError) -> Vec<&str> {
    error.details["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect()
}

#[test]
fn a_full_id_a_title_an_owner_and_name_and_an_ending_each_resolve() {
    with(|records| {
        assert_eq!(records.item(&id("b1")).unwrap(), id("b1"));
        assert_eq!(
            records.item(&id("b1").to_uppercase()).unwrap(),
            id("b1"),
            "a full id in any case"
        );
        assert_eq!(records.item("Weed").unwrap(), id("12abcdef"));
        assert_eq!(records.item("2ABCDEF").unwrap(), id("12abcdef"), "an ending in any case");
        assert_eq!(records.repository("iexalt/ACYCLIC").unwrap(), id("e1"));
        // A deleted record's id names nothing, though another's title is its ending.
        assert_eq!(records.item("c0ffee").unwrap(), id("b1"));
    });
}

#[test]
fn a_title_matches_only_as_typed() {
    with(|records| assert_eq!(records.item("weed").unwrap_err().code, "not-found"));
}

#[test]
fn what_names_several_records_or_too_little_is_refused_with_the_candidates() {
    with(|records| {
        let shared = records.item("Plant").unwrap_err();
        assert_eq!(
            (shared.code, candidates(&shared)),
            ("ambiguous", vec![id("a1").as_str(), id("a2").as_str()])
        );
        // Water's id ends in "abcdef", and so does Weed's; "Water"'s title alone is fine.
        let endings = records.item("abcdef").unwrap_err();
        assert_eq!(candidates(&endings), [id("abcdef"), id("12abcdef")]);
        let short = records.item("cdef").unwrap_err();
        assert_eq!(
            (short.code, short.details["candidates"].as_array().unwrap().len()),
            ("ambiguous", 2)
        );
        assert!(short.message.contains("at least 6"), "{}", short.message);
        // One record ends so, but the reference is too short to be an ending: not ambiguous.
        let unique = records.item("b1").unwrap_err();
        assert_eq!(unique.code, "usage");
        assert!(unique.message.contains("at least 6"), "{}", unique.message);
        assert!(unique.message.contains("2 characters"), "{}", unique.message);
        assert!(!unique.message.contains("ambiguous"), "{}", unique.message);
        let bare = records.repository("acyclic").unwrap_err();
        assert_eq!(
            (bare.code, candidates(&bare)),
            ("not-found", vec![id("e1").as_str(), id("e2").as_str()])
        );
        // Done work is no thread; a loose task is its own, named by its id, an ending of it or its title.
        assert_eq!(records.thread(&id("a2")).unwrap_err().code, "not-found");
        let one = acyclic_core::ids::one(&id("a1"));
        assert_eq!(records.thread(&id("a1")).unwrap(), one);
        assert_eq!(records.thread("Water").unwrap(), acyclic_core::ids::one(&id("abcdef")));
    });
}

#[test]
fn a_title_that_is_another_records_ending_is_refused_naming_both() {
    let mut r = tracker();
    add_item(&mut r, &id("3d"), "12abcdef", BY).unwrap();
    // The title of one and an ending of the other.
    let model = Model::read(r.state()).unwrap();
    let records = Records::of(&model);
    let clash = records.item("12abcdef").unwrap_err();
    assert_eq!(
        (clash.code, candidates(&clash)),
        ("ambiguous", vec![id("3d").as_str(), id("12abcdef").as_str()])
    );
    // A title that is an ending of its own record's id names it.
    add_item(&mut r, &id("feed01"), "feed01", BY).unwrap();
    let model = Model::read(r.state()).unwrap();
    assert_eq!(Records::of(&model).item("feed01").unwrap(), id("feed01"));
}

#[test]
fn every_printed_ending_names_its_own_record() {
    with(|records| {
        let endings = records.endings(true);
        assert_eq!(endings.len(), 5, "the deleted item has none");
        assert_eq!(endings[&id("abcdef")], "0abcdef", "longer where another id shares its end");
        for (full, ending) in &endings {
            assert!(ending.len() >= MIN_ENDING);
            assert_eq!(&records.item(ending).unwrap(), full, "{ending}");
        }
    });
}

#[test]
fn an_ending_another_records_title_holds_grows_until_it_names_its_own() {
    let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    add_item(&mut r, &id("c0ffee"), "Brew", BY).unwrap();
    // Titled as the other's ending, and as its own.
    add_item(&mut r, &id("abcdef"), "c0ffee", BY).unwrap();
    add_item(&mut r, &id("123456"), "123456", BY).unwrap();
    let model = Model::read(r.state()).unwrap();
    let records = Records::of(&model);
    let endings = records.endings(true);
    assert_eq!(endings[&id("c0ffee")], "0c0ffee");
    assert_eq!(endings[&id("123456")], "123456", "its own title names it too");
    for (full, ending) in &endings {
        assert_eq!(&records.item(ending).unwrap(), full, "{ending}");
    }
}

#[test]
fn an_id_shorter_than_an_ending_is_its_own_name() {
    let endings = endings(["t", "abc", "0abc"]);
    assert_eq!(
        endings.into_iter().collect::<Vec<_>>(),
        [("0abc", "0abc"), ("abc", "abc"), ("t", "t")].map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
}

#[test]
fn a_removal_finds_an_element_among_those_held_first() {
    let held = [id("c0ffee"), id("abcdef")];
    assert_eq!(held_one(&held, &id("c0ffee").to_uppercase()), Some(id("c0ffee")));
    assert_eq!(held_one(&held, "C0FFEE"), Some(id("c0ffee")));
    assert_eq!(held_one(&held, "ffee"), None, "too short");
    assert_eq!(held_one(&held, "Plant"), None);
}

fn held_one(list: &[String], reference: &str) -> Option<String> {
    held(list, reference)
}
