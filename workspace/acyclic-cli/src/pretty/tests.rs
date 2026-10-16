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
