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
    assert_eq!(sh.run("add a1 --title First", AGENT), 0);
    assert_eq!(sh.run("add a2 --title Second", AGENT), 0);
    assert_eq!(sh.run("add a2 --title Second", AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run("add a2 --title Other", AGENT), 1);
    assert_eq!(sh.run("link a2 blocks a1", AGENT), 0);
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("claim a2", AGENT), 1, "a2 waits on a1");
    assert_eq!(sh.run("claim a1", AGENT), 0);
    assert_eq!(sh.run("done a1 --note shipped", AGENT), 0);
    assert_eq!(sh.run("ready", AGENT), 0);
    assert_eq!(sh.run("thread t1 --title Work", AGENT), 0);
    assert_eq!(sh.run("member a2 t1", AGENT), 0);
    assert_eq!(sh.run("approve t1", AGENT), 1, "only the user approves");
    assert_eq!(sh.run("approve t1 --as-user", AGENT), 0);
    let (code, proposed) = sh.output("member a1 t1", AGENT);
    assert_eq!((code, proposed["data"]["outcome"].as_str()), (0, Some("proposed")));
    assert_eq!(sh.run("needs", AGENT), 0);
    let proposal = proposed["data"]["proposal"].as_str().unwrap();
    assert_eq!(sh.run(&format!("keep {proposal} --as-user"), AGENT), 0);
    assert_eq!(sh.run("show a1", AGENT), 0);
    assert_eq!(sh.run("show nope", AGENT), 3);
    assert_eq!(sh.run("link a1 sideways a2", AGENT), 2);
    assert_eq!(sh.run("ready", &[("ACYCLIC_ACTOR", "actor/user")]), 2);
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn an_unreadable_state_points_to_doctor() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
    sh.run("add a1 --title First", AGENT);
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
