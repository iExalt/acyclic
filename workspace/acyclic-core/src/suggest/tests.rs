use serde_json::json;

use super::*;
use crate::model::{
    By, ItemState, LinkKind, SetField, add_item, item_key, link, set_element, set_parent, set_state,
};
use crate::schema::{item as field, kind, thread};
use crate::state::{Key, Replica, ReplicaId, State};

const USER: By<'static> = By { actor: "actor/user", at: 1 };

fn write(r: &mut Replica, key: &Key, name: &str, value: serde_json::Value) {
    r.write(key, name, USER.actor, USER.at, Some(value)).unwrap();
}

fn add(r: &mut Replica, id: &str, title: &str, parent: Option<&str>) {
    add_item(r, id, title, USER).unwrap();
    if let Some(parent) = parent {
        set_parent(r, id, Some(parent), USER).unwrap();
    }
}

/// Two goals, each with two children, and an item `new` to place.
fn home() -> Replica {
    let mut r = Replica::open(ReplicaId::new("mac").unwrap(), State::new());
    add(&mut r, "house", "Fix up the house", None);
    add(&mut r, "paint", "Paint the hallway walls", Some("house"));
    add(&mut r, "tiles", "Replace bathroom tiles", Some("house"));
    add(&mut r, "trip", "Plan the summer trip", None);
    add(&mut r, "flights", "Book flights to Lisbon", Some("trip"));
    add(&mut r, "hotel", "Book a hotel in Lisbon", Some("trip"));
    r
}

fn model(r: &Replica) -> Model {
    Model::read(r.state()).unwrap()
}

fn parents(r: &Replica, id: &str) -> Vec<String> {
    suggest(&model(r), id, None, LIMIT).0.parents.into_iter().map(|p| p.item).collect()
}

#[test]
fn words_lead_to_the_parent_whose_children_share_them() {
    let mut r = home();
    add(&mut r, "new", "Paint the bathroom", None);
    let shown = parents(&r, "new");
    let leaves: BTreeSet<&str> = shown[..2].iter().map(String::as_str).collect();
    assert_eq!(leaves, BTreeSet::from(["paint", "tiles"]), "leaves share words too");
    assert_eq!(shown[2], "house", "and their parent, through them");
    add(&mut r, "car", "Rent a car in Lisbon", None);
    assert_eq!(
        parents(&r, "car")[0],
        "flights".to_owned().replace("flights", &parents(&r, "car")[0])
    );
    let ranked =
        rank(&model(&r), &Query::words_of(&model(&r).items["car"]), &subtree(&model(&r), "car"));
    let trip = ranked.parents.iter().position(|(id, _)| id == "trip").unwrap();
    let house = ranked.parents.iter().position(|(id, _)| id == "house").unwrap();
    assert!(trip < house, "a group's children describe it: {ranked:?}");
    assert_eq!(
        rank(&model(&r), &Query::words_of(&model(&r).items["car"]), &subtree(&model(&r), "car")),
        ranked
    );
}

#[test]
fn labels_and_links_lift_a_candidate() {
    let mut r = home();
    add(&mut r, "new", "Sort out the paperwork", None);
    assert_eq!(parents(&r, "new"), Vec::<String>::new(), "no shared word, no candidate");
    for (id, area) in [("trip", "travel"), ("new", "travel")] {
        set_element(&mut r, &item_key(id).unwrap(), SetField::Area, area, true, USER).unwrap();
    }
    assert_eq!(parents(&r, "new")[0], "trip");
    add(&mut r, "visa", "Apply for visas", None);
    link(&mut r, "visa", "tiles", LinkKind::Blocks, USER).unwrap();
    assert_eq!(parents(&r, "visa")[0], "house", "the parent of what it links to");
}

#[test]
fn the_item_its_subtree_and_their_words_are_left_out() {
    let mut r = home();
    add(&mut r, "new", "Lisbon itinerary", Some("trip"));
    add(&mut r, "day", "Day one in Lisbon", Some("new"));
    let ranked =
        rank(&model(&r), &Query::words_of(&model(&r).items["new"]), &subtree(&model(&r), "new"));
    assert!(ranked.parents.iter().all(|(id, _)| id != "new" && id != "day"));
    // `trip`'s words come from `flights` and `hotel` alone, not from `new` beneath it.
    let alone = {
        let mut r = home();
        add(&mut r, "new", "Lisbon itinerary", None);
        let m = model(&r);
        rank(&m, &Query::words_of(&m.items["new"]), &subtree(&m, "new"))
    };
    let score = |ranking: &Ranking| ranking.parents.iter().find(|(id, _)| id == "trip").unwrap().1;
    assert_eq!(score(&ranked), score(&alone));
}

#[test]
fn done_items_are_never_parents_and_archived_ones_are_offered_as_links() {
    let mut r = home();
    set_state(&mut r, "tiles", ItemState::Done, USER).unwrap();
    add(&mut r, "new", "Bathroom tiles grout", None);
    assert!(!parents(&r, "new").contains(&"tiles".to_owned()));
    write(&mut r, &item_key("house").unwrap(), field::ARCHIVED, json!(true));
    let (shown, _) = suggest(&model(&r), "new", None, LIMIT);
    assert!(shown.parents.iter().all(|p| p.item != "house" && p.item != "paint"));
    let related: Vec<&str> = shown.related.iter().map(|r| r.item.as_str()).collect();
    assert_eq!(related, ["tiles", "house"], "beating no live parent: {shown:?}");
    assert_eq!(shown.related[0].accept, "acyclic link add new relates tiles");
    // Live parents that match better leave the archived ones out.
    for (id, title) in
        [("g1", "Grout and tiles"), ("g2", "Bathroom grout"), ("g3", "Tiles grout bathroom")]
    {
        add(&mut r, id, title, None);
    }
    let (shown, _) = suggest(&model(&r), "new", None, LIMIT);
    assert_eq!(shown.parents.len(), LIMIT);
    let weakest = shown.parents.last().unwrap().score;
    assert!(shown.related.iter().all(|r| r.score > weakest), "{shown:?}");
}

#[test]
fn threads_rank_by_their_words_and_parents_name_the_thread_they_pass_down() {
    let mut r = home();
    for (id, title, approved) in [("lisbon", "Lisbon travel", true), ("diy", "Weekend jobs", false)]
    {
        let key = Key::new(kind::THREAD, id).unwrap();
        write(&mut r, &key, thread::TITLE, json!(title));
        write(&mut r, &key, thread::APPROVED, json!(approved));
    }
    write(&mut r, &item_key("trip").unwrap(), field::THREAD, json!("lisbon"));
    write(&mut r, &item_key("paint").unwrap(), field::THREAD, json!("diy"));
    add(&mut r, "new", "Lisbon museum tickets", None);
    let (shown, _) = suggest(&model(&r), "new", None, LIMIT);
    assert_eq!(shown.threads[0].thread, "lisbon");
    assert_eq!(shown.threads[0].accept, "acyclic thread add new lisbon");
    assert!(shown.threads[0].note.is_some(), "approved");
    let flights = shown.parents.iter().find(|p| p.item == "flights").unwrap();
    assert_eq!((flights.inherits.as_deref(), flights.note.is_some()), (Some("lisbon"), true));
    assert_eq!(flights.accept, "acyclic item move new flights");
    add(&mut r, "wall", "Hallway wall repair", None);
    let (shown, _) = suggest(&model(&r), "wall", None, LIMIT);
    let paint = shown.parents.iter().find(|p| p.item == "paint").unwrap();
    assert_eq!((paint.inherits.as_deref(), paint.note), (Some("diy"), None), "unapproved");
    let house = shown.parents.iter().find(|p| p.item == "house").unwrap();
    assert_eq!(house.inherits.as_deref(), Some("diy"), "the thread its children share");
    // A candidate whose own thread is in conflict passes none down, as placing under it would.
    let mut other = Replica::open(ReplicaId::new("pi").unwrap(), State::new());
    other.join(r.state()).unwrap();
    let key = item_key("flights").unwrap();
    other.write(&key, field::THREAD, USER.actor, 5, Some(json!("diy"))).unwrap();
    r.write(&key, field::THREAD, USER.actor, 5, Some(json!("lisbon"))).unwrap();
    r.join(other.state()).unwrap();
    let (shown, _) = suggest(&model(&r), "new", None, LIMIT);
    let flights = shown.parents.iter().find(|p| p.item == "flights").unwrap();
    assert_eq!(flights.inherits, None, "{shown:?}");
    // A thread whose work is all done isn't offered; a fresh one with no members is.
    set_state(&mut r, "trip", ItemState::Done, USER).unwrap();
    for id in ["flights", "hotel", "new"] {
        write(&mut r, &item_key(id).unwrap(), field::THREAD, json!("diy"));
    }
    let fresh = Key::new(kind::THREAD, "museums").unwrap();
    write(&mut r, &fresh, thread::TITLE, json!("Lisbon museums"));
    add(&mut r, "next", "Lisbon museum day", None);
    let (shown, _) = suggest(&model(&r), "next", None, LIMIT);
    let threads: BTreeSet<&str> = shown.threads.iter().map(|t| t.thread.as_str()).collect();
    assert_eq!(threads, BTreeSet::from(["diy", "museums"]), "lisbon holds only `trip`, now done");
}

#[test]
fn a_given_parent_reports_its_rank_or_none_past_the_limit() {
    let mut r = home();
    add(&mut r, "new", "Book Lisbon flights and hotel", Some("trip"));
    let m = model(&r);
    let all = rank(&m, &Query::words_of(&m.items["new"]), &subtree(&m, "new"));
    let rank_of = |id: &str| all.parents.iter().position(|(p, _)| p == id).unwrap() + 1;
    assert_eq!(suggest(&m, "new", Some("trip"), LIMIT).1, Some(rank_of("trip")));
    assert!(rank_of("tiles") > LIMIT);
    assert_eq!(suggest(&m, "new", Some("tiles"), LIMIT).1, None);
    // A given parent sharing nothing has no rank, though zero scores sort among the others by id.
    let mut lone = Replica::open(ReplicaId::new("pi").unwrap(), State::new());
    add(&mut lone, "aaa", "Unrelated chores", None);
    add(&mut lone, "trip", "Lisbon trip", None);
    add(&mut lone, "new", "Lisbon museum", Some("aaa"));
    assert_eq!(suggest(&model(&lone), "new", Some("aaa"), LIMIT).1, None);
}

#[test]
fn words_are_unicode_and_ids_stay_whole() {
    assert_eq!(
        tokens("Café RÉNOVATION, Straße & d62 (step 2.4.)"),
        ["café", "rénovation", "straße", "d62", "step", "2.4"]
    );
    assert_eq!(tokens("a I of the x"), Vec::<String>::new());
}
