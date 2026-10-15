use super::*;
use crate::import::State as Planned;
use crate::import::tests::{item, valid};
use crate::model::{ItemState, Value};
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
    assert_eq!(
        (report.repositories, report.items, report.links, report.threads, report.proposals.len()),
        (1, 3, 2, 1, 0)
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
    let a = &Model::read(&r.state).unwrap().items["a"];
    assert_eq!(a.title, Value::One("Renamed".to_owned()));
    assert_eq!(a.notes, Value::One("kept".to_owned()));
}

#[test]
fn an_invalid_document_or_a_user_only_one_from_an_agent_is_refused() {
    let agent = by("run/worker", 1);
    let mut doc = valid();
    doc.items.push(item("a", Planned::Backlog, None));
    assert!(matches!(apply(&doc, &mut replica(), agent), Err(ApplyError::Invalid(_))));

    let mut doc = valid();
    doc.threads[0].approved = true;
    assert!(matches!(apply(&doc, &mut replica(), agent), Err(ApplyError::UserOnly(_))));
    apply(&doc, &mut replica(), by(USER, 1)).unwrap();

    let mut doc = valid();
    doc.items[2].archived = true;
    assert!(matches!(apply(&doc, &mut replica(), agent), Err(ApplyError::UserOnly(_))));
}

#[test]
fn an_agents_addition_to_approved_scope_is_proposed() {
    let mut r = imported();
    model::set(
        &mut r,
        &Key::new(kind::THREAD, "t").unwrap(),
        thread::APPROVED,
        Some(&true),
        by(USER, 2),
    )
    .unwrap();
    let mut doc = valid();
    doc.items.push(item("c", Planned::Backlog, Some("goal")));
    doc.threads[0].items.push("c".to_owned());
    let report = apply(&doc, &mut r, by("run/worker", 3)).unwrap();
    assert_eq!((report.items, report.proposals.len()), (1, 1));
    assert_eq!(Model::read(&r.state).unwrap().thread_of("c"), None);
    // Importing again while the proposal waits files nothing new.
    let before = r.state.clone();
    assert_eq!(apply(&doc, &mut r, by("run/worker", 4)).unwrap().proposals, report.proposals);
    assert_eq!(r.state, before);
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
