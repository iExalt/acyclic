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

    /// Sets up the tracker with the one initiative every test's items go under, since an item needs a parent and
    /// `item add` puts one in the tracker's only initiative. The initiative is made off the transcript.
    fn init(&mut self) -> u8 {
        let code = self.run("init tracker --replica mac", &[]);
        let line = format!("initiative add --id {} --title Default", id(DEFAULT_INITIATIVE));
        let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
        // Two hours before NOW, so a test may still write at an earlier time than NOW's, as one of them does.
        let earlier = [("ACYCLIC_ACTOR", "session/mac"), ("ACYCLIC_NOW", "1792015200000")];
        let made = execute(args, &self.env(&earlier));
        assert_eq!(made.code, 0, "{made:?}");
        code
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

const AGENT: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/mac")];

/// The default initiative made by [`Shell::init`].
const DEFAULT_INITIATIVE: &str = "ff";

/// A fixed `UUIDv7` ending in `tail`, a hex string, so a transcript is the same each run.
fn id(tail: &str) -> String {
    format!("01900000-0000-7000-8000-{tail:0>12}")
}

#[test]
#[expect(clippy::too_many_lines, reason = "one transcript, command by command")]
fn a_journey_through_the_contract() {
    let mut sh = Shell::new();
    let [a1, a2, a3, a4] = ["a1", "a2", "a3", "a4"].map(id);
    let [t0, t1, t2, t3] = ["f0", "f1", "f2", "f3"].map(id);
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    // An item needs a parent, and the tracker has no initiative to default to yet.
    let i0 = id("b0");
    let (code, out) = sh.output("item add --title Nowhere", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("parent-required")), "{out}");
    assert_eq!(sh.run(&format!("initiative add --id {i0} --title Garden"), AGENT), 0);
    assert_eq!(sh.run(&format!("initiative add --id {i0} --title Garden"), AGENT), 0, "a repeat");
    assert_eq!(
        sh.run(&format!("initiative add --id {i0} --title Other"), AGENT),
        1,
        "another title"
    );
    assert_eq!(
        sh.run(&format!("item add --id {i0} --title Clash --parent Garden"), AGENT),
        1,
        "its id"
    );
    // With one initiative, an add that names no parent goes under it.
    assert_eq!(sh.run(&format!("item add --id {a1} --title First"), AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {a2} --title Second"), AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {a2} --title Second"), AGENT), 0, "a repeat");
    assert_eq!(sh.run(&format!("item add --id {a2} --title Other"), AGENT), 1);
    // A record is named by its title, or by the end of its id.
    assert_eq!(sh.run("item edit Second --waits-for First", AGENT), 0);
    let (_, ready) = sh.output("thread ready", AGENT);
    // An item in no thread is its own thread of one: First is ready, and Second waits on it.
    assert_eq!(ready["data"]["ready"][0]["task"], json!(a1), "{ready}");
    assert_eq!(ready["data"]["ready"].as_array().unwrap().len(), 1, "{ready}");
    assert_eq!(sh.run(&format!("thread add --id {t0} --title Start"), AGENT), 0);
    for ending in ["0000a1", "0000a2"] {
        assert_eq!(sh.run(&format!("item edit {ending} --thread Start"), AGENT), 0);
    }
    assert_eq!(sh.run("thread ready", AGENT), 0);
    let (code, out) = sh.output("thread claim First", AGENT);
    assert_eq!(
        (code, &out["error"]["details"]["thread"]),
        (3, &json!(t0)),
        "threads are claimed, not items"
    );
    assert_eq!(sh.run("thread claim Start", AGENT), 0);
    assert_eq!(sh.run("item done Second", AGENT), 1, "Second waits on First");
    assert_eq!(sh.run("item edit Second --state done", AGENT), 2, "item done finishes items");
    let pi = [("ACYCLIC_ACTOR", "session/pi")];
    assert_eq!(sh.run("thread claim Start", &pi), 1, "another agent's claim");
    assert_eq!(sh.run("item done First", &pi), 1, "and its items");
    let (code, done) = sh.output("item done First -e shipped", AGENT);
    assert_eq!(
        (code, &done["data"]["evidence"]),
        (0, &json!({"commit": null, "text": "shipped"})),
        "done echoes what it recorded: {done}"
    );
    assert_eq!(
        sh.run("item done First --evidence shipped", AGENT),
        0,
        "finishing again changes nothing"
    );
    let (code, shown) = sh.output("thread show Start", AGENT);
    assert_eq!(
        (code, &shown["data"][0]["claims"][0]["holder"]),
        (0, &json!("session/mac")),
        "done releases nothing"
    );
    for (released, changed) in [(true, true), (false, false)] {
        let (code, out) = sh.output("thread release Start", AGENT);
        assert_eq!(
            (code, out["data"]["released"].as_bool(), out["data"]["changed"].as_bool()),
            (0, Some(released), Some(changed)),
            "releasing nothing isn't an error"
        );
    }
    assert_eq!(sh.run("thread ready", AGENT), 0);
    assert_eq!(sh.run(&format!("thread add --id {t1} --title Work"), AGENT), 0);
    assert_eq!(sh.run(&format!("thread add --id {t1} --title Work"), AGENT), 0, "a repeat");
    assert_eq!(sh.run(&format!("thread add --id {t1} --title Other"), AGENT), 1, "a conflict");
    assert_eq!(sh.run("thread add --title Work", AGENT), 1, "a second thread with its title");
    assert_eq!(sh.run("item edit Second --thread nope", AGENT), 3, "a thread that isn't there");
    // An edit refused in part writes nothing: the title is kept when the parent it came with is missing.
    assert_eq!(sh.run("item edit Second --title Renamed --parent nope", AGENT), 3);
    assert_eq!(
        sh.output("item show Second", AGENT).1["data"][0]["title"],
        "Second",
        "refused as a whole"
    );
    assert_eq!(sh.run("item edit Second --thread Work", AGENT), 0);
    let (_, ready) = sh.output("thread ready", AGENT);
    assert_eq!(ready["data"]["ready"][0]["id"], json!(t1), "{ready}");
    assert_eq!(sh.run("item edit First --thread Work", AGENT), 0);
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run("item list", AGENT), 0);
    assert_eq!(sh.run("thread list", AGENT), 0);
    // JSON stays the default, byte for byte, and `-o pretty` prints the same data for people, an error on stderr.
    for line in ["item list", "thread ready", "thread list", "item show First", "item show nope"] {
        let stdout = |line: String| {
            let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
            execute(args, &sh.env(AGENT)).stdout
        };
        assert_eq!(stdout(format!("{line} -o json")), stdout(line.to_owned()), "{line}");
    }
    let add_again = format!("item add --id {a1} --title First");
    for line in ["item list", "thread ready", "thread list", "item show First", add_again.as_str()]
    {
        assert_eq!(sh.run(&format!("{line} -o pretty"), AGENT), 0, "{line}");
    }
    assert_eq!(sh.run("item show nope -o pretty", AGENT), 3);
    assert_eq!(sh.run("thread show Work", AGENT), 0);
    assert_eq!(
        sh.run("thread show Work nope", AGENT),
        3,
        "one missing thread fails the whole command"
    );
    let (code, shown) = sh.output("item show First Second", AGENT);
    assert_eq!((code, &shown["data"][1]["id"]), (0, &json!(a2)), "in the order given");
    assert_eq!(
        sh.run("item show First nope", AGENT),
        3,
        "one missing item fails the whole command"
    );
    let (code, out) = sh.output("item edit Second --no-thread", AGENT);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)));
    assert_eq!(sh.run("repository add --name me/code", AGENT), 2, "a new record needs every field");
    let r1 = id("e1");
    let full = format!("repository add --id {r1} --name me/code --default-branch main");
    assert_eq!(sh.run(&full, AGENT), 0);
    assert_eq!(sh.run(&full, AGENT), 0, "a repeat changes nothing");
    assert_eq!(sh.run(&full.replace("main", "trunk"), AGENT), 1);
    assert_eq!(
        sh.run("repository add --name ME/Code --default-branch main", AGENT),
        1,
        "another record with its owner/name"
    );
    assert_eq!(
        sh.run("repository edit me/code --default-branch trunk", AGENT),
        0,
        "an edit takes any subset"
    );
    assert_eq!(sh.run("repository edit nope --default-branch main", AGENT), 3);
    let (code, bare) = sh.output("repository show code", AGENT);
    assert_eq!((code, &bare["error"]["details"]["candidates"][0]["name"]), (3, &json!("me/code")));
    assert_eq!(sh.run("repository show me/code", AGENT), 0);
    assert_eq!(sh.run("repository list", AGENT), 0);
    // A thread is deleted once no item is in it and no one holds it.
    assert_eq!(sh.run(&format!("thread add --id {t3} --title Spare"), AGENT), 0);
    let (code, out) = sh.output("thread delete Work", AGENT);
    assert_eq!((code, &out["error"]["details"]["reasons"][0]["why"]), (1, &json!("has-members")));
    assert_eq!(sh.run("thread delete Spare", AGENT), 0);
    let (code, out) = sh.output(&format!("thread delete {t3}"), AGENT);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(false)), "a repeat");
    assert_eq!(sh.run("thread show Spare", AGENT), 3);
    let (code, out) = sh.output("item edit First --title Renamed --waits-for Second", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("waits-for-cycle")), "{out}");
    assert_eq!(
        sh.output("item show First", AGENT).1["data"][0]["title"],
        "First",
        "refused as a whole"
    );
    let (code, unlinked) = sh.output("item edit Second --no-waits-for 0000a1", AGENT);
    assert_eq!((code, unlinked["data"]["changed"].as_bool()), (0, Some(true)), "a clear changes");
    let links = &sh.output("item show Second", AGENT).1["data"][0]["links"];
    assert_eq!(links, &json!({"waits_for": [], "waited_on_by": []}));
    // Second left Work; an agent groups it into a new thread.
    assert_eq!(sh.run(&format!("thread add --id {t2} --title Follow-ups"), AGENT), 0);
    assert_eq!(sh.run("item edit Second --thread Follow-ups", AGENT), 0);
    // A thread where an item belongs says which command was meant.
    let (code, out) = sh.output("item edit Second --parent Follow-ups", AGENT);
    let meant = out["error"]["details"]["meant"].as_str();
    assert_eq!((code, meant), (3, Some("acyclic item edit Second --thread Follow-ups")));
    assert_eq!(sh.run("needs", AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {a3} --title Second-follow-up"), AGENT), 0);
    assert_eq!(
        sh.run(&format!("item add --id {a3} --title Second-follow-up --parent Second"), AGENT),
        1,
        "another parent"
    );
    let (_, under) =
        sh.output(&format!("item add --id {a4} --title Third-follow-up --parent Second"), AGENT);
    assert_eq!(under["data"]["placed"][0]["thread"], json!(t2), "a parent passes its thread down");
    // A parent reads done once its children are, and is never finished by hand, even forced.
    let [g1, g1a, g2, g2a] = ["c1", "c2", "c3", "c4"].map(id);
    sh.run(&format!("item add --id {g1} --title Goal"), AGENT);
    sh.run(&format!("item add --id {g1a} --title Part --parent Goal"), AGENT);
    let (code, out) = sh.output("item done Goal", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("parent")));
    sh.run("item edit Part --thread Follow-ups", AGENT);
    sh.run("thread claim Follow-ups", AGENT);
    sh.run("item done Part", AGENT);
    assert_eq!(sh.output("item show Goal", AGENT).1["data"][0]["stage"], "done");
    // A done item's title is free again, though the title then names both.
    sh.run(&format!("item add --id {g2} --title Goal"), AGENT);
    assert_eq!(sh.output("item show Goal", AGENT).1["error"]["code"], "ambiguous");
    sh.run(&format!("item add --id {g2a} --title Part --parent 0000c3"), AGENT);
    assert_eq!(sh.run("item done 0000c3 --force", AGENT), 1, "forcing doesn't finish a parent");
    // Only an item nothing refers to is deleted, and it is gone for good.
    let [d1, d1a] = ["d1", "d2"].map(id);
    assert_eq!(sh.run(&format!("item add --id {d1} --title Doomed"), AGENT), 0);
    assert_eq!(
        sh.run(&format!("item add --id {d1a} --title Doomed-part --parent Doomed"), AGENT),
        0
    );
    let (code, out) = sh.output("item delete Doomed", AGENT);
    assert_eq!((code, &out["error"]["details"]["reasons"][0]["why"]), (1, &json!("has-children")));
    assert_eq!(sh.run("item delete First", AGENT), 1, "item done work stays");
    assert_eq!(sh.run("item delete Doomed-part", AGENT), 0);
    assert_eq!(sh.run("item delete Doomed", AGENT), 0);
    let (code, out) = sh.output(&format!("item delete {d1}"), AGENT);
    assert_eq!(
        (code, out["data"]["changed"].as_bool()),
        (0, Some(false)),
        "a repeat changes nothing"
    );
    assert_eq!(sh.run("item show Doomed", AGENT), 3);
    assert_eq!(
        sh.run(&format!("item add --id {d1} --title Doomed"), AGENT),
        1,
        "its id is retired"
    );
    assert_eq!(sh.run("item delete nope", AGENT), 3);
    // A second initiative: an add naming no parent is refused, naming the choices, and `--parent` takes either kind.
    let (orchard, pear, grove) = (id("b1"), id("b2"), id("b3"));
    let line = format!("initiative add --id {orchard} --title Orchard --notes Trees");
    assert_eq!(sh.run(&line, AGENT), 0);
    let (code, out) = sh.output("item add --title Floating", AGENT);
    let choices = out["error"]["details"]["initiatives"].as_array().map(Vec::len);
    assert_eq!(
        (code, out["error"]["code"].as_str(), choices),
        (1, Some("parent-required"), Some(2))
    );
    let line = format!("item add --id {pear} --title Pear --parent Orchard");
    assert_eq!(sh.run(&line, AGENT), 0);
    let line = format!("item add --id {grove} --title Orchard --parent Garden");
    assert_eq!(sh.run(&line, AGENT), 0);
    let (code, out) = sh.output("item add --title Plum --parent Orchard", AGENT);
    assert_eq!(
        (code, out["error"]["code"].as_str()),
        (2, Some("ambiguous")),
        "an item's and an initiative's title"
    );
    assert_eq!(out["error"]["details"]["candidates"].as_array().map(Vec::len), Some(2));
    // Reads: the item says which initiative it is under, and `thread ready --initiative` keeps its threads.
    assert_eq!(sh.output("item show Pear", AGENT).1["data"][0]["initiative"], json!(orchard));
    let (code, scoped) = sh.output("thread ready --initiative Orchard", AGENT);
    let tasks: Vec<&serde_json::Value> =
        scoped["data"]["ready"].as_array().unwrap().iter().map(|row| &row["task"]).collect();
    assert_eq!((code, tasks), (0, vec![&json!(pear)]), "{scoped}");
    let (_, wide) = sh.output("thread ready --initiative Garden", AGENT);
    let kept =
        wide["data"]["ready"].as_array().unwrap().iter().any(|row| row["task"] == json!(pear));
    assert!(!kept, "{wide}");
    assert_eq!(sh.run("thread ready --initiative nope", AGENT), 3);
    assert_eq!(sh.run("initiative edit Orchard --no-notes", AGENT), 0);
    assert_eq!(sh.run("initiative edit Orchard --notes x --no-notes", AGENT), 2);
    assert_eq!(sh.run("initiative list", AGENT), 0);
    assert_eq!(sh.run("initiative list -o pretty", AGENT), 0);
    assert_eq!(sh.run("initiative show Garden Orchard", AGENT), 0);
    assert_eq!(sh.run("initiative show nope", AGENT), 3);
    // No flag clears an item's parent; it moves to another, and an initiative is deleted once it holds no item.
    assert_eq!(sh.run("item edit Pear --no-parent", AGENT), 2);
    let (code, out) = sh.output("initiative delete Orchard", AGENT);
    assert_eq!((code, &out["error"]["details"]["reasons"][0]["why"]), (1, &json!("holds-items")));
    assert_eq!(sh.run(&format!("item edit Pear --parent {i0}"), AGENT), 0);
    assert_eq!(sh.run("initiative delete Orchard", AGENT), 0);
    let (code, out) = sh.output(&format!("initiative delete {orchard}"), AGENT);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(false)), "a repeat");
    assert_eq!(sh.run("initiative show Orchard", AGENT), 3);
    assert_eq!(sh.run("initiative delete nope", AGENT), 3);
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn an_add_takes_only_a_uuidv7_in_any_case_and_references_match_as_the_rules_say() {
    let mut sh = Shell::new();
    sh.init();
    let sow = id("5eed01");
    let add = format!("item add --id {sow} --title Sow");
    let (code, out) = sh.output(&add, AGENT);
    assert_eq!(
        (code, &out["data"]["item"], &out["data"]["changed"]),
        (0, &json!(sow), &json!(true))
    );
    let (code, out) = sh.output(&add, AGENT);
    assert_eq!((code, &out["data"]["changed"]), (0, &json!(false)), "the same add changes nothing");
    let (code, out) = sh.output(&add.replace(&sow, &sow.to_uppercase()), AGENT);
    assert_eq!(
        (code, &out["data"]["item"], &out["data"]["changed"]),
        (0, &json!(sow), &json!(false))
    );
    let (code, out) = sh.output("item add --id plant --title Plant", AGENT);
    assert_eq!((code, &out["error"]["code"]), (2, &json!("invalid-id")));
    let (code, out) = sh.output("item show 5EED01", AGENT);
    assert_eq!((code, &out["data"][0]["id"]), (0, &json!(sow)), "an ending in upper case");
    assert_eq!(sh.run("item show sow", AGENT), 3, "a title in the wrong case");
    // An add with no id makes one, and a retry is refused naming the first.
    let (_, made) = sh.output("item add --title Weed", AGENT);
    let weed = made["data"]["item"].as_str().unwrap().to_owned();
    assert!(acyclic_core::ids::v7(&weed).is_some(), "{made}");
    let (code, retry) = sh.output("item add --title Weed", AGENT);
    assert_eq!((code, &retry["error"]["details"]["id"]), (1, &json!(weed)));
}

#[test]
fn done_names_its_commit_by_repository_and_sha() {
    let mut sh = Shell::new();
    sh.init();
    sh.run(
        &format!("repository add --id {} --name iExalt/acyclic --default-branch main", id("e1")),
        AGENT,
    );
    sh.run(&format!("item add --id {} --title Ship", id("a1")), AGENT);
    let (code, out) = sh.output("item done Ship --force --commit abc1234", AGENT);
    assert_eq!((code, &out["error"]["code"]), (2, &json!("invalid-commit")));
    let message = out["error"]["message"].as_str().unwrap();
    assert!(message.contains("<OWNER/NAME>@abc1234"), "{message}");
    let (code, out) = sh.output("item done Ship --force --commit iExalt/acyclic@abc1234", AGENT);
    assert_eq!((code, &out["data"]["evidence"]["commit"]), (0, &json!("iExalt/acyclic@abc1234")));
    // The store keeps the repository's id apart from the SHA, so a rename renames the evidence.
    sh.run("repository edit iexalt/acyclic --name iExalt/tracker", AGENT);
    let (_, shown) = sh.output("item show Ship", AGENT);
    assert_eq!(shown["data"][0]["evidence"]["commit"], "iExalt/tracker@abc1234");
}

#[test]
fn a_state_this_build_cant_read_hints_at_the_fix_and_doctor_never_at_itself() {
    let mut sh = Shell::new();
    sh.init();
    sh.run(&format!("item add --id {} --title First", id("a1")), AGENT);
    // The replica's state as an earlier build wrote it: the same files under the previous format version.
    let tracker = sh.path().join("tracker");
    let git = |args: &[&str]| {
        // An identity of its own, since a machine may have none to commit as.
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&tracker)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "acyclic test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "acyclic test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    let header = git(&["show", "refs/heads/state/mac:state.json"]);
    let current = format!("\"format_version\": {}", acyclic_core::state::FORMAT_VERSION);
    assert!(header.contains(&current), "{header}");
    let earlier = format!("\"format_version\": {}", acyclic_core::state::FORMAT_VERSION - 1);
    let write_header = |text: String| {
        std::fs::write(tracker.join("header"), text + "\n").unwrap();
        git(&["read-tree", "refs/heads/state/mac"]);
        let blob = git(&["hash-object", "-w", "header"]);
        git(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},state.json")]);
        let tree = git(&["write-tree"]);
        let commit = git(&["commit-tree", &tree, "-p", "refs/heads/state/mac", "-m", "header"]);
        git(&["update-ref", "refs/heads/state/mac", &commit]);
    };
    write_header(header.replace(&current, &earlier));
    let mut refused = |line: &str| {
        let (code, out) = sh.output(line, AGENT);
        assert_eq!((code, out["error"]["code"].as_str()), (6, Some("unreadable")), "{line}: {out}");
        out["error"].clone()
    };
    // Every command, doctor too, says what reads it.
    for line in ["item list", "doctor"] {
        let error = refused(line);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("is read by that version, or rebuilt with `acyclic import`"),
            "{line}: {error}"
        );
        assert_eq!(
            error["details"]["hint"],
            "written by an earlier acyclic: read it with that version, or rebuild it with `acyclic import` from a \
             version-4 document"
        );
    }
    // Any other unreadable state points to doctor, except from doctor itself.
    write_header("not a header".to_owned());
    assert_eq!(refused("item list")["details"]["hint"], "run acyclic doctor");
    assert_eq!(refused("doctor")["details"], json!(null));
}

#[test]
fn doctor_names_the_first_noncanonical_entity_file() {
    let mut sh = Shell::new();
    let items = [id("a1"), id("a2")];
    sh.init();
    for (index, item) in items.iter().enumerate() {
        sh.run(&format!("item add --id {item} --title Item-{index}"), AGENT);
    }
    let tracker = sh.path().join("tracker");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&tracker)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "acyclic test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "acyclic test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };

    let mut entities: Vec<String> = items
        .iter()
        .map(|item| acyclic_core::state::Key::new("item", item).unwrap().path())
        .collect();
    entities.sort();
    git(&["read-tree", "refs/heads/state/mac"]);
    for (index, entity) in entities.iter().enumerate() {
        let original = git(&["show", &format!("refs/heads/state/mac:{entity}")]);
        let name = format!("noncanonical-{index}");
        std::fs::write(tracker.join(&name), format!("{original} \n")).unwrap();
        let blob = git(&["hash-object", "-w", &name]);
        let cacheinfo = format!("100644,{blob},{entity}");
        git(&["update-index", "--add", "--cacheinfo", &cacheinfo]);
    }
    let tree = git(&["write-tree"]);
    let commit = git(&["commit-tree", &tree, "-p", "refs/heads/state/mac", "-m", "test"]);
    git(&["update-ref", "refs/heads/state/mac", &commit]);

    let (code, out) = sh.output("doctor", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (6, Some("unreadable")), "{out}");
    assert_eq!(
        out["error"]["message"],
        format!("unreadable state: {} is not in canonical form", entities[0]),
        "{out}"
    );
    assert_eq!(out["error"]["details"], json!(null), "doctor must not tell itself to run doctor");
}

#[test]
fn an_unreadable_state_points_to_doctor() {
    let mut sh = Shell::new();
    sh.init();
    let (a1, a2) = (id("a1"), id("a2"));
    sh.run(&format!("item add --id {a1} --title First"), AGENT);
    sh.run(&format!("item add --id {a2} --title Second"), AGENT);
    // What this build no longer reads: the stored in-progress state and a relates link older versions wrote.
    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into() };
    let item = acyclic_core::model::item_key(&a1).unwrap();
    let link = acyclic_core::state::Key::new("link", &format!("relates-36-{a2}-{a1}")).unwrap();
    let fields = [
        (&item, "state", json!("in-progress")),
        (&link, "from", json!(a2)),
        (&link, "to", json!(a1)),
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

/// The wiring of a new child's thread: the flags, the refusal and its code, and what a write reports.
#[test]
fn a_new_child_joins_its_siblings_thread_and_the_flags_say_otherwise() {
    let mut sh = Shell::new();
    let [epic, t1, t2] = [id("e1"), id("f1"), id("f2")];
    sh.init();
    for line in [
        format!("item add --id {epic} --title Epic"),
        format!("thread add --id {t1} --title One"),
        format!("thread add --id {t2} --title Two"),
        format!("item add --title First --parent {epic} --thread {t1}"),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    // Left out, the thread follows the siblings', and the write says so.
    let (code, out) = sh.output(&format!("item add --title Second --parent {epic}"), AGENT);
    assert_eq!((code, &out["data"]["placed"][0]["thread"]), (0, &json!(t1)), "{out}");
    // Siblings that differ, or sit in none, refuse it; each way to say where it goes is accepted.
    assert_eq!(sh.run(&format!("item add --title Third --parent {epic} --thread {t2}"), AGENT), 0);
    let (code, out) = sh.output(&format!("item add --title Fourth --parent {epic}"), AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("thread-unclear")), "{out}");
    assert_eq!(out["error"]["details"]["parent"], json!(epic));
    assert!(out["error"]["message"].as_str().unwrap().contains("--no-thread"), "{out}");
    assert_eq!(sh.run(&format!("item add --title Fourth --parent {epic} --no-thread"), AGENT), 0);
    assert_eq!(sh.run("item add --title Both --thread x --no-thread", AGENT), 2, "contradiction");
    assert_eq!(sh.run("item add --title Both --claim --no-thread", AGENT), 2, "contradiction");
    // `--claim` claims the item's thread in the add's own write, and an item in none claims its thread of one.
    let (code, out) = sh.output(&format!("item add --title Mine --thread {t2} --claim"), AGENT);
    assert_eq!(
        (code, &out["data"]["claim"]["thread"], &out["data"]["claim"]["claim"]["holder"]),
        (0, &json!(t2), &json!("session/mac")),
        "{out}"
    );
    let (code, out) = sh.output("item add --title Loose --claim", AGENT);
    let loose = out["data"]["item"].as_str().unwrap_or_default().to_owned();
    assert_eq!(
        (code, &out["data"]["claim"]["thread"], &out["data"]["claim"]["claim"]["holder"]),
        (0, &json!(format!("{loose}.one")), &json!("session/mac")),
        "{out}"
    );
    let (_, shown) = sh.output(&format!("item show {loose}"), AGENT);
    assert_eq!(
        shown["data"][0]["thread"],
        json!(format!("{loose}.one")),
        "named on the task: {shown}"
    );
    assert_eq!(shown["data"][0]["claims"][0]["holder"], json!("session/mac"), "{shown}");
    // The first child of a threaded item takes its thread, and the emptied parent shows nothing of it.
    assert_eq!(sh.run(&format!("item add --id {} --title Leaf --thread {t1}", id("a9")), AGENT), 0);
    let (_, out) = sh.output(&format!("item add --title Kid --parent {}", id("a9")), AGENT);
    assert_eq!(out["data"]["placed"][0]["thread"], json!(t1), "{out}");
    let (_, shown) = sh.output(&format!("item show {}", id("a9")), AGENT);
    assert_eq!(shown["data"][0]["thread"], json!(null), "{shown}");
}

/// The wiring of a loose task as a thread of one: how it is named, what the rows say, and the claim's one write.
#[test]
fn a_loose_task_is_claimed_as_its_own_thread_by_title_or_id_and_its_thread_is_never_deleted() {
    let mut sh = Shell::new();
    let [x, y] = [id("a1"), id("a2")];
    let [one_x, one_y] = [format!("{x}.one"), format!("{y}.one")];
    sh.init();
    for line in [
        format!("item add --id {x} --title Alpha"),
        format!("item add --id {y} --title Beta"),
        format!("item add --id {} --title Twin", id("a3")),
        format!("item add --id {} --title Twin", id("a4")),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    // Each loose task is a ready row of its own, shown as the task, with its id derived from the task's.
    let (_, listed) = sh.output("thread list", AGENT);
    let rows = listed["data"]["threads"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "one row per loose task: {listed}");
    assert_eq!(
        (
            &rows[0]["id"],
            &rows[0]["task"],
            &rows[0]["title"],
            &rows[0]["progress"]["counts"]["ready"]
        ),
        (&json!(one_x), &json!(x), &json!("Alpha"), &json!(1))
    );
    let (_, ready) = sh.output("thread ready", AGENT);
    assert_eq!(ready["data"]["ready"].as_array().unwrap().len(), 4, "{ready}");
    // An ambiguous title is refused, naming both tasks; the full id of either names it.
    let (code, out) = sh.output("thread claim Twin", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (2, Some("ambiguous")), "{out}");
    assert_eq!(out["error"]["details"]["candidates"].as_array().unwrap().len(), 2);

    // By title: the thread is made and claimed, and the task names it, in one commit.
    let commits = |sh: &Shell| {
        let out = std::process::Command::new("git")
            .args([
                "-C",
                sh.path().join("tracker").to_str().unwrap(),
                "rev-list",
                "--all",
                "--count",
            ])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().parse::<u32>().unwrap()
    };
    // One write, as an add is: the same count of commits the tracker keeps for it.
    let before = commits(&sh);
    sh.run(&format!("item add --id {} --title Gamma", id("a5")), AGENT);
    let ordinary = commits(&sh) - before;
    let before = commits(&sh);
    let (code, claimed) = sh.output("thread claim Alpha", AGENT);
    assert_eq!(
        (code, &claimed["data"]["thread"], &claimed["data"]["claim"]["holder"]),
        (0, &json!(one_x), &json!("session/mac")),
        "{claimed}"
    );
    assert_eq!(
        commits(&sh) - before,
        ordinary,
        "the thread and the task's naming it are one write, as costly as any claim"
    );
    let (_, shown) = sh.output("item show Alpha", AGENT);
    assert_eq!(shown["data"][0]["thread"], json!(one_x), "{shown}");
    assert_eq!(shown["data"][0]["stage"], json!("in-progress"), "{shown}");
    // By the task's full id, and renewing what it holds.
    assert_eq!(sh.run(&format!("thread claim {y}"), AGENT), 0);
    let pi = [("ACYCLIC_ACTOR", "session/pi")];
    assert_eq!(sh.run("thread claim Alpha", &pi), 1, "another agent's claim");
    // Graduated or not, the thread is the task's own and is never deleted.
    let (code, out) = sh.output(&format!("thread delete {one_x}"), AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("in-use")), "{out}");
    assert!(out["error"]["message"].as_str().unwrap().contains("task's own thread"), "{out}");
    // Once claimed, the row is the same row, and the held thread is not ready to anyone else.
    let (_, ready) = sh.output("thread ready", &pi);
    assert_eq!(
        ready["data"]["ready"].as_array().unwrap().len(),
        3,
        "only the Twins and Gamma: {ready}"
    );
    // The lifecycle: release, finish, and delete.
    assert_eq!(sh.run("thread release Alpha", AGENT), 0);
    assert_eq!(sh.run("item done Alpha", AGENT), 0, "a loose task needs no claim to finish");
    assert_eq!(sh.run("item delete Beta", AGENT), 0, "its own claim goes with the task");
    let (_, listed) = sh.output("thread list", AGENT);
    let ids: Vec<&str> = listed["data"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&one_y.as_str()), "emptied and unheld: hidden: {ids:?}");
    let (_, needs) = sh.output("needs", AGENT);
    assert_eq!(needs["data"]["needs"], json!([]), "nothing asks for a thread: {needs}");
}

#[test]
fn finishing_with_an_unreadable_claim_exits_six() {
    let mut sh = Shell::new();
    let (thread, item) = (id("f1"), id("a1"));
    sh.init();
    sh.run(&format!("thread add --id {thread} --title Work"), AGENT);
    sh.run(&format!("item add --id {item} --title First --thread {thread}"), AGENT);

    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into() };
    let claim = acyclic_core::state::Key::new("thread", &thread).unwrap();
    tracker
        .batch(&meta, NOW.parse().unwrap(), false, &|_, _| None, |replica, at| {
            replica.write(&claim, "claim", "actor/user", at, Some(json!("not a claim")))?;
            Ok::<(), acyclic_core::state::Error>(())
        })
        .unwrap();

    let (code, out) = sh.output(&format!("item done {item}"), AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (6, Some("unreadable")), "{out}");
    assert_eq!(out["error"]["details"]["hint"], "run acyclic doctor");
}

#[test]
fn an_import_is_an_upsert_and_a_dry_run_writes_nothing() {
    let mut sh = Shell::new();
    sh.init();
    let mut plan = acyclic_fixtures::generate(50, 7);
    // Every new item needs a parent: what the plan leaves at the top goes under the tracker's initiative.
    for item in plan.items.iter_mut().filter(|item| item.parent.is_none()) {
        item.parent = Some(id(DEFAULT_INITIATIVE));
    }
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
    // Nothing in a first import has siblings in the tracker to follow, and no thread above it is passed down.
    let inherited = dry["placed"].clone();
    assert_eq!(inherited, json!([]), "{dry}");
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
    let show = format!("item show {}", plan.items[0].id);
    let original = sh.output(&show, AGENT).1["data"][0].clone();
    plan.items[0].title.push_str(" renamed");
    plan.items[0].notes = Some("Checked the update".into());
    std::fs::write(&file, plan.to_json()).unwrap();
    let pretty = |sh: &Shell, extra: &str| {
        let line = format!("acyclic import plan.json -o pretty{extra}");
        let outcome = execute(line.split(' ').map(Into::into), &sh.env(&[]));
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        outcome.stdout
    };
    assert_eq!(pretty(&sh, " --dry-run"), "would import: update 1 item (2 fields)\n");
    assert_eq!(sh.output(&show, AGENT).1["data"][0], original, "a dry run leaves the item alone");
    assert_eq!(refs(&sh), before);
    let renamed = import(&mut sh, "");
    assert_eq!(
        (renamed["changed"].as_bool(), renamed["counts"]["fields"].as_u64()),
        (Some(true), Some(2))
    );
    assert_eq!(renamed["counts"]["items"], 0, "JSON counts still count creates");
    assert_eq!(renamed["changes"].as_array().unwrap().len(), 1, "two fields update one item");
    assert_eq!(renamed["changes"][0]["fields"]["title"]["to"], plan.items[0].title);
    let stored = &sh.output(&show, AGENT).1["data"][0];
    assert_eq!(stored["title"], plan.items[0].title);
    assert_eq!(stored["notes"], "Checked the update");
    assert_eq!(pretty(&sh, ""), "no change\n");
    assert_eq!(sh.run("import missing.json", &[]), 3);
}

#[test]
fn a_parent_that_is_a_thread_suggests_a_command_that_pastes_as_it_is() {
    let mut sh = Shell::new();
    sh.init();
    assert_eq!(sh.run("thread add --title Q&A", AGENT), 0);
    assert_eq!(sh.run("item add --title It's", AGENT), 0);
    let meant = |sh: &mut Shell, line: &str| {
        let (_, out) = sh.output(line, AGENT);
        out["error"]["details"]["meant"].as_str().map(str::to_owned)
    };
    assert_eq!(
        meant(&mut sh, "item edit It's --parent Q&A").as_deref(),
        Some(r"acyclic item edit 'It'\''s' --thread 'Q&A'")
    );
    assert_eq!(
        meant(&mut sh, "item add --title New --parent Q&A").as_deref(),
        Some("acyclic item add --title <TITLE> --thread 'Q&A'")
    );
}

#[test]
fn pretty_output_names_a_record_the_command_just_added_among_the_rest() {
    let mut sh = Shell::new();
    assert_eq!(sh.init(), 0);
    // Two ids sharing their last six characters, so each is named by seven once both exist.
    let [old, new] = ["0abc12", "10abc12"].map(id);
    assert_eq!(sh.run(&format!("item add --id {old} --title Old"), AGENT), 0);
    let pretty = |sh: &Shell, line: String| {
        let args = std::iter::once("acyclic").chain(line.split(' ')).map(Into::into);
        execute(args, &sh.env(AGENT)).stdout
    };
    let added = pretty(&sh, format!("item add --id {new} --title New -o pretty"));
    assert!(added.contains("item 10abc12 "), "{added}");
    let listed = pretty(&sh, "item list -o pretty".to_owned());
    assert!(listed.contains("00abc12") && listed.contains("10abc12"), "{listed}");
}

/// An item not started reads as `open` and is written as `backlog`, which replicas of older versions read: from an
/// add, `--state open` or the older `--state backlog`, and an app edit whose expected state is the `open` it read.
#[test]
fn open_is_how_a_stored_backlog_reads_and_backlog_is_what_is_written() {
    let mut sh = Shell::new();
    sh.init();
    let ids = ["a1", "a2", "a3", "a4", "a5"].map(id);
    let [added, open, alias, edited, app] = ids.each_ref().map(String::as_str);
    for line in [
        format!("item add --id {added} --title Added"),
        format!("item add --id {open} --title Open --state open"),
        format!("item add --id {alias} --title Alias --state backlog"),
        format!("item add --id {edited} --title Edited --state parked"),
        format!("item edit {edited} --state open"),
        format!("item add --id {app} --title App"),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    // The app's edit, and its undo, each expecting the state as the app read it.
    let tracker = sh.path().join("tracker");
    let app_edit = |state: &str, expected: &str| {
        let command =
            serde_json::from_value(json!({"item": {"edit": {"item": app, "state": state}}}));
        let expect = serde_json::from_value(
            json!([{"kind": "item-field", "item": app, "field": "state", "value": expected}]),
        );
        crate::call_expected(command.unwrap(), tracker.clone(), &sh.env(&[]), expect.unwrap())
    };
    let (code, out) = app_edit("parked", "parked");
    assert_eq!((code, &out["error"]["code"]), (1, &json!("changed-since")), "{out}");
    assert_eq!(app_edit("parked", "open").0, 0);
    assert_eq!(app_edit("open", "parked").0, 0);

    let (_, listed) = sh.output("item list", AGENT);
    let states: Vec<&serde_json::Value> =
        listed["data"]["items"].as_array().unwrap().iter().map(|row| &row["state"]).collect();
    assert_eq!(states, [&json!("open"); 5], "{listed}");
    let (tracker, _) = acyclic_core::store::Tracker::open(&tracker).unwrap();
    let (_, state) = tracker.load().unwrap();
    for id in &ids {
        let key = acyclic_core::state::Key::new("item", id).unwrap();
        let stored: Vec<&serde_json::Value> =
            state.field(&key, "state").unwrap().values().map(|stamped| &stamped.value).collect();
        assert_eq!(stored, [&json!("backlog")], "{id}");
    }
}

/// The wiring of canceling, which core's tests prove: the flag parses, the item reads `canceled` in state and stage, and
/// a refusal is a `refused` error with its code and message, `--force` passing the one over a claim.
#[test]
fn canceling_parses_reads_back_and_refuses_in_the_clis_words() {
    let mut sh = Shell::new();
    sh.init();
    let [cut, done, held, work] = ["a1", "a2", "a3", "f1"].map(id);
    for line in [
        format!("thread add --id {work} --title Work"),
        format!("item add --id {cut} --title Cut --thread {work}"),
        format!("item add --id {done} --title Done --thread {work}"),
        format!("item add --id {held} --title Held --thread {work}"),
        format!("item done {done} --evidence shipped --force"),
        format!("item edit {cut} --state canceled"),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    let shown = sh.output(&format!("item show {cut}"), AGENT).1["data"][0].clone();
    assert_eq!((&shown["state"], &shown["stage"]), (&json!("canceled"), &json!("canceled")));
    let (code, out) = sh.output(&format!("item edit {done} --state canceled"), &[]);
    assert_eq!((code, &out["error"]["code"]), (1, &json!("done")), "{out}");
    assert!(out["error"]["message"].as_str().unwrap().contains("item reopen"), "{out}");
    assert_eq!(sh.run(&format!("thread claim {work}"), AGENT), 0);
    let (code, out) = sh.output(&format!("item edit {held} --state canceled"), &[]);
    assert_eq!((code, &out["error"]["code"]), (1, &json!("claimed")), "{out}");
    assert!(out["error"]["message"].as_str().unwrap().contains("--force"), "{out}");
    assert_eq!(sh.run(&format!("item edit {held} --state canceled --force"), &[]), 0);
}

/// A thread's notes read as `notes` and are written under the older field `brief`, which replicas of older versions
/// read: from `--notes`, the older `--brief` and `--no-brief`, the app's edit expecting the notes it read (or, as
/// older clients say, its brief), and an import naming them `notes` or `brief`.
#[test]
fn notes_are_how_a_stored_brief_reads_and_brief_is_what_is_written() {
    let mut sh = Shell::new();
    sh.init();
    let ids = ["f1", "f2", "f3", "f4", "f5", "f6"].map(id);
    let [added, alias, edited, app, imported, older] = ids.each_ref().map(String::as_str);
    for line in [
        format!("thread add --id {added} --title Added --notes One"),
        format!("thread add --id {alias} --title Alias --brief One"),
        format!("thread add --id {edited} --title Edited --notes Gone"),
        format!("thread edit {edited} --no-brief"),
        format!("thread edit {edited} -b One"),
        format!("thread add --id {app} --title App"),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    let tracker = sh.path().join("tracker");
    let app_edit = |sent: &str, notes: &str, field: &str, expected: serde_json::Value| {
        let command =
            serde_json::from_value(json!({"thread": {"edit": {"thread": app, sent: notes}}}));
        let expect = serde_json::from_value(
            json!([{"kind": "thread-field", "thread": app, "field": field, "value": expected}]),
        );
        crate::call_expected(command.unwrap(), tracker.clone(), &sh.env(&[]), expect.unwrap())
    };
    let (code, out) = app_edit("notes", "Two", "notes", json!("Else"));
    assert_eq!((code, &out["error"]["code"]), (1, &json!("changed-since")), "{out}");
    assert_eq!(app_edit("notes", "Two", "notes", json!(null)).0, 0);
    assert_eq!(app_edit("brief", "One", "brief", json!("Two")).0, 0);
    let plan = json!({"format": "acyclic-import", "version": 4, "threads": [
        {"id": imported, "title": "Imported", "notes": "One"},
        {"id": older, "title": "Older", "brief": "One"},
    ]});
    std::fs::write(sh.path().join("plan.json"), plan.to_string()).unwrap();
    let (code, out) = sh.output("import plan.json", &[]);
    assert_eq!(code, 0, "{out}");
    let fields: Vec<&serde_json::Value> =
        out["data"]["changes"].as_array().unwrap().iter().map(|change| &change["fields"]).collect();
    assert!(fields.iter().all(|f| f["notes"]["to"] == "One" && f.get("brief").is_none()), "{out}");

    let (_, shown) = sh.output(&format!("thread show {}", ids.join(" ")), AGENT);
    let notes: Vec<&serde_json::Value> =
        shown["data"].as_array().unwrap().iter().map(|row| &row["notes"]).collect();
    assert_eq!(notes, [&json!("One"); 6], "{shown}");
    let (tracker, _) = acyclic_core::store::Tracker::open(&tracker).unwrap();
    let (_, state) = tracker.load().unwrap();
    for id in &ids {
        let key = acyclic_core::state::Key::new("thread", id).unwrap();
        let stored: Vec<&serde_json::Value> =
            state.field(&key, "brief").unwrap().values().map(|stamped| &stamped.value).collect();
        assert_eq!(stored, [&json!("One")], "{id}");
        assert!(state.field(&key, "notes").is_none(), "{id}");
    }
}

/// The retired words never come out: no key or value in the contract's transcripts, the pretty tables or the
/// examples the app's tests share says `backlog`, `blocked`, `brief` or `link-cycle`; no key is `entity` or `status`;
/// and no `progress` has a `state`. Text may still name `backlog` as what is stored, and a need's identity, an opaque
/// token kept stable across versions, names a thread's notes by their stored field, `brief`.
#[test]
fn no_output_says_a_retired_word() {
    const RETIRED: [&str; 4] = ["backlog", "blocked", "brief", "link-cycle"];
    const RETIRED_KEYS: [&str; 2] = ["entity", "status"];
    fn walk(value: &serde_json::Value, at: &str, identity: bool) {
        let check = |text: &str| {
            let stored = identity && text == "brief";
            assert!(!RETIRED.contains(&text) || stored, "{at} says {text}");
        };
        match value {
            serde_json::Value::String(text) => {
                check(text);
                // A need's identity is JSON in a string.
                if let Ok(inner @ serde_json::Value::Object(_)) = serde_json::from_str(text) {
                    walk(&inner, at, true);
                }
            },
            serde_json::Value::Array(values) => values.iter().for_each(|v| walk(v, at, identity)),
            serde_json::Value::Object(fields) => fields.iter().for_each(|(key, v)| {
                check(key);
                assert!(!RETIRED_KEYS.contains(&key.as_str()), "{at} has the key {key}");
                if key == "progress" {
                    assert!(v.get("state").is_none(), "{at} has a progress state");
                }
                walk(v, at, identity);
            }),
            _ => {},
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["commands.examples.json", "needs.examples.json"] {
        let path = root.join("../acyclic-app/src").join(name);
        let json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        walk(&json, name, false);
    }
    let mut read = 0;
    for dir in ["src/tests/snapshots", "src/pretty/snapshots", "src/snapshots"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            // A JSON key or value is the word in quotes (a need's identity escapes its own); a table cell or a
            // listed error code is the word alone.
            let words = RETIRED.iter().chain(&RETIRED_KEYS);
            let cells: Vec<&str> = text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .filter(|word| !word.is_empty())
                .collect();
            for word in words {
                let cell = !dir.ends_with("tests/snapshots")
                    && cells.iter().any(|w| w.eq_ignore_ascii_case(word));
                assert!(
                    !text.contains(&format!("\"{word}\"")) && !cell,
                    "{} says {word}",
                    path.display()
                );
            }
            read += 1;
        }
    }
    assert!(read > 2, "the snapshots moved");
}

/// Leases renew once a command's own writes are in, and a thread whose lease ran out with work ready reads stalled.
#[test]
fn a_finish_renews_the_lease_a_refused_command_does_not_and_a_lapsed_lease_stalls_the_thread() {
    const MINUTE: i64 = 60_000;
    let mut sh = Shell::new();
    let start: i64 = NOW.parse().unwrap();
    let at = |minutes: i64| (start + minutes * MINUTE).to_string();
    let run = |sh: &mut Shell, line: &str, actor: &str, minutes: i64| {
        let now = at(minutes);
        sh.output(line, &[("ACYCLIC_ACTOR", actor), ("ACYCLIC_NOW", &now)])
    };
    let lease_end = |sh: &mut Shell, minutes: i64| {
        let (_, shown) = run(sh, "thread show Work", "session/mac", minutes);
        shown["data"][0]["claims"][0]["lease_expires_at"].as_str().map(str::to_owned)
    };
    let minute = |minutes: i64| acyclic_core::time::rfc3339(start + minutes * MINUTE);
    assert_eq!(sh.init(), 0);
    for line in [
        "thread add --title Work",
        "item add --title First --thread Work",
        "item add --title Second --thread Work",
        "thread claim Work",
    ] {
        assert_eq!(run(&mut sh, line, "session/mac", 0).0, 0, "{line}");
    }
    assert_eq!(lease_end(&mut sh, 1), Some(minute(60)));
    // Another agent's refused finish writes nothing and renews nothing, though the lease is nearly out.
    assert_eq!(run(&mut sh, "item done First", "session/pi", 44).0, 1);
    assert_eq!(lease_end(&mut sh, 44), Some(minute(60)));
    // The holder's own finish at 0:45 counts as activity, and the lease reads renewed.
    assert_eq!(run(&mut sh, "item done First", "session/mac", 45).0, 0);
    assert_eq!(lease_end(&mut sh, 45), Some(minute(105)));
    // Finishing the last item renews nothing: the lease runs out on its own, with no work left to stall.
    assert_eq!(run(&mut sh, "item done Second", "session/mac", 100).0, 0);
    assert_eq!(lease_end(&mut sh, 100), Some(minute(105)));

    // Past the lease, new work in the thread is ready under only an expired claim: stalled, and listed for anyone.
    assert_eq!(run(&mut sh, "item add --title Third --thread Work", "session/pi", 106).0, 0);
    let (_, ready) = run(&mut sh, "thread ready", "session/pi", 106);
    assert_eq!(ready["data"]["ready"][0]["stalled"], json!(true), "{ready}");
    assert_eq!(ready["data"]["ready"][0]["items"][0]["stage"], json!("stalled"));
    let (_, shown) = run(&mut sh, "thread show Work", "session/pi", 106);
    assert_eq!(shown["data"][0]["readiness"]["readiness"], json!("stalled"));
    assert_eq!(shown["data"][0]["progress"]["counts"]["stalled"], json!(1));
    let (_, needs) = run(&mut sh, "needs", "session/pi", 106);
    let stalled: Vec<&serde_json::Value> = needs["data"]["needs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "stalled")
        .collect();
    assert_eq!(stalled.len(), 1, "{needs}");
    assert_eq!(
        (&stalled[0]["holder"], &stalled[0]["expired_at"], &stalled[0]["notify"]),
        (&json!("session/mac"), &json!(minute(105)), &json!(false))
    );
    // Claiming it works as claiming a ready thread does.
    assert_eq!(run(&mut sh, "thread claim Work", "session/pi", 106).0, 0);
    let (_, ready) = run(&mut sh, "thread ready", "session/pi", 107);
    assert_eq!(ready["data"]["ready"][0]["stalled"], json!(false));
}

/// A thread whose holder stopped without releasing it, with work ready: claimed two hours ago for a minute. It goes
/// first, since a replica refuses a write stamped before its last.
fn add_stalled_thread(sh: &mut Shell) {
    let earlier = [("ACYCLIC_ACTOR", "session/mac"), ("ACYCLIC_NOW", "1792015200000")];
    for line in [
        format!("thread add --id {} --title Stale", id("f3")),
        format!("item add --id {} --title Abandoned --thread {}", id("a8"), id("f3")),
        format!("thread claim {} --lease 1", id("f3")),
    ] {
        let (code, out) = sh.output(&line, &earlier);
        assert_eq!(code, 0, "{line}: {out}");
    }
}

/// The needs list the app's tests read, made here so they read what this build sends, with a conflict for each way a
/// value is kept. `UPDATE_EXAMPLES=1` writes it; otherwise it must match.
#[test]
#[expect(clippy::too_many_lines, reason = "one list of every need the app reads")]
fn the_needs_examples_are_this_builds_and_each_command_they_offer_settles_its_conflict() {
    let mut sh = Shell::new();
    sh.init();
    add_stalled_thread(&mut sh);
    let (a1, p1, e1, t1, r1, r2) = (id("a1"), id("b1"), id("e1"), id("f1"), id("c1"), id("c2"));
    for line in [
        format!("repository add --id {r1} --name me/code --default-branch main"),
        format!("repository add --id {r2} --name me/app --default-branch main"),
        format!("thread add --id {t1} --title Work"),
        format!("item add --id {a1} --title First --thread {t1}"),
        format!("item add --id {p1} --title Parent --thread {t1}"),
        format!("item add --id {e1} --title Shipped --thread {t1}"),
        format!("item done {e1} --commit me/code@abc1234 --force"),
        // Two needs that aren't conflicts, for the app to read the core's words for.
        format!("item add --id {} --title Loose", id("a2")),
        format!("item add --id {} --title Late --thread {t1}", id("a3")),
        format!("item add --id {} --title Early --thread {t1} --waits-for {}", id("a4"), id("a3")),
        format!("item done {} --commit me/code@abc1234 --force", id("a4")),
        // Work that waits for canceled work, and an item to be canceled on one replica and done on another.
        format!("item add --id {} --title Dropped --thread {t1}", id("a5")),
        format!(
            "item add --id {} --title Waiting --thread {t1} --waits-for {}",
            id("a6"),
            id("a5")
        ),
        format!("item add --id {} --title Disputed --thread {t1}", id("a7")),
        format!("item edit {} --state canceled", id("a5")),
        // A thread with nothing in it.
        format!("thread add --id {} --title Idle", id("f2")),
    ] {
        assert_eq!(sh.run(&line, AGENT), 0, "{line}");
    }
    // Each field written apart by another replica that hadn't seen this one's write.
    let key = |kind: &str, id: &str| acyclic_core::state::Key::new(kind, id).unwrap();
    let evidence = |sha: &str| json!({"repository": r1, "commit": sha});
    let apart = [
        (key("item", &a1), "title", json!("One"), json!("Two")),
        (key("item", &a1), "state", json!("done"), json!("parked")),
        (key("item", &id("a7")), "state", json!("done"), json!("canceled")),
        // Stored as `backlog`, shown and kept as `open`.
        (key("item", &p1), "state", json!("backlog"), json!("parked")),
        // An item moved under another, and under an initiative.
        (key("item", &a1), "parent", json!(p1), json!(id(DEFAULT_INITIATIVE))),
        (key("thread", &t1), "brief", json!("Why"), json!(null)),
        (key("repository", &r2), "owner", json!("me"), json!("you")),
        (key("item", &e1), "evidence", evidence("abc1234"), evidence("def5678")),
    ];
    let lost = [(id("a9"), None), (id("aa"), Some("gone"))];
    let (tracker, _) = acyclic_core::store::Tracker::open(&sh.path().join("tracker")).unwrap();
    let meta = acyclic_core::store::Meta { actor: "actor/user".into() };
    tracker
        .batch(&meta, NOW.parse().unwrap(), false, &|_, _| None, |replica, at| {
            let id = acyclic_core::state::ReplicaId::new("pi").unwrap();
            let mut other = acyclic_core::state::Replica::open(id, replica.state().clone());
            // Two items no command makes, as an older replica or a sync could leave them: one with no parent, one
            // whose parent is gone. Each is on Needs you as a missing parent.
            for (made, parent) in &lost {
                let key = acyclic_core::state::Key::new("item", made).unwrap();
                replica.write(
                    &key,
                    "title",
                    "actor/user",
                    at,
                    Some(json!(format!("Lost {made}"))),
                )?;
                replica.write(&key, "state", "actor/user", at, Some(json!("backlog")))?;
                if let Some(parent) = parent {
                    replica.write(&key, "parent", "actor/user", at, Some(json!(parent)))?;
                }
            }
            for (key, name, mine, theirs) in &apart {
                replica.write(key, name, "actor/user", at, Some(mine.clone()))?;
                other.write(key, name, "actor/user", at, Some(theirs.clone()))?;
            }
            replica.join(other.state())
        })
        .unwrap();
    let (_, needs) = sh.output("needs", AGENT);
    let needs = &needs["data"]["needs"];
    let conflicts: Vec<&serde_json::Value> =
        needs.as_array().unwrap().iter().filter(|need| need["kind"] == "conflict").collect();
    assert_eq!(conflicts.len(), apart.len(), "{needs}");

    let examples = json!({
        "about": "A needs-you list as this build sends it, with a conflict for each way a value is kept. Written by \
                  the CLI's contract tests with UPDATE_EXAMPLES=1; the app's tests read it.",
        "needs": needs,
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../acyclic-app/src/needs.examples.json");
    let text = format!("{}\n", serde_json::to_string_pretty(&examples).unwrap());
    if std::env::var_os("UPDATE_EXAMPLES").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap_or_default(),
        text,
        "run with UPDATE_EXAMPLES=1"
    );

    // Each command parses as the API reads it, and the first each conflict offers settles it.
    let path = sh.path().join("tracker");
    for conflict in conflicts {
        let mut settled = false;
        for kept in conflict["keeps"].as_array().unwrap() {
            if kept["command"].is_null() {
                assert!(kept["why"].is_string(), "{kept}");
                continue;
            }
            let command: crate::args::Command =
                serde_json::from_value(kept["command"].clone()).unwrap();
            if !settled {
                let (code, out) = crate::call(command, path.clone(), &sh.env(&[]));
                assert_eq!(code, 0, "{kept}: {out}");
                let (_, after) = sh.output("needs", AGENT);
                let keys: Vec<&serde_json::Value> = after["data"]["needs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|need| &need["key"])
                    .collect();
                assert!(!keys.contains(&&conflict["key"]), "{kept} left {}", conflict["key"]);
                settled = true;
            }
        }
        assert!(settled || conflict["field"] == "state", "nothing keeps {}", conflict["key"]);
    }
}

/// Git in `dir` with no user or system settings and a fixed identity; its trimmed output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.test")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A code repository `origin` and its clone, the clone on a branch `work` that `origin` never merges, and a home
/// whose `clones.toml` maps `me/code` to the clone. Returns the home and the commit only `work` has.
fn code(sh: &Shell) -> (String, String) {
    let (origin, clone, home) =
        (sh.path().join("origin"), sh.path().join("clone"), sh.path().join("home"));
    git(sh.path(), &["init", "-q", "-b", "main", origin.to_str().unwrap()]);
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "base"]);
    git(sh.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
    git(&clone, &["switch", "-q", "-c", "work"]);
    git(&clone, &["commit", "-q", "--allow-empty", "-m", "unmerged"]);
    let work = git(&clone, &["rev-parse", "HEAD"]);
    std::fs::create_dir_all(home.join(".config/acyclic")).unwrap();
    let mapping = format!("[clones]\n\"me/code\" = \"{}\"\n", clone.display());
    std::fs::write(home.join(".config/acyclic/clones.toml"), mapping).unwrap();
    (home.to_str().unwrap().to_owned(), work)
}

#[test]
fn a_commit_is_checked_when_it_is_written_and_never_when_read() {
    let mut sh = Shell::new();
    let (home, work) = code(&sh);
    let mapped: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/mac"), ("HOME", &home)];
    sh.init();
    sh.run("repository add --name me/code --default-branch main", mapped);
    sh.run("repository add --name me/other --default-branch main", mapped);
    for (n, title) in [("a1", "Ship"), ("a2", "Elsewhere"), ("a3", "Later")] {
        sh.run(&format!("item add --id {} --title {title}", id(n)), mapped);
    }
    let refused = |sh: &mut Shell, line: &str| {
        let (code, out) = sh.output(line, mapped);
        assert_eq!((code, &out["error"]["code"]), (1, &json!("unmerged")), "{out}");
        out["error"]["details"].clone()
    };
    // A branch's commit and a typo are refused, forced or not, and nothing is written.
    let details = refused(&mut sh, &format!("item done Ship --force --commit me/code@{work}"));
    assert_eq!(details["reason"], "not-merged");
    let details = refused(&mut sh, "item done Ship --force --commit me/code@abcdef1");
    assert_eq!(details["reason"], "unknown-commit");
    assert_eq!(sh.output("item show Ship", mapped).1["data"][0]["state"], "open");
    // A squash commit merged since the clone last fetched is fetched and passes, abbreviated, with the checkout
    // untouched.
    let origin = sh.path().join("origin");
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "squashed"]);
    let squashed = git(&origin, &["rev-parse", "HEAD"]);
    let (code, out) =
        sh.output(&format!("item done Ship --force --commit me/code@{}", &squashed[..7]), mapped);
    assert_eq!(code, 0, "{out}");
    assert!(out["data"].get("unchecked_commits").is_none(), "{out}");
    assert_eq!(git(&sh.path().join("clone"), &["branch", "--show-current"]), "work");
    // A repository this machine has no clone of is written, and says it couldn't be checked.
    let (code, out) = sh.output("item done Elsewhere --force --commit me/other@abcdef1", mapped);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["data"]["unchecked_commits"][0]["repository"], "me/other");
    // A fetch that fails refuses, saying so, and writes nothing.
    git(&origin, &["commit", "-q", "--allow-empty", "-m", "later"]);
    let later = git(&origin, &["rev-parse", "HEAD"]);
    std::fs::rename(&origin, sh.path().join("gone")).unwrap();
    let details = refused(&mut sh, &format!("item done Later --force --commit me/code@{later}"));
    assert_eq!(details["reason"], "fetch-failed");
    assert!(details["fetch_error"].as_str().is_some_and(|e| !e.is_empty()), "{details}");
    assert_eq!(sh.output("item show Later", mapped).1["data"][0]["state"], "open");
    // Doctor checks what is stored as last fetched, fetching nothing: the unmapped commit isn't counted as checked.
    let doctor = sh.output("doctor", mapped).1;
    assert_eq!(doctor["data"]["commits"], json!({"checked": 1, "unchecked": 1, "unmerged": []}));
    // Reads run no git: with every clone mapped to a path that doesn't exist, they read the same.
    std::fs::write(
        sh.path().join("home/.config/acyclic/clones.toml"),
        "[clones]\n\"me/code\" = \"/nonexistent\"\n\"me/other\" = \"/nonexistent\"\n",
    )
    .unwrap();
    for line in ["item list", "thread ready", "needs", "item show Ship"] {
        assert_eq!(sh.output(line, mapped).1, sh.output(line, AGENT).1, "{line}");
    }
}

#[test]
fn an_import_checks_the_commits_it_writes_and_a_dry_run_too() {
    let mut sh = Shell::new();
    let (home, work) = code(&sh);
    let mapped: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/mac"), ("HOME", &home)];
    sh.init();
    let plan = |commit: &str| {
        json!({
            "format": "acyclic-import", "version": 4,
            "repositories": [{"id": id("c1"), "name": "me/code", "default_branch": "main"}],
            "items": [{"id": id("a1"), "title": "Ship", "state": "done",
                       "parent": id(DEFAULT_INITIATIVE),
                       "evidence": {"commit": format!("me/code@{commit}")}}],
        })
        .to_string()
    };
    std::fs::write(sh.path().join("plan.json"), plan(&work)).unwrap();
    for line in ["import plan.json --dry-run", "import plan.json"] {
        let (code, out) = sh.output(line, mapped);
        assert_eq!((code, &out["error"]["details"]["reason"]), (1, &json!("not-merged")), "{out}");
    }
    assert_eq!(sh.output("doctor", mapped).1["data"]["counts"]["items"], 0, "nothing written");
    let base = git(&sh.path().join("origin"), &["rev-parse", "HEAD"]);
    std::fs::write(sh.path().join("plan.json"), plan(&base)).unwrap();
    let (code, out) = sh.output("import plan.json", mapped);
    assert_eq!(code, 0, "{out}");
}
