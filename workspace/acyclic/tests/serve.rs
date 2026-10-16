//! `acyclic serve` as a process: it says where it listens, refuses a request without the user's identity, and answers
//! a read as the CLI does.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

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

#[test]
fn the_server_refuses_strangers_and_reads_as_the_cli_does() {
    let dir = tempfile::tempdir().unwrap();
    let tracker = dir.path().join("tracker");
    let output =
        acyclic(dir.path()).args(["init", "tracker", "--replica", "mac"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    cli(&tracker, "item add a --title A --as-user");

    let mut child = acyclic(&tracker)
        .args(["serve", "--listen", "127.0.0.1:0", "--login", LOGIN, "--host", HOST])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _server = Server(child);
    let started: Value = serde_json::from_str(&line).unwrap();
    let address = started["data"]["listening"].as_str().unwrap();

    let json = ("Content-Type", "application/json");
    let (status, refused) = request(address, &[("Host", HOST), json], r#""list""#);
    assert_eq!((status, &refused["error"]["code"]), (403, &Value::from("no-identity")));

    let (status, shown) = request(
        address,
        &[("Host", HOST), ("Tailscale-User-Login", LOGIN), json],
        r#"{"show": {"items": ["a"]}}"#,
    );
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown, cli(&tracker, "show a"));
}
