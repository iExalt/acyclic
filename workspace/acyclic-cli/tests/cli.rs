//! The CLI contract across two machines syncing through a shared repository: each test drives two tracker clones.

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

/// The second machine's actor and tracker clone.
const PI: &[(&str, &str)] = &[("ACYCLIC_ACTOR", "session/pi"), ("ACYCLIC_TRACKER", "pi")];

/// A shell holding a bare repository, `shared.git`, for clones to sync through.
fn shared() -> Shell {
    let sh = Shell::new();
    git(sh.path(), &["init", "-q", "--bare", "shared.git"]);
    // The remote's own repacking is the host's business, not these tests'.
    git(&sh.path().join("shared.git"), &["config", "receive.autogc", "false"]);
    sh
}

/// Two machines syncing through `shared.git`: `mac`, which added a1, and `pi`, which has synced it.
fn two_machines() -> Shell {
    let mut sh = shared();
    assert_eq!(sh.run("init tracker --replica mac --remote ../shared.git", &[]), 0);
    sh.run("item add a1 --title First", AGENT);
    sh.run("sync", AGENT);
    assert_eq!(sh.run("init pi --replica pi --remote ../shared.git", &[]), 0);
    sh.run("sync", PI);
    sh
}

/// The needs, leaving out the items in no thread, which these tests never thread.
fn needs(sh: &mut Shell) -> Vec<serde_json::Value> {
    let (_, out) = sh.output("needs", AGENT);
    let all = out["data"]["needs"].as_array().unwrap().clone();
    all.into_iter().filter(|entry| entry["kind"] != "unthreaded").collect()
}

#[test]
fn two_machines_sync_through_a_shared_repository() {
    let mut sh = shared();
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
    let (code, out) = sh.output("sync", PI);
    assert_eq!(
        (code, &out["data"]["fetched"], out["data"]["joined"].as_bool()),
        (0, &serde_json::json!(["mac"]), Some(true))
    );
    assert_eq!(sh.output("item show a1", PI).0, 0);
    sh.run("item add b1 --title Second", PI);
    sh.run("sync", PI);
    let (_, out) = sh.output("sync", AGENT);
    assert_eq!(out["data"]["fetched"], serde_json::json!(["mac", "pi"]));
    assert_eq!(sh.output("item show b1", AGENT).0, 0, "and the first sees the second's");
}

#[test]
fn a_write_asked_to_push_reports_it_and_a_failed_push_shows_in_needs_until_a_sync() {
    let mut sh = shared();
    assert_eq!(sh.run("init tracker --replica mac --remote ../shared.git", &[]), 0);
    git(&sh.path().join("tracker"), &["config", "acyclic.pushAfterWrite", "true"]);
    let (_, out) = sh.output("item add a2 --title Pushed", AGENT);
    assert_eq!(out["data"]["push"]["ok"].as_bool(), Some(true), "{out}");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../nowhere.git"]);
    let (code, out) = sh.output("item add a3 --title Stranded", AGENT);
    assert_eq!((code, out["data"]["push"]["ok"].as_bool()), (0, Some(false)), "the write stands");
    assert_eq!(needs(&mut sh)[0]["kind"], "unpushed");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../shared.git"]);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn a_move_to_the_top_racing_another_machines_move_is_a_conflict_until_moved_again() {
    let mut sh = two_machines();
    sh.run("item add a2 --title Parent", AGENT);
    sh.run("item edit a1 --parent a2", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", PI);
    sh.run("item edit a1 --no-parent", AGENT);
    sh.run("item add b1 --title Other", PI);
    sh.run("item edit a1 --parent b1", PI);
    sh.run("sync", PI);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["kind"], "conflict");
    let (_, out) = sh.output("item edit a1 --no-parent", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "a move to the top resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn taking_an_item_out_of_a_thread_racing_another_machine_threading_it_is_a_conflict_until_taken_out_again()
 {
    let mut sh = two_machines();
    sh.run("thread add t1 --title One", AGENT);
    sh.run("thread add t2 --title Two", AGENT);
    sh.run("item edit a1 --thread t1", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", PI);
    sh.run("item edit a1 --no-thread", AGENT);
    sh.run("item edit a1 --thread t2", PI);
    sh.run("sync", PI);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["field"], "thread");
    let (_, out) = sh.output("item edit a1 --no-thread", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "taking it out again resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn threads_each_machine_numbers_alike_clash_once_a_sync_brings_them_together() {
    let mut sh = two_machines();
    sh.run("thread add --title Beds", AGENT);
    sh.run("thread add --title Rows", PI);
    sh.run("sync", PI);
    let (_, out) = sh.output("sync", AGENT);
    let clash = &out["data"]["number_clashes"][0];
    assert_eq!(
        (clash["number"].as_u64(), clash["threads"].as_array().map(Vec::len)),
        (Some(1), Some(2)),
        "{out}"
    );
    let listed = needs(&mut sh);
    assert_eq!(
        (listed.len(), &listed[0]["kind"]),
        (1, &serde_json::json!("number-clash")),
        "{listed:?}"
    );
}
