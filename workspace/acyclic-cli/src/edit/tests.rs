use acyclic_core::scope::USER;
use acyclic_core::state::{ReplicaId, State};

use super::*;

fn replica() -> Replica {
    Replica::open(ReplicaId::new("mac").unwrap(), State::new())
}

const AGENT: By<'static> = By { actor: "session/mac", at: 2, seen: None };
const USER_BY: By<'static> = By { actor: USER, at: 1, seen: None };

fn titled(title: &str) -> ItemFields {
    ItemFields { title: Some(title.to_owned()), ..ItemFields::default() }
}

fn code(result: Result<Written, CliError>) -> &'static str {
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
fn an_agents_membership_and_repository_edits_to_approved_scope_wait_as_proposals() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let repository = RepositoryFields { owner: Some("o".into()), ..RepositoryFields::default() };
    crate::edit::add_repository(&mut r, "r", &repository, USER_BY).unwrap();
    let in_t = ItemFields { thread: Change::Set("t".into()), ..titled("A") };
    add_item(&mut r, "a", &in_t, USER_BY).unwrap();
    acyclic_core::scope::approve(&mut r, "t", true, USER_BY).unwrap();
    let out_of_t = ItemFields {
        title: Some("Renamed".into()),
        thread: Change::Clear,
        repositories: Delta { add: vec!["r".into()], remove: vec![] },
        ..ItemFields::default()
    };
    let written = edit_item(&mut r, "a", &out_of_t, AGENT).unwrap();
    let fields: Vec<&str> = written.proposals.iter().map(|p| p.field.as_str()).collect();
    assert_eq!(fields, ["thread", "repositories"], "membership and repository: {written:?}");
    let held = item(&r, "a");
    assert_eq!(
        (
            held.title.one().map(String::as_str),
            held.thread.one().map(String::as_str),
            held.repositories.len()
        ),
        (Some("Renamed"), Some("t"), 0),
        "the plain field applies and the scope waits"
    );
    // The same agent asking again gets its proposals back; the user's edit applies at once.
    let before = r.changes();
    let again = edit_item(&mut r, "a", &out_of_t, AGENT).unwrap();
    assert_eq!((again.proposals, r.changes()), (written.proposals, before));
    let written = edit_item(&mut r, "a", &out_of_t, USER_BY).unwrap();
    assert_eq!(item(&r, "a").thread.one(), None);
    assert_eq!(written.proposals, []);
    // A thread's own repositories are scope too, and an agent's add of one waits.
    let more = ThreadFields {
        repositories: Delta { add: vec!["r".into()], remove: vec![] },
        ..ThreadFields::default()
    };
    assert_eq!(edit_thread(&mut r, "t", &more, AGENT).unwrap().proposals.len(), 1);
    // An agent can't set a level, which is the user's.
    let level = ThreadFields { level: Change::Set(Level::Everything), ..ThreadFields::default() };
    assert_eq!(code(edit_thread(&mut r, "t", &level, AGENT)), "user-only");
    edit_thread(&mut r, "t", &level, USER_BY).unwrap();
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
        public: Some(true),
    };
    add_repository(&mut r, "r", &repo, AGENT).unwrap();
    let before = r.changes();
    add_repository(&mut r, "r", &repo, AGENT).unwrap();
    assert_eq!(r.changes(), before);
    let private = RepositoryFields { public: Some(false), ..repo };
    assert_eq!(add_repository(&mut r, "r", &private, AGENT).unwrap_err().code, "already-exists");
    assert_eq!(edit_repository(&mut r, "r", &private, AGENT).map_err(|e| e.code), Ok(()));
    assert_eq!(
        edit_repository(&mut r, "ghost", &private, AGENT).map_err(|e| e.code),
        Err("not-found")
    );
}

#[test]
fn every_proposal_a_write_leaves_is_listed_including_the_placement_ones() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    let in_t = ItemFields { thread: Change::Set("t".into()), ..titled("A") };
    add_item(&mut r, "a", &in_t, USER_BY).unwrap();
    acyclic_core::scope::approve(&mut r, "t", true, USER_BY).unwrap();
    // An agent's item under a member of the approved thread would join it, which waits as a proposal.
    let under = ItemFields { parent: Change::Set("a".into()), ..titled("B") };
    let written = add_item(&mut r, "b", &under, AGENT).unwrap();
    assert_eq!(written.proposals, [], "no edit of its own proposed anything");
    let data = written.item("b");
    let placed = data["placed"][0]["proposal"].as_str().unwrap().to_owned();
    assert_eq!(
        (&data["placed"][0]["outcome"], &data["proposals"]),
        (&json!("proposed"), &json!([{"id": placed, "field": "thread"}]))
    );
    // Together with its own edits' proposals, each once and in order.
    let proposed = |id: &str, field: &str| Proposed { id: id.into(), field: field.into() };
    assert_eq!(
        all_proposals(
            &[proposed("z-1", "repositories"), proposed(&placed, "thread")],
            &[Inherited { item: "b".into(), thread: "t".into(), proposal: Some(placed.clone()) }]
        ),
        [proposed(&placed, "thread"), proposed("z-1", "repositories")]
    );
}

#[test]
fn an_import_that_places_into_an_approved_thread_lists_the_proposal() {
    let mut r = replica();
    let thread = ThreadFields { title: Some("T".into()), ..ThreadFields::default() };
    add_thread(&mut r, "t", &thread, USER_BY).unwrap();
    add_item(&mut r, "a", &ItemFields { thread: Change::Set("t".into()), ..titled("A") }, USER_BY)
        .unwrap();
    acyclic_core::scope::approve(&mut r, "t", true, USER_BY).unwrap();
    let plan = json!({"format": "acyclic-import", "version": 1, "items": [
        {"key": "a", "title": "A", "state": "backlog"},
        {"key": "c", "title": "C", "state": "backlog", "parent": "a"}]});
    let doc = acyclic_core::import::Import::from_json(&plan.to_string()).unwrap();
    // An agent's item under a member of the approved thread would join it, which waits for the user.
    let report = acyclic_core::import::apply(&doc, &mut r, AGENT).unwrap();
    let placed =
        acyclic_core::place::place(&mut r, &["a".to_owned(), "c".to_owned()], AGENT).unwrap();
    let data = crate::commands::import_json(&report, &placed, false);
    let proposal = data["placed"][0]["proposal"].as_str().unwrap();
    assert_eq!(data["proposals"], json!([{"id": proposal, "field": "thread"}]), "{data}");
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
