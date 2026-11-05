//! Journeys through the binary on a machine where `git` does not exist, in the way that matters: the binary's `PATH`
//! holds a `git`, and the programs Git would start for a transport, that record their calls and fail like missing
//! commands, so a call that production code swallows (a skipped upkeep, a marker left behind) still fails the journey.
//! The fixtures and the assertions use real Git, as an oracle, outside the binary's environment: after each journey
//! `git fsck --strict` reads everything the binary wrote.
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

/// The whole calls to `git` that don't come from acyclic: the library's own probes for where Git's system
/// configuration and programs are, which find no Git and carry on.
const PROBES: [&str; 3] = [
    "git --exec-path",
    "git config -lz --show-origin --no-includes --name-only",
    "git config -lz --show-origin --no-includes --show-scope --name-only",
];

/// Programs that Git, or a transport it speaks, would start. Each is as missing as `git` is.
const PROGRAMS: [&str; 7] = [
    "git",
    "git-upload-pack",
    "git-receive-pack",
    "git-credential-store",
    "git-credential-cache",
    "git-remote-https",
    "ssh",
];

/// A `PATH` of programs that aren't there: each records its name and arguments and fails like a missing command.
struct Tripwire {
    dir: TempDir,
}

impl Tripwire {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let script = dir.path().join("missing");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$(basename \"$0\") $*\" >> '{}'\necho \"$0: not found\" >&2\nexit 127\n",
                dir.path().join("calls").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        for program in PROGRAMS {
            std::os::unix::fs::symlink(&script, dir.path().join(program)).unwrap();
        }
        Self { dir }
    }

    fn control() -> bool {
        std::env::var("ACYCLIC_NO_GIT_CONTROL").is_ok_and(|value| value == "1")
    }

    fn path(&self) -> OsString {
        if Self::control() {
            std::env::var_os("PATH").unwrap_or_default()
        } else {
            self.dir.path().as_os_str().to_owned()
        }
    }

    /// `acyclic` in `dir`, in a clean environment whose `PATH` is the tripwire, with `home` as the user's home.
    fn acyclic(&self, dir: &Path, home: &Path) -> Command {
        let mut command = Command::new(BIN);
        command
            .env_clear()
            .env("PATH", self.path())
            .env("HOME", home)
            .env("ACYCLIC_NOW", NOW)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .current_dir(dir);
        command
    }

    fn run(&self, home: &Path, tracker: &Path, line: &str) -> Value {
        let output = self
            .acyclic(tracker, home)
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
            .filter(|call| !PROBES.contains(call))
            .map(str::to_owned)
            .collect()
    }

    fn assert_no_git_was_called(&self) {
        assert_eq!(
            self.calls(),
            Vec::<String>::new(),
            "the binary ran git or a program of its transports"
        );
    }
}

/// Real Git, as the oracle, with no configuration of its own.
fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_COMMITTER_NAME", "acyclic test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid");
    command
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_command(dir).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// How the repositories of a journey are made: Git's object format and its reference storage.
#[derive(Clone, Copy, Debug)]
struct Format {
    object_format: &'static str,
    ref_format: &'static str,
}

const FORMATS: [Format; 3] = [
    Format { object_format: "sha1", ref_format: "files" },
    Format { object_format: "sha256", ref_format: "files" },
    Format { object_format: "sha1", ref_format: "reftable" },
];

impl Format {
    fn init(self, dir: &Path, name: &str, bare: bool) -> PathBuf {
        let mut args = vec![
            "init".to_owned(),
            "--quiet".to_owned(),
            format!("--object-format={}", self.object_format),
            format!("--ref-format={}", self.ref_format),
        ];
        if bare {
            args.push("--bare".to_owned());
        }
        args.push(name.to_owned());
        git(dir, &args.iter().map(String::as_str).collect::<Vec<_>>());
        dir.join(name)
    }
}

/// Sets up the clone `name` in `dir` as a replica of the same name, through the binary.
fn init(tripwire: &Tripwire, home: &Path, dir: &Path, name: &str, remote: &str) -> PathBuf {
    let output = tripwire
        .acyclic(dir, home)
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

/// Everything the binary wrote has to be readable and consistent for Git.
fn assert_git_reads_it(repository: &Path) {
    let output = git_command(repository).args(["fsck", "--full", "--strict"]).output().unwrap();
    assert!(
        output.status.success(),
        "git fsck of {}: {}{}",
        repository.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Waits until no upkeep is running in `repository`, so nothing outlives the journey's checks.
fn wait_for_upkeep_to_end(repository: &Path) {
    let common = repository.join(".git");
    let deadline = Instant::now() + Duration::from_secs(30);
    while ["gc.pid", "objects/maintenance.lock"].iter().any(|file| common.join(file).exists()) {
        assert!(Instant::now() < deadline, "upkeep never ended in {}", repository.display());
        std::thread::sleep(Duration::from_millis(100));
    }
}

const A: &str = "01900000-0000-7000-8000-0000000000a1";
const B: &str = "01900000-0000-7000-8000-0000000000b1";

fn two_machines(format: Format) {
    let tripwire = Tripwire::new();
    let dir = TempDir::new().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let shared = format.init(dir.path(), "shared.git", true);
    let mac = format.init(dir.path(), "mac", false);
    let pi = format.init(dir.path(), "pi", false);
    for name in ["mac", "pi"] {
        init(&tripwire, &home, dir.path(), name, "../shared.git");
    }
    git(&mac, &["config", "acyclic.pushAfterWrite", "true"]);

    tripwire.run(&home, &mac, &format!("item add --id {A} --title A"));
    assert!(
        git(&shared, &["for-each-ref", "--format=%(refname)", "refs/heads/state/"])
            .contains("refs/heads/state/mac"),
        "a write that asks to push reached the shared repository ({format:?})"
    );
    tripwire.run(&home, &pi, &format!("item add --id {B} --title B"));
    tripwire.run(&home, &pi, "sync");
    tripwire.run(&home, &mac, "sync");
    tripwire.run(&home, &mac, &format!("item edit {B} --title B2"));
    tripwire.run(&home, &mac, "sync");
    tripwire.run(&home, &pi, "sync");
    let shown = tripwire.run(&home, &pi, &format!("item show {B}"));
    assert!(
        shown.to_string().contains("B2"),
        "an edit made on another machine reached this one: {shown}"
    );

    // What the CLI swallows if Git is missing: a marker that was never cleared.
    for (clone, name) in [(&mac, "mac"), (&pi, "pi")] {
        let marker = git_command(clone)
            .args(["config", "--get", "acyclic.replica-unused"])
            .output()
            .unwrap();
        assert_eq!(
            marker.status.code(),
            Some(1),
            "the first write cleared the unused marker in {}",
            clone.display()
        );
        // The state Git reads in the tree is the state the binary shows.
        let tree =
            git(clone, &["ls-tree", "-r", "--name-only", &format!("refs/heads/state/{name}")]);
        assert!(tree.lines().count() >= 2, "the state tree lists its files: {tree}");
    }
    for repository in [&mac, &pi, &shared] {
        assert_git_reads_it(repository);
    }
    tripwire.assert_no_git_was_called();
}

#[test]
fn two_machines_sync_through_a_shared_repository_without_git() {
    two_machines(FORMATS[0]);
}

#[test]
fn two_machines_sync_through_a_shared_repository_without_git_in_sha256() {
    two_machines(FORMATS[1]);
}

#[test]
fn two_machines_sync_through_a_shared_repository_without_git_in_reftable() {
    two_machines(FORMATS[2]);
}

#[test]
fn many_small_writes_stay_packed_and_complete_without_git() {
    let tripwire = Tripwire::new();
    let dir = TempDir::new().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    git(dir.path(), &["init", "--quiet", "--bare", "shared.git"]);
    let mac = init(&tripwire, &home, dir.path(), "mac", "../shared.git");
    git(&mac, &["config", "gc.autoPackLimit", "3"]);
    for n in 0..12 {
        tripwire.run(
            &home,
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
    wait_for_upkeep_to_end(&mac);
    let listed = tripwire.run(&home, &mac, "item list").to_string();
    for n in 0..12 {
        assert!(listed.contains(&format!("c{n}\"")), "repacking lost c{n}: {listed}");
    }
    assert_git_reads_it(&mac);
    tripwire.assert_no_git_was_called();
}

/// The user's Git configuration and hooks keep applying as they did with Git: a global `insteadOf` rule names the
/// shared repository, and a `reference-transaction` hook sees the state ref move, which is how a user watches or vetoes
/// the tracker's writes.
#[test]
fn the_users_git_configuration_and_hooks_apply_without_git() {
    let tripwire = Tripwire::new();
    let dir = TempDir::new().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let shared = dir.path().join("shared.git");
    git(dir.path(), &["init", "--quiet", "--bare", "shared.git"]);
    std::fs::write(
        home.join(".gitconfig"),
        format!("[url \"{}/\"]\n\tinsteadOf = shared:\n", dir.path().display()),
    )
    .unwrap();
    let mac = init(&tripwire, &home, dir.path(), "mac", "shared:shared.git");
    let log = dir.path().join("hook.log");
    let hook = mac.join(".git/hooks/reference-transaction");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nlines=$(cat | tr '\\n' '|')\necho \"$1 $lines\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(&mac, &["config", "acyclic.pushAfterWrite", "true"]);

    tripwire.run(&home, &mac, &format!("item add --id {A} --title A"));
    let seen = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        seen.lines()
            .any(|line| line.starts_with("committed ") && line.contains(" refs/heads/state/mac|")),
        "the hook saw the state ref move: {seen:?}"
    );
    assert!(
        git(&shared, &["for-each-ref", "--format=%(refname)", "refs/heads/state/"])
            .contains("refs/heads/state/mac"),
        "the push went to the repository that the global insteadOf names"
    );
    tripwire.assert_no_git_was_called();
}

/// No production code names `git` or starts a process, except by exact line: the stable error code that says Git
/// failed, which callers match on. The tests, which use Git as an oracle, and the maintainers' own scripts are exempt.
const ALLOWED: [&str; 3] =
    ["CliError::new(\"git\", exit::STORE,", "Self::new(\"git\", exit::STORE,", "\"git\","];

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
            } else if path.extension().is_some_and(|ext| ext == "rs") && name != "tests.rs" {
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    let line = line.trim();
                    let spawns = line.contains("Command::new(")
                        || line.contains("process::Command")
                        || line.contains("git_at(");
                    let names_git = line.contains("\"git\"")
                        && !ALLOWED.iter().any(|allowed| line.starts_with(allowed));
                    if spawns || names_git {
                        found.push(format!(
                            "{}:{}: {line}",
                            path.strip_prefix(&root).unwrap().display(),
                            n + 1
                        ));
                    }
                }
            }
        }
    }
    assert_eq!(found, Vec::<String>::new(), "production code still names git or starts a process");
}

/// The server's image has no Git: it is not needed, and a missing one is how the server journey proves it.
#[test]
fn the_image_installs_no_git() {
    let containerfile =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Containerfile"))
            .unwrap();
    let installs: Vec<&str> = containerfile
        .lines()
        .filter(|line| line.contains("apk add") && line.contains("git"))
        .collect();
    assert_eq!(installs, Vec::<&str>::new(), "the image still installs git");
}
