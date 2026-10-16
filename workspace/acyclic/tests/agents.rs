//! Several agents working one replica at once, each a process in its own worktree running the binary as an agent does:
//! `ready`, `claim`, `done`, until nothing is left. Their audit log shows every item finished once, by the agent that
//! claimed it, after everything it waited on, and no write lost.

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
const CONTESTED: &str = "f1";

/// What waits on what: two diamonds, a chain of four and free leaves.
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
    let (code, contest) = acyclic(tracker, &actor, &format!("claim {CONTESTED}"));
    // No one moves on, and the winner finishes nothing, until every agent has tried: each loser sees a live claim.
    std::fs::write(report.with_extension("contested"), "").unwrap();
    wait_for(&(0..AGENTS).map(|agent| format!("agent{agent}.contested")).collect::<Vec<_>>());
    let mut claimed = Vec::new();
    let mut done = Vec::new();
    if code == 0 {
        claimed.push(CONTESTED.to_owned());
    }
    let mut mine = (code == 0).then(|| CONTESTED.to_owned());
    loop {
        assert!(Instant::now() < deadline, "{actor} still working at the deadline");
        if let Some(item) = mine.take() {
            let (code, out) = acyclic(tracker, &actor, &format!("done {item} --note by-{actor}"));
            assert_eq!(code, 0, "{out}");
            done.push(item);
            continue;
        }
        let (code, ready) = acyclic(tracker, &actor, "ready");
        assert_eq!(code, 0, "{ready}");
        let ready = ready["data"]["ready"].as_array().unwrap().clone();
        if ready.is_empty() {
            let (_, list) = acyclic(tracker, &actor, "list");
            let items = list["data"]["items"].as_array().unwrap();
            if items.iter().all(|item| item["state"] == "done") {
                break;
            }
            // Waiting on another agent: poll gently, so idle agents don't crowd the busy ones.
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        // Agents start at different places in the list, so they mostly want different items.
        let index: usize = actor.trim_start_matches("session/agent").parse().unwrap();
        let item = ready[index % ready.len()]["id"].as_str().unwrap().to_owned();
        let (code, out) = acyclic(tracker, &actor, &format!("claim {item}"));
        match code {
            0 => {
                claimed.push(item.clone());
                mine = Some(item);
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
    let plan = json!({
        "format": "acyclic-import",
        "version": 1,
        "items": items().iter().map(|key| json!({"key": key, "title": key, "state": "backlog"})).collect::<Vec<_>>(),
        "links": WAITS.iter().map(|(from, to)| json!({"from": from, "to": to, "kind": "blocks"})).collect::<Vec<_>>(),
    });
    let file = dir.path().join("plan.json");
    std::fs::write(&file, plan.to_string()).unwrap();
    let (code, out) =
        acyclic(&tracker, "actor/user", &format!("import {} --as-user", file.display()));
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

    // The audit log, in order: the import, then each item's one claim and one done.
    let log = git(&tracker, &["for-each-ref", "--format=%(refname)", "refs/heads/log/"]);
    let reference = log.trim();
    assert_eq!(reference.lines().count(), 1, "one replica, one segment: {log}");
    let files = git(&tracker, &["ls-tree", "-r", "--name-only", reference]);
    // One file, so its lines are the log's order.
    assert_eq!(files.lines().count(), 1, "{files}");
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
    assert_eq!(events.len(), 1 + 2 * items.len(), "no write lost or repeated");
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
    for item in &items {
        let (claim, done) = (&claims[*item], &dones[*item]);
        assert_eq!((claim.len(), done.len()), (1, 1), "{item}: one claim, one done");
        assert_eq!(claim[0].1, done[0].1, "{item}: finished by the agent that claimed it");
        for (_, dependency) in WAITS.iter().filter(|(waiting, _)| waiting == item) {
            assert!(
                claim[0].0 > dones[*dependency][0].0,
                "{item} claimed before {dependency} was done"
            );
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
    let (_, list) = acyclic(&tracker, "session/check", "list");
    let list = list["data"]["items"].as_array().unwrap();
    assert!(list.iter().all(|item| item["state"] == "done"), "{list:?}");
    let every: Vec<&str> = items.iter().copied().collect();
    let (_, shown) = acyclic(&tracker, "session/check", &format!("show {}", every.join(" ")));
    for item in shown["data"].as_array().unwrap() {
        assert_eq!(item["claims"], json!([]), "nothing left claimed: {item}");
    }
}
