//! A run working through `acyclic up --runners`, end to end: the operator starts it and gives it a token, and its CLI,
//! in remote mode, reads and writes the tracker through the runners' listener, as the run and within its thread.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use acyclic_cli::Env;
use acyclic_core::model::RunState;
use acyclic_core::state::Key;
use acyclic_core::store::Tracker;
use acyclic_operator::runs::{Clone, Operator};
use acyclic_operator::token::Signer;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z.
const NOW: &str = "1792022400000";
const HOUR: i64 = 60 * 60 * 1000;
/// A synthetic term the deny-list holds, standing in for a model key.
const SECRET: &str = "synthetic-inference-value-7f3a";

/// Kills the server however the test ends.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn base(command: &mut Command, dir: &Path) {
    command
        .env_remove("ACYCLIC_REMOTE")
        .env_remove("ACYCLIC_TOKEN")
        .env("ACYCLIC_NOW", NOW)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir);
}

/// `acyclic <line>` as the user on the tracker, with its exit code and JSON.
fn user(dir: &Path, line: &str) -> Value {
    let mut command = Command::new(BIN);
    base(&mut command, dir);
    let output = command
        .args(line.split(' '))
        .args(["--as-user"])
        .env("ACYCLIC_TRACKER", dir.join("tracker"))
        .env("HOME", "/nonexistent")
        .output()
        .unwrap();
    assert!(output.status.success(), "{line}: {}", String::from_utf8_lossy(&output.stdout));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// `acyclic <args>` in remote mode with `token`, with its exit code and JSON.
fn remote(dir: &Path, runners: &str, token: &str, args: &[&str]) -> (i32, Value) {
    let mut command = Command::new(BIN);
    base(&mut command, dir);
    let output = command
        .args(args)
        .env("ACYCLIC_REMOTE", format!("http://{runners}"))
        .env("ACYCLIC_TOKEN", token)
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("HOME", "/nonexistent")
        .env_remove("ACYCLIC_TRACKER")
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_default();
    (output.status.code().unwrap(), json)
}

/// One HTTP/1.1 request, written whole; the status and the JSON body.
fn request(address: &str, head: &str, body: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).unwrap();
    let request = format!(
        "{head}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(request.as_bytes());
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response[9..12].parse().unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    (status, serde_json::from_str(body).unwrap_or_default())
}

/// Who wrote each of an item's titles.
fn titles(dir: &Path, item: &str) -> Vec<(String, Value)> {
    let state = Tracker::open(&dir.join("tracker")).unwrap().0.load().unwrap().1;
    let key = Key::new("item", item).unwrap();
    state
        .field(&key, "title")
        .into_iter()
        .flatten()
        .map(|(_, v)| (v.actor.clone(), v.value.clone()))
        .collect()
}

#[test]
fn a_run_reads_and_writes_the_tracker_through_the_runners_listener_as_itself() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let home = dir.join("home");
    let deny = home.join(".config/acyclic/deny-list");
    acyclic_core::private::dir(deny.parent().unwrap()).unwrap();
    acyclic_core::private::place(&deny, format!("{SECRET}\n").as_bytes(), true).unwrap();
    let mut init = Command::new(BIN);
    base(&mut init, dir);
    assert!(init.args(["init", "tracker", "--replica", "mac"]).output().unwrap().status.success());
    for line in [
        "thread add t0 --title Mine",
        "thread add t9 --title Other",
        "item add a --title A --thread t0",
        "item add x --title X --thread t9",
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
    let (run, token) = operator.start("t0", now, HOUR).unwrap();

    let mut up = Command::new(BIN);
    base(&mut up, dir);
    let mut child = up
        .args(["up", "--port", "0", "--runners", "0"])
        .env("ACYCLIC_TRACKER", dir.join("tracker"))
        .env("HOME", &home)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    let runners = started["data"]["runners"].as_str().unwrap().to_owned();
    let listening = started["data"]["listening"].as_str().unwrap().to_owned();
    let session = std::fs::read_to_string(started["data"]["session"].as_str().unwrap()).unwrap();
    let port = runners.rsplit(':').next().unwrap().to_owned();

    // A read and a write, as the run.
    let (code, shown) = remote(dir, &runners, &token, &["show", "a"]);
    assert_eq!((code, &shown["data"][0]["title"]), (0, &json!("A")), "{shown}");
    let (code, edited) = remote(dir, &runners, &token, &["item", "edit", "a", "--title", "Better"]);
    assert_eq!(code, 0, "{edited}");
    assert_eq!(titles(dir, "a"), [(format!("run/{run}"), json!("Better"))]);

    // Refused at once: outside its thread, or holding the deny-listed term, plain or escaped.
    let (code, outside) = remote(dir, &runners, &token, &["item", "edit", "x", "--title", "Mine"]);
    assert_eq!((code, &outside["error"]["code"]), (1, &json!("not-in-thread")), "{outside}");
    let (code, denied) = remote(dir, &runners, &token, &["item", "edit", "a", "--notes", SECRET]);
    assert_eq!((code, &denied["error"]["code"]), (1, &json!("deny-listed")), "{denied}");
    assert!(!denied.to_string().contains(SECRET));
    let escaped = format!("\\u{:04x}{}", u32::from(SECRET.as_bytes()[0]), &SECRET[1..]);
    let body = format!(r#"{{"item": {{"edit": {{"item": "a", "notes": "{escaped}"}}}}}}"#);
    let bearer = format!("Authorization: Bearer {token}");
    let post = |seq: u64, host: &str| {
        format!(
            "POST /api/command HTTP/1.1\r\nHost: {host}\r\n{bearer}\r\nAcyclic-Operation: {seq}"
        )
    };
    let applied = || remote_applied(&runners, &bearer);
    assert_eq!(
        request(&runners, &post(applied() + 1, &runners), &body).1["error"]["code"],
        "deny-listed"
    );

    // An answer lost on the way: the next command follows it, and the lost one sent again is found applied.
    let lost = applied() + 1;
    let edit = r#"{"item": {"edit": {"item": "a", "title": "Lost"}}}"#;
    let host = format!("host.containers.internal:{port}");
    assert_eq!(request(&runners, &post(lost, &host), edit).0, 200, "any name reaches the runners");
    let (code, next) = remote(dir, &runners, &token, &["item", "edit", "a", "--title", "Next"]);
    assert_eq!(code, 0, "{next}");
    assert_eq!(applied(), lost + 1);
    let (status, again) = request(&runners, &post(lost, &runners), edit);
    assert_eq!((status, &again["data"]["duplicate"]), (200, &json!(true)), "{again}");
    // Another command with that number was sent by someone else: it is told so, not that it applied.
    let other = r#"{"item": {"edit": {"item": "a", "title": "Other"}}}"#;
    let (status, taken) = request(&runners, &post(lost + 1, &runners), other);
    assert_eq!((status, &taken["error"]["code"]), (409, &json!("seq-taken")), "{taken}");
    assert_eq!(titles(dir, "a"), [(format!("run/{run}"), json!("Next"))]);

    // The run supersedes only what it read: the user's edit since its last read stays beside the run's write.
    user(dir, "item edit a --title Theirs");
    remote(dir, &runners, &token, &["item", "edit", "a", "--title", "Unread"]);
    assert_eq!(titles(dir, "a").len(), 2, "{:?}", titles(dir, "a"));
    assert_eq!(remote(dir, &runners, &token, &["show", "a"]).0, 0);
    remote(dir, &runners, &token, &["item", "edit", "a", "--title", "Read"]);
    assert_eq!(titles(dir, "a"), [(format!("run/{run}"), json!("Read"))]);

    // Each credential counts on its own listener alone, and a forged token nowhere.
    let forged = Signer::from_seed([9; 32]).mint(&run, "t0", now, HOUR).unwrap();
    let (code, refused) = remote(dir, &runners, &forged, &["show", "a"]);
    assert_eq!((code, &refused["error"]["code"]), (1, &json!("no-token")), "{refused}");
    let read = r#"{"show": {"items": ["a"]}}"#;
    let on_user = format!("POST /api/command HTTP/1.1\r\nHost: {listening}\r\n{bearer}");
    assert_eq!(request(&listening, &on_user, read).0, 401);
    let session = format!(
        "POST /api/command HTTP/1.1\r\nHost: {runners}\r\nAuthorization: Bearer {}",
        session.trim()
    );
    assert_eq!(request(&runners, &session, read).0, 401);

    // An ended run's token opens nothing.
    operator.end(&run, RunState::Done).unwrap();
    let (code, ended) = remote(dir, &runners, &token, &["show", "a"]);
    assert_eq!((code, &ended["error"]["code"]), (1, &json!("inactive")), "{ended}");
}

/// The run's latest applied operation, as the server says.
fn remote_applied(runners: &str, bearer: &str) -> u64 {
    let mut stream = TcpStream::connect(runners).unwrap();
    let request = format!(
        "GET /api/run HTTP/1.1\r\nHost: {runners}\r\n{bearer}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    body["data"]["applied"].as_u64().unwrap()
}
