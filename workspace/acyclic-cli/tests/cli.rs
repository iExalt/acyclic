//! The CLI contract against a real tracker: each command's JSON and exit code, snapshotted.

use std::fmt::Write as _;
use std::path::Path;

use acyclic_cli::{Applied, Env, Operation, execute};
use acyclic_core::state::{Context, Key, State};
use acyclic_core::store::Tracker;
use serde_json::json;
use tempfile::TempDir;

/// 2026-10-15T00:00:00Z.
const NOW: &str = "1792022400000";

struct Shell {
    dir: TempDir,
    transcript: String,
}

impl Shell {
    fn new() -> Self {
        Self { dir: TempDir::new().unwrap(), transcript: String::new() }
    }

    fn env(&self, extra: &[(&str, &str)]) -> Env {
        let mut env =
            Env { vars: Default::default(), cwd: self.dir.path().to_path_buf(), terminal: None };
        for (name, value) in
            [("ACYCLIC_NOW", NOW), ("ACYCLIC_TRACKER", "tracker"), ("HOME", "/nonexistent")]
                .iter()
                .chain(extra)
        {
            env.vars.insert((*name).to_owned(), (*value).to_owned());
        }
        env
    }

    /// Runs `acyclic <line>` and records the exit code and output.
    fn run(&mut self, line: &str, extra: &[(&str, &str)]) -> u8 {
        self.output(line, extra).0
    }

    fn output(&mut self, line: &str, extra: &[(&str, &str)]) -> (u8, serde_json::Value) {
        let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
        let outcome = execute(args, &self.env(extra));
        let code = outcome.code;
        let output = outcome.stdout.replace(self.dir.path().to_str().unwrap(), "[dir]");
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap_or_default();
        let shown: Vec<&str> =
            extra.iter().filter(|(n, _)| *n == "ACYCLIC_ACTOR").map(|(_, v)| *v).collect();
        let prefix =
            shown.first().map(|actor| format!("ACYCLIC_ACTOR={actor} ")).unwrap_or_default();
        writeln!(self.transcript, "$ {prefix}acyclic {line}\n# exit {code}\n{output}").unwrap();
        if !outcome.stderr.is_empty() {
            writeln!(self.transcript, "# stderr\n{}", outcome.stderr).unwrap();
        }
        (code, parsed)
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

const AGENT: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/mac")];

#[test]
fn a_journey_through_the_contract() {
    let mut sh = Shell::new();
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    assert_eq!(sh.run("item add a1 --title First", AGENT), 0);
    assert_eq!(sh.run("item add a2 --title Second", AGENT), 0);
    assert_eq!(sh.run("item add a2 --title Second", AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("item add a2 --title Other", AGENT), 1);
    assert_eq!(sh.run("link add a2 waits-for a1", AGENT), 0);
    let (_, ready) = sh.output("ready", AGENT);
    assert_eq!(ready["data"]["ready"], json!([]), "nothing is ready outside a thread");
    assert_eq!(sh.run("thread add t0 --title Start", AGENT), 0);
    for id in ["a1", "a2"] {
        assert_eq!(sh.run(&format!("item edit {id} --thread t0"), AGENT), 0);
    }
    assert_eq!(sh.run("ready", AGENT), 0);
    let (code, out) = sh.output("claim a1", AGENT);
    assert_eq!(
        (code, &out["error"]["details"]["thread"]),
        (3, &json!("t0")),
        "threads are claimed, not items"
    );
    assert_eq!(sh.run("claim t0", AGENT), 0);
    assert_eq!(sh.run("done a2", AGENT), 1, "a2 waits on a1");
    assert_eq!(sh.run("item edit a2 --state done", AGENT), 2, "done finishes items");
    let pi = [("ACYCLIC_ACTOR", "session/pi")];
    assert_eq!(sh.run("claim t0", &pi), 1, "another agent's claim");
    assert_eq!(sh.run("done a1", &pi), 1, "and its items");
    let (code, done) = sh.output("done a1 -n shipped", AGENT);
    assert_eq!(
        (code, &done["data"]["evidence"]),
        (0, &json!({"repository": null, "commit": null, "note": "shipped"})),
        "done echoes what it recorded: {done}"
    );
    assert_eq!(sh.run("done a1 --note shipped", AGENT), 0, "finishing again changes nothing");
    let (code, shown) = sh.output("thread show t0", AGENT);
    assert_eq!(
        (code, &shown["data"][0]["claims"][0]["holder"]),
        (0, &json!("session/mac")),
        "done releases nothing"
    );
    for (released, changed) in [(true, true), (false, false)] {
        let (code, out) = sh.output("release t0", AGENT);
        assert_eq!(
            (code, out["data"]["released"].as_bool(), out["data"]["changed"].as_bool()),
            (0, Some(released), Some(changed)),
            "releasing nothing isn't an error"
        );
    }
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("thread add t1 --title Work", AGENT), 0);
    assert_eq!(sh.run("thread add t1 --title Work", AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("thread add t1 --title Other", AGENT), 1, "a conflicting add is refused");
    assert_eq!(sh.run("item edit a2 --thread nope", AGENT), 3, "a thread that isn't there");
    // An edit refused in part writes nothing: the title is kept when the parent it came with is missing.
    assert_eq!(sh.run("item edit a2 --title Renamed --parent nope", AGENT), 3);
    assert_eq!(sh.output("show a2", AGENT).1["data"][0]["title"], "Second", "refused as a whole");
    assert_eq!(sh.run("item edit a2 --thread t1", AGENT), 0);
    assert_eq!(sh.run("thread approve t1", AGENT), 1, "only the user approves");
    assert_eq!(sh.run("thread approve t1 --as-user", AGENT), 0);
    let (_, ready) = sh.output("ready --approved", AGENT);
    assert_eq!(ready["data"]["ready"][0]["id"], "t1", "{ready}");
    let (code, proposed) = sh.output("item edit a1 --thread t1", AGENT);
    let proposal = proposed["data"]["proposals"][0]["id"].as_str().unwrap();
    assert_eq!(
        (code, &proposed["data"]["proposals"][0]["field"]),
        (0, &json!("thread")),
        "{proposed}"
    );
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run(&format!("proposal show {proposal}"), AGENT), 0);
    assert_eq!(sh.run("proposal list", AGENT), 0);
    assert_eq!(sh.run("proposal show nope", AGENT), 3);
    assert_eq!(sh.run(&format!("proposal keep {proposal} --as-user"), AGENT), 0);
    assert_eq!(sh.run("list", AGENT), 0);
    // a1 joined after the approval, so the approval didn't see it.
    assert_eq!(sh.run("thread list", AGENT), 0);
    // JSON stays the default, byte for byte, and `-o pretty` prints the same data for people, an error on stderr.
    for line in ["list", "ready", "thread list", "show a1", "show nope"] {
        let stdout = |line: String| {
            let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
            execute(args, &sh.env(AGENT)).stdout
        };
        assert_eq!(stdout(format!("{line} -o json")), stdout(line.to_owned()), "{line}");
    }
    for line in ["list", "ready", "thread list", "show a1", "item add a1 --title First"] {
        assert_eq!(sh.run(&format!("{line} -o pretty"), AGENT), 0, "{line}");
    }
    assert_eq!(sh.run("show nope -o pretty", AGENT), 3);
    assert_eq!(sh.run("thread show t1", AGENT), 0);
    assert_eq!(
        sh.run("thread show t1 nope", AGENT),
        3,
        "one missing thread fails the whole command"
    );
    let (code, shown) = sh.output("show a1 a2", AGENT);
    assert_eq!((code, shown["data"][1]["id"].as_str()), (0, Some("a2")), "in the order given");
    assert_eq!(sh.run("show a1 nope", AGENT), 3, "one missing item fails the whole command");
    let (code, out) = sh.output("item edit a2 --no-thread --as-user", AGENT);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)));
    assert_eq!(sh.run("repo add r1 --name code", AGENT), 2, "a new record needs every field");
    let full = "repo add r1 --owner me --name code --default-branch main --public";
    assert_eq!(sh.run(full, AGENT), 0);
    assert_eq!(sh.run(full, AGENT), 0, "a repeat changes nothing");
    assert_eq!(
        sh.run("repo add r1 --owner me --name code --default-branch main --private", AGENT),
        1
    );
    assert_eq!(sh.run("repo edit r1 --default-branch trunk", AGENT), 0, "an edit takes any subset");
    assert_eq!(sh.run("repo edit nope --private", AGENT), 3);
    for (flag, present) in [("--repository", true), ("--no-repository", false)] {
        let (code, out) = sh.output(&format!("item edit a2 {flag} r1"), AGENT);
        assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)), "{flag}");
        assert_eq!(
            sh.output("repo show r1", AGENT).1["data"][0]["items"],
            json!(present.then_some("a2").into_iter().collect::<Vec<_>>())
        );
    }
    assert_eq!(sh.run("repo list", AGENT), 0);
    let (code, out) = sh.output("thread revoke t1 --as-user", AGENT);
    assert_eq!((code, out["data"]["approved"].as_bool()), (0, Some(false)));
    assert_eq!(sh.run("graph --thread t1", AGENT), 0, "DOT on standard output");
    let (code, out) = sh.output("graph --format mermaid --file graph.mmd", AGENT);
    assert_eq!((code, out["data"]["counts"]["items"].as_u64()), (0, Some(2)));
    let mermaid = std::fs::read_to_string(sh.path().join("graph.mmd")).unwrap();
    assert!(mermaid.starts_with("flowchart TB\n"), "{mermaid}");
    assert_eq!(sh.run("graph --root nope", AGENT), 3);
    assert_eq!(sh.run("graph --format html", AGENT), 2, "a page never goes to standard output");
    let opener = [("ACYCLIC_ACTOR", "session/mac"), ("ACYCLIC_OPENER", "true")];
    let (code, out) = sh.output("graph --open --thread t1", &opener);
    assert_eq!((code, out["data"]["opened"].as_bool()), (0, Some(true)), "{out}");
    let path = out["data"]["path"].as_str().unwrap().replace("[dir]", sh.path().to_str().unwrap());
    let page = std::fs::read_to_string(path).unwrap();
    assert!(page.contains("data-renderer=\"graphviz\""));
    assert_eq!(sh.run("link add a1 sideways a2", AGENT), 2);
    assert_eq!(sh.run("link add a1 blocks a2", AGENT), 2, "the old words are refused");
    let (code, unlinked) = sh.output("link remove a2 waits-for a1", AGENT);
    assert_eq!((code, unlinked["data"]["changed"].as_bool()), (0, Some(true)), "a clear changes");
    assert_eq!(sh.run("ready", &[("ACYCLIC_ACTOR", "actor/user")]), 2);
    // a2 left t1; an agent groups it into a new thread, which the user then sees awaiting approval.
    assert_eq!(sh.run("thread add t2 --title Follow-ups", AGENT), 0);
    let (code, out) = sh.output("item edit a2 --thread t2", AGENT);
    assert_eq!((code, &out["data"]["proposals"]), (0, &json!([])), "unapproved scope");
    // A thread where an item belongs says which command was meant.
    let (code, out) = sh.output("item edit a2 --parent t2", AGENT);
    let meant = out["error"]["details"]["meant"].as_str();
    assert_eq!((code, meant), (3, Some("acyclic item edit a2 --thread t2")));
    assert_eq!(sh.run("level", AGENT), 0);
    assert_eq!(sh.run("level everything", AGENT), 1, "only the user sets the level");
    assert_eq!(sh.run("level everything --as-user", AGENT), 0);
    assert_eq!(sh.run("thread edit t2 --level blockers --as-user", AGENT), 0);
    assert_eq!(sh.run("level", AGENT), 0);
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run("item add a3 --title Second-follow-up", AGENT), 0);
    assert_eq!(
        sh.run("item add a3 --title Second-follow-up --parent a2", AGENT),
        1,
        "another parent"
    );
    let (_, under) = sh.output("item add a4 --title Third-follow-up --parent a2", AGENT);
    assert_eq!(under["data"]["placed"][0]["thread"], "t2", "a parent passes its thread down");
    // A parent is finished once its children are.
    sh.run("item add g1 --title Goal", AGENT);
    sh.run("item add g1-a --title Part --parent g1", AGENT);
    let (code, out) = sh.output("done g1", AGENT);
    assert_eq!((code, &out["error"]["details"]["open"]), (1, &serde_json::json!(["g1-a"])));
    sh.run("item edit g1-a --thread t2", AGENT);
    sh.run("claim t2", AGENT);
    sh.run("done g1-a", AGENT);
    assert_eq!(sh.run("done g1", AGENT), 0, "a parent is finished once its children are");
    sh.run("item add g2 --title Goal", AGENT);
    sh.run("item add g2-a --title Part --parent g2", AGENT);
    assert_eq!(sh.run("done g2 --force", AGENT), 0, "or when forced");
    // Only an item nothing refers to is deleted, and it is gone for good.
    assert_eq!(sh.run("item add d1 --title Doomed", AGENT), 0);
    assert_eq!(sh.run("item add d1-a --title Part --parent d1", AGENT), 0);
    let (code, out) = sh.output("item delete d1", AGENT);
    assert_eq!((code, &out["error"]["details"]["reasons"][0]["why"]), (1, &json!("has-children")));
    assert_eq!(sh.run("item delete a1", AGENT), 1, "done work stays");
    assert_eq!(sh.run("item delete d1-a", AGENT), 0);
    assert_eq!(sh.run("item delete d1", AGENT), 0);
    let (code, out) = sh.output("item delete d1", AGENT);
    assert_eq!(
        (code, out["data"]["changed"].as_bool()),
        (0, Some(false)),
        "a repeat changes nothing"
    );
    assert_eq!(sh.run("show d1", AGENT), 3);
    assert_eq!(sh.run("item add d1 --title Doomed", AGENT), 1, "its id is retired");
    assert_eq!(sh.run("item delete nope", AGENT), 3);
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn an_unreadable_state_points_to_doctor() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
    sh.run("item add a1 --title First", AGENT);
    // A value from a newer version: an item state this build doesn't know.
    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into(), run: None };
    let key = acyclic_core::model::item_key("a1").unwrap();
    tracker
        .batch(&meta, NOW.parse().unwrap(), false, &|_, _| None, |replica, at| {
            replica
                .write(&key, "state", "actor/user", at, Some(serde_json::json!("from-the-future")))
                .map(|_| ())
        })
        .unwrap();
    assert_eq!(sh.run("ready", AGENT), 6);
    assert_eq!(sh.run("doctor", AGENT), 0);
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn a_follows_link_an_older_version_stored_is_removed_as_relates_to() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
    for id in ["x", "z"] {
        sh.run(&format!("item add {id} --title {id}"), AGENT);
    }
    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into(), run: None };
    let key =
        acyclic_core::model::link_key("z", "x", acyclic_core::model::LinkKind::After).unwrap();
    tracker
        .batch(&meta, NOW.parse().unwrap(), false, &|_, _| None, |replica, at| {
            for (field, value) in [
                ("from", json!("z")),
                ("to", json!("x")),
                ("kind", json!("after")),
                ("present", json!(true)),
            ] {
                replica.write(&key, field, "actor/user", at, Some(value))?;
            }
            Ok::<(), acyclic_core::state::Error>(())
        })
        .unwrap();
    assert_eq!(sh.output("show z", AGENT).1["data"][0]["links"]["relates_to"], json!(["x"]));
    let (code, removed) = sh.output("link remove z relates-to x", AGENT);
    assert_eq!((code, &removed["data"]["changed"]), (0, &json!(true)), "{removed}");
    assert_eq!(sh.output("show z", AGENT).1["data"][0]["links"]["relates_to"], json!([]));
}

#[test]
fn an_import_is_an_upsert_and_a_dry_run_writes_nothing() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
    let mut plan = acyclic_fixtures::generate(50, 7);
    let file = sh.path().join("plan.json");
    std::fs::write(&file, plan.to_json()).unwrap();
    let import = |sh: &mut Shell, extra: &str| {
        let (code, out) = sh.output(&format!("import plan.json --as-user{extra}"), &[]);
        assert_eq!(code, 0, "{out}");
        out["data"].clone()
    };
    let dry = import(&mut sh, " --dry-run");
    assert_eq!(
        (dry["changed"].as_bool(), dry["counts"]["items"].as_u64()),
        (Some(false), Some(50))
    );
    // Each record is listed as created, with its fields, and nothing has been written.
    let creates =
        dry["changes"].as_array().unwrap().iter().filter(|c| c["action"] == "create").count();
    assert_eq!(
        creates,
        50 + dry["counts"]["links"].as_u64().unwrap() as usize
            + dry["counts"]["threads"].as_u64().unwrap() as usize
            + dry["counts"]["repositories"].as_u64().unwrap() as usize
    );
    // The plan's unthreaded work joins the threads above it.
    let inherited = dry["placed"].clone();
    assert!(inherited.as_array().is_some_and(|i| !i.is_empty()), "{dry}");
    assert_eq!(sh.output("doctor", AGENT).1["data"]["counts"]["items"], 0);
    let done = import(&mut sh, "");
    assert_eq!((done["changed"].as_bool(), &done["placed"]), (Some(true), &inherited));
    let refs = |sh: &Shell| {
        let out = std::process::Command::new("git")
            .args(["for-each-ref", "--format=%(refname) %(objectname)"])
            .current_dir(sh.path().join("tracker"))
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let before = refs(&sh);
    assert!(before.contains("refs/heads/state/"), "{before}");
    let again = import(&mut sh, "");
    assert_eq!(
        (&again["changed"], &again["changes"]),
        (&json!(false), &json!([])),
        "a repeat changes nothing"
    );
    assert_eq!(refs(&sh), before, "and commits nothing");
    plan.items[0].title.push_str(" renamed");
    std::fs::write(&file, plan.to_json()).unwrap();
    let renamed = import(&mut sh, "");
    assert_eq!(
        (renamed["changed"].as_bool(), renamed["counts"]["fields"].as_u64()),
        (Some(true), Some(1))
    );
    assert_eq!(renamed["changes"][0]["fields"]["title"]["to"], plan.items[0].title);
    assert_eq!(sh.run("import missing.json --as-user", &[]), 3);
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        // Without an identity, a command that writes a reflog looks the host name up to make an email address, which
        // stalls each call by seconds on a machine with a slow resolver.
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn two_machines_sync_through_a_shared_repository() {
    let mut sh = Shell::new();
    let pi: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/pi"), ("ACYCLIC_TRACKER", "pi")];
    git(sh.path(), &["init", "-q", "--bare", "shared.git"]);
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    let (code, out) = sh.output("sync", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (5, Some("no-remote")));
    let (code, out) = sh.output("init tracker --replica mac --remote ../shared.git", &[]);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)), "a remote added later");
    sh.run("item add a1 --title First", AGENT);
    let (code, out) = sh.output("sync", AGENT);
    assert_eq!((code, out["data"]["pushed"].as_u64().is_some_and(|n| n > 0)), (0, true));
    // A second machine starts from the shared repository and sees the first's work.
    assert_eq!(sh.run("init pi --replica pi --remote ../shared.git", &[]), 0);
    let (code, out) = sh.output("sync", pi);
    assert_eq!(
        (code, &out["data"]["fetched"], out["data"]["joined"].as_bool()),
        (0, &serde_json::json!(["mac"]), Some(true))
    );
    assert_eq!(sh.output("show a1", pi).0, 0);
    sh.run("item add b1 --title Second", pi);
    sh.run("sync", pi);
    let (_, out) = sh.output("sync", AGENT);
    assert_eq!(out["data"]["fetched"], serde_json::json!(["mac", "pi"]));
    assert_eq!(sh.output("show b1", AGENT).0, 0, "and the first sees the second's");
    // Asked to push every write, a write reports its push; one that fails shows in needs until a sync.
    git(&sh.path().join("tracker"), &["config", "acyclic.pushAfterWrite", "true"]);
    let (_, out) = sh.output("item add a2 --title Pushed", AGENT);
    assert_eq!(out["data"]["push"]["ok"].as_bool(), Some(true), "{out}");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../nowhere.git"]);
    let (code, out) = sh.output("item add a3 --title Stranded", AGENT);
    assert_eq!((code, out["data"]["push"]["ok"].as_bool()), (0, Some(false)), "the write stands");
    // The needs, leaving out the items in no thread, which this test never threads.
    let needs = |sh: &mut Shell| {
        let (_, out) = sh.output("needs", AGENT);
        let all = out["data"]["needs"].as_array().unwrap().clone();
        all.into_iter().filter(|entry| entry["kind"] != "unthreaded").collect::<Vec<_>>()
    };
    assert_eq!(needs(&mut sh)[0]["kind"], "unpushed");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../shared.git"]);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
    // The user's move to the top racing the other machine's move is a conflict, and moving it again settles it.
    sh.run("item edit a1 --parent a2", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", pi);
    sh.run("item edit a1 --no-parent --as-user", AGENT);
    sh.run("item edit a1 --parent b1", pi);
    sh.run("sync", pi);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["kind"], "conflict");
    let (_, out) = sh.output("item edit a1 --no-parent --as-user", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "a move to the top resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
    // Likewise taking an item out of a thread while the other machine puts it in one.
    sh.run("thread add t1 --title One", AGENT);
    sh.run("thread add t2 --title Two", AGENT);
    sh.run("item edit a1 --thread t1", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", pi);
    sh.run("item edit a1 --no-thread --as-user", AGENT);
    sh.run("item edit a1 --thread t2", pi);
    sh.run("sync", pi);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["field"], "thread");
    let (_, out) = sh.output("item edit a1 --no-thread --as-user", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "taking it out again resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

/// A run's operation applied to the shell's tracker as run `r1`.
fn operation(
    sh: &Shell,
    seq: u64,
    seen: &Context,
    command: serde_json::Value,
    extra: &[(&str, &str)],
) -> Result<Applied, (u8, serde_json::Value)> {
    let operation = Operation { run: "r1".into(), thread: "t0".into(), seq, seen: seen.clone() };
    let command = serde_json::from_value(command).unwrap();
    acyclic_cli::apply(operation, command, sh.path().join("tracker"), &sh.env(extra))
}

fn state(sh: &Shell) -> State {
    Tracker::open(&sh.path().join("tracker")).unwrap().0.load().unwrap().1
}

/// Who wrote each of an item's titles, in order.
fn titles(sh: &Shell, item: &str) -> Vec<(String, serde_json::Value)> {
    let key = Key::new("item", item).unwrap();
    let state = state(sh);
    state
        .field(&key, "title")
        .into_iter()
        .flatten()
        .map(|(_, v)| (v.actor.clone(), v.value.clone()))
        .collect()
}

#[test]
fn a_runs_operations_apply_once_each_as_the_run() {
    let mut sh = Shell::new();
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    for line in [
        "thread add t0 --title T",
        "item add a1 --title One --thread t0",
        "item add a2 --title Two --thread t0",
    ] {
        assert_eq!(sh.run(&format!("{line} --as-user"), &[]), 0, "{line}");
    }
    let base = state(&sh).context;
    let ok =
        |code: u8| move |applied: &Applied| matches!(applied, Applied::Ran(c, _) if *c == code);
    let edit = |item: &str, title: &str| json!({"item": {"edit": {"item": item, "title": title}}});

    // The run's edit is its own, whatever actor it might claim elsewhere.
    let first =
        operation(&sh, 1, &base, edit("a1", "Run's"), &[("ACYCLIC_ACTOR", "session/other")])
            .unwrap();
    assert!(ok(0)(&first), "{first:?}");
    assert_eq!(titles(&sh, "a1"), [("run/r1".to_owned(), json!("Run's"))]);
    // Sent again, it applies nothing and says how it went.
    let before = state(&sh);
    let again = operation(&sh, 1, &base, edit("a1", "Changed"), &[]).unwrap();
    assert_eq!(again, Applied::Duplicate(Some(json!({"ok": true}))));
    assert_eq!(state(&sh), before);

    // A gap, or a clock that stepped back, says nothing of the command, so the number stays unspent.
    let (code, gap) = operation(&sh, 3, &base, edit("a1", "Later"), &[]).unwrap_err();
    assert_eq!((code, &gap["error"]["code"]), (1, &json!("sequence-gap")));
    let (_, stepped) =
        operation(&sh, 2, &base, edit("a1", "Early"), &[("ACYCLIC_NOW", "1000")]).unwrap_err();
    assert_eq!(stepped["error"]["code"], "clock-jump");

    // A refused command spends its number, so the run moves on, and sending it again gets the same refusal.
    let approve = json!({"thread": {"approve": {"thread": "t0"}}});
    let refused = operation(&sh, 2, &base, approve.clone(), &[]).unwrap();
    let Applied::Ran(1, envelope) = &refused else { panic!("{refused:?}") };
    let code = envelope["error"]["code"].clone();
    assert_eq!(
        operation(&sh, 2, &base, approve, &[]).unwrap(),
        Applied::Duplicate(Some(json!({"ok": false, "code": code})))
    );
    // So does a usage error, which never reaches the tracker.
    let usage = operation(&sh, 3, &base, json!({"show": {"items": []}}), &[]).unwrap();
    assert!(ok(2)(&usage), "{usage:?}");
    // A command that writes nothing still spends its number.
    assert!(ok(0)(&operation(&sh, 4, &base, json!({"show": {"items": ["a1"]}}), &[]).unwrap()));
    assert!(matches!(operation(&sh, 4, &base, json!("list"), &[]), Ok(Applied::Duplicate(_))));

    // An edit the run never saw survives beside the run's own; the run's earlier edit doesn't.
    assert_eq!(sh.run("item edit a2 --title Mine --as-user", &[]), 0);
    assert!(ok(0)(&operation(&sh, 5, &base, edit("a2", "Run's first"), &[]).unwrap()));
    assert!(ok(0)(&operation(&sh, 6, &base, edit("a2", "Run's"), &[]).unwrap()));
    assert_eq!(
        titles(&sh, "a2"),
        [("actor/user".to_owned(), json!("Mine")), ("run/r1".to_owned(), json!("Run's"))]
    );
}

#[test]
fn a_run_writes_only_within_its_thread() {
    let mut sh = Shell::new();
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    for line in [
        "thread add t0 --title Mine",
        "thread add t9 --title Other",
        "item add a --title A --thread t0",
        "item add b --title B --thread t0",
        "item add x --title X --thread t9",
        "link add b waits-for a",
    ] {
        assert_eq!(sh.run(&format!("{line} --as-user"), &[]), 0, "{line}");
    }
    let base = state(&sh).context;
    let mut seq = 0;
    let mut send = |command: serde_json::Value| {
        seq += 1;
        match operation(&sh, seq, &base, command.clone(), &[]).unwrap() {
            Applied::Ran(0, _) => "ok".to_owned(),
            Applied::Ran(_, envelope) => envelope["error"]["code"].as_str().unwrap().to_owned(),
            Applied::Duplicate(_) => panic!("{command}"),
        }
    };
    let edit = |item: &str, extra: serde_json::Value| {
        let mut edit = json!({"item": item});
        edit.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({"item": {"edit": edit}})
    };
    for (command, expected) in [
        (json!({"claim": {"thread": "t9"}}), "not-in-thread"),
        (json!({"item": {"add": {"item": "n", "title": "N"}}}), "not-in-thread"),
        (json!({"item": {"add": {"item": "n", "title": "N", "thread": "t9"}}}), "not-in-thread"),
        (
            json!({"item": {"add": {"item": "n", "title": "N", "thread": "t0", "parent": "x"}}}),
            "not-in-thread",
        ),
        (
            json!({"item": {"add": {"item": "n", "title": "N", "thread": "t0", "parent": "a"}}}),
            "ok",
        ),
        (edit("x", json!({"title": "Mine now"})), "not-in-thread"),
        (edit("a", json!({"thread": "t9"})), "not-in-thread"),
        (edit("a", json!({"no-thread": true})), "not-in-thread"),
        (edit("n", json!({"no-parent": true})), "not-in-thread"),
        (edit("a", json!({"parent": "x"})), "not-in-thread"),
        (edit("a", json!({"title": "Better"})), "ok"),
        (json!({"done": {"item": "x", "note": "n"}}), "not-in-thread"),
        (json!({"link": {"add": {"from": "x", "kind": "waits-for", "to": "a"}}}), "not-in-thread"),
        // A run may make its own work wait on another thread's, and undo that, but not the user's ordering.
        (json!({"link": {"add": {"from": "a", "kind": "waits-for", "to": "x"}}}), "ok"),
        (json!({"link": {"remove": {"from": "a", "kind": "waits-for", "to": "x"}}}), "ok"),
        (
            json!({"link": {"remove": {"from": "b", "kind": "waits-for", "to": "a"}}}),
            "not-the-runs",
        ),
        (json!({"claim": {"thread": "t0"}}), "ok"),
    ] {
        assert_eq!(send(command.clone()), expected, "{command}");
    }
}
