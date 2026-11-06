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
        .env("GIT_AUTHOR_NAME", "acyclic test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
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

/// Two machines syncing through `shared.git`: `mac`, which added First, and `pi`, which has synced it.
fn two_machines() -> Shell {
    let mut sh = shared();
    assert_eq!(sh.run("init tracker --replica mac --remote ../shared.git", &[]), 0);
    sh.run("item add --title First", AGENT);
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
    sh.run("item add --title First", AGENT);
    let (code, out) = sh.output("sync", AGENT);
    assert_eq!((code, out["data"]["pushed"].as_u64().is_some_and(|n| n > 0)), (0, true));
    // A second machine starts from the shared repository and sees the first's work.
    assert_eq!(sh.run("init pi --replica pi --remote ../shared.git", &[]), 0);
    let (code, out) = sh.output("sync", PI);
    assert_eq!(
        (code, &out["data"]["fetched"], out["data"]["joined"].as_bool()),
        (0, &serde_json::json!(["mac"]), Some(true))
    );
    assert_eq!(sh.output("item show First", PI).0, 0);
    sh.run("item add --title Second", PI);
    sh.run("sync", PI);
    let (_, out) = sh.output("sync", AGENT);
    assert_eq!(out["data"]["fetched"], serde_json::json!(["mac", "pi"]));
    assert_eq!(sh.output("item show Second", AGENT).0, 0, "and the first sees the second's");
}

#[test]
fn a_fresh_replica_can_continue_after_the_old_clock_jumps_ahead() {
    let mut sh = shared();
    assert_eq!(sh.run("init tracker --replica mac --remote ../shared.git", &[]), 0);
    let thread = "01900000-0000-7000-8000-0000000000f1".to_owned();
    let item = "01900000-0000-7000-8000-0000000000a1".to_owned();
    assert_eq!(sh.run(&format!("thread add --id {thread} --title Work"), AGENT), 0);
    assert_eq!(sh.run(&format!("item add --id {item} --title First --thread {thread}"), AGENT), 0);

    // A debug-only clock moves this synthetic replica forward without touching the host clock.
    let future = (NOW.parse::<i64>().unwrap() + 86_400_000).to_string();
    let future_at = future.parse::<i64>().unwrap();
    let future_writer = [("ACYCLIC_ACTOR", "session/mac"), ("ACYCLIC_NOW", future.as_str())];
    assert_eq!(sh.run("thread claim Work --lease 5", &future_writer), 0);
    assert_eq!(sh.run(&format!("item done {item} --force --evidence shipped"), &future_writer), 0);

    let old_path = sh.path().join("tracker");
    let (old, _) = acyclic_core::store::Tracker::open(&old_path).unwrap();
    let (_, before) = old.load_strict().unwrap();
    let old_id = acyclic_core::state::ReplicaId::new("mac").unwrap();
    assert_eq!(before.context.clocks.get(&old_id), Some(&future_at));
    let old_model = acyclic_core::model::Model::read(&before).unwrap();
    assert_eq!(old_model.items[&item].done_at, Some(future_at));
    assert_eq!(old_model.items[&item].evidence.one().unwrap().text.as_deref(), Some("shipped"));
    let thread_key = acyclic_core::claim::thread_key(&thread).unwrap();
    let future_claim =
        acyclic_core::claim::winner(&before, &thread_key, NOW.parse().unwrap()).unwrap().unwrap();
    assert_eq!(future_claim.lease_expires, Some(future_at + 5 * 60_000));

    let (code, out) = sh.output(&format!("item edit {item} --notes blocked"), AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("clock-jump")), "{out}");
    assert_eq!(
        out["error"]["details"]["hint"],
        "Correct the system clock and wait until it is within five minutes of this replica's saved time. If this clone has a shared remote, sync it and continue from a fresh clone with an unused replica id",
        "{out}"
    );
    let (_, unchanged) = old.load_strict().unwrap();
    assert_eq!(unchanged, before, "the refused write leaves old replica state unchanged");
    assert_eq!(sh.run("sync", AGENT), 0, "sync does not stamp a write");
    let (_, pushed) = old.load_strict().unwrap();
    assert_eq!(pushed.context.clocks.get(&old_id), Some(&future_at));

    assert_eq!(sh.run("init fresh --replica pi --remote ../shared.git", &[]), 0);
    let fresh = [("ACYCLIC_ACTOR", "session/mac"), ("ACYCLIC_TRACKER", "fresh")];
    assert_eq!(sh.run("sync", &fresh), 0);
    let (new, _) = acyclic_core::store::Tracker::open(&sh.path().join("fresh")).unwrap();
    let (_, joined) = new.load_strict().unwrap();
    assert_eq!(joined.context.clocks.get(&old_id), Some(&future_at));
    assert_eq!(
        joined.context.clocks.get(&acyclic_core::state::ReplicaId::new("pi").unwrap()),
        None,
        "the new replica has its own clock"
    );
    let joined_model = acyclic_core::model::Model::read(&joined).unwrap();
    assert_eq!(joined_model.items[&item].done_at, Some(future_at));
    assert_eq!(joined_model.items[&item].evidence.one().unwrap().text.as_deref(), Some("shipped"));
    let inherited_claim =
        acyclic_core::claim::winner(&joined, &thread_key, NOW.parse().unwrap()).unwrap().unwrap();
    assert_eq!(inherited_claim.lease_expires, Some(future_at + 5 * 60_000));

    let (code, out) = sh.output(
        "thread claim Work --force",
        &[("ACYCLIC_ACTOR", "session/other"), ("ACYCLIC_TRACKER", "fresh")],
    );
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("claimed")), "{out}");
    let (_, after_refusal) = new.load_strict().unwrap();
    assert_eq!(after_refusal, joined, "force does not replace another holder's live claim");

    assert_eq!(sh.run(&format!("item edit {item} --notes recovered"), &fresh), 0);
    let (_, continued) = new.load_strict().unwrap();
    let new_id = acyclic_core::state::ReplicaId::new("pi").unwrap();
    assert_eq!(continued.context.clocks.get(&old_id), Some(&future_at));
    assert_eq!(continued.context.clocks.get(&new_id), Some(&NOW.parse().unwrap()));
    let continued_model = acyclic_core::model::Model::read(&continued).unwrap();
    assert_eq!(continued_model.items[&item].done_at, Some(future_at));
    assert_eq!(
        continued_model.items[&item].evidence.one().unwrap().text.as_deref(),
        Some("shipped")
    );
    assert_eq!(continued_model.items[&item].notes.one().map(String::as_str), Some("recovered"));

    let (code, out) = sh.output("thread release Work", &fresh);
    assert_eq!((code, out["data"]["released"].as_bool()), (0, Some(true)), "{out}");
    let (_, released) = new.load_strict().unwrap();
    assert!(
        acyclic_core::claim::winner(&released, &thread_key, NOW.parse().unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn init_checks_the_effective_origin_before_changing_a_new_destination() {
    let mut sh = shared();
    let (code, out) = sh.output("init tracker --replica mac --remote ../shared.git", &[]);
    assert_eq!(code, 0, "{out}");
    sh.run("item add --title First", AGENT);
    sh.run("sync", AGENT);

    let (code, out) = sh.output("init tracker --replica mac --remote ../shared.git", &[]);
    assert_eq!((code, out["data"]["changed"].as_bool()), (0, Some(false)), "same-clone re-init");

    let (code, out) = sh.output("init duplicate --replica mac --remote ../shared.git", &[]);
    assert_eq!((code, out["error"]["code"].as_str()), (1, Some("duplicate-replica")), "{out}");
    assert!(out["error"]["message"].as_str().unwrap().contains("choose a different --replica id"));
    assert!(!sh.path().join("duplicate").exists(), "duplicate id must not create its destination");

    git(sh.path(), &["init", "-q", "--bare", "empty.git"]);
    let override_path = sh.path().join("override");
    std::fs::create_dir(&override_path).unwrap();
    git(&override_path, &["init", "-q"]);
    git(&override_path, &["remote", "add", "origin", "../shared.git"]);
    let (code, out) = sh.output("init override --replica mac --remote ../empty.git", &[]);
    assert_eq!(code, 0, "an explicit remote overrides the existing origin: {out}");
    assert!(
        std::fs::read_to_string(override_path.join(".git/config"))
            .unwrap()
            .contains("../empty.git")
    );

    let failed = sh.path().join("failed");
    std::fs::create_dir(&failed).unwrap();
    std::fs::write(failed.join("keep"), "unchanged").unwrap();
    git(&failed, &["init", "-q"]);
    git(&failed, &["remote", "add", "origin", "../missing.git"]);
    let (code, out) = sh.output("init failed --replica other", &[]);
    assert_eq!((code, out["error"]["code"].as_str()), (4, Some("git")), "{out}");
    assert_eq!(std::fs::read_to_string(failed.join("keep")).unwrap(), "unchanged");
    let config = std::fs::read_to_string(failed.join(".git/config")).unwrap();
    assert!(config.contains("../missing.git"));
    assert!(!config.contains("acyclic.replica"), "failed preflight must not configure a replica");
}

#[test]
fn a_write_asked_to_push_reports_it_and_a_failed_push_shows_in_needs_until_a_sync() {
    let mut sh = shared();
    assert_eq!(sh.run("init tracker --replica mac --remote ../shared.git", &[]), 0);
    git(&sh.path().join("tracker"), &["config", "acyclic.pushAfterWrite", "true"]);
    let (_, out) = sh.output("item add --title Pushed", AGENT);
    assert_eq!(out["data"]["push"]["ok"].as_bool(), Some(true), "{out}");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../nowhere.git"]);
    let (code, out) = sh.output("item add --title Stranded", AGENT);
    assert_eq!((code, out["data"]["push"]["ok"].as_bool()), (0, Some(false)), "the write stands");
    assert_eq!(needs(&mut sh)[0]["kind"], "unpushed");
    git(&sh.path().join("tracker"), &["remote", "set-url", "origin", "../shared.git"]);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn a_move_to_the_top_racing_another_machines_move_is_a_conflict_until_moved_again() {
    let mut sh = two_machines();
    sh.run("item add --title Parent", AGENT);
    sh.run("item edit First --parent Parent", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", PI);
    sh.run("item edit First --no-parent", AGENT);
    sh.run("item add --title Other", PI);
    sh.run("item edit First --parent Other", PI);
    sh.run("sync", PI);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["kind"], "conflict");
    let (_, out) = sh.output("item edit First --no-parent", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "a move to the top resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn taking_an_item_out_of_a_thread_racing_another_machine_threading_it_is_a_conflict_until_taken_out_again()
 {
    let mut sh = two_machines();
    sh.run("thread add --title One", AGENT);
    sh.run("thread add --title Two", AGENT);
    sh.run("item edit First --thread One", AGENT);
    sh.run("sync", AGENT);
    sh.run("sync", PI);
    sh.run("item edit First --no-thread", AGENT);
    sh.run("item edit First --thread Two", PI);
    sh.run("sync", PI);
    sh.run("sync", AGENT);
    assert_eq!(needs(&mut sh)[0]["field"], "thread");
    let (_, out) = sh.output("item edit First --no-thread", AGENT);
    assert_eq!(out["data"]["changed"].as_bool(), Some(true), "taking it out again resolves it");
    assert_eq!(needs(&mut sh), Vec::<serde_json::Value>::new());
}

#[test]
fn a_merged_commit_counts_done_from_another_thread_through_the_clone_mapped_by_owner_and_name() {
    let mut sh = Shell::new();
    // A code clone whose main holds one commit, mapped in clones.toml by the repository's owner/name.
    let code = sh.path().join("code");
    git(sh.path(), &["init", "-q", "-b", "main", "code"]);
    git(&code, &["commit", "-q", "--allow-empty", "-m", "c"]);
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&code)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let sha = String::from_utf8(head.stdout).unwrap();
    let home = sh.path().join("home");
    std::fs::create_dir_all(home.join(".config/acyclic")).unwrap();
    let mapping = format!("[clones]\n\"Me/Code\" = {:?}\n", code.to_str().unwrap());
    std::fs::write(home.join(".config/acyclic/clones.toml"), mapping).unwrap();
    let home = home.to_str().unwrap().to_owned();
    let at_home = [("HOME", home.as_str()), ("ACYCLIC_ACTOR", "session/mac")];
    assert_eq!(sh.run("init tracker --replica mac", &[]), 0);
    for line in [
        "repository add --name me/code --default-branch main",
        "thread add --title Build",
        "thread add --title Use",
        "item add --title Made --thread Build",
        "item add --title Used --thread Use --waits-for Made",
        "thread claim Build",
    ] {
        assert_eq!(sh.run(line, &at_home), 0, "{line}");
    }
    let done = format!("item done Made --commit me/code@{}", sha.trim());
    let (code, out) = sh.output(&done, &at_home);
    assert_eq!(code, 0, "{out}");
    let (_, made) = sh.output("item show Made", &at_home);
    assert_eq!(made["data"][0]["readiness"]["status"], "done", "{made}");
    let (_, used) = sh.output("item show Used", &at_home);
    assert_eq!(used["data"][0]["readiness"]["status"], "ready", "{used}");
}

/// A directory that isn't a tracker is a setup problem, `no-replica`, as it always was, and not a Git failure.
#[test]
fn a_directory_that_is_no_repository_is_no_replica_not_a_git_failure() {
    let mut sh = Shell::new();
    std::fs::create_dir(sh.path().join("tracker")).unwrap();
    let (code, out) = sh.output("item list", AGENT);
    assert_eq!((code, out["error"]["code"].as_str()), (5, Some("no-replica")), "{out}");
}
