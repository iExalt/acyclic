//! Runs reaching their thread's code through `acyclic up --runners`, end to end with git itself: each run clones by
//! the forge's URL, which its environment sends to the server's mirror, sees and pushes only its own refs, and can't
//! reach another run's work or a repository outside its thread.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use acyclic_cli::Env;
use acyclic_operator::runs::{self, Clone, Operator};
use acyclic_operator::token::Signer;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z.
const NOW: &str = "1792022400000";
const HOUR: i64 = 60 * 60 * 1000;
/// A synthetic term the deny-list holds.
const TERM: &str = "synthetic-private-codename-4b1d";

/// Kills the server however the test ends.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Git in `dir` with no settings but `vars`.
fn git(dir: &Path, vars: &BTreeMap<String, String>, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_AUTHOR_NAME", "Run")
        .env("GIT_AUTHOR_EMAIL", "run@example.com")
        .env("GIT_COMMITTER_NAME", "Run")
        .env("GIT_COMMITTER_EMAIL", "run@example.com")
        .env("HOME", dir)
        .envs(vars)
        .output()
        .unwrap()
}

fn ok(output: &Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn refused(output: &Output, why: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success() && stderr.contains(why), "{stderr}");
}

/// Commits a file with `text` in the work tree at `dir`, and returns the commit.
fn commit(dir: &Path, vars: &BTreeMap<String, String>, file: &str, text: &str) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    ok(&git(dir, vars, &["add", file]));
    ok(&git(dir, vars, &["commit", "-q", "-m", file]));
    ok(&git(dir, vars, &["rev-parse", "HEAD"])).trim().to_owned()
}

/// `acyclic <line>` as the user on the tracker.
fn user(dir: &Path, line: &str) {
    let output = Command::new(BIN)
        .args(line.split(' '))
        .arg("--as-user")
        .current_dir(dir)
        .env("ACYCLIC_NOW", NOW)
        .env("ACYCLIC_TRACKER", dir.join("tracker"))
        .env("HOME", "/nonexistent")
        .env_remove("ACYCLIC_REMOTE")
        .output()
        .unwrap();
    assert!(output.status.success(), "{line}: {}", String::from_utf8_lossy(&output.stdout));
}

/// One push, written by hand: `update` with an empty pack, as a client that sends no objects would. The response, with
/// the hook's messages in it.
fn push_without_objects(runners: &str, token: &str, update: &str, pack: &[u8]) -> String {
    let command = format!("{update}\0report-status side-band-64k\n");
    let mut body = format!("{:04x}{command}0000", command.len() + 4).into_bytes();
    body.extend_from_slice(pack);
    let mut stream = TcpStream::connect(runners).unwrap();
    let head = format!(
        "POST /git/o1/app.git/git-receive-pack HTTP/1.1\r\nHost: {runners}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/x-git-receive-pack-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(&[head.as_bytes(), &body].concat()).unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    String::from_utf8_lossy(&response).into_owned()
}

#[test]
fn runs_reach_their_threads_code_through_the_mirrors_and_push_only_their_own_refs() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let none = BTreeMap::new();
    let home = dir.join("home");
    let deny = home.join(".config/acyclic/deny-list");
    acyclic_core::private::dir(deny.parent().unwrap()).unwrap();
    acyclic_core::private::place(&deny, format!("{TERM}\n").as_bytes(), true).unwrap();

    // The forge: o1/app and o1/lib, each with a commit on main.
    let upstream = dir.join("upstream");
    for name in ["app", "lib"] {
        let bare = upstream.join(format!("o1/{name}.git"));
        std::fs::create_dir_all(&bare).unwrap();
        ok(&git(&bare, &none, &["init", "-q", "--bare", "-b", "main"]));
    }
    let seed = dir.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    ok(&git(&seed, &none, &["init", "-q", "-b", "main"]));
    commit(&seed, &none, "README", "first\n");
    for name in ["app", "lib"] {
        let url = upstream.join(format!("o1/{name}.git"));
        ok(&git(&seed, &none, &["push", "-q", url.to_str().unwrap(), "main"]));
    }

    let output = Command::new(BIN)
        .args(["init", "tracker", "--replica", "mac"])
        .current_dir(dir)
        .env("ACYCLIC_NOW", NOW)
        .output()
        .unwrap();
    assert!(output.status.success());
    for line in [
        "repo add app --owner o1 --name app --default-branch main --public",
        "repo add lib --owner o1 --name lib --default-branch main --public",
        "thread add t0 --title Mine",
        "thread add t1 --title Also",
        "thread add t9 --title Other",
        "item add a --title A --thread t0 --repository app",
        "item add b --title B --thread t1 --repository app",
        "item add x --title X --thread t9 --repository lib",
    ] {
        user(dir, line);
    }
    let (signer, _) = Signer::open(&home).unwrap();
    let env = Env {
        vars: [("ACYCLIC_NOW".to_owned(), NOW.to_owned())].into(),
        cwd: dir.to_path_buf(),
        terminal: None,
    };
    let operator = Operator::new("mac", signer, Clone { tracker: dir.join("tracker"), env });
    let now: i64 = NOW.parse().unwrap();
    let (run_a, token_a) = operator.start("t0", now, HOUR).unwrap();
    let (run_b, token_b) = operator.start("t1", now, HOUR).unwrap();

    let mut child = Command::new(BIN)
        .args(["up", "--port", "0", "--runners", "0", "--sync-every", "0"])
        .current_dir(dir)
        .env("ACYCLIC_NOW", NOW)
        .env("ACYCLIC_TRACKER", dir.join("tracker"))
        .env("ACYCLIC_UPSTREAM", format!("file://{}", upstream.display()))
        .env("HOME", &home)
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    let runners = started["data"]["runners"].as_str().unwrap().to_owned();
    let server = format!("http://{runners}");
    let owners = ["o1".to_owned()].into();
    let vars_a = runs::environment(&server, &token_a, &owners);
    let vars_b = runs::environment(&server, &token_b, &owners);

    // A clones by the forge's URL, from the mirror, on its default branch, and pushes its own ref.
    ok(&git(dir, &vars_a, &["clone", "-q", "https://github.com/o1/app", "a"]));
    let a = dir.join("a");
    assert_eq!(ok(&git(&a, &none, &["branch", "--show-current"])).trim(), "main");
    let work = commit(&a, &none, "work", "a's work\n");
    ok(&git(&a, &vars_a, &["push", "-q", "origin", &format!("HEAD:refs/runs/{run_a}/work")]));
    // Not the branches, nor another run's refs.
    refused(&git(&a, &vars_a, &["push", "origin", "HEAD:refs/heads/main"]), "alone");
    let theirs = format!("HEAD:refs/runs/{run_b}/work");
    refused(&git(&a, &vars_a, &["push", "origin", &theirs]), "hidden ref");
    // Nor a term the deny-list holds, in a public repository.
    commit(&a, &none, "notes", &format!("{TERM}\n"));
    refused(
        &git(&a, &vars_a, &["push", "origin", &format!("HEAD:refs/runs/{run_a}/notes")]),
        "deny-list",
    );
    // Nor a repository outside its thread.
    refused(
        &git(dir, &vars_a, &["clone", "https://github.com/o1/lib", "lib"]),
        "not in this thread",
    );

    // B, in another thread holding app, doesn't see A's ref, can't fetch its commit by id, even over the second
    // protocol, and can't point a ref at it without sending it.
    ok(&git(dir, &vars_b, &["clone", "-q", "git@github.com:o1/app.git", "b"]));
    let b = dir.join("b");
    let listed = ok(&git(&b, &vars_b, &["ls-remote", "origin"]));
    assert!(listed.contains("refs/heads/main") && !listed.contains("refs/runs/"), "{listed}");
    let by_id = git(&b, &vars_b, &["-c", "protocol.version=2", "fetch", "origin", &work]);
    refused(&by_id, "unadvertised object");
    let empty = git(&b, &none, &["pack-objects", "--stdout"]).stdout;
    let zero = "0".repeat(40);
    let update = format!("{zero} {work} refs/runs/{run_b}/stolen");
    let response = push_without_objects(&runners, &token_b, &update, &empty);
    assert!(response.contains("can't see") && !response.contains("ok refs/"), "{response}");
    assert!(!ok(&git(&b, &vars_b, &["ls-remote", "origin"])).contains("stolen"));
    // Work of its own that happens to match A's is B's to push.
    commit(&b, &none, "work", "a's work\n");
    ok(&git(&b, &vars_b, &["push", "-q", "origin", &format!("HEAD:refs/runs/{run_b}/same")]));

    // The forge moves on; a fetch brings it, and A's ref is still there.
    let moved = commit(&seed, &none, "later", "later\n");
    let url = upstream.join("o1/app.git");
    ok(&git(&seed, &none, &["push", "-q", url.to_str().unwrap(), "main"]));
    ok(&git(&a, &vars_a, &["fetch", "-q", "origin"]));
    assert_eq!(ok(&git(&a, &none, &["rev-parse", "origin/main"])).trim(), moved);
    let listed = ok(&git(&a, &vars_a, &["ls-remote", "origin"]));
    assert!(listed.contains(&format!("{work}\trefs/runs/{run_a}/work")), "{listed}");
}
