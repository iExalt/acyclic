use serde_json::json;

use super::*;
use crate::args::Cli;

#[test]
fn ids_print_as_the_names_people_use() {
    let (plant, beds, code) = (
        "01900000-0000-7000-8000-0000000000a1",
        "01900000-0000-7000-8000-0000000000b1",
        "01900000-0000-7000-8000-0000000000e1",
    );
    let names = BTreeMap::from(
        [(plant, "0000a1"), (beds, "0000b1"), (code, "me/garden")]
            .map(|(id, name)| (id.to_owned(), name.to_owned())),
    );
    let shown = json!([{"id": plant, "title": "Plant", "repositories": [code],
        "readiness": {"status": "blocked", "waiting": {beds: {"why": "open"}}},
        "evidence": {"commit": "me/garden@abc1234", "text": null}}]);
    let text = render(&command("item show Plant"), &shown, None, &names);
    for name in ["0000a1", "me/garden", "blocked by 0000b1 (not done)", "me/garden@abc1234"] {
        assert!(text.contains(name), "{name} in {text}");
    }
    assert!(!text.contains("01900000"), "{text}");
}

fn command(line: &str) -> Command {
    use clap::Parser as _;
    Cli::try_parse_from(std::iter::once("acyclic").chain(line.split(' '))).unwrap().command
}

fn item(id: &str, title: &str, status: &str) -> Json {
    json!({"id": id, "title": title, "state": "backlog", "parent": null, "thread": "t1",
           "readiness": {"status": status}})
}

#[test]
fn lists_are_tables_fitted_to_a_terminal_and_bare_in_a_pipe() {
    let list = json!({"items": [
        item("beds", "Build the raised beds along the south fence", "ready"),
        json!({"id": "plant", "title": "Plant tomatoes", "state": "backlog", "parent": "beds", "thread": "t1",
               "readiness": {"status": "blocked", "waiting": {"beds": {"why": "open"}}}}),
    ]});
    let piped = render(&command("item list"), &list, None, &BTreeMap::new());
    let narrow = render(&command("item list"), &list, Some(60), &BTreeMap::new());
    // Piped, a row is one line with no borders, so `grep` and `cut` work on it.
    assert!(piped.lines().all(|line| line == line.trim()), "{piped}");
    assert!(narrow.lines().all(|line| line.chars().count() <= 60), "{narrow}");
    insta::assert_snapshot!(format!("{piped}\n{narrow}"));
    assert_eq!(
        render(&command("item list"), &json!({"items": []}), None, &BTreeMap::new()),
        "none\n"
    );
}

#[test]
fn ready_names_each_thread_once_and_thread_list_shows_counts_and_waves() {
    let ready = json!({"ready": [
        {"id": "t1", "title": "Garden", "heads": ["beds"],
         "counts": {"members": 2, "done": 0, "conflicted": 0},
         "items": [item("beds", "Beds", "ready"), item("path", "Path", "ready")]},
        {"id": "t2", "title": "Kitchen", "heads": ["tap"],
         "counts": {"members": 1, "done": 0, "conflicted": 0},
         "items": [item("tap", "Tap", "ready")]},
    ]});
    let threads = json!({"threads": ready["ready"], "waves": [["t1"]], "anytime": ["t2"]});
    insta::assert_snapshot!(format!(
        "{}\n{}",
        render(&command("thread ready"), &ready, None, &BTreeMap::new()),
        render(&command("thread list"), &threads, None, &BTreeMap::new())
    ));
}

#[test]
fn records_are_blocks_and_writes_are_one_line() {
    let shown = json!([{"id": "plant", "title": "Plant", "notes": null, "repositories": [], "archived": false,
        "links": {"waits_for": ["beds"], "waited_on_by": []},
        "claims": [{"holder": "session/s", "replica": "mac", "lease_started_at": "2026-10-15T00:00:00Z",
                    "lease_expires_at": "2026-10-15T01:00:00Z"}]}]);
    let edited =
        json!({"changed": true, "item": "plant", "placed": [{"item": "seed", "thread": "t1"}]});
    let unchanged = json!({"changed": false, "thread": "t1"});
    // A write that carries its entity's title names it right after the id, and leaves out a title it hasn't got.
    let titled = json!({"changed": true, "thread": "t1", "title": "Sow the beds"});
    let untitled = json!({"changed": true, "thread": "t2", "title": null});
    insta::assert_snapshot!(format!(
        "{}\n{}{}{}{}",
        render(&command("item show plant"), &shown, None, &BTreeMap::new()),
        render(&command("item edit plant --thread t1"), &edited, None, &BTreeMap::new()),
        render(&command("thread add --title T"), &unchanged, None, &BTreeMap::new()),
        render(&command("thread add --title T"), &titled, None, &BTreeMap::new()),
        render(&command("thread edit t2 --brief B"), &untitled, None, &BTreeMap::new())
    ));
}

#[test]
fn an_error_is_one_line_naming_its_code() {
    let envelope = json!({"ok": false, "error": {"code": "not-found", "message": "no item nope", "details": {}}});
    assert_eq!(error(&envelope), "error: no item nope [not-found]\n");
}

#[test]
fn an_import_and_a_waits_for_edit_say_what_they_wrote() {
    let counts = |items: u64, links: u64| json!({"items": items, "threads": 1, "links": links, "repositories": 0, "fields": 14});
    let import = |dry_run: bool, counts: Json| {
        let line = if dry_run { "import plan.json -n" } else { "import plan.json" };
        let data = json!({"changes": [{"entity": "item", "id": "a1", "action": "create"}], "counts": counts,
            "dry_run": dry_run, "placed": []});
        render(&command(line), &data, None, &BTreeMap::new())
    };
    let none = json!({"items": 0, "threads": 0, "links": 0, "repositories": 0, "fields": 0});
    let edited = |changed: bool| json!({"changed": changed, "item": "a2", "placed": []});
    insta::assert_snapshot!(format!(
        "{}{}{}{}{}{}{}",
        import(false, counts(3, 1)),
        import(true, counts(1, 2)),
        import(false, none),
        render(
            &command("item edit a2 --waits-for b1 --waits-for b2"),
            &edited(true),
            None,
            &BTreeMap::new()
        ),
        render(&command("item edit a2 --no-waits-for b1"), &edited(true), None, &BTreeMap::new()),
        render(&command("item edit a2 --no-waits-for b1"), &edited(false), None, &BTreeMap::new()),
        render(
            &command("item add --title Sow --waits-for b1"),
            &json!({"changed": true, "item": "a3", "title": "Sow", "placed": []}),
            None,
            &BTreeMap::new()
        )
    ));
}

#[test]
fn a_lone_list_is_its_table_and_notes_keep_their_column() {
    let needs = json!({"needs": [
        {"kind": "thread-cycle", "threads": [{"id": "t1", "title": "T"}, {"id": "t2", "title": "U"}]},
        {"kind": "unthreaded", "item": {"id": "a1"}, "fix": "acyclic item edit a1 --thread <THREAD>"},
    ]});
    let empty = json!({"repositories": []});
    let noted = json!([{"id": "a1", "notes": "first line\nsecond line"}]);
    insta::assert_snapshot!(format!(
        "{}{}{}",
        render(&command("needs"), &needs, None, &BTreeMap::new()),
        render(&command("repository list"), &empty, None, &BTreeMap::new()),
        render(&command("item show a1"), &noted, None, &BTreeMap::new())
    ));
    let details = json!({"ok": false, "error": {"code": "not-found", "message": "no thread a1",
        "details": {"thread": "t"}}});
    assert_eq!(error(&details), "error: no thread a1 [not-found] (thread t)\n");
}

#[test]
fn every_readiness_reads_as_a_phrase() {
    use acyclic_core::ready::{Status, ThreadStatus, Wait};
    let phrase = |status: Json| readiness(status.as_object().unwrap());
    let s = |text: &str| text.to_owned();
    let waiting = [
        (s("a"), Wait::Open),
        (s("b"), Wait::InReview),
        (s("c"), Wait::Unverified { repository: s("r"), reason: s("no clone") }),
    ];
    let items: Vec<(Json, &str)> = vec![
        (json!(Status::Ready), "ready"),
        (
            json!(Status::Blocked { waiting: waiting.into_iter().collect() }),
            "blocked by a (not done), b (in review), c (merge unverified: no clone)",
        ),
        (json!(Status::Unthreaded), "unthreaded"),
        (json!(Status::Claimed { holder: s("session/s") }), "claimed by session/s"),
        (
            json!(Status::Contested { holders: vec![s("session/1"), s("session/2")] }),
            "contested by session/1, session/2",
        ),
        (json!(Status::InCycle), "in a cycle"),
        (
            json!(Status::Conflicted { fields: vec![s("state"), s("thread")] }),
            "conflicted: state, thread",
        ),
        (json!(Status::Parent), "parent"),
        (json!(Status::Parked), "parked"),
        (json!(Status::Done), "done"),
        (json!(Status::InReview), "in review"),
        (
            json!(Status::Unverified { repository: s("r"), reason: s("no clone") }),
            "done, merge unverified: no clone (r)",
        ),
        (json!(Status::Archived), "archived"),
        (json!(ThreadStatus::Missing), "missing"),
        (json!(ThreadStatus::Ready { items: vec![s("a"), s("b")] }), "ready: a, b"),
        (json!(ThreadStatus::NothingReady), "nothing ready"),
        (
            json!(ThreadStatus::Claimed { holder: s("session/s"), items: vec![s("a")] }),
            "claimed by session/s; ready: a",
        ),
        (
            json!(ThreadStatus::Claimed { holder: s("session/s"), items: vec![] }),
            "claimed by session/s",
        ),
        (
            json!(ThreadStatus::Contested {
                holders: vec![s("session/1"), s("session/2")],
                items: vec![s("a")]
            }),
            "contested by session/1, session/2; ready: a",
        ),
    ];
    for (status, expected) in items {
        assert_eq!(phrase(status.clone()).as_deref(), Some(expected), "{status}");
    }
    // A status a later build adds, with fields, still prints field by field rather than as its name alone.
    assert_eq!(phrase(json!({"status": "new-kind", "detail": "x"})), None);
    assert_eq!(compact(&json!({"status": "new-kind", "detail": "x"})), "new-kind (detail x)");
}
