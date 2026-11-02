//! The CLI contract against a real tracker: each command's JSON and exit code, snapshotted.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use crate::{Env, execute};

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
        let mut env = Env {
            vars: std::collections::BTreeMap::default(),
            cwd: self.dir.path().to_path_buf(),
            terminal: None,
        };
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
#[expect(clippy::too_many_lines, reason = "one transcript, command by command")]
fn a_journey_through_the_contract() {
    let mut sh = Shell::new();
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    assert_eq!(sh.run("item add a1 --title First", AGENT), 0);
    assert_eq!(sh.run("item add a2 --title Second", AGENT), 0);
    assert_eq!(sh.run("item add a2 --title Second", AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("item add a2 --title Other", AGENT), 1);
    assert_eq!(sh.run("item edit a2 --waits-for a1", AGENT), 0);
    let (_, ready) = sh.output("thread ready", AGENT);
    assert_eq!(ready["data"]["ready"], json!([]), "nothing is ready outside a thread");
    assert_eq!(sh.run("thread add t0 --title Start", AGENT), 0);
    for id in ["a1", "a2"] {
        assert_eq!(sh.run(&format!("item edit {id} --thread t0"), AGENT), 0);
    }
    assert_eq!(sh.run("thread ready", AGENT), 0);
    let (code, out) = sh.output("thread claim a1", AGENT);
    assert_eq!(
        (code, &out["error"]["details"]["thread"]),
        (3, &json!("t0")),
        "threads are claimed, not items"
    );
    assert_eq!(sh.run("thread claim t0", AGENT), 0);
    assert_eq!(sh.run("item done a2", AGENT), 1, "a2 waits on a1");
    assert_eq!(sh.run("item edit a2 --state done", AGENT), 2, "item done finishes items");
    let pi = [("ACYCLIC_ACTOR", "session/pi")];
    assert_eq!(sh.run("thread claim t0", &pi), 1, "another agent's claim");
    assert_eq!(sh.run("item done a1", &pi), 1, "and its items");
    let (code, done) = sh.output("item done a1 -e shipped", AGENT);
    assert_eq!(
        (code, &done["data"]["evidence"]),
        (0, &json!({"repository": null, "commit": null, "note": "shipped"})),
        "done echoes what it recorded: {done}"
    );
    assert_eq!(
        sh.run("item done a1 --evidence shipped", AGENT),
        0,
        "finishing again changes nothing"
    );
    let (code, shown) = sh.output("thread show t0", AGENT);
    assert_eq!(
        (code, &shown["data"][0]["claims"][0]["holder"]),
        (0, &json!("session/mac")),
        "done releases nothing"
    );
    for (released, changed) in [(true, true), (false, false)] {
        let (code, out) = sh.output("thread release t0", AGENT);
        assert_eq!(
            (code, out["data"]["released"].as_bool(), out["data"]["changed"].as_bool()),
            (0, Some(released), Some(changed)),
            "releasing nothing isn't an error"
        );
    }
    assert_eq!(sh.run("thread ready", AGENT), 0);
    assert_eq!(sh.run("thread add t1 --title Work", AGENT), 0);
    assert_eq!(sh.run("thread add t1 --title Work", AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("thread add t1 --title Other", AGENT), 1, "a conflicting add is refused");
    assert_eq!(sh.run("item edit a2 --thread nope", AGENT), 3, "a thread that isn't there");
    // An edit refused in part writes nothing: the title is kept when the parent it came with is missing.
    assert_eq!(sh.run("item edit a2 --title Renamed --parent nope", AGENT), 3);
    assert_eq!(
        sh.output("item show a2", AGENT).1["data"][0]["title"],
        "Second",
        "refused as a whole"
    );
    assert_eq!(sh.run("item edit a2 --thread t1", AGENT), 0);
    let (_, ready) = sh.output("thread ready", AGENT);
    assert_eq!(ready["data"]["ready"][0]["id"], "t1", "{ready}");
    assert_eq!(sh.run("item edit a1 --thread t1", AGENT), 0);
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run("item list", AGENT), 0);
    assert_eq!(sh.run("thread list", AGENT), 0);
    // JSON stays the default, byte for byte, and `-o pretty` prints the same data for people, an error on stderr.
    for line in ["item list", "thread ready", "thread list", "item show a1", "item show nope"] {
        let stdout = |line: String| {
            let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
            execute(args, &sh.env(AGENT)).stdout
        };
        assert_eq!(stdout(format!("{line} -o json")), stdout(line.to_owned()), "{line}");
    }
    for line in
        ["item list", "thread ready", "thread list", "item show a1", "item add a1 --title First"]
    {
        assert_eq!(sh.run(&format!("{line} -o pretty"), AGENT), 0, "{line}");
    }
    assert_eq!(sh.run("item show nope -o pretty", AGENT), 3);
    assert_eq!(sh.run("thread show t1", AGENT), 0);
    assert_eq!(
        sh.run("thread show t1 nope", AGENT),
        3,
        "one missing thread fails the whole command"
    );
    let (code, shown) = sh.output("item show a1 a2", AGENT);
    assert_eq!((code, shown["data"][1]["id"].as_str()), (0, Some("a2")), "in the order given");
    assert_eq!(sh.run("item show a1 nope", AGENT), 3, "one missing item fails the whole command");
    let (code, out) = sh.output("item edit a2 --no-thread", AGENT);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)));
    assert_eq!(sh.run("repository add r1 --name code", AGENT), 2, "a new record needs every field");
    let full = "repository add r1 --owner me --name code --default-branch main";
    assert_eq!(sh.run(full, AGENT), 0);
    assert_eq!(sh.run(full, AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("repository add r1 --owner me --name code --default-branch trunk", AGENT), 1);
    assert_eq!(
        sh.run("repository edit r1 --default-branch trunk", AGENT),
        0,
        "an edit takes any subset"
    );
    assert_eq!(sh.run("repository edit nope --default-branch main", AGENT), 3);
    for (flag, present) in [("--repository", true), ("--no-repository", false)] {
        let (code, out) = sh.output(&format!("item edit a2 {flag} r1"), AGENT);
        assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)), "{flag}");
        assert_eq!(
            sh.output("repository show r1", AGENT).1["data"][0]["items"],
            json!(present.then_some("a2").into_iter().collect::<Vec<_>>())
        );
    }
    assert_eq!(sh.run("repository list", AGENT), 0);
    // The tracker assigns a thread's id when the add leaves it out. Only a thread whose number another thread's id
    // shares is renumbered, which this one's doesn't.
    let (code, added) = sh.output("thread add --title Spare", AGENT);
    let spare = added["data"]["thread"].as_str().unwrap().to_owned();
    assert_eq!(code, 0, "{added}");
    let (code, out) = sh.output(&format!("thread renumber {spare}"), AGENT);
    assert_eq!(
        (code, &out["error"]["details"]["reasons"][0]["why"]),
        (1, &json!("no-clash")),
        "{out}"
    );
    let (code, out) = sh.output("item edit a1 --title Renamed --waits-for a2", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("link-cycle")), "{out}");
    assert_eq!(
        sh.output("item show a1", AGENT).1["data"][0]["title"],
        "First",
        "refused as a whole"
    );
    let (code, unlinked) = sh.output("item edit a2 --no-waits-for a1", AGENT);
    assert_eq!((code, unlinked["data"]["changed"].as_bool()), (0, Some(true)), "a clear changes");
    let links = &sh.output("item show a2", AGENT).1["data"][0]["links"];
    assert_eq!(links, &json!({"waits_for": [], "waited_on_by": []}));
    // a2 left t1; an agent groups it into a new thread.
    assert_eq!(sh.run("thread add t2 --title Follow-ups", AGENT), 0);
    assert_eq!(sh.run("item edit a2 --thread t2", AGENT), 0);
    // A thread where an item belongs says which command was meant.
    let (code, out) = sh.output("item edit a2 --parent t2", AGENT);
    let meant = out["error"]["details"]["meant"].as_str();
    assert_eq!((code, meant), (3, Some("acyclic item edit a2 --thread t2")));
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
    let (code, out) = sh.output("item done g1", AGENT);
    assert_eq!((code, &out["error"]["details"]["open"]), (1, &serde_json::json!(["g1-a"])));
    sh.run("item edit g1-a --thread t2", AGENT);
    sh.run("thread claim t2", AGENT);
    sh.run("item done g1-a", AGENT);
    assert_eq!(sh.run("item done g1", AGENT), 0, "a parent is finished once its children are");
    sh.run("item add g2 --title Goal", AGENT);
    sh.run("item add g2-a --title Part --parent g2", AGENT);
    assert_eq!(sh.run("item done g2 --force", AGENT), 0, "or when forced");
    // Only an item nothing refers to is deleted, and it is gone for good.
    assert_eq!(sh.run("item add d1 --title Doomed", AGENT), 0);
    assert_eq!(sh.run("item add d1-a --title Part --parent d1", AGENT), 0);
    let (code, out) = sh.output("item delete d1", AGENT);
    assert_eq!((code, &out["error"]["details"]["reasons"][0]["why"]), (1, &json!("has-children")));
    assert_eq!(sh.run("item delete a1", AGENT), 1, "item done work stays");
    assert_eq!(sh.run("item delete d1-a", AGENT), 0);
    assert_eq!(sh.run("item delete d1", AGENT), 0);
    let (code, out) = sh.output("item delete d1", AGENT);
    assert_eq!(
        (code, out["data"]["changed"].as_bool()),
        (0, Some(false)),
        "a repeat changes nothing"
    );
    assert_eq!(sh.run("item show d1", AGENT), 3);
    assert_eq!(sh.run("item add d1 --title Doomed", AGENT), 1, "its id is retired");
    assert_eq!(sh.run("item delete nope", AGENT), 3);
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn an_unreadable_state_points_to_doctor() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
    sh.run("item add a1 --title First", AGENT);
    sh.run("item add a2 --title Second", AGENT);
    // What this build no longer reads: the stored in-progress state and a relates link older versions wrote.
    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into() };
    let item = acyclic_core::model::item_key("a1").unwrap();
    let link = acyclic_core::state::Key::new("link", "relates-2-a2-a1").unwrap();
    let fields = [
        (&item, "state", json!("in-progress")),
        (&link, "from", json!("a2")),
        (&link, "to", json!("a1")),
        (&link, "kind", json!("relates")),
        (&link, "present", json!(true)),
    ];
    tracker
        .batch(&meta, NOW.parse().unwrap(), false, &|_, _| None, |replica, at| {
            for (key, name, value) in &fields {
                replica.write(key, name, "actor/user", at, Some(value.clone()))?;
            }
            Ok::<(), acyclic_core::state::Error>(())
        })
        .unwrap();
    assert_eq!(sh.run("thread ready", AGENT), 6);
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
        let (code, out) = sh.output(&format!("import plan.json{extra}"), &[]);
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
    let counted: u64 = ["links", "threads", "repositories"]
        .iter()
        .map(|k| dry["counts"][k].as_u64().unwrap())
        .sum();
    assert_eq!(u64::try_from(creates).unwrap(), 50 + counted);
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
    assert_eq!(sh.run("import missing.json", &[]), 3);
}
