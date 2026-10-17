use serde_json::json;

use super::*;
use crate::args::Cli;

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
    let piped = render(&command("list"), &list, None);
    let narrow = render(&command("list"), &list, Some(60));
    // Piped, a row is one line with no borders, so `grep` and `cut` work on it.
    assert!(piped.lines().all(|line| line == line.trim()), "{piped}");
    assert!(narrow.lines().all(|line| line.chars().count() <= 60), "{narrow}");
    insta::assert_snapshot!(format!("{piped}\n{narrow}"));
    assert_eq!(render(&command("list"), &json!({"items": []}), None), "none\n");
}

#[test]
fn ready_names_each_thread_once_and_thread_list_shows_counts() {
    let ready = json!({"ready": [
        {"id": "t1", "title": "Garden", "approved": true, "heads": ["beds"],
         "counts": {"members": 2, "done": 0, "conflicted": 0},
         "items": [with_approved(item("beds", "Beds", "ready")), with_approved(item("path", "Path", "ready"))]},
        {"id": "t2", "title": "Kitchen", "approved": false, "heads": ["tap"],
         "counts": {"members": 1, "done": 0, "conflicted": 0},
         "items": [item("tap", "Tap", "ready")]},
    ]});
    let threads = json!({"threads": ready["ready"]});
    insta::assert_snapshot!(format!(
        "{}\n{}",
        render(&command("ready"), &ready, None),
        render(&command("thread list"), &threads, None)
    ));
}

fn with_approved(mut row: Json) -> Json {
    row["approved"] = json!(true);
    row
}

#[test]
fn records_are_blocks_and_writes_are_one_line() {
    let shown = json!([{"id": "plant", "title": "Plant", "notes": null, "areas": [], "archived": false,
        "links": {"waits_for": ["beds"], "relates_to": []},
        "claims": [{"holder": "session/s", "replica": "mac", "lease_started_at": "2026-10-15T00:00:00Z",
                    "lease_expires_at": "2026-10-15T01:00:00Z"}]}]);
    let edited = json!({"changed": true, "item": "plant", "placed": [],
        "proposals": [{"id": "mac-9", "field": "thread"}]});
    let unchanged = json!({"changed": false, "thread": "t1"});
    insta::assert_snapshot!(format!(
        "{}\n{}{}",
        render(&command("show plant"), &shown, None),
        render(&command("item edit plant --thread t1"), &edited, None),
        render(&command("thread add t1 --title T"), &unchanged, None)
    ));
}

#[test]
fn an_error_is_one_line_naming_its_code() {
    let envelope = json!({"ok": false, "error": {"code": "not-found", "message": "no item nope", "details": {}}});
    assert_eq!(error(&envelope), "error: no item nope [not-found]\n");
}

#[test]
fn an_import_and_a_link_say_what_they_wrote() {
    let counts = |items: u64, links: u64| json!({"items": items, "threads": 1, "links": links, "repositories": 0, "fields": 14});
    let import = |dry_run: bool, counts: Json| {
        let line = if dry_run { "import plan.json -n" } else { "import plan.json" };
        let data = json!({"changes": [{"entity": "item", "id": "a1", "action": "create"}], "counts": counts,
            "dry_run": dry_run, "placed": [], "proposals": []});
        render(&command(line), &data, None)
    };
    let none = json!({"items": 0, "threads": 0, "links": 0, "repositories": 0, "fields": 0});
    let link = json!({"changed": true, "from": "a2", "kind": "relates-to", "to": "b1"});
    insta::assert_snapshot!(format!(
        "{}{}{}{}{}",
        import(false, counts(3, 1)),
        import(true, counts(1, 2)),
        import(false, none),
        render(&command("link add a2 relates-to b1"), &link, None),
        render(
            &command("link remove a2 relates-to b1"),
            &json!({"changed": false, "from": "a2",
            "kind": "relates-to", "to": "b1"}),
            None
        )
    ));
}

#[test]
fn a_lone_list_is_its_table_and_notes_keep_their_column() {
    let needs = json!({"needs": [
        {"kind": "awaiting-approval", "thread": {"id": "t1", "title": "T"}, "counts": {"open_members": 1}},
        {"kind": "unthreaded", "item": {"id": "a1"}, "fix": "acyclic item edit a1 --thread <THREAD>"},
    ]});
    let empty = json!({"proposals": []});
    let noted = json!([{"id": "a1", "notes": "first line\nsecond line"}]);
    insta::assert_snapshot!(format!(
        "{}{}{}",
        render(&command("needs"), &needs, None),
        render(&command("proposal list"), &empty, None),
        render(&command("show a1"), &noted, None)
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
            json!(Status::Contested { holders: vec![s("run/1"), s("run/2")] }),
            "contested by run/1, run/2",
        ),
        (
            json!(Status::ResourceBusy { resource: s("gpu"), thread: s("t"), holder: s("run/1") }),
            "waiting for gpu, held by thread t (run/1)",
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
                holders: vec![s("run/1"), s("run/2")],
                items: vec![s("a")]
            }),
            "contested by run/1, run/2; ready: a",
        ),
    ];
    for (status, expected) in items {
        assert_eq!(phrase(status.clone()).as_deref(), Some(expected), "{status}");
    }
    // A status a later build adds, with fields, still prints field by field rather than as its name alone.
    assert_eq!(phrase(json!({"status": "new-kind", "detail": "x"})), None);
    assert_eq!(compact(&json!({"status": "new-kind", "detail": "x"})), "new-kind (detail x)");
}
