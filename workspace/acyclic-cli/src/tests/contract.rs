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
    assert_eq!(sh.run(&format!("item add --id {a1} --title First"), AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {a2} --title Second"), AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {a2} --title Second"), AGENT), 0, "a repeat");
    assert_eq!(sh.run(&format!("item add --id {a2} --title Other"), AGENT), 1);
    // A record is named by its title, or by the end of its id.
    assert_eq!(sh.run("item edit Second --waits-for First", AGENT), 0);
    let (_, ready) = sh.output("thread ready", AGENT);
    assert_eq!(ready["data"]["ready"], json!([]), "nothing is ready outside a thread");
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
    for (flag, present) in [("--repository ME/CODE", true), ("--no-repository me/code", false)] {
        let (code, out) = sh.output(&format!("item edit Second {flag}"), AGENT);
        assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(true)), "{flag}");
        assert_eq!(
            sh.output("repository show me/code", AGENT).1["data"][0]["items"],
            json!(present.then_some(&a2).into_iter().collect::<Vec<_>>())
        );
    }
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
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("link-cycle")), "{out}");
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
    // A parent is finished once its children are.
    let [g1, g1a, g2, g2a] = ["c1", "c2", "c3", "c4"].map(id);
    sh.run(&format!("item add --id {g1} --title Goal"), AGENT);
    sh.run(&format!("item add --id {g1a} --title Part --parent Goal"), AGENT);
    let (code, out) = sh.output("item done Goal", AGENT);
    assert_eq!((code, &out["error"]["details"]["open"]), (1, &json!([g1a])));
    sh.run("item edit Part --thread Follow-ups", AGENT);
    sh.run("thread claim Follow-ups", AGENT);
    sh.run("item done Part", AGENT);
    assert_eq!(sh.run("item done Goal", AGENT), 0, "a parent is finished once its children are");
    // A done item's title is free again, though the title then names both.
    sh.run(&format!("item add --id {g2} --title Goal"), AGENT);
    assert_eq!(sh.output("item show Goal", AGENT).1["error"]["code"], "ambiguous");
    sh.run(&format!("item add --id {g2a} --title Part --parent 0000c3"), AGENT);
    assert_eq!(sh.run("item done 0000c3 --force", AGENT), 0, "or when forced");
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
    insta::assert_snapshot!(sh.transcript);
}

#[test]
fn an_add_takes_only_a_uuidv7_in_any_case_and_references_match_as_the_rules_say() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
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
    sh.run("init tracker --replica mac", &[]);
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
    sh.run("init tracker --replica mac", &[]);
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
        assert!(error["message"].as_str().unwrap().contains("must be migrated"), "{line}: {error}");
        assert_eq!(
            error["details"]["hint"],
            "written by an earlier acyclic: read it with that version, or rebuild it with `acyclic import` from a \
             version-3 document"
        );
    }
    // Any other unreadable state points to doctor, except from doctor itself.
    write_header("not a header".to_owned());
    assert_eq!(refused("item list")["details"]["hint"], "run acyclic doctor");
    assert_eq!(refused("doctor")["details"], json!(null));
}

#[test]
fn an_unreadable_state_points_to_doctor() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
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

#[test]
fn a_parent_that_is_a_thread_suggests_a_command_that_pastes_as_it_is() {
    let mut sh = Shell::new();
    sh.run("init tracker --replica mac", &[]);
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
