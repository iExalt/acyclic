//! Journeys through the binary on a machine where `git` does not exist, in the way that matters: the binary's `PATH`
//! holds a `git` that records each call and fails, so a call that production code swallows (a skipped upkeep, a scratch
//! ref left behind) still fails the journey. The fixtures and the assertions use real Git, as an oracle, outside the
//! binary's environment.
//!
//! `ACYCLIC_NO_GIT_CONTROL=1` gives the binary the real `PATH` instead, which shows that a journey is valid with Git
//! before it is trusted without it.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z.
const NOW: &str = "1792022400000";

/// The calls to `git` that don't come from acyclic: the library's own probes for where Git's system configuration is,
/// which find no Git and carry on.
const PROBES: [&str; 2] = ["--exec-path", "config -lz --show-origin"];

/// A `git` that isn't there: it records its arguments and fails like a missing command.
struct Tripwire {
    dir: TempDir,
}

impl Tripwire {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("git");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\necho 'git: not found' >&2\nexit 127\n",
                dir.path().join("calls").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn control() -> bool {
        std::env::var_os("ACYCLIC_NO_GIT_CONTROL").is_some()
    }

    fn path(&self) -> OsString {
        if Self::control() {
            std::env::var_os("PATH").unwrap_or_default()
        } else {
            self.dir.path().as_os_str().to_owned()
        }
    }

    /// `acyclic` in `dir`, in a clean environment whose `PATH` is the tripwire.
    fn acyclic(&self, dir: &Path) -> Command {
        let mut command = Command::new(BIN);
        command
            .env_clear()
            .env("PATH", self.path())
            .env("HOME", "/nonexistent")
            .env("ACYCLIC_NOW", NOW)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(dir);
        command
    }

    fn run(&self, tracker: &Path, line: &str) -> Value {
        let output = self
            .acyclic(tracker)
            .env("ACYCLIC_TRACKER", tracker)
            .args(line.split(' '))
            .output()
            .unwrap();
        assert!(output.status.success(), "{line}: {}", String::from_utf8_lossy(&output.stdout));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// The calls the binary made, other than the library's probes.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .filter(|call| !PROBES.iter().any(|probe| call.starts_with(probe)))
            .map(str::to_owned)
            .collect()
    }

    fn assert_no_git_was_called(&self) {
        assert_eq!(self.calls(), Vec::<String>::new(), "the binary ran git");
    }
}

/// Real Git, as the oracle.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn init(tripwire: &Tripwire, dir: &Path, name: &str, remote: &str) -> PathBuf {
    let output = tripwire
        .acyclic(dir)
        .args(["init", name, "--replica", name, "--remote", remote])
        .output()
        .unwrap();
    assert!(output.status.success(), "init {name}: {}", String::from_utf8_lossy(&output.stdout));
    dir.join(name)
}

fn count_files(dir: &Path, extension: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == extension))
                .count()
        })
        .unwrap_or(0)
}

const A: &str = "01900000-0000-7000-8000-0000000000a1";
const B: &str = "01900000-0000-7000-8000-0000000000b1";

#[test]
fn two_machines_set_up_write_and_sync_through_a_shared_repository_without_git() {
    let tripwire = Tripwire::new();
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "--quiet", "--bare", "shared.git"]);
    let mac = init(&tripwire, dir.path(), "mac", "../shared.git");
    let pi = init(&tripwire, dir.path(), "pi", "../shared.git");
    git(&mac, &["config", "acyclic.pushAfterWrite", "true"]);

    tripwire.run(&mac, &format!("item add --id {A} --title A"));
    let shared = dir.path().join("shared.git");
    assert!(
        git(&shared, &["for-each-ref", "--format=%(refname)", "refs/heads/state/"])
            .contains("refs/heads/state/mac"),
        "a write that asks to push reached the shared repository"
    );
    tripwire.run(&pi, &format!("item add --id {B} --title B"));
    tripwire.run(&pi, "sync");
    tripwire.run(&mac, "sync");
    tripwire.run(&mac, &format!("item edit {B} --title B2"));
    tripwire.run(&mac, "sync");
    tripwire.run(&pi, "sync");
    let shown = tripwire.run(&pi, &format!("item show {B}"));
    assert!(
        shown.to_string().contains("B2"),
        "an edit made on another machine reached this one: {shown}"
    );

    // What the CLI swallows if Git is missing: a marker that was never cleared.
    for clone in [&mac, &pi] {
        assert_eq!(
            Command::new("git")
                .arg("-C")
                .arg(clone)
                .args(["config", "--get", "acyclic.replica-unused"])
                .output()
                .unwrap()
                .status
                .code(),
            Some(1),
            "the first write cleared the unused marker in {}",
            clone.display()
        );
    }
    tripwire.assert_no_git_was_called();
}

#[test]
fn many_small_writes_stay_packed_without_git() {
    let tripwire = Tripwire::new();
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "--quiet", "--bare", "shared.git"]);
    let mac = init(&tripwire, dir.path(), "mac", "../shared.git");
    git(&mac, &["config", "gc.autoPackLimit", "3"]);
    for n in 0..12 {
        tripwire.run(
            &mac,
            &format!("item add --id 01900000-0000-7000-8000-00000000c{n:03x} --title c{n}"),
        );
    }
    // Upkeep may run in the background, so give it time; the packs of 12 writes would number 12 without it.
    let packs = mac.join(".git/objects/pack");
    let deadline = Instant::now() + Duration::from_secs(30);
    while count_files(&packs, "pack") > 4 {
        assert!(
            Instant::now() < deadline,
            "the store never repacked: {} packs",
            count_files(&packs, "pack")
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    tripwire.assert_no_git_was_called();
}

/// No production code starts or builds a `git` command. The tests, which use Git as an oracle, are the exception.
#[test]
fn no_production_code_runs_git() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    "tests" | "benches" | "target" | "node_modules" | "acyclic-app"
                ) {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && name != "tests.rs"
                && name != "no_git.rs"
            {
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    if line.contains("Command::new(\"git\")") || line.contains("git_at(") {
                        found.push(format!(
                            "{}:{}: {}",
                            path.strip_prefix(&root).unwrap().display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
    assert_eq!(found, Vec::<String>::new(), "production code still runs git");
}
