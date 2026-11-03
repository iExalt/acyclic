//! `acyclic serve` and `acyclic up` as processes: each says where it listens and answers a read as the CLI does;
//! `serve` refuses a request without the user's identity, writes as the user and syncs with the other replicas, and
//! `up` lets in anything on the machine but a page from another origin.

use std::io::{BufRead as _, BufReader};
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
    exchange(TcpStream::connect(address).unwrap(), "/api/command", headers, body)
}

/// Sends one HTTP/1.1 request through a Unix socket, as `tailscale serve` does.
fn request_unix(socket: &Path, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    exchange(
        std::os::unix::net::UnixStream::connect(socket).unwrap(),
        "/api/command",
        headers,
        body,
    )
}

/// Sends a command to `path` and returns the status and the body, null if empty.
fn exchange(
    mut stream: impl std::io::Read + std::io::Write,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Value) {
    let mut head =
        format!("POST {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    // In one write: a server that refuses on the headers alone may close before reading the body, and a body
    // written after that fails while the refusal waits to be read.
    let _ = stream.write_all(format!("{head}\r\n{body}").as_bytes());
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response[9..12].parse().unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    (status, if body.is_empty() { Value::Null } else { serde_json::from_str(body).unwrap() })
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
/// and the line it printed.
fn up(tracker: &Path, home: &Path, extra: &[&str]) -> (Server, String, Value) {
    let mut child = acyclic(tracker)
        .env("HOME", home)
        .args(["up", "--port", "0"])
        .args(extra)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    let address = started["data"]["listening"].as_str().unwrap().to_owned();
    (server, address, started)
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
    cli(&tracker, "item add --id 01900000-0000-7000-8000-0000000000a1 --title A");
    let (_server, address) = serve(&tracker, &[]);

    let anonymous = [("Host", HOST), ("Content-Type", "application/json")];
    let (status, refused) = request(&address, &anonymous, r#"{"item": "list"}"#);
    assert_eq!((status, &refused["error"]["code"]), (403, &Value::from("no-identity")));

    let (status, shown) = command(
        &address,
        r#"{"item": {"show": {"items": ["01900000-0000-7000-8000-0000000000a1"]}}}"#,
    );
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "item show A"));
}

#[test]
fn up_serves_anything_on_the_machine_but_other_origins() {
    let dir = tempfile::tempdir().unwrap();
    let tracker = init(dir.path(), "mac", None);
    cli(&tracker, "item add --id 01900000-0000-7000-8000-0000000000a1 --title A");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let (server, address, _) = up(&tracker, &home, &[]);
    let show = r#"{"item": {"show": {"items": ["01900000-0000-7000-8000-0000000000a1"]}}}"#;
    let json = ("Content-Type", "application/json");

    let (status, shown) = request(&address, &[("Host", &address), json], show);
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "item show A"));

    // The app's own page writes; a page on another port of the machine doesn't.
    let add = r#"{"item": {"add": {"id": "01900000-0000-7000-8000-0000000000b1", "title": "B"}}}"#;
    let own = format!("http://{address}");
    let (status, added) = request(&address, &[("Host", &address), ("Origin", &own), json], add);
    assert_eq!(status, 200, "{added}");
    let other = [("Host", address.as_str()), ("Origin", "http://localhost:3000"), json];
    let (status, refused) = request(&address, &other, add);
    assert_eq!((status, &refused["error"]["code"]), (403, &Value::from("wrong-origin")));

    // A second server can't take the same port, and says so.
    let port = address.rsplit_once(':').unwrap().1;
    let taken = acyclic(&tracker).env("HOME", &home).args(["up", "--port", port]).output().unwrap();
    let refused: Value = serde_json::from_slice(&taken.stdout).unwrap();
    assert_eq!(refused["error"]["code"], "serve", "{refused}");

    drop(server);
    // The socket's path is short, as a Unix socket's must be.
    let short = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let socket = short.path().join("ts.sock");
    let socket_arg = socket.to_str().unwrap();
    let tailscale = ["--tailscale", "--login", LOGIN, "--host", HOST, "--socket", socket_arg];
    let (server, _, started) = up(&tracker, &home, &tailscale);
    assert_eq!(
        started["data"]["tailscale"]["serve"],
        format!("tailscale serve --bg unix:{socket_arg}")
    );

    // Through Tailscale, its identity is what counts.
    let identity = [("Host", HOST), ("Tailscale-User-Login", LOGIN), json];
    let (status, shown) = request_unix(&socket, &identity, show);
    assert_eq!(status, 200, "{shown}");
    let (status, refused) = request_unix(&socket, &[("Host", HOST), json], show);
    assert_eq!((status, &refused["error"]["code"]), (403, &Value::from("no-identity")));

    // Without --socket, Tailscale reaches a port, and `tailscale serve` is pointed at its root.
    drop(server);
    let tailscale = ["--tailscale", "--login", LOGIN, "--host", HOST, "--tailscale-port", "0"];
    let (_server, _, started) = up(&tracker, &home, &tailscale);
    let serve = started["data"]["tailscale"]["serve"].as_str().unwrap();
    let port = serve.strip_prefix("tailscale serve --bg http://").unwrap();
    assert_eq!(port, started["data"]["tailscale"]["listening"]);
    let (status, shown) =
        exchange(TcpStream::connect(port).unwrap(), "/api/command", &identity, show);
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "item show A"));
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
    let (status, added) = command(
        &address,
        r#"{"item": {"add": {"id": "01900000-0000-7000-8000-0000000000a1", "title": "A"}}}"#,
    );
    assert_eq!(status, 200, "{added}");
    let shared = dir.path().join("shared.git");
    let logs = git(&shared, &["for-each-ref", "--format=%(refname)", "refs/heads/log/server/"]);
    let log = logs.lines().next().expect("the server's log was pushed");
    let events = git(&shared, &["log", "-p", "--format=", log]);
    let event = events
        .lines()
        .find(|line| line.contains(r#""id":"01900000-0000-7000-8000-0000000000a1""#))
        .expect("the add is logged");
    assert!(event.contains(r#""actor":"actor/user""#), "{event}");

    // Another replica's write reaches the server's API through its sync loop.
    cli(&phone, "item add --id 01900000-0000-7000-8000-0000000000b1 --title B");
    cli(&phone, "sync");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, shown) = command(
            &address,
            r#"{"item": {"show": {"items": ["01900000-0000-7000-8000-0000000000b1"]}}}"#,
        );
        if status == 200 {
            break;
        }
        assert!(Instant::now() < deadline, "the server never synced b: {shown}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
