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
        let (code, output) = execute(args, &self.env(extra));
        let output = output.replace(self.dir.path().to_str().unwrap(), "[dir]");
        let shown: Vec<&str> =
            extra.iter().filter(|(n, _)| *n == "ACYCLIC_ACTOR").map(|(_, v)| *v).collect();
        let prefix =
            shown.first().map(|actor| format!("ACYCLIC_ACTOR={actor} ")).unwrap_or_default();
        writeln!(self.transcript, "$ {prefix}acyclic {line}\n# exit {code}\n{output}").unwrap();
        (code, serde_json::from_str(&output).unwrap_or_default())
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
    assert_eq!(sh.run("link add a2 blocks a1", AGENT), 0);
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("claim a2", AGENT), 1, "a2 waits on a1");
    assert_eq!(sh.run("claim a1", AGENT), 0);
    assert_eq!(sh.run("done a1 --note shipped", AGENT), 0);
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("thread set t1 --title Work", AGENT), 0);
    assert_eq!(sh.run("thread add a2 t1", AGENT), 0);
    assert_eq!(sh.run("thread approve t1", AGENT), 1, "only the user approves");
    assert_eq!(sh.run("thread approve t1 --as-user", AGENT), 0);
    let (code, proposed) = sh.output("thread add a1 t1", AGENT);
    assert_eq!((code, proposed["data"]["outcome"].as_str()), (0, Some("proposed")));
    assert_eq!(sh.run("needs", AGENT), 0);
    let proposal = proposed["data"]["proposal"].as_str().unwrap();
    assert_eq!(sh.run(&format!("proposal keep {proposal} --as-user"), AGENT), 0);
    assert_eq!(sh.run("list", AGENT), 0);
    let (code, shown) = sh.output("show a1 a2", AGENT);
    assert_eq!(
        (code, shown["data"][1]["item"]["id"].as_str()),
        (0, Some("a2")),
        "in the order given"
    );
    assert_eq!(sh.run("show a1 nope", AGENT), 3, "one missing item fails the whole command");
    let (code, out) = sh.output("thread remove a2 --as-user", AGENT);
    assert_eq!((code, out["data"]["edit"]["thread"].is_null()), (0, true));
    assert_eq!(sh.run("repo set r1 --name code", AGENT), 0);
    for (verb, present) in [("attach", true), ("detach", false)] {
        let (code, out) = sh.output(&format!("repo {verb} r1 --item a2"), AGENT);
        let edit = (out["data"]["outcome"].as_str(), out["data"]["edit"]["present"].as_bool());
        assert_eq!((code, edit), (0, (Some("applied"), Some(present))), "{verb}");
    }
    let (code, out) = sh.output("thread revoke t1 --as-user", AGENT);
    assert_eq!((code, out["data"]["approved"].as_bool()), (0, Some(false)));
    assert_eq!(sh.run("link add a1 sideways a2", AGENT), 2);
    let (code, unlinked) = sh.output("link remove a2 blocks a1", AGENT);
    assert_eq!((code, unlinked["data"]["changed"].as_bool()), (0, Some(true)), "a clear changes");
    assert_eq!(sh.run("ready", &[("ACYCLIC_ACTOR", "actor/user")]), 2);
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
    assert_eq!(sh.output("doctor", AGENT).1["data"]["items"], 0);
    assert_eq!(import(&mut sh, "")["changed"], true);
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
