use acyclic_core::model;
use acyclic_core::schema::kind;
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
        &Violation {
            change: None,
            at: None,
            rule: Rule::NotReady(Some(acyclic_core::ready::Readiness::Parent)),
        },
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
fn an_add_repeated_changes_nothing_and_one_that_differs_is_refused() {
    let mut r = replica();
    let full = ItemFields {
        notes: Change::Set("n".into()),
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
fn clearing_an_already_clear_thread_is_a_noop() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let in_t = ItemFields { thread: Change::Set("t".into()), ..titled("A") };
    add_item(&mut r, "a", &in_t, USER_BY).unwrap();
    let clear_thread = ItemFields { thread: Change::Clear, ..ItemFields::default() };
    edit_item(&mut r, "a", &clear_thread, AGENT).unwrap();
    assert_eq!(item(&r, "a").thread.one(), None);
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
fn adding_a_thread_or_repository_again_needs_the_same_fields() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), notes: Change::Set("B".into()) };
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
fn an_edit_naming_a_missing_thread_writes_nothing() {
    let mut r = replica();
    add_item(&mut r, "a", &titled("Before"), AGENT).unwrap();
    let before = r.changes();
    let fields = ItemFields { thread: Change::Set("nope".into()), ..titled("After") };
    assert_eq!(code(edit_item(&mut r, "a", &fields, AGENT)), "not-found");
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
fn a_retry_ignores_done_and_deleted_items() {
    let mut r = replica();
    add_item(&mut r, "c", &titled("Weed"), USER_BY).unwrap();
    model::set_state(&mut r, "c", ItemState::Done, USER_BY).unwrap();
    add_item(&mut r, "b", &titled("Rows"), USER_BY).unwrap();
    acyclic_core::delete::delete_item(&mut r, "b", USER_BY).unwrap();
    for (n, title) in ["Weed", "Rows"].into_iter().enumerate() {
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
}

#[test]
fn a_refusal_reads_as_the_cli_has_always_told_it_and_names_every_violation() {
    let mut r = replica();
    for id in ["p", "c", "o"] {
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
    let waits = |on: &str| ItemFields {
        waits_for: Delta { add: vec![on.into()], remove: vec![] },
        ..ItemFields::default()
    };
    let err = edit_item(&mut r, "p", &waits("c"), USER_BY).unwrap_err();
    assert_eq!(
        (err.code, err.details.clone()),
        ("lineage-link", json!({"item": "p", "waits_for": "c"}))
    );
    edit_item(&mut r, "p", &waits("o"), USER_BY).unwrap();
    let back = waits("p");
    let err = edit_item(&mut r, "o", &back, USER_BY).unwrap_err();
    assert_eq!((err.code, err.details.clone()), ("waits-for-cycle", json!({"items": ["o", "p"]})));
    // Several at once: the one naming nothing speaks first, and every one is listed.
    let both = ItemFields { thread: Change::Set("nowhere".into()), ..back };
    let err = edit_item(&mut r, "o", &both, USER_BY).unwrap_err();
    assert_eq!(err.code, "not-found");
    assert_eq!(err.details["violations"].as_array().map(Vec::len), Some(2));
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
