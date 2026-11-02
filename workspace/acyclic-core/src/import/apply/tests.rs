use serde_json::json;

use super::*;
use crate::import::LinkKind;
use crate::import::State as Planned;
use crate::import::tests::{item, link, valid};
use crate::model::{ItemState, Value};
use crate::scope::USER;
use crate::state::{Millis, ReplicaId, State};

fn replica() -> Replica {
    Replica::open(ReplicaId::new("base").unwrap(), State::new())
}

fn by(actor: &str, at: Millis) -> By<'_> {
    By { actor, at }
}

fn imported() -> Replica {
    let mut r = replica();
    apply(&valid(), &mut r, by(USER, 1)).unwrap();
    r
}

#[test]
fn a_document_reads_back_and_importing_it_again_writes_nothing() {
    let mut r = replica();
    let report = apply(&valid(), &mut r, by(USER, 1)).unwrap();
    assert_eq!((report.repositories, report.items, report.links, report.threads), (1, 3, 2, 1));
    // Each record is a creation with the fields it was given; a set shows whole, and a link names its ends.
    let changes = serde_json::to_value(&report.changes).unwrap();
    let find = |entity: &str, id: &str| {
        changes.as_array().unwrap().iter().find(|c| c["entity"] == entity && c["id"] == id).unwrap()
    };
    assert_eq!(changes.as_array().unwrap().len(), 1 + 1 + 3 + 2);
    assert_eq!(find("repository", "r")["action"], "create");
    assert_eq!(
        find("repository", "r")["fields"]["default_branch"],
        json!({"from": null, "to": "main"})
    );
    assert_eq!(find("item", "a")["fields"]["repositories"], json!({"from": [], "to": ["r"]}));
    assert_eq!(find("item", "a")["fields"]["parent"], json!({"from": null, "to": "goal"}));
    assert_eq!(find("item", "b")["fields"]["thread"], json!({"from": null, "to": "t"}));
    let link = changes.as_array().unwrap().iter().find(|c| c["entity"] == "link").unwrap();
    assert_eq!(
        (&link["from"], &link["kind"], &link["action"], &link["fields"]),
        (&json!("a"), &json!("waits-for"), &json!("create"), &json!({}))
    );
    let model = Model::read(&r.state).unwrap();
    let a = &model.items["a"];
    assert_eq!(a.title, Value::One("Item a".to_owned()));
    assert_eq!(a.parent, Value::One("goal".to_owned()));
    assert_eq!(a.repositories, ["r".to_owned()].into());
    assert_eq!(model.items["b"].state, Value::One(ItemState::Done));
    assert!(model.items["b"].evidence.one().is_some());
    assert_eq!(model.thread_of("b"), Some("t"));
    assert_eq!(model.links.len(), 2);
    assert_eq!(model.repositories["r"].default_branch, Value::One("main".to_owned()));

    let before = r.state.clone();
    let again = apply(&valid(), &mut r, by(USER, 2)).unwrap();
    assert_eq!(again, Report::default());
    assert_eq!(r.state, before);
}

#[test]
fn a_changed_title_is_one_write_and_what_the_document_omits_is_kept() {
    let mut r = imported();
    let key = model::item_key("a").unwrap();
    model::set(&mut r, &key, field::NOTES, Some(&"kept"), by(USER, 2)).unwrap();
    let mut doc = valid();
    doc.items[1].title = "Renamed".to_owned();
    let report = apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!((report.items, report.fields), (0, 1));
    assert_eq!(
        serde_json::to_value(&report.changes).unwrap(),
        json!([{"entity": "item", "id": "a", "action": "update",
                "fields": {"title": {"from": "Item a", "to": "Renamed"}}}])
    );
    let a = &Model::read(&r.state).unwrap().items["a"];
    assert_eq!(a.title, Value::One("Renamed".to_owned()));
    assert_eq!(a.notes, Value::One("kept".to_owned()));
}

#[test]
fn an_invalid_document_is_refused() {
    let mut doc = valid();
    doc.items.push(item("a", Planned::Backlog, None));
    assert!(matches!(apply(&doc, &mut replica(), by(USER, 1)), Err(ApplyError::Invalid(_))));
}

#[test]
fn a_document_closing_cycles_of_its_own_is_refused_naming_each_once() {
    let mut doc = valid();
    for id in ["c", "d", "e"] {
        doc.items.push(item(id, Planned::Backlog, None));
    }
    // goal and a under each other; a and b waiting for each other, which c only reaches; d and e likewise. The
    // valid document's relates-to link from b to a orders nothing, so closes none.
    doc.items[0].parent = Some("a".to_owned());
    doc.links.push(link("b", "a", LinkKind::WaitsFor));
    doc.links.push(link("c", "a", LinkKind::WaitsFor));
    doc.links.push(link("d", "e", LinkKind::WaitsFor));
    doc.links.push(link("e", "d", LinkKind::WaitsFor));
    let mut r = replica();
    let Err(ApplyError::Cycles(mut cycles)) = apply(&doc, &mut r, by(USER, 1)) else {
        panic!("cycles accepted")
    };
    cycles.sort();
    assert_eq!(cycles, [["a", "b"], ["a", "goal"], ["d", "e"]]);
}

#[test]
fn a_cycle_closed_with_what_the_tracker_holds_is_refused() {
    let mut r = imported();
    let mut first = valid();
    first.items.push(item("x", Planned::Backlog, Some("a")));
    apply(&first, &mut r, by(USER, 2)).unwrap();
    // On its own the document is acyclic, since it leaves x's parent alone; with the tracker's x under a it isn't.
    let mut second = valid();
    second.items.push(item("x", Planned::Backlog, None));
    second.items[0].parent = Some("x".to_owned());
    assert_eq!(second.validate(), vec![]);
    let Err(ApplyError::Cycles(cycles)) = apply(&second, &mut r, by(USER, 3)) else {
        panic!("cycle accepted")
    };
    assert_eq!(cycles.len(), 1);
}
