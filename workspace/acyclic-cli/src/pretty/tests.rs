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
    let shown = json!([{"id": "plant", "title": "Plant", "notes": null, "repositories": [],
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
fn cleared_conflict_sides_are_visible_but_unset_fields_stay_omitted() {
    let shown = json!([{"id": "a1", "parent": {"conflict": [null, "ideas"]}, "notes": null}]);
    let text = render(&command("item show a1"), &shown, None, &BTreeMap::new());
    assert!(text.contains("conflict none, ideas"), "{text}");
    assert!(!text.contains("notes"), "{text}");

    let needs = json!({"needs": [{"kind": "conflict", "entity": "item", "id": "a1",
    "field": "parent", "values": [null, "ideas"], "keeps": [
        {"value": null, "command": {}, "run": "acyclic item edit a1 --no-parent", "why": null},
    ]}]});
    let text = render(&command("needs"), &needs, None, &BTreeMap::new());
    assert!(text.contains("none, ideas"), "{text}");
    assert!(text.contains("keep none: acyclic item edit a1 --no-parent"), "{text}");
    assert_eq!(compact(&json!([null, "unrelated"])), ", unrelated");
}

#[test]
fn an_import_and_a_waits_for_edit_say_what_they_wrote() {
    let counts = |items: u64, links: u64| json!({"items": items, "threads": 1, "links": links, "repositories": 0, "fields": 14});
    let import = |dry_run: bool, counts: Json| {
        let line = if dry_run { "import plan.json -n" } else { "import plan.json" };
        let changes: Vec<Json> = [("items", "item"), ("threads", "thread"), ("links", "link")]
            .into_iter()
            .flat_map(|(key, entity)| {
                (0..counts[key].as_u64().unwrap()).map(move |id| {
                    if entity == "link" {
                        json!({"entity": entity, "from": "a1", "kind": "waits-for", "to": format!("b{id}"), "action": "create"})
                    } else {
                        json!({"entity": entity, "id": format!("{entity}{id}"), "action": "create"})
                    }
                })
            })
            .collect();
        let data = json!({"changes": changes, "counts": counts,
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
fn import_summaries_count_updates_removals_and_placement_without_claiming_dry_run_writes() {
    let update = json!({"entity": "item", "id": "a1", "action": "update",
        "fields": {"title": {"from": "Old", "to": "New"}, "notes": {"from": null, "to": "Checked"}}});
    let removal = json!({"entity": "link", "from": "a1", "kind": "waits-for", "to": "b1", "action": "remove"});
    for (changes, fields, actual, dry) in [
        (
            json!([update]),
            2,
            "imported: updated 1 item (2 fields)",
            "would import: update 1 item (2 fields)",
        ),
        (
            json!([removal]),
            1,
            "imported: removed 1 waits-for (1 field)",
            "would import: remove 1 waits-for (1 field)",
        ),
        (
            json!([{"entity": "item", "id": "a2", "action": "create"}, update, removal]),
            5,
            "imported: created 1 item; updated 1 item; removed 1 waits-for (5 fields)",
            "would import: create 1 item; update 1 item; remove 1 waits-for (5 fields)",
        ),
        (json!([]), 0, "no change", "would make no change"),
    ] {
        for (dry_run, expected) in [(false, actual), (true, dry)] {
            let data = json!({"changes": changes, "counts": {"fields": fields}, "dry_run": dry_run, "placed": []});
            assert_eq!(import(&data), expected);
        }
    }
    for (dry_run, expected) in
        [(false, "placed item a1, thread t1"), (true, "would place item a1, thread t1")]
    {
        let data = json!({"changes": [], "counts": {"fields": 0}, "dry_run": dry_run,
            "placed": [{"item": "a1", "thread": "t1"}]});
        assert_eq!(import(&data), expected);
    }
}

#[test]
fn init_and_sync_name_the_tracker_and_distinguish_local_changes_from_pushes() {
    for (created, changed, verb) in
        [(true, true, "created"), (false, true, "changed"), (false, false, "no change to")]
    {
        let data = json!({"path": "/tmp/example", "replica": "local", "created": created, "changed": changed});
        assert_eq!(
            render(&command("init /tmp/example --replica local"), &data, None, &BTreeMap::new()),
            format!("{verb} tracker /tmp/example · replica local\n")
        );
    }
    for (joined, pushed, word) in [(false, 0, "no"), (false, 2, "no"), (true, 1, "yes")] {
        let data = json!({"remote": "origin", "fetched": 2, "joined": joined, "pushed": pushed, "changed": joined});
        assert_eq!(
            render(&command("sync"), &data, None, &BTreeMap::new()),
            format!("synced tracker with origin · fetched 2 · joined {word} · pushed {pushed}\n")
        );
    }
}

#[test]
fn a_lone_list_is_its_table_and_notes_keep_their_column() {
    let needs = json!({"needs": [
        {"kind": "thread-cycle", "threads": [{"id": "t1", "title": "T"}, {"id": "t2", "title": "U"}]},
        {"kind": "unthreaded", "item": {"id": "a1"}, "fix": "acyclic item edit a1 --thread <THREAD>"},
        {"kind": "conflict", "entity": "item", "id": "a1", "field": "state", "keeps": [
            {"value": "done", "command": null, "run": null, "why": "Keep done in a terminal"},
            {"value": "parked", "command": {}, "run": "acyclic item edit a1 --state parked", "why": null},
        ]},
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
    use acyclic_core::ready::{Readiness, ThreadReadiness, Wait};
    let phrase = |status: Json| readiness(status.as_object().unwrap());
    let s = |text: &str| text.to_owned();
    let waiting = [(s("a"), Wait::Open), (s("b"), Wait::Open)];
    let items: Vec<(Json, &str)> = vec![
        (json!(Readiness::Ready), "ready"),
        (
            json!(Readiness::Blocked { waiting: waiting.into_iter().collect() }),
            "blocked by a (not done), b (not done)",
        ),
        (json!(Readiness::Unthreaded), "unthreaded"),
        (json!(Readiness::Claimed { holder: s("session/s") }), "claimed by session/s"),
        (
            json!(Readiness::Contested { holders: vec![s("session/1"), s("session/2")] }),
            "contested by session/1, session/2",
        ),
        (json!(Readiness::InCycle), "in a cycle"),
        (
            json!(Readiness::Conflicted { fields: vec![s("state"), s("thread")] }),
            "conflicted: state, thread",
        ),
        (json!(Readiness::Parent), "parent"),
        (json!(Readiness::Parked), "parked"),
        (json!(Readiness::Done), "done"),
        (json!(ThreadReadiness::Missing), "missing"),
        (json!(ThreadReadiness::Ready { items: vec![s("a"), s("b")] }), "ready: a, b"),
        (json!(ThreadReadiness::NothingReady), "nothing ready"),
        (
            json!(ThreadReadiness::Claimed { holder: s("session/s"), items: vec![s("a")] }),
            "claimed by session/s; ready: a",
        ),
        (
            json!(ThreadReadiness::Claimed { holder: s("session/s"), items: vec![] }),
            "claimed by session/s",
        ),
        (
            json!(ThreadReadiness::Contested {
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
