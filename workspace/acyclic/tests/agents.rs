//! Several agents working one replica at once, each a process in its own worktree running the binary as an agent does:
//! `thread ready`, `thread claim` a thread, `item done` its items, `thread release` it, until nothing is left. Their
//! audit log shows every thread claimed once, every item finished once by the agent holding its thread, after
//! everything it waited on, and no write lost.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z: one fixed time, so no lease lapses mid-test.
const NOW: &str = "1792022400000";
const AGENTS: usize = 4;
/// Every agent's first claim, made at once.
const CONTESTED: &str = "t-f1";

/// What waits on what: two diamonds, a chain of four and free leaves, each shape its own thread.
const WAITS: &[(&str, &str)] = &[
    ("d1-top", "d1-left"),
    ("d1-top", "d1-right"),
    ("d1-left", "d1-base"),
    ("d1-right", "d1-base"),
    ("d2-top", "d2-left"),
    ("d2-top", "d2-right"),
    ("d2-left", "d2-base"),
    ("d2-right", "d2-base"),
    ("c4", "c3"),
    ("c3", "c2"),
    ("c2", "c1"),
];
const FREE: &[&str] = &["f1", "f2", "f3", "f4", "f5", "f6"];

fn items() -> BTreeSet<&'static str> {
    WAITS.iter().flat_map(|(a, b)| [*a, *b]).chain(FREE.iter().copied()).collect()
}

/// An item's thread: `t-` and the item's shape, so the diamonds, the chain and each free leaf.
fn thread_of(item: &str) -> String {
    let shape = item.split('-').next().unwrap();
    format!("t-{}", if shape.starts_with('c') { "c" } else { shape })
}

/// The id of the item or thread named `name`: a `UUIDv7` made from the name, so a run names each record alike.
fn id(name: &str) -> String {
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("01900000-0000-7000-8000-{:012x}", hash & 0xffff_ffff_ffff)
}

fn acyclic(tracker: &Path, actor: &str, line: &str) -> (i32, Value) {
    let output = Command::new(BIN)
        .args(line.split(' '))
        .env("ACYCLIC_TRACKER", tracker)
        .env("ACYCLIC_ACTOR", actor)
        .env("ACYCLIC_NOW", NOW)
        .env("HOME", "/nonexistent")
        .current_dir(tracker)
        .output()
        .unwrap();
    let code = output.status.code().unwrap();
    (code, serde_json::from_slice(&output.stdout).unwrap_or_default())
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        // With no configured identity, a command that writes a reflog, as `worktree add` does, looks the machine's host
        // name up to make an email address, which stalled each call by 5 seconds on a machine with a slow resolver.
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

/// One agent: the contested claim once every agent is waiting, then work until no item is open.
#[test]
fn agent_child() {
    let (Ok(tracker), Ok(actor), Ok(report)) = (
        std::env::var("ACYCLIC_AGENT_TRACKER"),
        std::env::var("ACYCLIC_AGENT_ACTOR"),
        std::env::var("ACYCLIC_AGENT_REPORT"),
    ) else {
        return;
    };
    let (tracker, report) = (Path::new(&tracker), Path::new(&report));
    // About five times the run takes alone, so a loaded machine doesn't fail it and a stuck agent still does.
    let deadline = Instant::now() + Duration::from_secs(30);
    let wait_for = |names: &[String]| {
        while !names.iter().all(|name| report.with_file_name(name).exists()) {
            assert!(Instant::now() < deadline, "{actor} waited past the deadline for {names:?}");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait_for(&["go".to_owned()]);
    let (code, contest) = acyclic(tracker, &actor, &format!("thread claim {}", id(CONTESTED)));
    // No one moves on, and the winner finishes nothing, until every agent has tried: each loser sees a live claim.
    std::fs::write(report.with_extension("contested"), "").unwrap();
    wait_for(&(0..AGENTS).map(|agent| format!("agent{agent}.contested")).collect::<Vec<_>>());
    let mut claimed = Vec::new();
    let mut done = Vec::new();
    let mut held = (code == 0).then(|| id(CONTESTED));
    claimed.extend(held.clone());
    loop {
        assert!(Instant::now() < deadline, "{actor} still working at the deadline");
        let (code, ready) = acyclic(tracker, &actor, "thread ready");
        assert_eq!(code, 0, "{ready}");
        // One row per thread, the actor's own first; flattened to its items.
        let ready: Vec<Value> = ready["data"]["ready"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|thread| thread["items"].as_array().unwrap().clone())
            .collect();
        if let Some(thread) = held.clone() {
            // Work the held thread's ready items in turn, then let it go.
            let ours = ready.iter().find(|row| row["thread"] == thread.as_str());
            if let Some(row) = ours {
                let item = row["id"].as_str().unwrap().to_owned();
                let (code, out) =
                    acyclic(tracker, &actor, &format!("item done {item} --evidence by-{actor}"));
                assert_eq!(code, 0, "{out}");
                done.push(item);
            } else {
                let (code, out) = acyclic(tracker, &actor, &format!("thread release {thread}"));
                assert_eq!((code, &out["data"]["released"]), (0, &json!(true)), "{out}");
                held = None;
            }
            continue;
        }
        if ready.is_empty() {
            let (_, list) = acyclic(tracker, &actor, "item list");
            let items = list["data"]["items"].as_array().unwrap();
            if items.iter().all(|item| item["state"] == "done") {
                break;
            }
            // Waiting on another agent: poll gently, so idle agents don't crowd the busy ones.
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // Agents start at different places in the list, so they mostly want different threads.
        let index: usize = actor.trim_start_matches("session/agent").parse().unwrap();
        let thread = ready[index % ready.len()]["thread"].as_str().unwrap().to_owned();
        let (code, out) = acyclic(tracker, &actor, &format!("thread claim {thread}"));
        match code {
            0 => {
                claimed.push(thread.clone());
                held = Some(thread);
            },
            1 => assert_eq!(out["error"]["code"], "not-ready", "{out}"),
            _ => panic!("{out}"),
        }
    }
    let summary = json!({"contest": contest, "claimed": claimed, "done": done});
    std::fs::write(report, summary.to_string()).unwrap();
}

#[test]
fn agents_in_separate_worktrees_finish_every_item_once_and_in_order() {
    let dir = TempDir::new().unwrap();
    let tracker = dir.path().join("tracker");
    git(dir.path(), &["init", "-q", "tracker"]);
    // Durability isn't under test, and fsync makes each write about ten times slower on macOS.
    git(&tracker, &["config", "core.fsync", "none"]);
    let init = format!("init {} --replica mac", tracker.display());
    assert_eq!(acyclic(&tracker, "actor/user", &init).0, 0);
    let mut threads: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for item in items() {
        threads.entry(thread_of(item)).or_default().push(item);
    }
    let plan = json!({
        "format": "acyclic-import",
        "version": 4,
        "initiatives": [{"id": id("home"), "title": "Home"}],
        "items": items().iter().map(|key| {
            let waits_for: Vec<String> =
                WAITS.iter().filter(|(from, _)| from == key).map(|(_, to)| id(to)).collect();
            json!({"id": id(key), "title": key, "parent": id("home"), "thread": id(&thread_of(key)), "waits_for": waits_for})
        }).collect::<Vec<_>>(),
        "threads": threads.keys().map(|key| json!({"id": id(key), "title": key})).collect::<Vec<_>>(),
    });
    let file = dir.path().join("plan.json");
    std::fs::write(&file, plan.to_string()).unwrap();
    let (code, out) = acyclic(&tracker, "actor/user", &format!("import {}", file.display()));
    assert_eq!(code, 0, "{out}");

    let children: Vec<_> = (0..AGENTS)
        .map(|agent| {
            let worktree = dir.path().join(format!("wt{agent}"));
            git(
                &tracker,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "--detach",
                    worktree.to_str().unwrap(),
                    "refs/heads/state/mac",
                ],
            );
            Command::new(std::env::current_exe().unwrap())
                .args(["agent_child", "--exact", "--nocapture"])
                .env("ACYCLIC_AGENT_TRACKER", &worktree)
                .env("ACYCLIC_AGENT_ACTOR", format!("session/agent{agent}"))
                .env("ACYCLIC_AGENT_REPORT", dir.path().join(format!("agent{agent}.json")))
                .spawn()
                .unwrap()
        })
        .collect();
    std::fs::write(dir.path().join("go"), "").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success(), "an agent failed");
    }
    let reports: BTreeMap<String, Value> = (0..AGENTS)
        .map(|agent| {
            let text =
                std::fs::read_to_string(dir.path().join(format!("agent{agent}.json"))).unwrap();
            (format!("session/agent{agent}"), serde_json::from_str(&text).unwrap())
        })
        .collect();

    // The contested claim: one winner, and every other agent told who holds it.
    let winners: Vec<&String> = reports
        .iter()
        .filter(|(_, r)| r["contest"]["ok"] == true)
        .map(|(actor, _)| actor)
        .collect();
    assert_eq!(winners.len(), 1, "{reports:#?}");
    for (actor, report) in &reports {
        if actor != winners[0] {
            let readiness = &report["contest"]["error"]["details"]["readiness"];
            assert_eq!(readiness["holder"], winners[0].as_str(), "{actor}: {readiness}");
        }
    }

    // The audit log: the import, then each thread's one claim and one release, and each item's one done.
    let log = git(&tracker, &["for-each-ref", "--format=%(refname)", "refs/heads/log/"]);
    let reference = log.trim();
    assert_eq!(reference.lines().count(), 1, "one replica, one segment: {log}");
    let files = git(&tracker, &["ls-tree", "-r", "--name-only", reference]);
    // The log has a file for the initiative and one for the threads. A file's lines are in order, so the order checks
    // below are between items, which share the initiative's file.
    assert_eq!(files.lines().count(), 2, "{files}");
    let events: Vec<Value> = files
        .lines()
        .flat_map(|file| {
            git(&tracker, &["show", &format!("{reference}:{file}")])
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    let items = items();
    // The import is one event in each file.
    assert_eq!(events.len(), 2 + 2 * threads.len() + items.len(), "no write lost or repeated");
    let mut dots = BTreeSet::new();
    let mut claims: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    let mut dones: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        let actor = event["actor"].as_str().unwrap().to_owned();
        for write in event["writes"].as_array().unwrap() {
            for value in write["values"].as_array().unwrap() {
                assert!(dots.insert(value["dot"].to_string()), "a duplicated dot: {value}");
            }
            let id = write["id"].as_str().unwrap().to_owned();
            let set = &write["values"][0]["value"];
            match write["field"].as_str().unwrap() {
                "claim" if !set.is_null() => {
                    claims.entry(id).or_default().push((index, actor.clone()));
                },
                "state" if set == "done" => {
                    dones.entry(id).or_default().push((index, actor.clone()))
                },
                _ => {},
            }
        }
    }
    for thread in threads.keys() {
        assert_eq!(claims[&id(thread)].len(), 1, "{thread}: one claim");
    }
    for item in &items {
        let (claim, done) = (&claims[&id(&thread_of(item))], &dones[&id(item)]);
        assert_eq!(done.len(), 1, "{item}: one done");
        assert_eq!(claim[0].1, done[0].1, "{item}: finished by the agent holding its thread");
        for (_, dependency) in WAITS.iter().filter(|(waiting, _)| waiting == item) {
            assert!(done[0].0 > dones[&id(dependency)][0].0, "{item} done before {dependency}");
        }
    }
    // Each agent's successful commands are the log's.
    for (actor, report) in &reports {
        for (field, logged) in [("claimed", &claims), ("done", &dones)] {
            let mut theirs: Vec<&str> =
                report[field].as_array().unwrap().iter().map(|i| i.as_str().unwrap()).collect();
            theirs.sort_unstable();
            let ours: Vec<&str> =
                logged.iter().filter(|(_, e)| e[0].1 == *actor).map(|(i, _)| i.as_str()).collect();
            assert_eq!(theirs, ours, "{actor} {field}");
        }
    }
    let (_, list) = acyclic(&tracker, "session/check", "item list");
    let list = list["data"]["items"].as_array().unwrap();
    assert!(list.iter().all(|item| item["state"] == "done"), "{list:?}");
    let every: Vec<&str> = items.iter().copied().collect();
    let (_, shown) = acyclic(&tracker, "session/check", &format!("item show {}", every.join(" ")));
    for item in shown["data"].as_array().unwrap() {
        assert_eq!(item["claims"], json!([]), "nothing left claimed: {item}");
    }
    let every: Vec<&str> = threads.keys().map(String::as_str).collect();
    let (_, shown) =
        acyclic(&tracker, "session/check", &format!("thread show {}", every.join(" ")));
    for thread in shown["data"].as_array().unwrap() {
        assert_eq!(thread["claims"], json!([]), "nothing left claimed: {thread}");
    }
}

#[test]
fn init_refuses_paths_inside_a_repository_without_changing_it() {
    let dir = TempDir::new().unwrap();
    let repository = dir.path().join("repository");
    git(dir.path(), &["init", "-q", "repository"]);
    let plain = repository.join("plain");
    std::fs::create_dir(&plain).unwrap();
    let missing = repository.join("new-tracker");
    let config = git(&repository, &["config", "--local", "--list"]);
    let files = git(&repository, &["status", "--porcelain", "--untracked-files=all"]);

    for path in [&plain, &missing] {
        let output = Command::new(BIN)
            .args(["init", path.to_str().unwrap(), "--replica", "mac"])
            .current_dir(dir.path())
            .env("HOME", "/nonexistent")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["error"]["code"], "nested-repository");
        assert_eq!(git(&repository, &["config", "--local", "--list"]), config);
        assert_eq!(git(&repository, &["status", "--porcelain", "--untracked-files=all"]), files);
    }
    assert!(!missing.exists());

    let bare = dir.path().join("tracker.git");
    git(dir.path(), &["init", "-q", "--bare", "tracker.git"]);
    let output = Command::new(BIN)
        .args(["init", bare.to_str().unwrap(), "--replica", "mac"])
        .current_dir(dir.path())
        .env("HOME", "/nonexistent")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    assert_eq!(git(&bare, &["config", "--local", "--get", "acyclic.replica"]).trim(), "mac");
}
