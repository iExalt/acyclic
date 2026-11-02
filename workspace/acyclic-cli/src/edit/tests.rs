use acyclic_core::scope::USER;
use acyclic_core::state::{ReplicaId, State};

use super::*;

fn replica() -> Replica {
    Replica::open(ReplicaId::new("mac").unwrap(), State::new())
}

const AGENT: By<'static> = By { actor: "session/mac", at: 2 };
const USER_BY: By<'static> = By { actor: USER, at: 1 };

fn titled(title: &str) -> ItemFields {
    ItemFields { title: Some(title.to_owned()), ..ItemFields::default() }
}

fn code<T>(result: Result<T, CliError>) -> &'static str {
    result.map_or_else(|err| err.code, |_| "ok")
}

fn item(replica: &Replica, id: &str) -> acyclic_core::model::Item {
    Model::read(replica.state()).unwrap().items.remove(id).unwrap()
}

#[test]
fn an_add_repeated_changes_nothing_and_one_that_differs_is_refused() {
    let mut r = replica();
    let full = ItemFields {
        notes: Change::Set("n".into()),
        size: Change::Set(Size::M),
        state: Some(ItemState::Parked),
        areas: Delta { add: vec!["x".into()], remove: vec![] },
        ..titled("A")
    };
    assert_eq!(code(add_item(&mut r, "a", &full, AGENT)), "ok");
    let held = item(&r, "a");
    assert_eq!(
        (held.notes.one().map(String::as_str), held.state.one(), held.areas.len()),
        (Some("n"), Some(&ItemState::Parked), 1),
        "add takes every field edit does"
    );
    let before = r.changes();
    assert_eq!(code(add_item(&mut r, "a", &full, AGENT)), "ok");
    assert_eq!(r.changes(), before, "an identical repeat writes nothing");
    // A field it names that differs, or a set element the item lacks, refuses the add, and nothing is written.
    for (what, other) in [
        ("title", titled("B")),
        ("notes", ItemFields { notes: Change::Set("m".into()), ..titled("A") }),
        ("state", ItemFields { state: Some(ItemState::Backlog), ..titled("A") }),
        (
            "area",
            ItemFields { areas: Delta { add: vec!["y".into()], remove: vec![] }, ..titled("A") },
        ),
    ] {
        assert_eq!(code(add_item(&mut r, "a", &other, AGENT)), "already-exists", "{what}");
    }
    assert_eq!(r.changes(), before);
    // The refusal names the flags that differ and the command that changes them.
    let notes = ItemFields { notes: Change::Set("m".into()), ..titled("A") };
    let refused = add_item(&mut r, "a", &notes, AGENT).unwrap_err();
    assert!(
        refused.message.ends_with("; change it with `acyclic item edit a --notes …`"),
        "{refused}"
    );
}

#[test]
fn an_edit_refuses_what_is_unknown_and_clears_what_it_is_told_to() {
    let mut r = replica();
    assert_eq!(code(edit_item(&mut r, "a", &titled("A"), AGENT)), "not-found");
    let with_notes = ItemFields { notes: Change::Set("n".into()), ..titled("A") };
    add_item(&mut r, "a", &with_notes, AGENT).unwrap();
    for (what, fields) in [
        ("thread", ItemFields { thread: Change::Set("nope".into()), ..ItemFields::default() }),
        (
            "repository",
            ItemFields {
                repositories: Delta { add: vec!["nope".into()], remove: vec![] },
                ..ItemFields::default()
            },
        ),
        ("parent", ItemFields { parent: Change::Set("nope".into()), ..ItemFields::default() }),
    ] {
        assert_eq!(code(edit_item(&mut r, "a", &fields, AGENT)), "not-found", "{what}");
    }
    let clear = ItemFields { notes: Change::Clear, ..ItemFields::default() };
    edit_item(&mut r, "a", &clear, AGENT).unwrap();
    assert_eq!(item(&r, "a").notes.one(), None);
    let before = r.changes();
    edit_item(&mut r, "a", &clear, AGENT).unwrap();
    assert_eq!(r.changes(), before, "clearing what is clear writes nothing");
}

#[test]
fn an_agents_membership_and_repository_edits_apply_and_a_repeat_writes_nothing() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let repository = RepositoryFields { owner: Some("o".into()), ..RepositoryFields::default() };
    crate::edit::add_repository(&mut r, "r", &repository, USER_BY).unwrap();
    let in_t = ItemFields { thread: Change::Set("t".into()), ..titled("A") };
    add_item(&mut r, "a", &in_t, USER_BY).unwrap();
    let out_of_t = ItemFields {
        thread: Change::Clear,
        repositories: Delta { add: vec!["r".into()], remove: vec![] },
        ..ItemFields::default()
    };
    edit_item(&mut r, "a", &out_of_t, AGENT).unwrap();
    let held = item(&r, "a");
    assert_eq!((held.thread.one(), held.repositories.len()), (None, 1));
    let before = r.changes();
    edit_item(&mut r, "a", &out_of_t, AGENT).unwrap();
    assert_eq!(r.changes(), before);
}

#[test]
fn adding_a_thread_or_repository_again_needs_the_same_fields() {
    let mut r = replica();
    let thread = ThreadFields {
        title: Some("T".into()),
        brief: Change::Set("B".into()),
        ..ThreadFields::default()
    };
    add_thread(&mut r, "t", &thread, AGENT).unwrap();
    let before = r.changes();
    add_thread(&mut r, "t", &thread, AGENT).unwrap();
    assert_eq!(r.changes(), before);
    let other = ThreadFields { title: Some("Other".into()), ..ThreadFields::default() };
    assert_eq!(code(add_thread(&mut r, "t", &other, AGENT)), "already-exists");
    assert_eq!(code(edit_thread(&mut r, "ghost", &other, AGENT)), "not-found");
    let repo = RepositoryFields {
        owner: Some("o".into()),
        name: Some("n".into()),
        default_branch: Some("main".into()),
    };
    add_repository(&mut r, "r", &repo, AGENT).unwrap();
    let before = r.changes();
    add_repository(&mut r, "r", &repo, AGENT).unwrap();
    assert_eq!(r.changes(), before);
    let trunk = RepositoryFields { default_branch: Some("trunk".into()), ..repo };
    assert_eq!(add_repository(&mut r, "r", &trunk, AGENT).unwrap_err().code, "already-exists");
    assert_eq!(edit_repository(&mut r, "r", &trunk, AGENT).map_err(|e| e.code), Ok(()));
    assert_eq!(
        edit_repository(&mut r, "ghost", &trunk, AGENT).map_err(|e| e.code),
        Err("not-found")
    );
}

#[test]
fn an_item_added_under_a_threads_member_joins_the_thread_and_says_so() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let in_t = ItemFields { thread: Change::Set("t".into()), ..titled("A") };
    add_item(&mut r, "a", &in_t, USER_BY).unwrap();
    let under = ItemFields { parent: Change::Set("a".into()), ..titled("B") };
    let data = add_item(&mut r, "b", &under, AGENT).unwrap().item("b");
    assert_eq!(data, json!({"item": "b", "placed": [{"item": "b", "thread": "t"}]}));
    assert_eq!(item(&r, "b").thread.one().map(String::as_str), Some("t"));
}

#[test]
fn an_edit_naming_a_missing_thread_or_repository_writes_nothing() {
    let mut r = replica();
    add_item(&mut r, "a", &titled("Before"), AGENT).unwrap();
    let before = r.changes();
    for (what, fields) in [
        ("thread", ItemFields { thread: Change::Set("nope".into()), ..titled("After") }),
        (
            "repository",
            ItemFields {
                repositories: Delta { add: vec!["nope".into()], remove: vec![] },
                ..titled("After")
            },
        ),
    ] {
        assert_eq!(code(edit_item(&mut r, "a", &fields, AGENT)), "not-found", "{what}");
    }
    assert_eq!((r.changes(), item(&r, "a").title.one().cloned()), (before, Some("Before".into())));
}

#[test]
fn an_add_without_an_id_is_given_the_next_and_a_retry_names_the_first() {
    let mut r = replica();
    let id = |r: &Replica, of: &str, given: Option<&str>, title: &str| {
        new_id(r, of, given, title).map_err(|err| (err.code, err.details))
    };
    assert_eq!(id(&r, kind::ITEM, None, "Sow"), Ok("item-mac-1".to_owned()));
    add_item(&mut r, "item-mac-1", &titled("Sow"), AGENT).unwrap();
    // A retry of the same add is refused, naming the item it made; an explicit id adds a second.
    assert_eq!(
        id(&r, kind::ITEM, None, "Sow"),
        Err(("already-exists", json!({"id": "item-mac-1"})))
    );
    assert_eq!(id(&r, kind::ITEM, Some("sow-again"), "Sow"), Ok("sow-again".to_owned()));
    // A new id in the tracker's form is refused; repeating the add of one that exists isn't.
    assert_eq!(id(&r, kind::ITEM, Some("item-x"), "X").unwrap_err().0, "reserved-id");
    assert_eq!(id(&r, kind::ITEM, Some("item-mac-1"), "Sow"), Ok("item-mac-1".to_owned()));
    // Threads match on title too, and number apart from items.
    assert_eq!(id(&r, kind::THREAD, None, "Sow"), Ok("thread-mac-1".to_owned()));
    let fields = ThreadFields { title: Some("Beds".into()), ..ThreadFields::default() };
    add_thread(&mut r, "thread-mac-1", &fields, USER_BY).unwrap();
    assert_eq!(id(&r, kind::THREAD, None, "Beds").unwrap_err().0, "already-exists");
    assert_eq!(id(&r, kind::THREAD, None, "Rows"), Ok("thread-mac-2".to_owned()));
}

#[test]
fn a_retry_ignores_done_archived_and_deleted_items() {
    let mut r = replica();
    add_item(&mut r, "c", &titled("Weed"), USER_BY).unwrap();
    model::set_state(&mut r, "c", ItemState::Done, USER_BY).unwrap();
    add_item(&mut r, "a", &titled("Sow"), USER_BY).unwrap();
    let key = model::item_key("a").unwrap();
    r.write(&key, field::ARCHIVED, USER, 1, Some(json!(true))).unwrap();
    add_item(&mut r, "b", &titled("Rows"), USER_BY).unwrap();
    acyclic_core::delete::delete_item(&mut r, "b", USER_BY).unwrap();
    for title in ["Weed", "Sow", "Rows"] {
        assert_eq!(new_id(&r, kind::ITEM, None, title).unwrap(), "item-mac-1", "{title}");
    }
}
