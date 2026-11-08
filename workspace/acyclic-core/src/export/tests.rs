use serde_json::json;

use super::*;
use crate::import;
use crate::model::Evidence as Stored;
use crate::state::{Replica, ReplicaId, State};

fn id(n: u8) -> String {
    format!("01900000-0000-7000-8000-{n:012x}")
}

fn replica() -> Replica {
    Replica::open(ReplicaId::new("base").unwrap(), State::new())
}

fn by(at: i64) -> By<'static> {
    By { actor: "actor/user", at }
}

/// Initiative 9; epic 10 under it with leaves 11 and 12 (11 waits for 12, 12 done in repository 1) in thread 20,
/// a loose leaf 13 beside them, and a parked leaf 14 under the initiative.
fn graph() -> Import {
    let doc = json!({
        "format": "acyclic-import", "version": 4,
        "repositories": [{"id": id(1), "name": "example/code", "default_branch": "main"}],
        "initiatives": [{"id": id(9), "title": "Initiative", "notes": "Plain"}],
        "threads": [{"id": id(20), "title": "Thread", "notes": "Words"}],
        "items": [
            {"id": id(10), "title": "Epic", "parent": id(9)},
            {"id": id(11), "title": "One", "parent": id(10), "thread": id(20), "waits_for": [id(12)]},
            {"id": id(12), "title": "Two", "parent": id(10), "thread": id(20), "state": "done",
             "evidence": {"commit": format!("{}@abc1234", id(1)), "text": "shipped"}},
            {"id": id(13), "title": "Loose", "parent": id(10), "thread": null},
            {"id": id(14), "title": "Parked", "parent": id(9), "state": "parked", "notes": "Later"},
        ],
    });
    import::Import::from_json(&doc.to_string()).unwrap()
}

fn read(r: &Replica) -> Model {
    Model::read(&r.state).unwrap()
}

fn imported(doc: &Import) -> Model {
    let mut r = replica();
    import::apply(doc, &mut r, by(1)).unwrap();
    read(&r)
}

#[test]
fn an_empty_tracker_exports_an_empty_document() {
    let doc = export(&Model::default()).unwrap();
    assert_eq!(doc, Import::new());
    assert!(import::apply(&doc, &mut replica(), by(1)).is_ok());
}

#[test]
fn a_graph_exports_complete_sorted_and_explicit() {
    let model = imported(&graph());
    let doc = export(&model).unwrap();
    let ids: Vec<&str> = doc.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, [id(10), id(11), id(12), id(13), id(14)]);
    let by_id = |n: u8| doc.items.iter().find(|i| i.id == id(n)).unwrap();
    // Every item says its thread and waits-for: an id or null, a list or [].
    assert!(doc.items.iter().all(|i| i.thread.is_some() && i.waits_for.is_some()));
    assert_eq!(by_id(10).thread, Some(None));
    assert_eq!(by_id(13).thread, Some(None));
    assert_eq!(by_id(11).thread, Some(Some(id(20))));
    assert_eq!(by_id(11).waits_for, Some(vec![id(12)]));
    assert_eq!(by_id(14).waits_for, Some(vec![]));
    // The word, not the stored `backlog`; evidence as `<repository id>@<sha>`.
    assert_eq!(by_id(11).state, Some(import::State::Backlog));
    assert!(doc.to_json().contains("\"state\": \"open\""));
    assert!(!doc.to_json().contains("backlog"));
    assert_eq!(
        by_id(12).evidence,
        Some(Evidence { commit: Some(format!("{}@abc1234", id(1))), text: Some("shipped".into()) })
    );
    assert_eq!(
        (doc.repositories[0].name.as_str(), doc.threads[0].notes.as_deref()),
        ("example/code", Some("Words"))
    );
    assert_eq!(doc.initiatives[0].notes.as_deref(), Some("Plain"));
}

#[test]
fn an_export_imports_into_a_fresh_tracker_as_the_same_graph_and_into_its_own_as_nothing() {
    let model = imported(&graph());
    let doc = export(&model).unwrap();
    let text = doc.to_json();
    assert_eq!(Import::from_json(&text).unwrap(), doc);
    // Again into the tracker it came from: nothing to write.
    let mut source = replica();
    import::apply(&graph(), &mut source, by(1)).unwrap();
    assert_eq!(import::preview(&doc, &source, by(2)).unwrap().changes, vec![]);
    // Into a fresh one: the same records, so it exports the same bytes.
    let mut fresh = replica();
    import::apply(&doc, &mut fresh, by(2)).unwrap();
    let copy = read(&fresh);
    assert_eq!(export(&copy).unwrap().to_json(), text);
    assert_eq!(copy.thread_of(&id(13)), None, "the loose item stays beside its threaded siblings");
    assert_eq!(copy.thread_of(&id(11)), Some(id(20).as_str()));
}

#[test]
fn a_loose_task_in_its_own_thread_of_one_exports_as_in_no_thread() {
    let mut model = imported(&graph());
    model.items.get_mut(&id(13)).unwrap().thread = Value::One(ids::one(&id(13)));
    let doc = export(&model).unwrap();
    assert_eq!(doc.items.iter().find(|i| i.id == id(13)).unwrap().thread, Some(None));
}

/// The blockers, as `(kind, id)` pairs, with the reasons joined.
fn refused(model: &Model) -> Vec<(String, String, String)> {
    export(model).unwrap_err().0.into_iter().map(|b| (b.kind, b.id, b.reason)).collect()
}

fn blocked(change: impl FnOnce(&mut Model), kind: &str, at: u8, reason: &str) {
    let mut model = imported(&graph());
    change(&mut model);
    let found = refused(&model);
    assert!(
        found.iter().any(|(k, i, r)| k == kind && *i == id(at) && r.contains(reason)),
        "{kind} {at} {reason:?} in {found:#?}"
    );
}

#[test]
fn what_a_document_cant_say_is_refused_naming_each_record() {
    blocked(|m| m.items.get_mut(&id(11)).unwrap().state = Value::None, "item", 11, "no state");
    blocked(|m| m.items.get_mut(&id(11)).unwrap().title = Value::None, "item", 11, "no title");
    blocked(|m| m.items.get_mut(&id(11)).unwrap().parent = Value::None, "item", 11, "no parent");
    blocked(
        |m| m.items.get_mut(&id(14)).unwrap().parent = Value::One(id(99)),
        "item",
        14,
        "is no item or initiative",
    );
    blocked(
        |m| m.items.get_mut(&id(10)).unwrap().thread = Value::One(id(20)),
        "item",
        10,
        "has children",
    );
    blocked(
        |m| m.items.get_mut(&id(10)).unwrap().state = Value::One(ItemState::Done),
        "item",
        10,
        "stored done",
    );
    blocked(
        |m| m.items.get_mut(&id(13)).unwrap().thread = Value::One(id(77)),
        "item",
        13,
        "has no record",
    );
    blocked(
        |m| m.items.get_mut(&id(13)).unwrap().thread = Value::One(ids::one(&id(14))),
        "item",
        13,
        "another item's thread of one",
    );
    // A thread of one that reads as a named thread holds items under an id no document can carry.
    let mut model = imported(&graph());
    let one = ids::one(&id(13));
    model.threads.insert(
        one.clone(),
        model::Thread { id: one.clone(), title: Value::One("Grown".into()), notes: Value::None },
    );
    model.items.get_mut(&id(13)).unwrap().thread = Value::One(one.clone());
    let found = refused(&model);
    assert!(
        found
            .iter()
            .any(|(k, i, r)| k == "thread" && *i == one && r.contains("reads as a named thread")),
        "{found:#?}"
    );
}

#[test]
fn records_missing_what_a_document_needs_are_refused() {
    blocked(|m| m.threads.get_mut(&id(20)).unwrap().title = Value::None, "thread", 20, "no title");
    blocked(
        |m| m.initiatives.get_mut(&id(9)).unwrap().title = Value::None,
        "initiative",
        9,
        "no title",
    );
    blocked(
        |m| m.repositories.get_mut(&id(1)).unwrap().default_branch = Value::None,
        "repository",
        1,
        "default branch",
    );
    blocked(
        |m| m.repositories.get_mut(&id(1)).unwrap().owner = Value::None,
        "repository",
        1,
        "owner",
    );
    // Evidence: half a commit, evidence on unfinished work, and a repository that isn't there.
    let stored = |repository: Option<&str>, commit: Option<&str>| {
        Value::One(Stored {
            repository: repository.map(str::to_owned),
            commit: commit.map(str::to_owned),
            text: None,
        })
    };
    blocked(
        |m| m.items.get_mut(&id(12)).unwrap().evidence = stored(None, Some("abc1234")),
        "item",
        12,
        "not both",
    );
    blocked(
        |m| m.items.get_mut(&id(12)).unwrap().evidence = stored(Some(&id(1)), None),
        "item",
        12,
        "not both",
    );
    blocked(
        |m| m.items.get_mut(&id(11)).unwrap().evidence = stored(None, None),
        "item",
        11,
        "isn't done",
    );
    blocked(
        |m| m.items.get_mut(&id(12)).unwrap().evidence = stored(Some(&id(5)), Some("abc1234")),
        "item",
        12,
        "no record is",
    );
    // An id a document can't hold.
    let mut model = imported(&graph());
    let mut item = model.items[&id(14)].clone();
    item.id = "Upper".into();
    model.items.insert(item.id.clone(), item);
    let found = refused(&model);
    assert!(
        found.iter().any(|(k, i, r)| k == "item" && i == "Upper" && r.contains("UUIDv7")),
        "{found:#?}"
    );
}

#[test]
fn a_conflict_and_a_dangling_link_are_refused() {
    blocked(
        |m| {
            m.conflicts.insert(("item".into(), id(11), "title".into()));
        },
        "item",
        11,
        "title holds different values",
    );
    blocked(|m| m.links.push(model::Link { from: id(13), to: id(88) }), "item", 13, "no item is");
}

#[test]
fn the_self_check_refuses_what_an_import_would_not_take() {
    // A link between an item and its ancestor passes the shape checks and fails the judge.
    blocked(
        |m| m.links.push(model::Link { from: id(11), to: id(10) }),
        "item",
        11,
        "above or below",
    );
}

#[test]
fn many_refusals_are_all_listed_sorted() {
    let mut model = imported(&graph());
    model.items.get_mut(&id(14)).unwrap().state = Value::None;
    model.items.get_mut(&id(11)).unwrap().state = Value::None;
    let found = refused(&model);
    let ids: Vec<&str> = found.iter().map(|(_, i, _)| i.as_str()).collect();
    assert_eq!(ids, [id(11), id(14)]);
}

#[test]
fn a_field_two_replicas_set_apart_is_refused_naming_its_record() {
    let mut a = replica();
    import::apply(&graph(), &mut a, by(1)).unwrap();
    let mut b = Replica::open(ReplicaId::new("other").unwrap(), State::new());
    b.join(&a.state).unwrap();
    let key = model::item_key(&id(11)).unwrap();
    model::put(&mut a, &key, crate::schema::item::NOTES, Some(&"first"), by(2)).unwrap();
    model::put(&mut b, &key, crate::schema::item::NOTES, Some(&"second"), by(2)).unwrap();
    a.join(&b.state).unwrap();
    let model = read(&a);
    assert!(model.items[&id(11)].notes.is_conflict());
    let found = refused(&model);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!((found[0].0.as_str(), found[0].1.as_str()), ("item", id(11).as_str()));
    assert!(found[0].2.contains("notes"), "{found:#?}");
}
