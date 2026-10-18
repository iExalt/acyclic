//! A run's push through the operator's git endpoint, end to end: the run's clone queues its writes and pushes them
//! over HTTP with its token, and the inbox's hook, the `acyclic` binary itself, checks the push and applies each
//! operation to the tracker once, as the run.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use acyclic_cli::Env;
use acyclic_core::model::RunState;
use acyclic_operator::endpoint::{self, Endpoint, Settings};
use acyclic_operator::forward::{self, Operator};
use acyclic_operator::token::Signer;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z.
const NOW: &str = "1792022400000";
const HOUR: i64 = 60 * 60 * 1000;
/// A synthetic secret the deny-list holds, standing in for a model key.
const SECRET: &str = "synthetic-inference-value-7f3a";

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
    String::from_utf8(output.stdout).unwrap()
}

/// `acyclic <line>` in `clone`, with its exit code, its JSON and its error output.
fn acyclic(dir: &Path, clone: &str, line: &str) -> (i32, Value, String) {
    let output = Command::new(BIN)
        .args(line.split(' '))
        .env("ACYCLIC_TRACKER", dir.join(clone))
        .env("ACYCLIC_NOW", NOW)
        .env("ACYCLIC_ACTOR", "session/run")
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(dir)
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_default();
    (output.status.code().unwrap(), json, String::from_utf8_lossy(&output.stderr).into_owned())
}

fn ok(dir: &Path, clone: &str, line: &str) -> Value {
    let (code, json, stderr) = acyclic(dir, clone, line);
    assert_eq!(code, 0, "{line}: {json} {stderr}");
    json
}

/// One HTTP/1.1 request; the status alone.
fn status(address: &str, request: &str) -> u16 {
    let mut stream = TcpStream::connect(address).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.split(' ').nth(1).and_then(|code| code.parse().ok()).unwrap_or(0)
}

/// The inbox's objects, loose and packed.
fn objects(inbox: &Path) -> String {
    git(inbox, &["count-objects", "-v"])
        .lines()
        .filter(|line| line.starts_with("count:") || line.starts_with("in-pack:"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_runs_push_reaches_the_tracker_once_through_the_operator() {
    let temp = tempfile::TempDir::new().unwrap();
    let dir = temp.path();
    let home = dir.join("home");
    let deny = home.join(".config/acyclic/deny-list");
    acyclic_core::private::dir(deny.parent().unwrap()).unwrap();
    acyclic_core::private::place(&deny, format!("{SECRET}\n").as_bytes(), true).unwrap();

    // The operator's tracker clone, shared with the run's through a bare repository.
    git(dir, &["init", "-q", "--bare", "shared.git"]);
    ok(dir, "tracker", "init tracker --replica op --remote ../shared.git");
    for line in ["thread add t0 --title Mine", "item add a --title A --thread t0"] {
        ok(dir, "tracker", &format!("{line} --as-user"));
    }
    let tracker = forward::Clone {
        tracker: dir.join("tracker"),
        env: Env {
            vars: [("ACYCLIC_NOW".to_owned(), NOW.to_owned())].into(),
            cwd: dir.to_path_buf(),
            terminal: None,
        },
    };
    let (signer, _) = Signer::open(&home).unwrap();
    let verifier = signer.verifier();
    let operator = Operator::new("mac", signer, tracker.clone());
    let now: i64 = NOW.parse().unwrap();
    let (run, token) = operator.start("t0", now, HOUR).unwrap();
    ok(dir, "tracker", "sync");

    // The endpoint, in this process; its hook is the binary.
    let operator_dir = home.join(".config/acyclic/operator");
    let endpoint = Endpoint::new(Settings {
        id: "mac".into(),
        verifier,
        tracker,
        dir: operator_dir.clone(),
        deny,
        home: home.clone(),
        executable: PathBuf::from(BIN),
    })
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, endpoint::router(Arc::new(endpoint))).await.unwrap();
        });
    });
    let url = format!("http://{address}/tracker.git");

    // The run's clone pulls from the shared repository and pushes its operations to the endpoint with its token.
    ok(dir, "run", "init run --replica rc --remote ../shared.git");
    let clone = dir.join("run");
    git(&clone, &["config", "acyclic.operations", &run]);
    git(&clone, &["config", "remote.origin.pushurl", &url]);
    git(
        &clone,
        &["config", &format!("http.{url}.extraHeader"), &format!("Authorization: Bearer {token}")],
    );
    ok(dir, "run", "sync");

    // Only pushes, and only with a valid token, on every request.
    let inboxes = operator_dir.join("inbox");
    let forged = Signer::from_seed([9; 32]).mint(&run, "t0", now, HOUR).unwrap();
    let get = |service: &str, token: &str| {
        format!(
            "GET /tracker.git/info/refs?service={service} HTTP/1.1\r\nHost: {address}\r\n\
             Authorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        )
    };
    let post = |token: &str| {
        format!(
            "POST /tracker.git/git-receive-pack HTTP/1.1\r\nHost: {address}\r\n\
             Authorization: Bearer {token}\r\nContent-Type: application/x-git-receive-pack-request\r\n\
             Content-Length: 4\r\nConnection: close\r\n\r\n0000"
        )
    };
    assert_eq!(status(&address, &get("git-receive-pack", &forged)), 401);
    assert_eq!(status(&address, &post(&forged)), 401);
    assert_eq!(status(&address, &get("git-upload-pack", &token)), 403);
    assert!(!inboxes.exists(), "a refused request starts no receive-pack");

    // The run's writes reach the tracker as the run.
    ok(dir, "run", "item edit a --title Better");
    ok(dir, "run", "item add n --title N --thread t0");
    let pushed = ok(dir, "run", "ops push");
    assert_eq!(pushed["data"]["settled"], 2, "{pushed}");
    assert_eq!(
        pushed["data"]["ops"],
        serde_json::json!([{"seq": 1, "ok": true}, {"seq": 2, "ok": true}])
    );
    let shown = ok(dir, "tracker", "show a");
    assert_eq!(shown["data"][0]["title"], "Better");

    // A push carrying the deny-listed value is refused before anything is forwarded, keeps none of its objects, and
    // never says the value back.
    let inbox = inboxes.join(format!("{run}.git"));
    let before = objects(&inbox);
    ok(dir, "run", &format!("item edit a --notes {SECRET}"));
    let (code, refused, stderr) = acyclic(dir, "run", "ops push");
    assert_eq!(
        (code, &refused["error"]["details"]["next"]),
        (1, &Value::from("reset")),
        "{refused}"
    );
    assert!(refused["error"]["message"].as_str().unwrap().contains("(deny-listed)"), "{refused}");
    assert!(!refused.to_string().contains(SECRET) && !stderr.contains(SECRET));
    assert_eq!(objects(&inbox), before, "the push stayed in quarantine");
    assert_eq!(ok(dir, "run", "ops reset")["data"]["dropped"], 1);

    // Sent again, operations already applied are found applied, not applied twice.
    git(&inbox, &["update-ref", "-d", &format!("refs/runs/{run}/ops")]);
    let resent = ok(dir, "run", "ops push");
    assert_eq!(
        resent["data"]["ops"],
        serde_json::json!([{"seq": 1, "ok": true}, {"seq": 2, "ok": true}])
    );

    // A run pushes its own ref alone.
    let tip = git(&clone, &["rev-parse", "ops"]);
    let other = Command::new("git")
        .arg("-C")
        .arg(&clone)
        .args(["push", "--porcelain", "origin", &format!("{}:refs/runs/r0/ops", tip.trim())])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&other.stderr).contains("acyclic-push refused wrong-ref"));

    // An ended run's token opens nothing.
    operator.end(&run, RunState::Done).unwrap();
    ok(dir, "run", "item edit a --title Late");
    let (code, late, _) = acyclic(dir, "run", "ops push");
    assert_eq!((code, &late["error"]["code"]), (1, &Value::from("push-refused")), "{late}");
    assert_eq!(ok(dir, "tracker", "show a")["data"][0]["title"], "Better");
}
