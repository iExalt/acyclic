use crate::archive;
use crate::claim;
use crate::export;
use crate::model::{By, ItemState, add_item, item_key, link, set_parent, set_state};
use crate::needs::{Entry, needs_you};
use crate::ready::{Ancestry, Readiness};
use crate::schema::kind;
use crate::scope::{ScopeEdit, edit};
use crate::state::{Millis, ReplicaId, State};

use super::*;

const NOW: Millis = 10_000;

fn user(at: Millis) -> By<'static> {
    By { actor: USER, at }
}

fn agent(at: Millis) -> By<'static> {
    By { actor: "session/mac", at }
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
        Err(DeleteError::InUse { reasons, .. }) => reasons
            .into_iter()
            .map(|reason| serde_json::to_value(reason).unwrap()["why"].as_str().unwrap().to_owned())
            .collect(),
        other => vec![format!("{other:?}")],
    }
}

#[test]
fn only_an_item_nothing_refers_to_is_deleted() {
    let mut r = with_items(&["parent", "kid", "a", "b", "done", "claimed", "free"]);
    set_parent(&mut r, "kid", Some("parent"), user(2)).unwrap();
    link(&mut r, "a", "b", crate::model::LinkKind::Blocks, user(2)).unwrap();
    set_state(&mut r, "done", ItemState::Done, user(2)).unwrap();
    let held = Key::new(kind::ITEM, "claimed").unwrap();
    claim::claim(&mut r, &held, "session/pi", NOW, Some(60_000), 480 * 60_000).unwrap();
    assert_eq!(refusal(&mut r, "parent", user(NOW)), ["has-children"]);
    // A link either way, and a link of any kind.
    assert_eq!(refusal(&mut r, "a", user(NOW)), ["linked"]);
    assert_eq!(refusal(&mut r, "b", user(NOW)), ["linked"]);
    assert_eq!(refusal(&mut r, "done", user(NOW)), ["finished"]);
    assert_eq!(refusal(&mut r, "claimed", user(NOW)), ["claimed"]);
    // The refusal says why in one line.
    let message = delete_item(&mut r, "parent", user(NOW)).unwrap_err().to_string();
    assert_eq!(message, "parent can't be deleted: it has children (kid)");
    let message = delete_item(&mut r, "a", user(NOW)).unwrap_err().to_string();
    assert_eq!(message, "a can't be deleted: a waits-for b");
    // The details name a link's kind as the command line does.
    let Err(DeleteError::InUse { reasons, .. }) = delete_item(&mut r, "a", user(NOW)) else {
        panic!("in use")
    };
    assert_eq!(serde_json::to_value(&reasons).unwrap()[0]["kind"], "waits-for");
    assert!(matches!(
        delete_item(&mut r, "nope", user(NOW)),
        Err(DeleteError::Write(WriteError::NoItem(_)))
    ));
    // A claim that has lapsed no longer holds it.
    assert_eq!(delete_item(&mut r, "claimed", user(NOW + 61_000)), Ok(true));
    assert_eq!(delete_item(&mut r, "free", user(NOW)), Ok(true));
    let before = r.changes();
    assert_eq!(delete_item(&mut r, "free", user(NOW)), Ok(false), "a repeat changes nothing");
    assert_eq!(r.changes(), before);
}

#[test]
fn in_an_approved_thread_only_the_user_deletes() {
    let mut r = with_items(&["a", "b"]);
    for id in ["a", "b"] {
        edit(&mut r, &ScopeEdit::Member { item: id.into(), thread: Some("t".into()) }, user(2))
            .unwrap();
    }
    crate::scope::approve(&mut r, "t", true, user(3)).unwrap();
    assert!(matches!(
        delete_item(&mut r, "a", agent(NOW)),
        Err(DeleteError::NotAllowed { thread, .. }) if thread == "t"
    ));
    assert_eq!(delete_item(&mut r, "a", user(NOW)), Ok(true));
    // Revoked, it is open to agents again.
    crate::scope::approve(&mut r, "t", false, user(4)).unwrap();
    assert_eq!(delete_item(&mut r, "b", agent(NOW)), Ok(true));
}

#[test]
fn a_deleted_item_is_missing_to_every_checked_write_and_its_id_is_retired() {
    let mut r = with_items(&["gone", "other"]);
    delete_item(&mut r, "gone", user(2)).unwrap();
    let key = item_key("gone").unwrap();
    assert!(r.state().entity(&key).is_some(), "the tombstone is state, so it joins");
    assert!(!Model::read(r.state()).unwrap().items.contains_key("gone"));
    let missing = |result: Result<(), WriteError>| matches!(result, Err(WriteError::NoItem(_)));
    let kind = crate::model::LinkKind::Blocks;
    assert!(missing(link(&mut r, "gone", "other", kind, user(3)).map(|_| ())));
    assert!(missing(link(&mut r, "other", "gone", kind, user(3)).map(|_| ())));
    assert!(missing(set_parent(&mut r, "gone", Some("other"), user(3))));
    assert!(missing(set_parent(&mut r, "other", Some("gone"), user(3))));
    assert!(missing(set_state(&mut r, "gone", ItemState::Parked, user(3))));
    let member = ScopeEdit::Member { item: "gone".into(), thread: Some("t".into()) };
    assert!(matches!(
        edit(&mut r, &member, user(3)),
        Err(crate::scope::ScopeError::Write(WriteError::NoItem(_)))
    ));
    assert!(matches!(
        archive::reopen(&mut r, "gone", user(3)),
        Err(archive::ArchiveError::Write(WriteError::NoItem(_)))
    ));
    assert!(matches!(
        archive::archive(&mut r, &Unmapped, "gone", user(3)),
        Err(archive::ArchiveError::Write(WriteError::NoItem(_)))
    ));
    assert!(matches!(add_item(&mut r, "gone", "Again", user(3)), Err(WriteError::Deleted(_))));
}

struct Unmapped;

impl Ancestry for Unmapped {
    fn merged(&self, _: &model::Repository, _: &str) -> Result<bool, String> {
        Err("no clone".into())
    }
}

/// What a reader sees of `r`: its needs, with every other reader run on it to show none of them fails.
fn needs_of(r: &Replica) -> Vec<Entry> {
    let model = Model::read(r.state()).unwrap();
    let readiness = Readiness::new(r.state(), &model, NOW, &Unmapped);
    let _ = readiness.ready(false).unwrap();
    for id in model.items.keys() {
        let _ = readiness.of(id).unwrap();
        let _ = (model.role(id), model.ancestors(id));
    }
    let view = export::view(&model, &export::Scope::default()).unwrap();
    let _ = (export::dot(&model, &view), export::mermaid(&model, &view));
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
    link(&mut pi, "other", "gone", crate::model::LinkKind::Blocks, user(2)).unwrap();
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
        let dangling: Vec<(String, String, String)> = needs_of(joined)
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Dangling { id, field, target, .. } if target == "gone" => {
                    Some((id, field, target))
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            dangling,
            [
                ("kid".to_owned(), "parent".to_owned(), "gone".to_owned()),
                ("other-gone".to_owned(), "end".to_owned(), "gone".to_owned()),
            ],
            "the child's parent and the link's end"
        );
    }
    // The same state in either order.
    assert_eq!(one.state().to_files(), other.state().to_files());
    assert_eq!(needs_of(&one), needs_of(&other));
}
