use acyclic_core::model;
use acyclic_core::schema::{item as field, kind};
use acyclic_core::scope::{ScopeEdit, USER};
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

#[test]
fn an_unreadable_readiness_on_item_done_is_exit_six_not_a_not_ready_refusal() {
    let before = Model::default();
    let unreadable = violation_error(
        &Violation { change: None, at: None, rule: Rule::NotReady(None) },
        None,
        &before,
    );
    assert_eq!((unreadable.code, unreadable.exit), ("unreadable", exit::UNREADABLE));
    assert_eq!(unreadable.details["hint"], DOCTOR_HINT);

    let not_ready = violation_error(
        &Violation { change: None, at: None, rule: Rule::NotReady(Some(Readiness::Unthreaded)) },
        None,
        &before,
    );
    assert_eq!((not_ready.code, not_ready.exit), ("not-ready", exit::REFUSED));
}

// The writes as the commands make them, each judged against the tracker as it is.
fn add_item(
    r: &mut Replica,
    id: &str,
    fields: &ItemFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, None, false, |_| Ok(item_changes(id, fields, Adding::Given)))
}

fn edit_item(
    r: &mut Replica,
    id: &str,
    fields: &ItemFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, None, false, |_| Ok(item_changes(id, fields, Adding::No)))
}

fn add_thread(
    r: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, None, false, |_| Ok(thread_changes(id, fields, Adding::Given)))
}

fn edit_thread(
    r: &mut Replica,
    id: &str,
    fields: &ThreadFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, None, false, |_| Ok(thread_changes(id, fields, Adding::No)))
}

fn add_repository(
    r: &mut Replica,
    id: &str,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, fields.name.as_deref(), false, |_| repository_changes(id, fields, Adding::Given))
}

fn edit_repository(
    r: &mut Replica,
    id: &str,
    fields: &RepositoryFields,
    by: By<'_>,
) -> Result<Outcome, CliError> {
    commit(r, by, fields.name.as_deref(), false, |_| repository_changes(id, fields, Adding::No))
}

fn item(replica: &Replica, id: &str) -> acyclic_core::model::Item {
    Model::read(replica.state()).unwrap().items.remove(id).unwrap()
}

#[test]
fn explicit_repository_adds_survive_unseen_removes_in_either_join_order() {
    let mut base = replica();
    let repository =
        RepositoryFields { name: Some("example/code".into()), default_branch: Some("main".into()) };
    add_repository(&mut base, "r", &repository, USER_BY).unwrap();
    add_item(
        &mut base,
        "a",
        &ItemFields {
            title: Some("A".into()),
            repositories: Delta { add: vec!["r".into()], remove: vec![] },
            ..ItemFields::default()
        },
        USER_BY,
    )
    .unwrap();
    add_thread(
        &mut base,
        "t",
        &ThreadFields {
            title: Some("T".into()),
            repositories: Delta { add: vec!["r".into()], remove: vec![] },
            ..ThreadFields::default()
        },
        USER_BY,
    )
    .unwrap();

    let base = base.state().clone();
    let mut added = Replica::open(ReplicaId::new("add").unwrap(), base.clone());
    let mut removed = Replica::open(ReplicaId::new("remove").unwrap(), base);
    let add_item_fields = ItemFields {
        repositories: Delta { add: vec!["r".into()], remove: vec![] },
        ..Default::default()
    };
    let remove_item_fields = ItemFields {
        repositories: Delta { add: vec![], remove: vec!["r".into()] },
        ..Default::default()
    };
    let add_thread_fields = ThreadFields {
        repositories: Delta { add: vec!["r".into()], remove: vec![] },
        ..Default::default()
    };
    let remove_thread_fields = ThreadFields {
        repositories: Delta { add: vec![], remove: vec!["r".into()] },
        ..Default::default()
    };
    let by_add = By { actor: "session/add", at: 2 };
    let by_remove = By { actor: "session/remove", at: 2 };
    assert!(edit_item(&mut added, "a", &add_item_fields, by_add).unwrap().changes());
    assert!(edit_thread(&mut added, "t", &add_thread_fields, by_add).unwrap().changes());
    edit_item(&mut removed, "a", &remove_item_fields, by_remove).unwrap();
    edit_thread(&mut removed, "t", &remove_thread_fields, by_remove).unwrap();
    let removed_at = removed.changes();
    assert!(!edit_item(&mut removed, "a", &remove_item_fields, by_remove).unwrap().changes());
    assert!(!edit_thread(&mut removed, "t", &remove_thread_fields, by_remove).unwrap().changes());
    assert_eq!(removed.changes(), removed_at);

    for (id, first, second) in [
        ("join-add-first", added.state().clone(), removed.state().clone()),
        ("join-remove-first", removed.state().clone(), added.state().clone()),
    ] {
        let mut joined = Replica::open(ReplicaId::new(id).unwrap(), State::new());
        joined.join(&first).unwrap();
        joined.join(&second).unwrap();
        let model = Model::read(joined.state()).unwrap();
        assert!(model.items["a"].repositories.contains("r"), "{id}");
        assert!(model.threads["t"].repositories.contains("r"), "{id}");
    }
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
fn an_explicit_repository_add_reasserts_while_clearing_a_thread_remains_a_noop() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let repository = RepositoryFields { name: Some("o/r".into()), ..RepositoryFields::default() };
    add_repository(&mut r, "r", &repository, USER_BY).unwrap();
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
    assert!(edit_item(&mut r, "a", &out_of_t, AGENT).unwrap().changes());
    assert!(r.changes() > before, "an explicit repository add records a fresh add dot");
    let clear_thread = ItemFields { thread: Change::Clear, ..ItemFields::default() };
    let before = r.changes();
    edit_item(&mut r, "a", &clear_thread, AGENT).unwrap();
    assert_eq!(r.changes(), before, "clearing an already-clear thread remains a no-op");
}

#[test]
fn a_waits_for_on_an_item_deleted_elsewhere_is_listed_and_let_go_of_by_its_id() {
    use acyclic_core::needs::{Entry, needs_you};
    use acyclic_core::ready::ReadinessEvaluator;
    let dangling = |r: &Replica| -> Vec<Entry> {
        let model = Model::read(r.state()).unwrap();
        let readiness = ReadinessEvaluator::new(r.state(), &model, 3);
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
    let model = Model::read(mac.state()).unwrap();
    let let_go = ItemFields {
        waits_for: Delta { add: vec![], remove: vec!["0000B1".into()] },
        ..ItemFields::default()
    }
    .resolve(&model, &Records::of(&model), Some(&a))
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
    assert_eq!(edit_repository(&mut r, "r", &trunk, AGENT).map(|_| ()).map_err(|e| e.code), Ok(()));
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
    let placed = add_item(&mut r, "b", &under, AGENT).unwrap().placed();
    assert_eq!(json!(placed), json!([{"item": "b", "thread": "t"}]));
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
fn a_minted_add_whose_title_open_work_has_is_refused_naming_it() {
    let mut r = replica();
    let minted = |r: &mut Replica, of: &str, id: &str, title: &str| {
        let changes = if of == kind::THREAD {
            let fields = ThreadFields { title: Some(title.to_owned()), ..ThreadFields::default() };
            thread_changes(id, &fields, Adding::Minted)
        } else {
            item_changes(id, &titled(title), Adding::Minted)
        };
        commit(r, USER_BY, None, false, |_| Ok(changes)).map_err(|err| (err.code, err.details))
    };
    minted(&mut r, kind::ITEM, "a", "Sow").unwrap();
    // A retry with a new id is refused, naming the item the first made; one with the same id is a repeat.
    assert_eq!(
        minted(&mut r, kind::ITEM, "b", "Sow").unwrap_err(),
        ("already-exists", json!({"id": "a"}))
    );
    assert!(add_item(&mut r, "a", &titled("Sow"), USER_BY).is_ok());
    assert!(
        add_item(&mut r, "c", &titled("Sow"), USER_BY).is_ok(),
        "an id the caller chose isn't judged by title"
    );
    // Threads match on title too, apart from items.
    minted(&mut r, kind::THREAD, "t", "Sow").unwrap();
    assert_eq!(minted(&mut r, kind::THREAD, "u", "Sow").unwrap_err().0, "already-exists");
    assert_eq!(given_id("plant").unwrap_err().code, "invalid-id");
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
    for (n, title) in ["Weed", "Sow", "Rows"].into_iter().enumerate() {
        let changes = item_changes(&format!("n{n}"), &titled(title), Adding::Minted);
        assert!(commit(&mut r, USER_BY, None, false, |_| Ok(changes)).is_ok(), "{title}");
    }
}

#[test]
fn adding_and_removing_one_record_is_refused_however_each_is_spelled() {
    let mut r = replica();
    add_item(&mut r, &id(1), &titled("Ship it"), USER_BY).unwrap();
    add_item(&mut r, &id(2), &titled("Wait"), USER_BY).unwrap();
    let garden =
        RepositoryFields { name: Some("me/garden".into()), default_branch: Some("main".into()) };
    add_repository(&mut r, &id(3), &garden, USER_BY).unwrap();
    let model = Model::read(r.state()).unwrap();
    let records = Records::of(&model);
    let both =
        |add: &str, remove: &str| Delta { add: vec![add.into()], remove: vec![remove.into()] };
    let ending = &id(1)[id(1).len() - 6..];
    let item = |fields: ItemFields| code(fields.resolve(&model, &records, Some(&id(2))));
    assert_eq!(
        item(ItemFields { waits_for: both("Ship it", ending), ..ItemFields::default() }),
        "usage"
    );
    assert_eq!(
        item(ItemFields { repositories: both("ME/Garden", &id(3)), ..ItemFields::default() }),
        "usage"
    );
    let thread =
        ThreadFields { repositories: both(&id(3), "me/garden"), ..ThreadFields::default() };
    assert_eq!(code(thread.resolve(&model, &records, None)), "usage");
}

#[test]
fn setting_done_work_back_reopens_it_out_of_its_archived_subtree() {
    let mut r = replica();
    add_item(&mut r, "p", &titled("Beds"), USER_BY).unwrap();
    add_item(
        &mut r,
        "c",
        &ItemFields { parent: Change::Set("p".into()), ..titled("Dig") },
        USER_BY,
    )
    .unwrap();
    for id in ["c", "p"] {
        model::set_state(&mut r, id, ItemState::Done, USER_BY).unwrap();
    }
    r.write(&model::item_key("p").unwrap(), field::ARCHIVED, USER, 1, Some(json!(true))).unwrap();
    assert!(Model::read(r.state()).unwrap().is_archived("c"));
    let parked = ItemFields { state: Some(ItemState::Parked), ..ItemFields::default() };
    edit_item(&mut r, "c", &parked, AGENT).unwrap();
    let model = Model::read(r.state()).unwrap();
    assert_eq!(model.items["c"].state.one(), Some(&ItemState::Parked));
    assert!(
        model.items["c"].reopened != model::Value::None,
        "flagged, so its dependents are listed"
    );
    assert!(!model.is_archived("c"), "out of the archived subtree");
}

#[test]
fn one_repository_named_twice_is_written_once() {
    let garden =
        RepositoryFields { name: Some("me/garden".into()), default_branch: Some("main".into()) };
    // Two spellings of one record resolve to the same id twice.
    let written = |named: Vec<String>| {
        let mut r = replica();
        add_repository(&mut r, &id(3), &garden, USER_BY).unwrap();
        add_item(&mut r, "a", &titled("A"), USER_BY).unwrap();
        let before = r.changes();
        let fields = ItemFields {
            repositories: Delta { add: named, remove: vec![] },
            ..ItemFields::default()
        };
        edit_item(&mut r, "a", &fields, AGENT).unwrap();
        r.changes() - before
    };
    assert_eq!(written(vec![id(3), id(3)]), written(vec![id(3)]));
}

#[test]
fn a_refusal_reads_as_the_cli_has_always_told_it_and_names_every_violation() {
    let mut r = replica();
    for id in ["p", "c"] {
        add_item(&mut r, id, &titled(id), USER_BY).unwrap();
    }
    edit_item(
        &mut r,
        "c",
        &ItemFields { parent: Change::Set("p".into()), ..ItemFields::default() },
        USER_BY,
    )
    .unwrap();
    let under_c = ItemFields { parent: Change::Set("c".into()), ..ItemFields::default() };
    let err = edit_item(&mut r, "p", &under_c, USER_BY).unwrap_err();
    assert_eq!(
        (err.code, err.message.as_str()),
        ("parent-cycle", "p can't go under itself or its own descendant")
    );
    let itself = ItemFields { parent: Change::Set("c".into()), ..ItemFields::default() };
    let err = edit_item(&mut r, "c", &itself, USER_BY).unwrap_err();
    assert_eq!(
        (err.code, err.message.as_str()),
        ("parent-cycle", "c can't go under itself or its own descendant")
    );
    edit_item(
        &mut r,
        "p",
        &ItemFields {
            waits_for: Delta { add: vec!["c".into()], remove: vec![] },
            ..ItemFields::default()
        },
        USER_BY,
    )
    .unwrap();
    let back = ItemFields {
        waits_for: Delta { add: vec!["p".into()], remove: vec![] },
        ..ItemFields::default()
    };
    let err = edit_item(&mut r, "c", &back, USER_BY).unwrap_err();
    assert_eq!((err.code, err.details.clone()), ("link-cycle", json!({"items": ["c", "p"]})));
    // Several at once: the one naming nothing speaks first, and every one is listed.
    let both = ItemFields { thread: Change::Set("nowhere".into()), ..back };
    let err = edit_item(&mut r, "c", &both, USER_BY).unwrap_err();
    assert_eq!(err.code, "not-found");
    assert_eq!(err.details["violations"].as_array().map(Vec::len), Some(2));
}

#[test]
fn work_moved_or_added_under_an_archived_root_is_refused() {
    let mut r = replica();
    add_item(&mut r, "p", &titled("Beds"), USER_BY).unwrap();
    model::set_state(&mut r, "p", ItemState::Done, USER_BY).unwrap();
    r.write(&model::item_key("p").unwrap(), field::ARCHIVED, USER, 1, Some(json!(true))).unwrap();
    add_item(&mut r, "o", &titled("Open"), USER_BY).unwrap();
    let under = ItemFields { parent: Change::Set("p".into()), ..ItemFields::default() };
    let err = edit_item(&mut r, "o", &under, USER_BY).unwrap_err();
    assert_eq!((err.code, err.details.clone()), ("archived", json!({"root": "p"})));
    let new = ItemFields { parent: Change::Set("p".into()), ..titled("New") };
    assert_eq!(code(add_item(&mut r, "n", &new, USER_BY)), "archived");
}

#[test]
fn a_finish_refused_for_its_evidence_reads_as_the_cli_has_always_told_it() {
    use acyclic_core::model::Evidence;
    let mut r = replica();
    let fields = RepositoryFields { name: Some("o/n".into()), ..RepositoryFields::default() };
    add_repository(&mut r, "r", &fields, USER_BY).unwrap();
    add_item(&mut r, "a", &titled("A"), USER_BY).unwrap();
    let finish = |r: &mut Replica, commit: &str, force: bool| {
        let evidence =
            Evidence { repository: Some("r".into()), commit: Some(commit.into()), text: None };
        let change = WriteChange::Finish { item: "a".into(), evidence: Some(evidence), force };
        super::commit(r, USER_BY, None, true, |_| Ok(ChangeSet(vec![change])))
            .map_err(|err| (err.code, err.message, err.details))
    };
    assert_eq!(
        finish(&mut r, "abc1234", true).unwrap_err(),
        (
            "incomplete-repository",
            "repository r is missing default-branch; set it with `acyclic repository edit r`"
                .into(),
            json!({"missing": ["default_branch"]})
        )
    );
    let fields =
        RepositoryFields { default_branch: Some("main".into()), ..RepositoryFields::default() };
    edit_repository(&mut r, "r", &fields, USER_BY).unwrap();
    finish(&mut r, "abc1234", true).unwrap();
    let (code, message, details) = finish(&mut r, "def5678", false).unwrap_err();
    assert_eq!(
        (code, message.as_str()),
        ("already-done", "item a is done with other evidence; pass --force to replace it")
    );
    assert_eq!(details["evidence"]["commit"], json!("o/n@abc1234"), "{details}");
}
