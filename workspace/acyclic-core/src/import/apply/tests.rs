use serde_json::json;

use super::*;
use crate::archive::{ArchiveError, Reason};
use crate::import::State as Planned;
use crate::import::tests::{id, item, valid};
use crate::model::{self, ItemState, Repository, Value};
use crate::ready::Ancestry;
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

struct Merged;

impl Ancestry for Merged {
    fn merged(&self, _: &Repository, _: &str) -> Result<bool, String> {
        Ok(true)
    }
}

struct Unmerged;

impl Ancestry for Unmerged {
    fn merged(&self, _: &Repository, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

fn apply(doc: &Import, replica: &mut Replica, by: By<'_>) -> Result<Report, ApplyError> {
    super::apply(doc, replica, by, &Merged)
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
    // Each record is a creation with the fields it was given; a set shows whole, and a link names its ends.
    let changes = serde_json::to_value(&report.changes).unwrap();
    let find = |entity: &str, n: u8| {
        changes
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["entity"] == entity && c["id"] == id(n))
            .unwrap()
    };
    assert_eq!(changes.as_array().unwrap().len(), 1 + 1 + 3 + 1);
    assert_eq!(find("repository", 1)["action"], "create");
    assert_eq!(find("repository", 1)["fields"]["owner"], json!({"from": null, "to": "example"}));
    assert_eq!(find("item", 11)["fields"]["repositories"], json!({"from": [], "to": [id(1)]}));
    assert_eq!(find("item", 11)["fields"]["parent"], json!({"from": null, "to": id(10)}));
    assert_eq!(find("item", 12)["fields"]["thread"], json!({"from": null, "to": id(20)}));
    let link = changes.as_array().unwrap().iter().find(|c| c["entity"] == "link").unwrap();
    assert_eq!(
        (&link["from"], &link["kind"], &link["action"]),
        (&json!(id(11)), &json!("waits-for"), &json!("create"))
    );
    let m = model(&r);
    let a = &m.items[&id(11)];
    assert_eq!(a.parent, Value::One(id(10)));
    assert_eq!(a.repositories, [id(1)].into());
    // A new item left without a state is in the backlog.
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
        (left.state, left.thread, left.repositories, left.waits_for, left.parent, left.evidence) =
            (None, None, None, None, None, None);
    }
    let report = apply(&doc, &mut r, by(USER, 3)).unwrap();
    assert_eq!(
        serde_json::to_value(&report.changes).unwrap(),
        json!([{"entity": "item", "id": id(11), "action": "update",
                "fields": {"title": {"from": "Item 11", "to": "Renamed"}}}])
    );
    let m = model(&r);
    let (kept, done) = (&m.items[&id(11)], &m.items[&id(12)]);
    assert_eq!((kept.parent.one(), kept.repositories.len()), (Some(&id(10)), 1));
    assert_eq!(m.thread_of(&id(11)), Some(id(20).as_str()));
    assert_eq!((done.state.one(), done.done_at), (Some(&ItemState::Done), done_at));
    assert!(done.evidence.one().is_some());
    assert_eq!(m.links.len(), 1);
}

#[test]
fn a_list_given_replaces_the_records_own() {
    let mut r = imported();
    let mut doc = valid();
    doc.items.push(item(13, None, None));
    doc.items[1].waits_for = Some(vec![id(13)]);
    doc.items[1].repositories = Some(vec![]);
    let report = apply(&doc, &mut r, by(USER, 2)).unwrap();
    let m = model(&r);
    assert_eq!(m.links, [model::Link { from: id(11), to: id(13) }]);
    assert!(m.items[&id(11)].repositories.is_empty());
    let changes = serde_json::to_value(&report.changes).unwrap();
    let removed: Vec<&Json> =
        changes.as_array().unwrap().iter().filter(|c| c["action"] == "remove").collect();
    assert_eq!(
        removed,
        [
            &json!({"entity": "link", "from": id(11), "kind": "waits-for", "to": id(12), "action": "remove", "fields": {}})
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
    next.thread = Some(id(20));
    next.waits_for = Some(vec![id(12)]);
    next.repositories = Some(vec![id(1)]);
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
fn evidence_and_archiving_are_judged_by_the_state_each_item_ends_with() {
    let mut r = imported();
    // 12 is done in the tracker, so evidence and archiving on it, its state left out, are fine.
    let mut doc = crate::import::Import::new();
    let mut done = item(12, None, None);
    done.evidence = Some(crate::import::Evidence { commit: None, text: Some("again".into()) });
    done.archived = true;
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
        doc.items.push(item(n, None, None));
    }
    // 10 and 11 under each other; 11 and 12 waiting for each other; 13 and 14 likewise.
    doc.items[0].parent = Some(id(11));
    doc.items[2].waits_for = Some(vec![id(11)]);
    doc.items[3].waits_for = Some(vec![id(14)]);
    doc.items[4].waits_for = Some(vec![id(13)]);
    let Err(ApplyError::Cycles(mut cycles)) = apply(&doc, &mut replica(), by(USER, 1)) else {
        panic!("cycles accepted")
    };
    cycles.sort();
    assert_eq!(cycles, [[id(10), id(11)], [id(11), id(12)], [id(13), id(14)]]);
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
}

#[test]
fn a_deleted_thread_is_refused_as_a_deleted_item_is() {
    let mut r = replica();
    let mut doc = Import::new();
    doc.threads = vec![crate::import::Thread {
        id: id(21),
        title: "Gone".into(),
        brief: None,
        repositories: None,
    }];
    apply(&doc, &mut r, by(USER, 1)).unwrap();
    assert!(crate::delete::delete_thread(&mut r, &id(21), by(USER, 2)).unwrap());
    let err = apply(&doc, &mut r, by(USER, 3)).unwrap_err();
    assert_eq!(err, ApplyError::Write(WriteError::Deleted(id(21))));
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
fn new_work_under_an_archived_root_is_refused_and_done_work_set_back_is_reopened() {
    let mut r = imported();
    // 10 and its subtree done, then archived.
    for n in [10, 11] {
        model::set_state(&mut r, &id(n), ItemState::Done, by(USER, 2)).unwrap();
    }
    model::put(
        &mut r,
        &model::item_key(&id(10)).unwrap(),
        field::ARCHIVED,
        Some(&true),
        by(USER, 3),
    )
    .unwrap();
    // New work under the archived root would be hidden.
    let mut doc = Import::new();
    doc.items = vec![item(30, None, Some(10)), item(31, None, Some(11))];
    let Err(ApplyError::Invalid(errors)) = apply(&doc, &mut r, by(USER, 4)) else {
        panic!("open work under an archived root accepted")
    };
    assert_eq!(errors, [ValidationError::ArchivedNotDone(id(10))]);
    // Done work set back is reopened, as `item reopen` does: flagged, and out of the archive above it.
    doc.items = vec![item(11, Some(Planned::Backlog), Some(10))];
    let report = apply(&doc, &mut r, by(USER, 5)).unwrap();
    let m = model(&r);
    assert_eq!(m.items[&id(11)].reopened, Value::One(model::Reopened { archived: true }));
    assert_eq!(m.items[&id(10)].archived, Value::None);
    let fields = |n: u8| -> Vec<&str> {
        let row = report.changes.iter().find(|c| c.name == Name::Id { id: id(n) }).unwrap();
        row.fields.keys().map(String::as_str).collect()
    };
    assert_eq!((fields(10), fields(11)), (vec!["archived"], vec!["reopened", "state"]));
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
    let mut next = item(30, None, None);
    next.thread = Some(id(20));
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

fn archive_import(clear_dependent: bool) -> Import {
    let mut doc = Import::new();
    let mut root = item(12, None, None);
    root.archived = true;
    doc.items.push(root);
    if clear_dependent {
        let mut dependent = item(11, None, None);
        dependent.waits_for = Some(vec![]);
        doc.items.push(dependent);
    }
    doc
}

#[test]
fn import_archive_uses_the_shared_grace_merge_and_external_dependent_rules() {
    let mut r = imported();
    let doc = archive_import(true);
    let preview = super::preview(&doc, &r, by(USER, 2), &Merged).unwrap_err();
    let before = r.state.clone();
    let grace = apply(&doc, &mut r, by(USER, 2)).unwrap_err();
    assert_eq!(preview, grace);
    assert_eq!(r.state, before, "a refused archive writes nothing");
    assert!(matches!(grace, ApplyError::Archive(ArchiveError::Ineligible { reasons, .. })
        if reasons.iter().any(|reason| matches!(reason, Reason::Waiting { .. }))));

    let mut r = imported();
    let dependent =
        apply(&archive_import(false), &mut r, by(USER, 31 * 24 * 60 * 60 * 1000)).unwrap_err();
    assert!(matches!(dependent, ApplyError::Archive(ArchiveError::Ineligible { reasons, .. })
        if reasons.iter().any(|reason| matches!(reason, Reason::LiveDependent { .. }))));

    let mut r = imported();
    let doc = archive_import(true);
    let unmerged =
        super::apply(&doc, &mut r, by(USER, 31 * 24 * 60 * 60 * 1000), &Unmerged).unwrap_err();
    assert!(matches!(unmerged, ApplyError::Archive(ArchiveError::Ineligible { reasons, .. })
        if reasons.iter().any(|reason| matches!(reason, Reason::Unmerged { .. }))));
}

#[test]
fn imported_archive_candidates_are_order_independent_and_preview_matches_apply() {
    let make_replica = || {
        let mut r = replica();
        let mut doc = valid();
        let mut parent = item(13, Some(Planned::Done), None);
        parent.evidence =
            Some(crate::import::Evidence { commit: None, text: Some("finished".into()) });
        let mut child = item(14, Some(Planned::Done), Some(13));
        child.evidence =
            Some(crate::import::Evidence { commit: None, text: Some("finished".into()) });
        doc.items.extend([parent, child]);
        apply(&doc, &mut r, by(USER, 1)).unwrap();
        r
    };
    let archived = |order: [u8; 2]| {
        let mut doc = Import::new();
        for n in order {
            let mut item = item(n, None, None);
            item.archived = true;
            doc.items.push(item);
        }
        doc
    };
    let now = 31 * 24 * 60 * 60 * 1000;
    let mut forward = make_replica();
    let forward_doc = archived([13, 14]);
    let preview = super::preview(&forward_doc, &forward, by(USER, now), &Merged).unwrap();
    let applied = super::apply(&forward_doc, &mut forward, by(USER, now), &Merged).unwrap();
    assert_eq!(preview, applied);

    let mut reverse = make_replica();
    super::preview(&archived([14, 13]), &reverse, by(USER, now), &Merged).unwrap();
    super::apply(&archived([14, 13]), &mut reverse, by(USER, now), &Merged).unwrap();
    for id in [id(13), id(14)] {
        assert!(model(&forward).items[&id].is_archived_root());
        assert!(model(&reverse).items[&id].is_archived_root());
    }
}
