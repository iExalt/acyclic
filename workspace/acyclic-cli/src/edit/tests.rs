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
        waits_for: Delta { add: vec!["x".into()], remove: vec![] },
        ..titled("A")
    };
    add_item(&mut r, "x", &titled("X"), AGENT).unwrap();
    assert_eq!(code(add_item(&mut r, "a", &full, AGENT)), "ok");
    let held = item(&r, "a");
    let waits = Model::read(r.state()).unwrap().links.len();
    assert_eq!(
        (held.notes.one().map(String::as_str), held.state.one(), waits),
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
            "waits-for",
            ItemFields {
                waits_for: Delta { add: vec!["y".into()], remove: vec![] },
                ..titled("A")
            },
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
    let repository = RepositoryFields { name: Some("o/r".into()), ..RepositoryFields::default() };
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
fn a_waits_for_on_an_item_deleted_elsewhere_is_listed_and_let_go_of_by_its_id() {
    use acyclic_core::needs::{Entry, needs_you};
    use acyclic_core::ready::{Ancestry, Readiness};
    struct NoClones;
    impl Ancestry for NoClones {
        fn merged(&self, _: &model::Repository, _: &str) -> Result<bool, String> {
            Err("no clone".into())
        }
    }
    let dangling = |r: &Replica| -> Vec<Entry> {
        let model = Model::read(r.state()).unwrap();
        let readiness = Readiness::new(r.state(), &model, 3, &NoClones);
        let needs = needs_you(r.state(), &model, &readiness, 3).unwrap();
        needs.into_iter().filter(|entry| matches!(entry, Entry::Dangling { .. })).collect()
    };
    let mut mac = replica();
    let (a, b) = (id(0xa1), id(0xb1));
    for id in [&a, &b] {
        add_item(&mut mac, id, &titled(id), USER_BY).unwrap();
    }
    // One replica deletes b while another makes a wait for it.
    let mut pi = Replica::open(ReplicaId::new("pi").unwrap(), mac.state().clone());
    acyclic_core::delete::delete_item(&mut pi, &b, AGENT).unwrap();
    let waits = ItemFields {
        waits_for: Delta { add: vec![b.clone()], remove: vec![] },
        ..ItemFields::default()
    };
    edit_item(&mut mac, &a, &waits, AGENT).unwrap();
    mac.join(pi.state()).unwrap();
    assert_eq!(
        dangling(&mac),
        [Entry::Dangling {
            kind: "item".into(),
            id: a.clone(),
            field: "waits-for".into(),
            target: b.clone(),
            fix: Some(format!("acyclic item edit {a} --no-waits-for {b}")),
        }]
    );
    // Named by 6 characters of its id, in upper case: no record has that id now, but the item holds it.
    let let_go = ItemFields {
        waits_for: Delta { add: vec![], remove: vec!["0000B1".into()] },
        ..ItemFields::default()
    }
    .resolve(&Model::read(mac.state()).unwrap(), Some(&a))
    .unwrap();
    assert_eq!(let_go.waits_for.remove, [b]);
    edit_item(&mut mac, &a, &let_go, AGENT).unwrap();
    assert_eq!(dangling(&mac), []);
}

#[test]
fn a_repository_with_no_record_is_removed_by_the_id_it_is_held_by() {
    let mut r = replica();
    add_item(&mut r, "a", &titled("A"), USER_BY).unwrap();
    add_thread(
        &mut r,
        "t",
        &ThreadFields { title: Some("T".into()), ..ThreadFields::default() },
        USER_BY,
    )
    .unwrap();
    // Held as if written by another replica whose record this one never got.
    let item_held =
        ScopeEdit::ItemRepository { item: "a".into(), repository: "gone".into(), present: true };
    let thread_held = ScopeEdit::ThreadRepository {
        thread: "t".into(),
        repository: "gone".into(),
        present: true,
    };
    for held in [&item_held, &thread_held] {
        acyclic_core::scope::edit(&mut r, held, USER_BY).unwrap();
    }
    let gone = Delta { add: vec![], remove: vec!["gone".into()] };
    edit_item(
        &mut r,
        "a",
        &ItemFields { repositories: gone.clone(), ..ItemFields::default() },
        AGENT,
    )
    .unwrap();
    edit_thread(
        &mut r,
        "t",
        &ThreadFields { repositories: gone, ..ThreadFields::default() },
        AGENT,
    )
    .unwrap();
    let model = Model::read(r.state()).unwrap();
    assert!(model.items["a"].repositories.is_empty() && model.threads["t"].repositories.is_empty());
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
    let repo = RepositoryFields { name: Some("o/n".into()), default_branch: Some("main".into()) };
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
    // Another record may not take its owner/name, in any case, and a name must be one.
    let same = RepositoryFields { name: Some("O/N".into()), default_branch: Some("main".into()) };
    assert_eq!(add_repository(&mut r, "s", &same, AGENT).unwrap_err().code, "already-exists");
    let bare = RepositoryFields { name: Some("n".into()), default_branch: Some("main".into()) };
    assert_eq!(add_repository(&mut r, "s", &bare, AGENT).unwrap_err().code, "usage");
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

/// A fixed `UUIDv7` told apart by `n`.
fn id(n: u16) -> String {
    format!("01900000-0000-7000-8000-{n:012x}")
}

#[test]
fn an_add_without_an_id_is_given_a_new_one_and_a_retry_names_the_first() {
    let mut r = replica();
    let new = |r: &Replica, of: &str, given: Option<&str>, title: &str| {
        new_id(r, of, given, title).map_err(|err| (err.code, err.details))
    };
    let made = new(&r, kind::ITEM, None, "Sow").unwrap();
    assert_eq!(acyclic_core::ids::v7(&made), Some(made.clone()));
    add_item(&mut r, &made, &titled("Sow"), AGENT).unwrap();
    // A retry of the same add is refused, naming the item it made, with an id of its own or none.
    let named = Err(("already-exists", json!({"id": made})));
    assert_eq!(new(&r, kind::ITEM, None, "Sow"), named);
    assert_eq!(new(&r, kind::ITEM, Some(&id(2)), "Sow"), named);
    // An id that exists is looked up before the title, in any case, so the add is a repeat.
    assert_eq!(new(&r, kind::ITEM, Some(&made.to_uppercase()), "Sow"), Ok(made.clone()));
    assert_eq!(new(&r, kind::ITEM, Some("plant"), "Plant").unwrap_err().0, "invalid-id");
    // Threads match on title too, apart from items.
    assert!(new(&r, kind::THREAD, None, "Sow").is_ok());
    let fields = ThreadFields { title: Some("Beds".into()), ..ThreadFields::default() };
    add_thread(&mut r, &id(3), &fields, USER_BY).unwrap();
    assert_eq!(new(&r, kind::THREAD, None, "Beds").unwrap_err().0, "already-exists");
    assert_eq!(new(&r, kind::THREAD, Some(&id(3)), "Beds"), Ok(id(3)));
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
        assert!(new_id(&r, kind::ITEM, None, title).is_ok(), "{title}");
    }
}
