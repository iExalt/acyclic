//! `acyclic serve` and `acyclic up` as processes: each says where it listens, refuses a request without the user's
//! identity or session, and answers a read as the CLI does; `serve` writes as the user and syncs with the other
//! replicas, and `up` keeps its session across restarts.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_acyclic");
/// 2026-10-15T00:00:00Z, for the server and the CLI alike.
const NOW: &str = "1792022400000";
const LOGIN: &str = "user@example.com";
const HOST: &str = "acyclic.example.ts.net";

fn acyclic(tracker: &Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .env("ACYCLIC_TRACKER", tracker)
        .env("ACYCLIC_NOW", NOW)
        .env("HOME", "/nonexistent")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(tracker);
    command
}

fn cli(tracker: &Path, line: &str) -> Value {
    let output = acyclic(tracker).args(line.split(' ')).output().unwrap();
    assert!(output.status.success(), "{line}: {}", String::from_utf8_lossy(&output.stdout));
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Kills the server however the test ends.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Sends one HTTP/1.1 request and returns the status and the body.
fn request(address: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).unwrap();
    let mut head = format!(
        "POST /api/command HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    write!(stream, "{head}\r\n{body}").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    (status, serde_json::from_str(body).unwrap())
}

/// Starts the server on `tracker` and returns it with the address it listens on.
fn serve(tracker: &Path, extra: &[&str]) -> (Server, String) {
    let mut child = acyclic(tracker)
        .args(["serve", "--listen", "127.0.0.1:0", "--login", LOGIN, "--host", HOST])
        .args(extra)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    (server, started["data"]["listening"].as_str().unwrap().to_owned())
}

/// Starts `acyclic up` on `tracker` with `home` as the home directory, and returns it with the address it listens on
/// and the session's file.
fn up(tracker: &Path, home: &Path) -> (Server, String, PathBuf) {
    let mut child = acyclic(tracker)
        .env("HOME", home)
        .args(["up", "--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    let address = started["data"]["listening"].as_str().unwrap().to_owned();
    (server, address, PathBuf::from(started["data"]["session"].as_str().unwrap()))
}

/// Sends a command as the user.
fn command(address: &str, body: &str) -> (u16, Value) {
    let headers =
        [("Host", HOST), ("Tailscale-User-Login", LOGIN), ("Content-Type", "application/json")];
    request(address, &headers, body)
}

/// Sets up the clone `name` in `dir` as a replica of the same name.
fn init(dir: &Path, name: &str, remote: Option<&str>) -> PathBuf {
    let mut line = vec!["init", name, "--replica", name];
    line.extend(remote.iter().flat_map(|remote| ["--remote", remote]));
    let output = acyclic(dir).args(line).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    dir.join(name)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_server_refuses_strangers_and_reads_as_the_cli_does() {
    let dir = tempfile::tempdir().unwrap();
    let tracker = init(dir.path(), "mac", None);
    cli(&tracker, "item add a --title A --as-user");
    let (_server, address) = serve(&tracker, &[]);

    let anonymous = [("Host", HOST), ("Content-Type", "application/json")];
    let (status, refused) = request(&address, &anonymous, r#""list""#);
    assert_eq!((status, &refused["error"]["code"]), (403, &Value::from("no-identity")));

    let (status, shown) = command(&address, r#"{"show": {"items": ["a"]}}"#);
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "show a"));
}

#[test]
fn up_serves_whoever_holds_the_session_and_keeps_it_across_restarts() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let tracker = init(dir.path(), "mac", None);
    cli(&tracker, "item add a --title A --as-user");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let (server, address, session) = up(&tracker, &home);
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode(session.parent().unwrap()), mode(&session)), (0o700, 0o600));
    let token = std::fs::read_to_string(&session).unwrap();
    let bearer = format!("Bearer {}", token.trim_end());
    let show = r#"{"show": {"items": ["a"]}}"#;
    let json = ("Content-Type", "application/json");

    let (status, refused) = request(&address, &[("Host", &address), json], show);
    assert_eq!((status, &refused["error"]["code"]), (401, &Value::from("no-session")));
    let (status, shown) =
        request(&address, &[("Host", &address), ("Authorization", &bearer), json], show);
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "show a"));

    // A second server can't take the same port, and says so.
    let port = address.rsplit_once(':').unwrap().1;
    let taken = acyclic(&tracker).env("HOME", &home).args(["up", "--port", port]).output().unwrap();
    let refused: Value = serde_json::from_slice(&taken.stdout).unwrap();
    assert_eq!(refused["error"]["code"], "serve", "{refused}");

    // A restart keeps the session, so whatever held it still gets in.
    drop(server);
    let (_server, address, _) = up(&tracker, &home);
    assert_eq!(std::fs::read_to_string(&session).unwrap(), token);
    let (status, shown) =
        request(&address, &[("Host", &address), ("Authorization", &bearer), json], show);
    assert_eq!(status, 200, "{shown}");
}

#[test]
fn the_server_writes_as_the_user_and_syncs_with_other_replicas() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--quiet", "--bare", "shared.git"]);
    let server = init(dir.path(), "server", Some("../shared.git"));
    let phone = init(dir.path(), "phone", Some("../shared.git"));
    git(&server, &["config", "acyclic.pushAfterWrite", "true"]);
    let (_server, address) = serve(&server, &["--sync-every", "1"]);

    // A write through the server is the user's, and reaches the shared repository at once.
    let (status, added) = command(&address, r#"{"item": {"add": {"item": "a", "title": "A"}}}"#);
    assert_eq!(status, 200, "{added}");
    let shared = dir.path().join("shared.git");
    let logs = git(&shared, &["for-each-ref", "--format=%(refname)", "refs/heads/log/server/"]);
    let log = logs.lines().next().expect("the server's log was pushed");
    let events = git(&shared, &["log", "-p", "--format=", log]);
    let event =
        events.lines().find(|line| line.contains(r#""id":"a""#)).expect("the add is logged");
    assert!(event.contains(r#""actor":"actor/user""#), "{event}");

    // Another replica's write reaches the server's API through its sync loop.
    cli(&phone, "item add b --title B --as-user");
    cli(&phone, "sync");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, shown) = command(&address, r#"{"show": {"items": ["b"]}}"#);
        if status == 200 {
            break;
        }
        assert!(Instant::now() < deadline, "the server never synced b: {shown}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
