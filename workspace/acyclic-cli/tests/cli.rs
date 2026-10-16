//! The CLI contract against a real tracker: each command's JSON and exit code, snapshotted.

use std::fmt::Write as _;
use std::path::Path;

use acyclic_cli::{Env, execute};
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
        let mut env = Env { vars: Default::default(), cwd: self.dir.path().to_path_buf() };
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
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("claim a2", AGENT), 1, "a2 waits on a1");
    assert_eq!(sh.run("done a2", AGENT), 1, "nor can it be finished");
    assert_eq!(sh.run("item state a2 done", AGENT), 2, "done finishes items");
    assert_eq!(sh.run("claim a1", AGENT), 0);
    let pi = [("ACYCLIC_ACTOR", "session/pi")];
    assert_eq!(sh.run("done a1", &pi), 1, "another agent's claim");
    assert_eq!(sh.run("done a1 --note shipped", AGENT), 0);
    assert_eq!(sh.run("done a1 --note shipped", AGENT), 0, "finishing again changes nothing");
    let (code, released) = sh.output("release a1", AGENT);
    assert_eq!(
        (code, released["data"]["released"].as_bool(), released["data"]["changed"].as_bool()),
        (0, Some(false), Some(false)),
        "done released the claim already, and releasing nothing isn't an error"
    );
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("thread set t1 --title Work", AGENT), 0);
    assert_eq!(sh.run("thread add a2 t1", AGENT), 0);
    assert_eq!(sh.run("thread approve t1", AGENT), 1, "only the user approves");
    assert_eq!(sh.run("thread approve t1 --as-user", AGENT), 0);
    let (_, ready) = sh.output("ready --approved", AGENT);
    assert_eq!(ready["data"]["ready"][0]["thread"], "t1", "{ready}");
    let (code, proposed) = sh.output("thread add a1 t1", AGENT);
    assert_eq!((code, proposed["data"]["outcome"].as_str()), (0, Some("proposed")));
    assert_eq!(sh.run("needs", AGENT), 0);
    let proposal = proposed["data"]["proposal"].as_str().unwrap();
    assert_eq!(sh.run(&format!("proposal keep {proposal} --as-user"), AGENT), 0);
    assert_eq!(sh.run("list", AGENT), 0);
    // a1 joined after the approval, so the approval didn't see it.
    assert_eq!(sh.run("thread list", AGENT), 0);
    assert_eq!(sh.run("thread show t1", AGENT), 0);
    assert_eq!(
        sh.run("thread show t1 nope", AGENT),
        3,
        "one missing thread fails the whole command"
    );
    let (code, shown) = sh.output("show a1 a2", AGENT);
    assert_eq!(
        (code, shown["data"][1]["item"]["id"].as_str()),
        (0, Some("a2")),
        "in the order given"
    );
    assert_eq!(sh.run("show a1 nope", AGENT), 3, "one missing item fails the whole command");
    let (code, out) = sh.output("thread remove a2 --as-user", AGENT);
    assert_eq!((code, out["data"]["edit"]["thread"].is_null()), (0, true));
    assert_eq!(sh.run("repo set r1 --name code", AGENT), 2, "a new record needs every field");
    let full = "repo set r1 --owner me --name code --public true --default-branch main";
    assert_eq!(sh.run(full, AGENT), 0);
    assert_eq!(sh.run("repo set r1 --default-branch trunk", AGENT), 0, "an edit takes any subset");
    for (verb, present) in [("attach", true), ("detach", false)] {
        let (code, out) = sh.output(&format!("repo {verb} r1 --item a2"), AGENT);
        let edit = (out["data"]["outcome"].as_str(), out["data"]["edit"]["present"].as_bool());
        assert_eq!((code, edit), (0, (Some("applied"), Some(present))), "{verb}");
    }
    let (code, out) = sh.output("thread revoke t1 --as-user", AGENT);
    assert_eq!((code, out["data"]["approved"].as_bool()), (0, Some(false)));
    assert_eq!(sh.run("graph --thread t1", AGENT), 0, "DOT on standard output");
    let (code, out) = sh.output("graph --format mermaid --output graph.mmd", AGENT);
    assert_eq!((code, out["data"]["items"].as_u64()), (0, Some(2)));
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
    let (code, unlinked) = sh.output("link remove a2 waits-for a1", AGENT);
    assert_eq!((code, unlinked["data"]["changed"].as_bool()), (0, Some(true)), "a clear changes");
    assert_eq!(sh.run("ready", &[("ACYCLIC_ACTOR", "actor/user")]), 2);
    // a2 left t1; an agent groups it into a new thread, which the user then sees awaiting approval.
    assert_eq!(sh.run("thread set t2 --title Follow-ups", AGENT), 0);
    let (code, out) = sh.output("thread add a2 t2", AGENT);
    assert_eq!((code, out["data"]["outcome"].as_str()), (0, Some("applied")), "unapproved scope");
    // A thread where an item belongs says which command was meant.
    for (line, meant) in [
        ("thread add t2 a2", "acyclic thread add a2 t2"),
        ("item move a2 t2", "acyclic thread add a2 t2"),
    ] {
        let (code, out) = sh.output(line, AGENT);
        assert_eq!((code, out["error"]["details"]["meant"].as_str()), (3, Some(meant)), "{line}");
    }
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run("item add a3 --title Second-follow-up", AGENT), 0);
    let (_, under) = sh.output("item add a3 --title Second-follow-up --parent a2", AGENT);
    assert_eq!(under["data"]["inherited"][0]["thread"], "t2", "a parent passes its thread down");
    // A parent is finished once its children are.
    sh.run("item add g1 --title Goal", AGENT);
    sh.run("item add g1-a --title Part --parent g1", AGENT);
    let (code, out) = sh.output("done g1", AGENT);
    assert_eq!((code, &out["error"]["details"]["open"]), (1, &serde_json::json!(["g1-a"])));
    sh.run("claim g1-a", AGENT);
    sh.run("done g1-a", AGENT);
    assert_eq!(sh.run("done g1", AGENT), 0, "a parent is finished once its children are");
    sh.run("item add g2 --title Goal", AGENT);
    sh.run("item add g2-a --title Part --parent g2", AGENT);
    assert_eq!(sh.run("done g2 --force", AGENT), 0, "or when forced");
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
        (dry["changed"].as_bool(), dry["report"]["items"].as_u64()),
        (Some(false), Some(50))
    );
    // The plan's unthreaded work joins the threads above it.
    let inherited = dry["inherited"].clone();
    assert!(inherited.as_array().is_some_and(|i| !i.is_empty()), "{dry}");
    assert_eq!(sh.output("doctor", AGENT).1["data"]["items"], 0);
    let done = import(&mut sh, "");
    assert_eq!((done["changed"].as_bool(), &done["inherited"]), (Some(true), &inherited));
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
    assert_eq!(import(&mut sh, "")["changed"], false, "a repeat changes nothing");
    assert_eq!(refs(&sh), before, "and commits nothing");
    plan.items[0].title.push_str(" renamed");
    std::fs::write(&file, plan.to_json()).unwrap();
    let renamed = import(&mut sh, "");
    assert_eq!(
        (renamed["changed"].as_bool(), renamed["report"]["fields"].as_u64()),
        (Some(true), Some(1))
    );
    assert_eq!(sh.run("import missing.json --as-user", &[]), 3);
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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
    let (_, out) = sh.output("needs", AGENT);
    assert_eq!(out["data"]["needs"][0]["entry"], "unpushed");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../shared.git"]);
    sh.run("sync", AGENT);
    assert_eq!(sh.output("needs", AGENT).1["data"]["needs"], serde_json::json!([]));
    // The user's move to the top racing the other machine's move is a conflict, and moving it again settles it.
    sh.run("item move a1 a2", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", pi);
    sh.run("item move a1 --as-user", AGENT);
    sh.run("item move a1 b1", pi);
    sh.run("sync", pi);
    sh.run("sync", AGENT);
    assert_eq!(sh.output("needs", AGENT).1["data"]["needs"][0]["entry"], "conflict");
    let (_, out) = sh.output("item move a1 --as-user", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "a move to the top resolves it");
    assert_eq!(sh.output("needs", AGENT).1["data"]["needs"], serde_json::json!([]));
    // Likewise taking an item out of a thread while the other machine puts it in one.
    sh.run("thread add a1 t1", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", pi);
    sh.run("thread remove a1 --as-user", AGENT);
    sh.run("thread add a1 t2", pi);
    sh.run("sync", pi);
    sh.run("sync", AGENT);
    assert_eq!(sh.output("needs", AGENT).1["data"]["needs"][0]["field"], "thread");
    let (_, out) = sh.output("thread remove a1 --as-user", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "taking it out again resolves it");
    assert_eq!(sh.output("needs", AGENT).1["data"]["needs"], serde_json::json!([]));
}
