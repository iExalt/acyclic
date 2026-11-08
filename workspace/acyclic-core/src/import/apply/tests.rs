use serde_json::json;

use super::*;
use crate::import::State as Planned;
use crate::import::tests::{id, item, valid};
use crate::model::{self, ItemState, Value};
use crate::schema::item as field;
use crate::scope::ScopeEdit;
use crate::scope::USER;
use crate::state::{Millis, ReplicaId, State};

fn replica() -> Replica {
    Replica::open(ReplicaId::new("base").unwrap(), State::new())
}

fn by(actor: &str, at: Millis) -> By<'_> {
    By { actor, at }
}

fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    super::apply(doc, replica, by)
}

fn imported() -> Replica {
    let mut r = replica();
    apply(&valid(), &mut r, by(USER, 1)).unwrap();
    r
}

fn model(r: &Replica) -> Model {
    Model::read(&r.state).unwrap()
}

#[test]
fn a_document_reads_back_and_importing_it_again_writes_nothing() {
    let mut r = replica();
    let report = apply(&valid(), &mut r, by(USER, 1)).unwrap();
    assert_eq!((report.repositories, report.items, report.links, report.threads), (1, 3, 1, 1));
    // Each record is a creation with the fields it was given, and a link names its ends.
    let changes = serde_json::to_value(&report.changes).unwrap();
    let find = |entity: &str, n: u8| {
        changes
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["record"] == entity && c["id"] == id(n))
            .unwrap()
    };
    assert_eq!(changes.as_array().unwrap().len(), 1 + 1 + 1 + 3 + 1);
    assert_eq!(report.initiatives, 1);
    assert_eq!(find("initiative", 9)["action"], "create");
    assert_eq!(find("repository", 1)["action"], "create");
    assert_eq!(find("repository", 1)["fields"]["owner"], json!({"from": null, "to": "example"}));
    assert_eq!(find("item", 11)["fields"]["parent"], json!({"from": null, "to": id(10)}));
    assert_eq!(find("item", 12)["fields"]["thread"], json!({"from": null, "to": id(20)}));
    let link = changes.as_array().unwrap().iter().find(|c| c["record"] == "link").unwrap();
    assert_eq!(
        (&link["from"], &link["kind"], &link["action"]),
        (&json!(id(11)), &json!("waits-for"), &json!("create"))
    );
    let m = model(&r);
    let a = &m.items[&id(11)];
    assert_eq!(a.parent, Value::One(id(10)));
    // A new item left without a state is open.
    assert_eq!(m.items[&id(10)].state, Value::One(ItemState::Backlog));
    // The evidence keeps the repository and the SHA apart.
    let evidence = m.items[&id(12)].evidence.one().unwrap();
    assert_eq!(
        (evidence.repository.as_deref(), evidence.commit.as_deref(), evidence.text.as_deref()),
        (Some(id(1).as_str()), Some("abc1234"), Some("shipped"))
    );
    assert_eq!(m.thread_of(&id(12)), Some(id(20).as_str()));
    assert_eq!(m.links, [model::Link { from: id(11), to: id(12) }]);

    // Again, later: finished work keeps its done time, since nothing is written.
    let before = r.state.clone();
    let again = apply(&valid(), &mut r, by(USER, 2)).unwrap();
    assert_eq!(again, Report::default());
    assert_eq!(r.state, before);
}

#[test]
fn what_a_document_leaves_out_is_kept() {
    let mut r = imported();
    let done_at = model(&r).items[&id(12)].done_at;
    let mut doc = valid();
    doc.items[1].title = "Renamed".to_owned();
    for left in &mut doc.items {
        (left.state, left.thread, left.waits_for, left.parent, left.evidence) =
            (None, None, None, None, None);
    }
    let report = apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!(
        serde_json::to_value(&report.changes).unwrap(),
        json!([{"record": "item", "id": id(11), "action": "update",
                "fields": {"title": {"from": "Item 11", "to": "Renamed"}}}])
    );
    let m = model(&r);
    let (kept, done) = (&m.items[&id(11)], &m.items[&id(12)]);
    assert_eq!(kept.parent.one(), Some(&id(10)));
    assert_eq!(m.thread_of(&id(11)), Some(id(20).as_str()));
    assert_eq!((done.state.one(), done.done_at), (Some(&ItemState::Done), done_at));
    assert!(done.evidence.one().is_some());
    assert_eq!(m.links.len(), 1);
}

#[test]
fn a_list_given_replaces_the_records_own() {
    let mut r = imported();
    let mut doc = valid();
    doc.items.push(item(13, None, Some(10)));
    doc.items[1].waits_for = Some(vec![id(13)]);
    let report = apply(&doc, &mut r, by(USER, 2)).unwrap();
    let m = model(&r);
    assert_eq!(m.links, [model::Link { from: id(11), to: id(13) }]);
    let changes = serde_json::to_value(&report.changes).unwrap();
    let removed: Vec<&Json> =
        changes.as_array().unwrap().iter().filter(|c| c["action"] == "remove").collect();
    assert_eq!(
        removed,
        [
            &json!({"record": "link", "from": id(11), "kind": "waits-for", "to": id(12), "action": "remove", "fields": {}})
        ]
    );
    assert_eq!(report.links, 1, "counts.links counts only created links");
}

#[test]
fn a_document_may_turn_a_link_around() {
    let mut r = imported();
    let mut doc = valid();
    doc.items[1].waits_for = Some(vec![]);
    doc.items[2].waits_for = Some(vec![id(11)]);
    apply(&doc, &mut r, by(USER, 2)).unwrap();
    assert_eq!(model(&r).links, [model::Link { from: id(12), to: id(11) }]);
}

#[test]
fn references_may_name_the_trackers_records_but_not_unknown_ones() {
    let mut r = imported();
    let mut doc = crate::import::Import::new();
    let mut next = item(30, None, Some(10));
    next.thread = Some(Some(id(20)));
    next.waits_for = Some(vec![id(12)]);
    doc.items.push(next);
    apply(&doc, &mut r, by(USER, 2)).unwrap();
    assert_eq!(model(&r).thread_of(&id(30)), Some(id(20).as_str()));
    doc.items[0].parent = Some(id(99));
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("unknown parent accepted")
    };
    assert_eq!(
        errors,
        [ValidationError::UnknownReference {
            at: format!("item {}, parent", id(30)),
            kind: KeyKind::Item,
            id: id(99)
        }]
    );
}

#[test]
fn evidence_is_judged_by_the_state_each_item_ends_with() {
    let mut r = imported();
    // 12 is done in the tracker, so evidence on it, its state left out, is fine.
    let mut doc = crate::import::Import::new();
    let mut done = item(12, None, None);
    done.evidence = Some(crate::import::Evidence { commit: None, text: Some("again".into()) });
    doc.items.push(done);
    let mut release = item(11, None, None);
    release.waits_for = Some(vec![]);
    doc.items.push(release);
    apply(&doc, &mut r, by(USER, 31 * 24 * 60 * 60 * 1000)).unwrap();
    // 11 isn't done, so evidence on it is refused.
    doc.items.clear();
    let mut open = item(11, None, None);
    open.evidence = Some(crate::import::Evidence { commit: None, text: Some("n".into()) });
    doc.items.push(open);
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("evidence on open work accepted")
    };
    assert_eq!(errors, [ValidationError::EvidenceNotDone(id(11))]);
}

#[test]
fn an_invalid_document_is_refused() {
    let mut doc = valid();
    doc.items.push(item(11, Some(Planned::Backlog), None));
    assert!(matches!(apply(&doc, &mut replica(), by(USER, 1)), Err(ApplyError::Invalid(_))));
}

#[test]
fn a_document_closing_cycles_of_its_own_is_refused_naming_each_once() {
    let mut doc = valid();
    for n in [13, 14] {
        doc.items.push(item(n, None, Some(9)));
    }
    // 10 and 11 parent each other. 11 and 12 wait for each other, but through that parent cycle only leaf 12 is in
    // the effective cycle, even though it is done. 13 and 14 wait for each other independently.
    doc.items[0].parent = Some(id(11));
    doc.items[2].waits_for = Some(vec![id(11)]);
    doc.items[3].waits_for = Some(vec![id(14)]);
    doc.items[4].waits_for = Some(vec![id(13)]);
    let Err(ApplyError::Cycles(mut cycles)) = apply(&doc, &mut replica(), by(USER, 1)) else {
        panic!("cycles accepted")
    };
    cycles.sort();
    assert_eq!(cycles, vec![vec![id(10), id(11)], vec![id(12)], vec![id(13), id(14)]]);
}

#[test]
fn a_cycle_closed_with_what_the_tracker_holds_is_refused() {
    let mut r = imported();
    // 11 waits for 12 in the tracker; a document making 12 wait for 11, leaving 11's list alone, closes a cycle.
    let mut doc = crate::import::Import::new();
    let mut back = item(12, None, None);
    back.waits_for = Some(vec![id(11)]);
    doc.items.push(back);
    assert_eq!(doc.validate(), vec![]);
    let Err(ApplyError::Cycles(cycles)) = apply(&doc, &mut r, by(USER, 2)) else {
        panic!("cycle accepted")
    };
    assert_eq!(cycles, [[id(11), id(12)]]);
    assert_eq!(
        ApplyError::Cycles(cycles).to_string(),
        format!("the import would close 1 cycle: {} -> {}", id(11), id(12))
    );
}

#[test]
fn an_inherited_wait_cycle_maps_to_cycles_and_leaves_replica_state_unchanged() {
    let mut doc = Import::new();
    doc.initiatives.push(crate::import::Initiative {
        id: id(9),
        title: "Initiative".into(),
        notes: None,
    });
    doc.items = vec![item(10, None, Some(9)), item(11, None, Some(10)), item(12, None, Some(9))];
    doc.items[0].waits_for = Some(vec![id(12)]);
    doc.items[2].waits_for = Some(vec![id(11)]);
    let mut r = replica();
    let before = r.state.clone();

    assert_eq!(
        apply(&doc, &mut r, by(USER, 1)),
        Err(ApplyError::Cycles(vec![vec![id(11), id(12)]]))
    );
    assert_eq!(r.state, before);
}

#[test]
fn a_deleted_thread_is_refused_as_a_deleted_item_is() {
    let mut r = replica();
    let mut doc = Import::new();
    doc.threads = vec![crate::import::Thread { id: id(21), title: "Gone".into(), notes: None }];
    apply(&doc, &mut r, by(USER, 1)).unwrap();
    assert!(crate::delete::delete_thread(&mut r, &id(21), by(USER, 2)).unwrap());
    let err = apply(&doc, &mut r, by(USER, 3)).unwrap_err();
    assert_eq!(err, ApplyError::Write(WriteError::Deleted(id(21))));
    assert_eq!(err.to_string(), format!("{} was deleted, and its id isn't used again", id(21)));
}

#[test]
fn a_repository_name_another_record_would_share_is_refused_in_any_case() {
    let repository = |n: u8, name: &str| crate::import::Repository {
        id: id(n),
        name: name.into(),
        default_branch: "main".into(),
    };
    let taken = |n: u8, name: &str, other: u8| ValidationError::RepositoryNameTaken {
        repository: id(n),
        name: name.into(),
        other: id(other),
    };
    let mut r = imported();
    let mut doc = Import::new();
    // The tracker's repository 1 is example/code; importing it again is no clash.
    doc.repositories = vec![repository(1, "example/code")];
    apply(&doc, &mut r, by(USER, 2)).unwrap();
    // Another record taking its name, in another case, is.
    doc.repositories = vec![repository(2, "Example/Code")];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("a taken name accepted")
    };
    assert_eq!(errors, [taken(2, "Example/Code", 1)]);
    // Two of the document's own records clash with each other.
    doc.repositories = vec![repository(2, "me/garden"), repository(3, "ME/garden")];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 4)) else {
        panic!("a name used twice accepted")
    };
    assert_eq!(errors, [taken(2, "me/garden", 3), taken(3, "ME/garden", 2)]);
    // A document that renames 1 frees its old name for another record in the same import.
    doc.repositories = vec![repository(1, "example/old"), repository(2, "example/code")];
    apply(&doc, &mut r, by(USER, 5)).unwrap();
}

#[test]
fn done_on_a_parent_is_refused_and_done_work_set_back_is_reopened() {
    let mut r = imported();
    model::set_state(&mut r, &id(11), ItemState::Done, by(USER, 2)).unwrap();
    // A parent reads done from its leaves, so done on one is refused.
    let mut doc = Import::new();
    doc.items = vec![item(10, Some(Planned::Done), None)];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("done on a parent accepted")
    };
    assert_eq!(errors, [ValidationError::DoneParent(id(10))]);
    // Done work set back is reopened, as `item reopen` does: open again, and nothing else.
    doc.items = vec![item(11, Some(Planned::Backlog), Some(10))];
    let report = apply(&doc, &mut r, by(USER, 5)).unwrap();
    assert_eq!(model(&r).items[&id(11)].state, Value::One(ItemState::Backlog));
    let row = report.changes.iter().find(|c| c.name == Name::Id { id: id(11) }).unwrap();
    assert_eq!(row.fields.keys().map(String::as_str).collect::<Vec<_>>(), ["state"]);
    // Reported as reads show it, though stored as `backlog`.
    assert_eq!(
        (&row.fields["state"].from, &row.fields["state"].to),
        (&json!("done"), &json!("open"))
    );
}

#[test]
fn an_import_that_says_canceled_cancels_open_work_and_refuses_done_work() {
    let mut r = imported();
    let mut doc = Import::new();
    doc.items = vec![item(11, Some(Planned::Canceled), Some(10))];
    apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!(model(&r).items[&id(11)].state, Value::One(ItemState::Canceled));
    // Done work is reopened before it is canceled.
    doc.items = vec![item(12, Some(Planned::Canceled), None)];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 4)) else {
        panic!("canceled on done work accepted")
    };
    assert_eq!(errors, [ValidationError::CanceledDone(id(12))]);
}

#[test]
fn a_deleted_thread_items_still_name_is_unknown_to_an_import() {
    let mut r = imported();
    // 20 emptied and deleted, while another replica's write still puts 11 in it.
    for n in [11, 12] {
        let out = ScopeEdit::Member { item: id(n), thread: None };
        crate::scope::edit(&mut r, &out, by(USER, 2)).unwrap();
    }
    assert!(crate::delete::delete_thread(&mut r, &id(20), by(USER, 3)).unwrap());
    let key = model::item_key(&id(11)).unwrap();
    r.write(&key, field::THREAD, USER, 4, Some(json!(id(20)))).unwrap();
    let mut doc = Import::new();
    let mut next = item(30, None, Some(10));
    next.thread = Some(Some(id(20)));
    doc.items.push(next);
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 5)) else {
        panic!("a deleted thread accepted")
    };
    assert_eq!(
        errors,
        [ValidationError::UnknownReference {
            at: format!("item {}, thread", id(30)),
            kind: KeyKind::Thread,
            id: id(20)
        }]
    );
}

#[test]
fn evidence_naming_a_tracker_repository_that_cant_be_verified_is_refused_naming_what_it_lacks() {
    let mut r = imported();
    let repository = crate::state::Key::new(crate::schema::kind::REPOSITORY, &id(1)).unwrap();
    let branch = crate::schema::repository::DEFAULT_BRANCH;
    model::set::<String>(&mut r, &repository, branch, None, by(USER, 2)).unwrap();
    let mut doc = valid();
    doc.repositories.clear();
    doc.items[2].evidence.as_mut().unwrap().commit = Some(format!("{}@def5678", id(1)));
    let refused = apply(&doc, &mut r, by(USER, 3)).unwrap_err();
    let incomplete = ValidationError::IncompleteRepository {
        item: id(12),
        repository: id(1),
        missing: vec!["default-branch".into()],
    };
    assert!(
        matches!(&refused, ApplyError::Invalid(errors) if errors == &[incomplete]),
        "{refused:?}"
    );
}

#[test]
fn an_unmapped_refusal_is_internal_even_with_mapped_violations() {
    let violations = vec![
        Violation { change: None, at: None, rule: Rule::Deleted(Record::Item("gone".into())) },
        Violation {
            change: None,
            at: None,
            rule: Rule::WaitsForCycle(vec!["a".into(), "b".into()]),
        },
        Violation {
            change: None,
            at: None,
            rule: Rule::EmptyRecord(Record::Repository("r".into())),
        },
    ];
    let error = refused(&Import::new(), &Model::default(), &violations);
    assert_eq!(error, ApplyError::Internal(violations.clone()));
    for violation in violations {
        assert!(error.to_string().contains(&violation.to_string()));
    }
}

#[test]
fn evidence_names_its_repository_by_owner_and_name_or_by_id() {
    let named = |commit: &str| {
        let mut doc = valid();
        doc.items[2].evidence =
            Some(crate::import::Evidence { commit: Some(commit.to_owned()), text: None });
        doc
    };
    let evidence = |r: &Replica| {
        let found = model(r).items[&id(12)].evidence.one().cloned().unwrap();
        (found.repository, found.commit)
    };
    // By the document's own repository, in any case.
    let mut r = replica();
    assert_eq!(named("Example/Code@abc1234").validate(), vec![]);
    apply(&named("Example/Code@abc1234"), &mut r, by(USER, 1)).unwrap();
    assert_eq!(evidence(&r), (Some(id(1)), Some("abc1234".into())));
    // By the tracker's, once the document no longer lists it, and by id as before.
    let mut later = named("example/code@def5678");
    later.repositories.clear();
    apply(&later, &mut r, by(USER, 2)).unwrap();
    assert_eq!(evidence(&r), (Some(id(1)), Some("def5678".into())));
    let mut r = replica();
    apply(&named(&format!("{}@abc1234", id(1))), &mut r, by(USER, 1)).unwrap();
    assert_eq!(evidence(&r), (Some(id(1)), Some("abc1234".into())));
    // A name no repository has is an unknown reference.
    let unknown = apply(&named("nobody/here@abc1234"), &mut replica(), by(USER, 1)).unwrap_err();
    assert!(
        matches!(&unknown, ApplyError::Invalid(errors)
        if matches!(&errors[..], [ValidationError::UnknownReference { kind: KeyKind::Repository, id, .. }]
            if id == "nobody/here")),
        "{unknown:?}"
    );
}

#[test]
fn new_children_join_their_siblings_thread_and_moved_work_never_changes_a_thread() {
    let mut r = imported();
    let mut doc = valid();
    doc.items.push(item(13, None, Some(10)));
    doc.items.push(item(14, None, Some(9)));
    let report = apply(&doc, &mut r, by(USER, 2)).unwrap();
    // 11 and 12 share thread 20, so the new child 13 follows them; 14 sits under the initiative, whose other
    // children are no siblings to follow.
    assert_eq!(report.placed, [Inherited { item: id(13), thread: id(20) }]);
    assert_eq!(model(&r).thread_of(&id(14)), None);
    // Moving 14 under 10 puts it in no thread, though the siblings share one.
    doc.items[4].parent = Some(id(10));
    let report = apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!(report.placed, Vec::new());
    assert_eq!(model(&r).thread_of(&id(14)), None);
    // Siblings in different threads leave a new child to the document, which says so in its own terms.
    doc.items[4].thread = Some(Some(id(21)));
    doc.threads.push(crate::import::Thread { id: id(21), title: "Other".into(), notes: None });
    apply(&doc, &mut r, by(USER, 4)).unwrap();
    doc.items.push(item(15, None, Some(10)));
    let before = r.state.clone();
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 5)) else {
        panic!("a new child among siblings in different threads is refused");
    };
    assert_eq!(errors, [ValidationError::ThreadUnclear { item: id(15), parent: id(10) }]);
    assert_eq!(r.state, before);
    // Given a thread, it goes where the document says.
    doc.items[5].thread = Some(Some(id(21)));
    apply(&doc, &mut r, by(USER, 6)).unwrap();
    assert_eq!(model(&r).thread_of(&id(15)), Some(id(21).as_str()));
}

#[test]
fn every_new_item_needs_a_parent_and_an_existing_one_keeps_its_own() {
    let mut r = imported();
    let mut doc = Import::new();
    doc.items = vec![item(40, None, None), item(11, None, None)];
    let before = r.state.clone();
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 2)) else {
        panic!("a new item with no parent accepted")
    };
    // Only the new item 40 is refused: 11 is in the tracker, and keeps the parent it holds.
    assert_eq!(errors, [ValidationError::ParentRequired(id(40))]);
    assert_eq!(r.state, before);
    doc.items[0].parent = Some(id(9));
    apply(&doc, &mut r, by(USER, 3)).unwrap();
    let m = model(&r);
    assert_eq!(
        (m.initiative_of(&id(40)), m.initiative_of(&id(11))),
        (Some(id(9).as_str()), Some(id(9).as_str()))
    );
}

#[test]
fn an_import_that_gives_an_item_and_an_initiative_one_id_is_refused_against_the_tracker_too() {
    let mut r = imported();
    // The tracker's initiative 9 can't be made an item by a document that names it so.
    let mut doc = Import::new();
    doc.items = vec![item(9, None, Some(10))];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 2)) else {
        panic!("an item taking an initiative's id accepted")
    };
    assert_eq!(errors, [ValidationError::SharedId(id(9))]);
    // And the other way round: initiative 10 over the tracker's item.
    let mut doc = Import::new();
    doc.initiatives =
        vec![crate::import::Initiative { id: id(10), title: "I".into(), notes: None }];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("an initiative taking an item's id accepted")
    };
    assert_eq!(errors, [ValidationError::SharedId(id(10))]);
}

#[test]
fn a_title_clash_is_an_invalid_import_naming_the_plan_titles_and_the_initiative() {
    let titled = |n: u8, title: &str, parent: u8| {
        let mut one = item(n, None, Some(parent));
        one.title = title.into();
        one
    };
    let clash = |a: u8, b: u8, title: &str, initiative: Option<&str>| ValidationError::TitleTaken {
        item: id(a),
        other: id(b),
        title: title.into(),
        initiative: initiative.map(Into::into),
    };
    // A plan that clashes with itself, named once, in either order of its items.
    let mut r = replica();
    let mut doc = Import::new();
    doc.initiatives =
        vec![crate::import::Initiative { id: id(9), title: "Garden".into(), notes: None }];
    doc.items = vec![titled(21, "Sow", 9), titled(20, "Sow", 9)];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 1)) else {
        panic!("a plan that clashes with itself accepted")
    };
    assert_eq!(errors, [clash(20, 21, "Sow", Some("Garden"))]);
    assert!(model(&r).items.is_empty(), "nothing is written");
    // A dry run says the same.
    let Err(ApplyError::Invalid(previewed)) = preview(&doc, &r, by(USER, 1)) else {
        panic!("a dry run accepted it")
    };
    assert_eq!(previewed, errors);

    // A plan that clashes with an item the tracker holds, whether it adds or renames one.
    let mut r = replica();
    doc.items = vec![titled(20, "Sow", 9)];
    apply(&doc, &mut r, by(USER, 2)).unwrap();
    doc.items = vec![titled(22, "Sow", 9)];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 3)) else {
        panic!("a plan that clashes with the tracker accepted")
    };
    assert_eq!(errors, [clash(20, 22, "Sow", Some("Garden"))]);
    doc.items = vec![titled(23, "Reap", 9)];
    apply(&doc, &mut r, by(USER, 4)).unwrap();
    doc.items = vec![titled(23, "Sow", 9)];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 4)) else {
        panic!("a rename onto the tracker's title accepted")
    };
    assert_eq!(errors, [clash(20, 23, "Sow", Some("Garden"))]);
    // The same title in another initiative is no clash.
    doc.initiatives.push(crate::import::Initiative {
        id: id(8),
        title: "Orchard".into(),
        notes: None,
    });
    doc.items = vec![titled(24, "Sow", 8)];
    apply(&doc, &mut r, by(USER, 5)).unwrap();
}

#[test]
fn a_null_thread_keeps_a_new_item_out_of_every_thread_and_clears_an_existing_one() {
    let mut r = imported();
    let mut doc = valid();
    // 11 and 12 share thread 20; a new child saying `null` stays loose instead of joining them.
    doc.items.push(item(13, None, Some(10)));
    doc.items[3].thread = Some(None);
    let report = apply(&doc, &mut r, by(USER, 2)).unwrap();
    assert_eq!(report.placed, Vec::new());
    assert_eq!(model(&r).thread_of(&id(13)), None);
    // Siblings in different threads or none make a new child unclear, unless it says `null`.
    doc.items[2].thread = Some(Some(id(21)));
    doc.threads.push(crate::import::Thread { id: id(21), title: "Other".into(), notes: None });
    doc.items.push(item(14, None, Some(10)));
    assert!(matches!(
        super::preview(&doc, &r, by(USER, 3)),
        Err(ApplyError::Invalid(errors))
            if errors == [ValidationError::ThreadUnclear { item: id(14), parent: id(10) }]
    ));
    doc.items[4].thread = Some(None);
    apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!(model(&r).thread_of(&id(14)), None);
    // On an item in a real thread, `null` clears it.
    assert_eq!(model(&r).thread_of(&id(11)), Some(id(20).as_str()));
    doc.items[1].thread = Some(None);
    let report = apply(&doc, &mut r, by(USER, 4)).unwrap();
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].fields["thread"].to, json!(null));
    assert_eq!(model(&r).thread_of(&id(11)), None);
}

#[test]
fn a_null_thread_on_an_item_with_none_or_only_its_own_thread_of_one_changes_nothing() {
    let mut r = imported();
    let mut doc = valid();
    doc.items.push(item(13, None, Some(9)));
    doc.items[3].thread = Some(None);
    apply(&doc, &mut r, by(USER, 2)).unwrap();
    assert_eq!(super::preview(&doc, &r, by(USER, 3)).unwrap().changes, vec![]);
    // A claim puts a loose task in its own thread of one: still no thread as the document says it.
    let own = crate::ids::one(&id(13));
    model::put(&mut r, &model::item_key(&id(13)).unwrap(), field::THREAD, Some(&own), by(USER, 3))
        .unwrap();
    assert_eq!(super::preview(&doc, &r, by(USER, 4)).unwrap().changes, vec![]);
    // Another item's thread of one is a thread like any other, which `null` leaves.
    let other = crate::ids::one(&id(14));
    doc.items.push(item(14, None, Some(9)));
    doc.items[4].thread = Some(None);
    apply(&doc, &mut r, by(USER, 5)).unwrap();
    model::put(
        &mut r,
        &model::item_key(&id(13)).unwrap(),
        field::THREAD,
        Some(&other),
        by(USER, 6),
    )
    .unwrap();
    assert_eq!(super::preview(&doc, &r, by(USER, 7)).unwrap().changes.len(), 1);
}
